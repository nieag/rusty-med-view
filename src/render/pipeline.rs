// src/render.rs
//
// Rendering infrastructure: pipeline setup, bind group creation, and frame rendering.

use crate::app::context::{GpuState, Pipelines, SceneState, VolumeResources};
use crate::components::*;
use crate::gui;
use crate::overlay::OverlayPrimitive;
use crate::render::{contours, geometry};
use crate::systems;
use hecs::World;
use std::sync::Arc;
use wgpu::util::DeviceExt;
use winit::window::Window;

/// Create the bind group layout for volume rendering.
/// Supports main volume + eight overlay labelmaps with one shared LUT.
pub fn create_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let mut entries = vec![
        wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                multisampled: false,
                view_dimension: wgpu::TextureViewDimension::D3,
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: Some(
                    std::num::NonZeroU64::new(std::mem::size_of::<Uniforms>() as u64).unwrap(),
                ),
            },
            count: None,
        },
    ];
    entries.extend(
        (0..MAX_VOXEL_OVERLAY_SLOTS).map(|slot| wgpu::BindGroupLayoutEntry {
            binding: 3 + slot as u32,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                multisampled: false,
                view_dimension: wgpu::TextureViewDimension::D3,
                sample_type: wgpu::TextureSampleType::Uint,
            },
            count: None,
        }),
    );
    entries.push(wgpu::BindGroupLayoutEntry {
        binding: 11,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            multisampled: false,
            view_dimension: wgpu::TextureViewDimension::D1,
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
        },
        count: None,
    });
    entries.push(wgpu::BindGroupLayoutEntry {
        binding: 12,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    });

    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        entries: &entries,
        label: Some("texture_bind_group_layout"),
    })
}

const MAX_OVERLAY_PRIMITIVES: usize = 64;

/// WebGPU minimum uniform buffer offset alignment (bytes).
/// TODO: query from device.limits().min_uniform_buffer_offset_alignment at runtime.
const UNIFORM_ALIGNMENT: u64 = 256;
/// Maximum number of viewports exposed by a hanging protocol.
const MAX_VIEWPORTS: u64 = 5;

const fn align_to(value: u64, alignment: u64) -> u64 {
    value.div_ceil(alignment) * alignment
}

const UNIFORM_STRIDE: u64 = align_to(std::mem::size_of::<Uniforms>() as u64, UNIFORM_ALIGNMENT);

/// Create the overlay primitives storage buffer.
pub fn create_overlay_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    let buffer_size = (std::mem::size_of::<OverlayPrimitive>() * MAX_OVERLAY_PRIMITIVES) as u64;
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Overlay Primitives Buffer"),
        size: buffer_size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Create the main render pipeline.
pub fn create_render_pipeline(
    device: &wgpu::Device,
    bind_group_layout: &wgpu::BindGroupLayout,
    surface_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Main Shader"),
        source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
            "../shaders/shader.wgsl"
        ))),
    });

    let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Render Pipeline Layout"),
        bind_group_layouts: &[bind_group_layout],
        push_constant_ranges: &[],
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Render Pipeline"),
        layout: Some(&render_pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[geometry::Vertex::desc()],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Back),
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}

/// Create the uniform buffer with proper alignment for all protocol viewports.
pub fn create_uniform_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    let uniform_buffer_size = UNIFORM_STRIDE * MAX_VIEWPORTS;
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Uniform Buffer"),
        size: uniform_buffer_size,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Create vertex and index buffers for the fullscreen quad.
pub fn create_geometry_buffers(device: &wgpu::Device) -> (wgpu::Buffer, wgpu::Buffer, u32) {
    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Vertex Buffer"),
        contents: bytemuck::cast_slice(geometry::QUAD_VERTICES),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Index Buffer"),
        contents: bytemuck::cast_slice(geometry::QUAD_INDICES),
        usage: wgpu::BufferUsages::INDEX,
    });
    let num_indices = geometry::QUAD_INDICES.len() as u32;
    (vertex_buffer, index_buffer, num_indices)
}

/// GPU resource handles needed to build a scene bind group.
pub struct SceneTextureViews<'a> {
    pub volume_view: &'a wgpu::TextureView,
    pub volume_sampler: &'a wgpu::Sampler,
    pub uniform_buffer: &'a wgpu::Buffer,
    pub overlay_views: [&'a wgpu::TextureView; MAX_VOXEL_OVERLAY_SLOTS],
    pub overlay_lut: &'a wgpu::TextureView,
    pub overlay_buffer: &'a wgpu::Buffer,
}

/// Create a bind group for the scene with volume and overlay textures.
pub fn create_scene_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    views: &SceneTextureViews<'_>,
) -> wgpu::BindGroup {
    let mut entries = vec![
        wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(views.volume_view),
        },
        wgpu::BindGroupEntry {
            binding: 1,
            resource: wgpu::BindingResource::Sampler(views.volume_sampler),
        },
        wgpu::BindGroupEntry {
            binding: 2,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: views.uniform_buffer,
                offset: 0,
                size: Some(
                    std::num::NonZeroU64::new(std::mem::size_of::<Uniforms>() as u64).unwrap(),
                ),
            }),
        },
    ];
    entries.extend(views.overlay_views.iter().enumerate().map(|(slot, view)| {
        wgpu::BindGroupEntry {
            binding: 3 + slot as u32,
            resource: wgpu::BindingResource::TextureView(view),
        }
    }));
    entries.push(wgpu::BindGroupEntry {
        binding: 11,
        resource: wgpu::BindingResource::TextureView(views.overlay_lut),
    });
    entries.push(wgpu::BindGroupEntry {
        binding: 12,
        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: views.overlay_buffer,
            offset: 0,
            size: None,
        }),
    });

    device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout,
        entries: &entries,
        label: Some("diffuse_bind_group"),
    })
}

// Viewport tuple: (entity, rect, uniform_index, mode)
type ViewportList = Vec<(hecs::Entity, [f32; 4], u32, ViewMode)>;

#[derive(Debug, Clone, Copy, Default)]
pub struct RenderFrameStats {
    pub viewport_uniform_count: u32,
    pub contour_batch_count: u32,
    pub mesh_batch_count: u32,
    pub mesh_chunks_uploaded: u32,
    pub mesh_chunks_reused: u32,
    pub roi_work_pending: bool,
    pub last_warning: Option<&'static str>,
    pub last_error: Option<&'static str>,
}

/// Run ECS systems and GUI prep for this frame.
fn run_frame_systems(
    scene: &mut SceneState,
    gui: &mut gui::Gui,
    gpu: &GpuState,
    volume_res: &VolumeResources,
    window: &Arc<Window>,
    event_proxy: winit::event_loop::EventLoopProxy<crate::AppEvent>,
) -> crate::app::roi_runtime::RoiWorkStatus {
    let roi_work = crate::app::roi_runtime::RoiWorkGpuContext {
        device: &gpu.device,
        queue: &gpu.queue,
        bind_groups: crate::app::roi_runtime::BindGroupResources {
            layout: &volume_res.texture_bind_group_layout,
            uniform_buffer: &volume_res.uniform_buffer,
            dummy_view: &volume_res.dummy_r8.1,
            dummy_sampler: &volume_res.dummy_r8.2,
            default_lut_view: &volume_res.default_lut.1,
            overlay_buffer: &volume_res.overlay_buffer,
        },
    };
    let roi_work_status =
        crate::app::roi_runtime::advance_roi_work(&mut scene.world, Some(&roi_work));
    systems::sys_handle_mouse_drag(&mut scene.world, &scene.entities);
    gui.prepare(window, &mut scene.world, &scene.entities, event_proxy);
    systems::sys_sync_annotations_to_overlay(&mut scene.world, &scene.entities);
    if let Ok(mut overlay) = scene
        .world
        .get::<&mut crate::overlay::OverlayManager>(scene.entities.overlay)
    {
        overlay.rebuild_primitives();
    }
    roi_work_status
}

/// Write overlay and per-viewport uniforms to GPU buffers. Returns viewport list.
fn prepare_uniforms(
    scene: &mut SceneState,
    gpu: &GpuState,
    volume_res: &VolumeResources,
) -> ViewportList {
    let (overlay_bytes, overlay_count, dragging_idx, overlay_mouse_uv) =
        systems::get_overlay_render_data(&scene.world, &scene.entities);
    if !overlay_bytes.is_empty() {
        gpu.queue
            .write_buffer(&volume_res.overlay_buffer, 0, &overlay_bytes);
    }

    let mut viewports = Vec::new();
    for (e, vp) in scene.world.query::<&Viewport>().iter() {
        viewports.push((e, vp.rect, vp.uniform_index, vp.mode));
    }
    for (e, _, u_idx, _) in &viewports {
        let mut u = systems::sys_prepare_render_data(&mut scene.world, &scene.entities, *e);
        u.overlay_primitive_count = overlay_count;
        u.overlay_dragging_idx = dragging_idx;
        u.overlay_mouse_uv = overlay_mouse_uv;
        let offset = *u_idx as u64 * UNIFORM_STRIDE;
        gpu.queue.write_buffer(
            &volume_res.uniform_buffer,
            offset,
            bytemuck::cast_slice(&[u]),
        );
    }
    viewports
}

/// Acquire the next surface texture, reconfiguring if needed. Returns None to skip the frame.
enum AcquireSurfaceResult {
    Frame(wgpu::SurfaceTexture),
    Warn(&'static str),
    Error(&'static str),
}

fn acquire_surface_texture(
    surface: &wgpu::Surface,
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> AcquireSurfaceResult {
    match surface.get_current_texture() {
        Ok(frame) => AcquireSurfaceResult::Frame(frame),
        Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
            log::warn!("Surface lost/outdated; reconfiguring surface");
            surface.configure(device, config);
            match surface.get_current_texture() {
                Ok(frame) => AcquireSurfaceResult::Frame(frame),
                Err(err) => {
                    log::warn!("Surface error after reconfigure: {err:?}; skipping frame");
                    AcquireSurfaceResult::Warn("render.surface_reconfigure_failed")
                }
            }
        }
        Err(wgpu::SurfaceError::Timeout) => {
            log::warn!("Surface timeout; skipping frame");
            AcquireSurfaceResult::Warn("render.surface_timeout")
        }
        Err(wgpu::SurfaceError::OutOfMemory) => {
            log::error!("Surface out of memory; skipping frame");
            AcquireSurfaceResult::Error("render.surface_out_of_memory")
        }
        Err(err) => {
            log::warn!("Surface error: {err:?}; skipping frame");
            AcquireSurfaceResult::Warn("render.surface_error")
        }
    }
}

/// Main volume raymarching pass (clears the color attachment).
fn render_volume_pass(
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    world: &World,
    volume_res: &VolumeResources,
    render_pipeline: &wgpu::RenderPipeline,
    viewports: &ViewportList,
) {
    let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Render Pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 1.0,
                }),
                store: wgpu::StoreOp::Store,
            },
            depth_slice: None,
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    });

    render_pass.set_pipeline(render_pipeline);
    render_pass.set_vertex_buffer(0, volume_res.vertex_buffer.slice(..));
    render_pass.set_index_buffer(volume_res.index_buffer.slice(..), wgpu::IndexFormat::Uint16);

    let mut query = world
        .query::<&GpuVolumeResources>()
        .with::<&MainVolumeTag>();
    if let Some((_, res)) = query.iter().next() {
        let bg = &res.bind_group;
        for (_, rect, u_idx, _) in viewports {
            render_pass.set_viewport(rect[0], rect[1], rect[2], rect[3], 0.0, 1.0);
            render_pass.set_bind_group(0, bg, &[(*u_idx as u64 * UNIFORM_STRIDE) as u32]);
            render_pass.draw_indexed(0..volume_res.num_indices, 0, 0..1);
        }
    }
}

/// Render a complete frame with all viewports.
pub fn render_frame(
    gpu: &GpuState,
    volume_res: &VolumeResources,
    pipelines: &mut Pipelines,
    scene: &mut SceneState,
    gui: &mut gui::Gui,
    window: &Arc<Window>,
    event_proxy: winit::event_loop::EventLoopProxy<crate::AppEvent>,
) -> (std::time::Duration, RenderFrameStats) {
    let mut stats = RenderFrameStats::default();
    if gpu.config.width == 0 || gpu.config.height == 0 {
        return (std::time::Duration::MAX, stats);
    }

    let roi_work_status = run_frame_systems(scene, gui, gpu, volume_res, window, event_proxy);
    stats.roi_work_pending = roi_work_status.pending;
    let viewports = prepare_uniforms(scene, gpu, volume_res);
    stats.viewport_uniform_count = viewports.len() as u32;
    let frame = match acquire_surface_texture(&gpu.surface, &gpu.device, &gpu.config) {
        AcquireSurfaceResult::Frame(f) => f,
        AcquireSurfaceResult::Warn(category) => {
            stats.last_warning = Some(category);
            return (std::time::Duration::from_millis(16), stats);
        }
        AcquireSurfaceResult::Error(category) => {
            stats.last_error = Some(category);
            return (std::time::Duration::from_millis(16), stats);
        }
    };
    let view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });

    render_volume_pass(
        &mut encoder,
        &view,
        &scene.world,
        volume_res,
        &pipelines.render,
        &viewports,
    );

    let mesh_data = crate::render::meshes::prepare_mesh_render_data(&scene.world, &scene.entities);
    stats.mesh_batch_count = mesh_data.batch_count() as u32;
    crate::render::meshes::upload_mesh_render_data(
        &gpu.device,
        &gpu.queue,
        &mut pipelines.mesh_overlay,
        &mesh_data,
    );
    stats.mesh_chunks_uploaded = pipelines.mesh_overlay.uploaded_chunks_last_frame;
    stats.mesh_chunks_reused = pipelines.mesh_overlay.reused_chunks_last_frame;
    crate::render::meshes::render_meshes(&mut encoder, &view, &pipelines.mesh_overlay);

    let contour_data = contours::prepare_contour_render_data(&scene.world, &scene.entities);
    stats.contour_batch_count = contour_data.batches.len() as u32;
    contours::upload_contour_render_data(
        &gpu.device,
        &gpu.queue,
        &mut pipelines.contour_overlay,
        &contour_data,
    );
    contours::render_contours(&mut encoder, &view, &pipelines.contour_overlay);

    let screen_descriptor = egui_wgpu::ScreenDescriptor {
        size_in_pixels: [gpu.config.width, gpu.config.height],
        pixels_per_point: window.scale_factor() as f32,
    };
    let repaint_after = gui.render(
        &gpu.device,
        &gpu.queue,
        &mut encoder,
        &view,
        &screen_descriptor,
    );

    gpu.queue.submit(std::iter::once(encoder.finish()));
    frame.present();
    (repaint_after, stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uniform_stride_covers_current_uniform_size() {
        let uniform_size = std::mem::size_of::<Uniforms>() as u64;
        assert!(UNIFORM_STRIDE >= uniform_size);
        assert_eq!(UNIFORM_STRIDE % UNIFORM_ALIGNMENT, 0);
    }

    #[test]
    fn test_eight_overlay_uniform_array_matches_wgsl_alignment() {
        assert_eq!(std::mem::size_of::<VoxelOverlayUniform>(), 80);
        assert_eq!(std::mem::size_of::<Uniforms>(), 832);
        assert_eq!(std::mem::offset_of!(Uniforms, voxel_overlays), 48);
        assert_eq!(std::mem::offset_of!(Uniforms, window_params), 688);
        assert_eq!(std::mem::offset_of!(Uniforms, oblique_origin_uv), 752);
        assert_eq!(std::mem::offset_of!(Uniforms, oblique_u_dir_length), 768);
        assert_eq!(std::mem::offset_of!(Uniforms, oblique_v_dir_length), 784);
        assert_eq!(std::mem::offset_of!(Uniforms, voxel_overlays) % 16, 0);
        assert_eq!(std::mem::offset_of!(Uniforms, window_params) % 16, 0);
        assert_eq!(std::mem::offset_of!(Uniforms, oblique_origin_uv) % 16, 0);
        assert_eq!(std::mem::offset_of!(Uniforms, oblique_u_dir_length) % 16, 0);
        assert_eq!(std::mem::offset_of!(Uniforms, oblique_v_dir_length) % 16, 0);
    }

    #[test]
    fn test_uniform_buffer_size_covers_all_viewport_slots() {
        let last_viewport_offset = (MAX_VIEWPORTS - 1) * UNIFORM_STRIDE;
        let uniform_size = std::mem::size_of::<Uniforms>() as u64;
        let buffer_size = UNIFORM_STRIDE * MAX_VIEWPORTS;
        assert!(last_viewport_offset + uniform_size <= buffer_size);
    }

    #[test]
    fn test_main_shader_and_eight_overlay_bindings_validate() {
        let instance = wgpu::Instance::default();
        let Ok(adapter) =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: None,
                force_fallback_adapter: true,
            }))
        else {
            return;
        };
        let Ok((device, _queue)) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
        else {
            return;
        };

        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let layout = create_bind_group_layout(&device);
        let _pipeline = create_render_pipeline(&device, &layout, wgpu::TextureFormat::Rgba8Unorm);
        let error = pollster::block_on(device.pop_error_scope());

        assert!(error.is_none(), "main shader validation failed: {error:?}");
    }
}
