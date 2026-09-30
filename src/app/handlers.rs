// src/load_handlers.rs
//! Handlers for async volume and labelmap loading results.
//!
//! This module extracts the load handling logic from lib.rs to improve code organization.

use crate::app::roi_runtime;
use crate::components::*;
use crate::nifti_loader::LoadedVolume;
use crate::volume;
use hecs::World;

pub struct LabelLoadOutcome {
    /// One ROI per non-zero label, in ascending label order (a single empty ROI when the map has
    /// no labels). The first is the one to make active.
    pub session: Vec<hecs::Entity>,
    pub dimensions: [u32; 3],
}

/// Handle a successfully loaded volume, updating ECS components and GPU resources.
///
/// Returns the dimensions of the loaded volume for status message construction.
pub fn handle_volume_load(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &mut World,
    session: &mut Session,
    loaded: &LoadedVolume,
) -> [u32; 3] {
    log::info!("Volume loaded: {:?} dimensions", loaded.dimensions);

    let (new_texture, new_view, new_sampler, volume_data) =
        volume::create_texture_from_nifti(device, queue, loaded);
    let volume_data_dimensions = volume_data.dimensions;

    // Update ONLY the main volume components in ECS
    if let Some((_, (vol, gpu_res))) = world
        .query_mut::<(&mut VolumeData, &mut GpuVolumeResources)>()
        .with::<&MainVolumeTag>()
        .into_iter()
        .next()
    {
        vol.dimensions = volume_data.dimensions;
        vol.geometry = volume_data.geometry;
        vol.intensities = volume_data.intensities.clone();
        vol.intensity_range = volume_data.intensity_range;

        gpu_res.texture = new_texture;
        gpu_res.view = new_view;
        gpu_res.sampler = new_sampler;
    }

    // Start every slice plane on a voxel centre, not on a layer boundary.
    {
        let cursor = &mut session.cursor;
        cursor.position = crate::convert::centered_cursor_uv(volume_data_dimensions);
    }

    // Reset user rotation when loading new volume
    for (_, (vp, vs)) in world.query_mut::<(&Viewport, &mut ViewportState)>() {
        if vp.mode == ViewMode::ThreeD {
            vs.user_rotation = [0.0, 0.0, 0.0, 1.0]; // Identity
        }
    }

    loaded.dimensions
}

/// Handle a successfully loaded labelmap by spawning one ROI per label.
///
/// Returns the new session and the dimensions of the loaded labelmap.
pub fn handle_label_load(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &mut World,
    loaded_label: &LoadedLabel,
) -> Result<LabelLoadOutcome, String> {
    log::info!("Labelmap loaded: {:?} dimensions", loaded_label.dimensions);
    let session = roi_runtime::create_voxel_rois_from_label(device, queue, world, loaded_label)?;

    Ok(LabelLoadOutcome {
        session,
        dimensions: loaded_label.dimensions,
    })
}

/// Update GUI status message
pub fn set_status_message(session: &mut Session, message: String) {
    session.gui.status_message = Some(message);
}
