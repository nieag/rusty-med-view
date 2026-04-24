// src/load_handlers.rs
//! Handlers for async volume and labelmap loading results.
//!
//! This module extracts the load handling logic from lib.rs to improve code organization.

use crate::app::roi_runtime;
use crate::components::*;
use crate::nifti_loader::LoadedVolume;
use crate::volume;
use hecs::World;

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
    let entity = roi_runtime::create_voxel_roi_from_label(device, queue, world, loaded_label);

    (entity, loaded_label.dimensions)
}

/// Update GUI status message
pub fn set_status_message(world: &mut World, entities: &AppEntities, message: String) {
    if let Ok(mut gui_state) = world.get::<&mut GuiState>(entities.gui_state) {
        gui_state.status_message = Some(message);
    }
}
