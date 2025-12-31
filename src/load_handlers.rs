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

        gpu_res.texture = new_texture;
        gpu_res.view = new_view;
        gpu_res.sampler = new_sampler;
        break;
    }

    loaded.dimensions
}

/// Handle a successfully loaded labelmap, updating ECS components and GPU resources.
///
/// Returns the dimensions of the loaded labelmap for status message construction.
pub fn handle_label_load(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &mut World,
    loaded_label: &LoadedLabel,
) -> [u32; 3] {
    log::info!("Labelmap loaded: {:?} dimensions", loaded_label.dimensions);

    let (new_texture, new_view, new_sampler) =
        volume::create_texture_from_labelmap(device, queue, loaded_label);

    // Find the first Segmentation entity and update its Voxel representation
    for (_, (seg, settings, repr)) in
        world.query_mut::<(&mut Segmentation, &mut LayerSettings, &mut Representation)>()
    {
        let Representation::Voxel(gpu_res) = repr;
        gpu_res.texture = new_texture;
        gpu_res.view = new_view;
        gpu_res.sampler = new_sampler;
        seg.is_visible = true;
        settings.opacity = 0.5;
        break;
    }

    loaded_label.dimensions
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
    let overlay1_view: Option<wgpu::TextureView>;

    // Query main volume view
    {
        let query = world.query::<&GpuVolumeResources>();
        let mut with_tag = query.with::<&MainVolumeTag>();
        main_view = with_tag.iter().next().map(|(_, res)| res.view.clone());
    }

    // Query overlay view
    {
        let mut query = world.query::<&Representation>();
        overlay1_view = query.iter().next().map(|(_, r)| {
            let Representation::Voxel(res) = r;
            res.view.clone()
        });
    }

    // Use actual views or fallback to dummy
    let main_view_ref = main_view.as_ref().unwrap_or(dummy_view);
    let overlay1_view_ref = overlay1_view.as_ref().unwrap_or(dummy_view);

    let new_bind_group = render::create_scene_bind_group(
        device,
        texture_bind_group_layout,
        main_view_ref,
        dummy_sampler,
        uniform_buffer,
        overlay1_view_ref,
        default_lut_view,
        dummy_view,
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
