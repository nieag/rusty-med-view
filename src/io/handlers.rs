// src/load_handlers.rs
//! Handlers for async volume and labelmap loading results.
//!
//! This module extracts the load handling logic from lib.rs to improve code organization.

use crate::app::segment::PrimaryShapeKind;
use crate::components::*;
use crate::convert::{
    build_tsdf_chunks_from_labelmap, extract_axis_aligned_contours_from_labelmap,
};
use crate::nifti_loader::LoadedVolume;
use crate::render::tsdf_compute_pipeline::TsdfComputePipeline;
use crate::systems::SegmentManager;
use crate::volume;
use hecs::World;

fn non_zero_labels(mask: &[u8]) -> Vec<u8> {
    let mut seen = [false; 256];
    let mut labels = Vec::new();
    for &v in mask {
        if v == 0 || seen[v as usize] {
            continue;
        }
        seen[v as usize] = true;
        labels.push(v);
    }
    labels.sort_unstable();
    labels
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
            is_visible: false,
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

/// Import a loaded labelmap as voxel-driven segments backed directly by TSDF chunks.
///
/// Tries the GPU JFA path first (`tsdf_pipeline` is `Some` on native WebGPU).
/// Falls back to the CPU separable-EDT path when the GPU pipeline is unavailable
/// (WebGL2, WASM) or when the ROI exceeds the 10-bit JFA coordinate limit.
///
/// Each segment gets `primary_shape = PrimaryShapeKind::Voxels` so that
/// `sys_update_segment_derivatives` skips the SDF build and goes straight to meshing.
pub fn import_labelmap_direct(
    world: &mut World,
    entities: &AppEntities,
    loaded_label: &LoadedLabel,
    #[cfg(not(target_arch = "wasm32"))] device: &wgpu::Device,
    #[cfg(not(target_arch = "wasm32"))] queue: &wgpu::Queue,
    tsdf_pipeline: Option<&TsdfComputePipeline>,
) -> Option<usize> {
    let spacing = world
        .query::<&VolumeData>()
        .with::<&MainVolumeTag>()
        .iter()
        .next()
        .map(|(_, v)| v.spacing)
        .unwrap_or([1.0, 1.0, 1.0]);

    let labels = non_zero_labels(&loaded_label.data);
    if labels.is_empty() {
        return None;
    }

    const CHUNK_SIZE: u32 = 32;
    const TRUNCATION_MM: f32 = 24.0;

    let colors = [
        [1.0, 0.3, 0.3, 0.8],
        [0.3, 1.0, 0.3, 0.8],
        [0.3, 0.3, 1.0, 0.8],
        [1.0, 1.0, 0.3, 0.8],
        [1.0, 0.3, 1.0, 0.8],
        [0.3, 1.0, 1.0, 0.8],
    ];

    if let Ok(mut mgr) = world.get::<&mut SegmentManager>(entities.segments) {
        let mut imported: Option<usize> = None;
        for label in labels {
            // Try GPU path first (native + WebGPU only).
            #[cfg(not(target_arch = "wasm32"))]
            let gpu_result = tsdf_pipeline.and_then(|p| {
                p.build_tsdf_sync(
                    device,
                    queue,
                    &loaded_label.data,
                    loaded_label.dimensions,
                    spacing,
                    label,
                    CHUNK_SIZE,
                    TRUNCATION_MM,
                )
            });
            #[cfg(target_arch = "wasm32")]
            let gpu_result: Option<_> = None;

            // CPU fallback when GPU path is unavailable or returns None.
            let result = gpu_result.or_else(|| {
                let (chunks, tsdf_dims, tsdf_spacing, tsdf_origin) =
                    build_tsdf_chunks_from_labelmap(
                        &loaded_label.data,
                        loaded_label.dimensions,
                        spacing,
                        label,
                        CHUNK_SIZE,
                        TRUNCATION_MM,
                    );
                if chunks.is_empty() {
                    None
                } else {
                    Some((chunks, tsdf_dims, tsdf_spacing, tsdf_origin))
                }
            });

            let Some((chunks, tsdf_dims, tsdf_spacing, tsdf_origin)) = result else {
                continue;
            };

            let chunk_keys: Vec<_> = chunks.keys().copied().collect();
            let idx = mgr.len();
            let color = colors[idx % colors.len()];
            let seg_idx =
                mgr.add_segment(&format!("{} (L{})", loaded_label.filename, label), color);

            if let Some(seg) = mgr.segments.get_mut(seg_idx) {
                seg.primary_shape = PrimaryShapeKind::Voxels;
                seg.chunk_runtime.tsdf_chunks = chunks;
                seg.chunk_runtime.tsdf_dims = tsdf_dims;
                seg.chunk_runtime.tsdf_spacing = tsdf_spacing;
                seg.chunk_runtime.tsdf_origin = tsdf_origin;
                seg.chunk_runtime.enqueue_dirty_mesh_chunks(chunk_keys);
                seg.mesh_dirty = true;
                seg.sdf_dirty = false; // TSDF is already populated
            }
            imported = Some(seg_idx);
        }
        imported
    } else {
        None
    }
}

/// Import loaded labelmap as editable contours into a new contour segment.
///
/// Retained for the contour-editing workflow where users want per-slice authored
/// contours as the primary shape.  For bulk labelmap loads prefer
/// [`import_labelmap_direct`] which avoids the O(slices × axes) contour extraction.
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

    let labels = non_zero_labels(&loaded_label.data);
    if labels.is_empty() {
        return None;
    }

    if let Ok(mut mgr) = world.get::<&mut SegmentManager>(entities.segments) {
        let colors = [
            [1.0, 0.3, 0.3, 0.8],
            [0.3, 1.0, 0.3, 0.8],
            [0.3, 0.3, 1.0, 0.8],
            [1.0, 1.0, 0.3, 0.8],
            [1.0, 0.3, 1.0, 0.8],
            [0.3, 1.0, 1.0, 0.8],
        ];
        let mut imported: Option<usize> = None;
        for label in labels {
            let contour_set = extract_axis_aligned_contours_from_labelmap(
                &loaded_label.data,
                loaded_label.dimensions,
                spacing,
                Some(label),
            );
            if contour_set.is_empty() {
                continue;
            }
            let idx = mgr.len();
            let color = colors[idx % colors.len()];
            let seg_idx = mgr.add_segment(
                &format!("{} (Contours L{})", loaded_label.filename, label),
                color,
            );
            if let Some(seg) = mgr.segments.get_mut(seg_idx) {
                seg.contours = contour_set;
                seg.sync_edited_slices_from_contours();
                seg.mark_dirty();
            }
            imported = Some(seg_idx);
        }
        imported
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
