// src/load_handlers.rs
//! Handlers for async volume and labelmap loading results.
//!
//! This module extracts the load handling logic from lib.rs to improve code organization.

use crate::components::*;
use crate::convert::extract_axis_aligned_contours_from_labelmap;
use crate::nifti_loader::LoadedVolume;
use crate::systems::SegmentManager;
use crate::volume;
use hecs::World;
use std::collections::VecDeque;

fn idx_3d(dims: [u32; 3], x: u32, y: u32, z: u32) -> usize {
    (x + y * dims[0] + z * dims[0] * dims[1]) as usize
}

fn is_fg(mask: &[u8], dims: [u32; 3], x: i32, y: i32, z: u32) -> bool {
    if x < 0 || y < 0 || x >= dims[0] as i32 || y >= dims[1] as i32 {
        return false;
    }
    mask[idx_3d(dims, x as u32, y as u32, z)] != 0
}

fn count_axial_components(mask: &[u8], dims: [u32; 3], z: u32) -> usize {
    let w = dims[0] as i32;
    let h = dims[1] as i32;
    if w <= 0 || h <= 0 {
        return 0;
    }

    let mut visited = vec![false; (dims[0] * dims[1]) as usize];
    let mut components = 0usize;
    let mut q = VecDeque::new();
    let neighbors = [(-1, 0), (1, 0), (0, -1), (0, 1)];

    for y in 0..h {
        for x in 0..w {
            let flat = (x as u32 + y as u32 * dims[0]) as usize;
            if visited[flat] || !is_fg(mask, dims, x, y, z) {
                continue;
            }

            components += 1;
            visited[flat] = true;
            q.push_back((x, y));

            while let Some((cx, cy)) = q.pop_front() {
                for (dx, dy) in neighbors {
                    let nx = cx + dx;
                    let ny = cy + dy;
                    if nx < 0 || ny < 0 || nx >= w || ny >= h {
                        continue;
                    }
                    let nflat = (nx as u32 + ny as u32 * dims[0]) as usize;
                    if visited[nflat] || !is_fg(mask, dims, nx, ny, z) {
                        continue;
                    }
                    visited[nflat] = true;
                    q.push_back((nx, ny));
                }
            }
        }
    }

    components
}

fn slice_has_foreground(mask: &[u8], dims: [u32; 3], z: u32) -> bool {
    let start = (z * dims[0] * dims[1]) as usize;
    let end = start + (dims[0] * dims[1]) as usize;
    mask[start..end].iter().any(|&v| v != 0)
}

fn signed_area_xy(points: &[[f32; 3]]) -> f32 {
    if points.len() < 3 {
        return 0.0;
    }
    let mut area2 = 0.0f32;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        area2 += a[0] * b[1] - b[0] * a[1];
    }
    0.5 * area2
}

/// Handle a successfully loaded volume, updating ECS components and GPU resources.
///
/// Returns the dimensions of the loaded volume for status message construction.
pub fn handle_volume_load(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &mut World,
    _entities: &AppEntities,
    loaded: &LoadedVolume,
) -> [u32; 3] {
    log::info!("Volume loaded: {:?} dimensions", loaded.dimensions);

    let (new_texture, new_view, new_sampler, volume_data) =
        volume::create_texture_from_nifti(device, queue, loaded);

    // Update ONLY the main volume components in ECS
    if let Some((_, (vol, gpu_res))) = world
        .query_mut::<(&mut VolumeData, &mut GpuVolumeResources)>()
        .with::<&MainVolumeTag>()
        .into_iter()
        .next()
    {
        vol.dimensions = volume_data.dimensions;
        vol.spacing = volume_data.spacing;
        vol.intensities = volume_data.intensities.clone();
        vol.intensity_range = volume_data.intensity_range;
        vol.orientation = volume_data.orientation;

        gpu_res.texture = new_texture;
        gpu_res.view = new_view;
        gpu_res.sampler = new_sampler;
    }

    // Reset user rotation when loading new volume
    for (_, (vp, vs)) in world.query_mut::<(&Viewport, &mut ViewportState)>() {
        if vp.mode == ViewMode::ThreeD {
            vs.user_rotation = [0.0, 0.0, 0.0, 1.0]; // Identity
        }
    }

    loaded.dimensions
}

/// Handle a successfully loaded labelmap by spawning a new layer entity.
///
/// Returns the new entity ID and dimensions of the loaded labelmap.
pub fn handle_label_load(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &mut World,
    loaded_label: &LoadedLabel,
) -> (hecs::Entity, [u32; 3]) {
    log::info!("Labelmap loaded: {:?} dimensions", loaded_label.dimensions);

    let (new_texture, new_view, new_sampler) =
        volume::create_texture_from_labelmap(device, queue, loaded_label);

    // Fetch an existing bind group as placeholder (will be fixed by recreate_bind_groups)
    let placeholder_bg = world
        .query::<&GpuVolumeResources>()
        .iter()
        .next()
        .map(|(_, res)| res.bind_group.clone())
        .expect("Main volume should exist");

    // Spawn a new layer entity with CPU data for painting support
    let entity = world.spawn((
        Segmentation {
            name: loaded_label.filename.clone(),
            // Show imported labelmap immediately for contour-vs-label inspection.
            is_visible: true,
        },
        LayerSettings { opacity: 0.5 },
        LabelmapData {
            dimensions: loaded_label.dimensions,
            raw_data: loaded_label.data.clone(),
        },
        Representation::Voxel(GpuVolumeResources {
            texture: new_texture,
            view: new_view,
            sampler: new_sampler,
            bind_group: placeholder_bg,
        }),
        SegmentationTag,
    ));

    (entity, loaded_label.dimensions)
}

/// Import loaded labelmap as editable contours into a new contour segment.
pub fn import_labelmap_as_contours(
    world: &mut World,
    entities: &AppEntities,
    loaded_label: &LoadedLabel,
) -> Option<usize> {
    let spacing = world
        .query::<&VolumeData>()
        .with::<&MainVolumeTag>()
        .iter()
        .next()
        .map(|(_, v)| v.spacing)
        .unwrap_or([1.0, 1.0, 1.0]);

    let contour_set = extract_axis_aligned_contours_from_labelmap(
        &loaded_label.data,
        loaded_label.dimensions,
        spacing,
        None,
    );
    if contour_set.is_empty() {
        return None;
    }

    // Import QA: compare axial connected components in labelmap vs extracted contour loops.
    let mut mismatched = 0usize;
    let mut roi_slices = 0usize;
    let mut total_components = 0usize;
    let mut total_loops = 0usize;
    let mut mismatch_rows: Vec<String> = Vec::new();
    for z in 0..loaded_label.dimensions[2] {
        if !slice_has_foreground(&loaded_label.data, loaded_label.dimensions, z) {
            continue;
        }
        roi_slices += 1;
        let components = count_axial_components(&loaded_label.data, loaded_label.dimensions, z);
        let slice_contours = contour_set.axial.get(&(z as i32));
        let loops = slice_contours.map(|v| v.len()).unwrap_or(0);
        total_components += components;
        total_loops += loops;
        if components != loops {
            mismatched += 1;
            if mismatch_rows.len() < 12 {
                let mut pos_oriented = 0usize;
                let mut neg_oriented = 0usize;
                if let Some(contours) = slice_contours {
                    for c in contours {
                        if signed_area_xy(&c.points) >= 0.0 {
                            pos_oriented += 1;
                        } else {
                            neg_oriented += 1;
                        }
                    }
                }
                mismatch_rows.push(format!(
                    "z={z}: components={components}, loops={loops}, loop_orientation(+/-)={pos_oriented}/{neg_oriented}"
                ));
            }
        }
    }
    if mismatched > 0 {
        log::warn!(
            "Labelmap->contour import mismatch on {}/{} ROI axial slices (components={}, contours={})",
            mismatched,
            roi_slices,
            total_components,
            total_loops
        );
        if !mismatch_rows.is_empty() {
            log::warn!(
                "Labelmap->contour mismatch details (first {}): {}",
                mismatch_rows.len(),
                mismatch_rows.join(" | ")
            );
        }
    } else {
        log::info!(
            "Labelmap->contour import parity OK across {} ROI axial slices (components={}, contours={})",
            roi_slices,
            total_components,
            total_loops
        );
    }

    if let Ok(mut mgr) = world.get::<&mut SegmentManager>(entities.segments) {
        let idx = mgr.len();
        let colors = [
            [1.0, 0.3, 0.3, 0.8],
            [0.3, 1.0, 0.3, 0.8],
            [0.3, 0.3, 1.0, 0.8],
            [1.0, 1.0, 0.3, 0.8],
            [1.0, 0.3, 1.0, 0.8],
            [0.3, 1.0, 1.0, 0.8],
        ];
        let color = colors[idx % colors.len()];
        let seg_idx = mgr.add_segment(&format!("{} (Contours)", loaded_label.filename), color);
        if let Some(seg) = mgr.segments.get_mut(seg_idx) {
            seg.contours = contour_set;
            seg.sync_edited_slices_from_contours();
            seg.mark_dirty();
        }
        Some(seg_idx)
    } else {
        None
    }
}

/// Recreate the scene bind group with current volume and overlay textures.
///
/// This should be called after loading a new volume or labelmap to ensure
/// all texture bindings are up to date.
pub fn recreate_bind_groups(
    device: &wgpu::Device,
    world: &mut World,
    texture_bind_group_layout: &wgpu::BindGroupLayout,
    uniform_buffer: &wgpu::Buffer,
    dummy_view: &wgpu::TextureView,
    dummy_sampler: &wgpu::Sampler,
    default_lut_view: &wgpu::TextureView,
    overlay_buffer: &wgpu::Buffer,
    active_layer: Option<hecs::Entity>,
) {
    // Collect the texture views we need (must satisfy borrow checker)
    let main_view: Option<wgpu::TextureView>;
    let mut overlay_views = Vec::new();

    // Query main volume view
    {
        let query = world.query::<&GpuVolumeResources>();
        let mut with_tag = query.with::<&MainVolumeTag>();
        main_view = with_tag.iter().next().map(|(_, res)| res.view.clone());
    }

    // Query overlay views
    {
        // 1. Prioritize active layer if it exists and is a voxel representation
        if let Some(active) = active_layer {
            if let (Ok(seg), Ok(repr)) = (
                world.get::<&Segmentation>(active),
                world.get::<&Representation>(active),
            ) {
                if seg.is_visible {
                    let Representation::Voxel(res) = &*repr;
                    overlay_views.push(res.view.clone());
                }
            }
        }

        // 2. Add other visible layers that aren't the active one
        let mut query = world.query::<(&Segmentation, &Representation)>();
        for (e, (seg, repr)) in query.iter() {
            if Some(e) == active_layer {
                continue;
            }
            if !seg.is_visible {
                continue;
            }
            let Representation::Voxel(res) = repr;
            overlay_views.push(res.view.clone());
        }
    }

    // Use actual views or fallback to dummy
    let main_view_ref = main_view.as_ref().unwrap_or(dummy_view);

    let overlay1_view = overlay_views.first().unwrap_or(dummy_view);
    let overlay2_view = overlay_views.get(1).unwrap_or(dummy_view);

    let new_bind_group = crate::render::pipeline::create_scene_bind_group(
        device,
        texture_bind_group_layout,
        main_view_ref,
        dummy_sampler,
        uniform_buffer,
        overlay1_view,
        default_lut_view,
        overlay2_view,
        default_lut_view,
        overlay_buffer,
    );

    // Update bind group in ALL relevant entities
    for (_, res) in world.query_mut::<&mut GpuVolumeResources>() {
        res.bind_group = new_bind_group.clone();
    }
    for (_, repr) in world.query_mut::<&mut Representation>() {
        let Representation::Voxel(res) = repr;
        res.bind_group = new_bind_group.clone();
    }
}

/// Update GUI status message
pub fn set_status_message(world: &mut World, entities: &AppEntities, message: String) {
    if let Ok(mut gui_state) = world.get::<&mut GuiState>(entities.gui_state) {
        gui_state.status_message = Some(message);
    }
}
