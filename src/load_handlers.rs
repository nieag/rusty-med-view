// src/load_handlers.rs
//! Handlers for async volume and labelmap loading results.
//!
//! This module extracts the load handling logic from lib.rs to improve code organization.

use crate::components::*;
use crate::nifti_loader::LoadedVolume;
use crate::render;
use crate::volume;
use hecs::World;

/// Handle a successfully loaded volume, updating ECS components and GPU resources.
///
/// Returns the dimensions of the loaded volume for status message construction.
pub fn handle_volume_load(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &mut World,
    loaded: &LoadedVolume,
) -> [u32; 3] {
    log::info!("Volume loaded: {:?} dimensions", loaded.dimensions);

    let (new_texture, new_view, new_sampler, volume_data) =
        volume::create_texture_from_nifti(device, queue, loaded);

    // Update ONLY the main volume components in ECS
    for (_, (vol, gpu_res)) in world
        .query_mut::<(&mut VolumeData, &mut GpuVolumeResources)>()
        .with::<&MainVolumeTag>()
    {
        vol.dimensions = volume_data.dimensions;
        vol.spacing = volume_data.spacing;
        vol.intensities = volume_data.intensities.clone();
        vol.intensity_range = volume_data.intensity_range;
        vol.orientation = volume_data.orientation;

        gpu_res.texture = new_texture;
        gpu_res.view = new_view;
        gpu_res.sampler = new_sampler;
        break;
    }

    // Initialize 3D view rotation with volume orientation from NIfTI
    for (_, view) in world.query_mut::<&mut ViewState>() {
        view.rotation[0] = loaded.orientation;
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
    let mut placeholder_bg = None;
    for (_, res) in world.query::<&GpuVolumeResources>().iter() {
        placeholder_bg = Some(res.bind_group.clone());
        break;
    }
    let placeholder_bg = placeholder_bg.expect("Main volume should exist");

    // Spawn a new layer entity with CPU data for painting support
    let entity = world.spawn((
        Segmentation {
            name: loaded_label.filename.clone(),
            is_visible: true,
        },
        LayerSettings {
            opacity: 0.5,
            active_representation: 0,
        },
        LabelmapData {
            dimensions: loaded_label.dimensions,
            spacing: loaded_label.spacing,
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
        // We need to iterate all components that have Representation::Voxel
        // We also want to respect some order, e.g. Entity ID or Name.
        // For simplicity, we just collect them all.
        // Note: we can't sort nicely without collecting first.
        let mut query = world.query::<&Representation>();
        for (_, r) in query.iter() {
            if let Representation::Voxel(res) = r {
                overlay_views.push(res.view.clone());
            }
        }
        // Limit to 2 for now as shader supports 2
        // Ideally we pick "visible" ones or "first 2".
        // Let's just take first 2 found.
    }

    // Use actual views or fallback to dummy
    let main_view_ref = main_view.as_ref().unwrap_or(dummy_view);

    let overlay1_view = overlay_views.get(0).unwrap_or(dummy_view);
    let overlay2_view = overlay_views.get(1).unwrap_or(dummy_view);

    let new_bind_group = render::create_scene_bind_group(
        device,
        texture_bind_group_layout,
        main_view_ref,
        dummy_sampler,
        uniform_buffer,
        overlay1_view,
        default_lut_view,
        overlay2_view,
        default_lut_view,
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
pub fn set_status_message(world: &mut World, message: String) {
    for (_, gui_state) in world.query_mut::<&mut GuiState>() {
        gui_state.status_message = Some(message.clone());
    }
}
