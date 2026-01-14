// src/render.rs
//
// Rendering infrastructure: pipeline setup, bind group creation, and frame rendering.

use crate::components::*;
use crate::geometry;
use crate::gui;
use crate::overlay::OverlayPrimitive;
use crate::systems;
use hecs::World;
use std::sync::Arc;
use wgpu::util::DeviceExt;
use winit::window::Window;

/// Create the bind group layout for volume rendering.
/// Supports main volume + 2 overlay labelmaps with LUTs.
pub fn create_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        entries: &[
            // 0: Main Volume Texture (R32Float - non-filterable on WebGL2)
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
            // 1: Main Volume Sampler (Non-filtering for R32Float)
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                count: None,
            },
            // 2: Uniforms
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
            // 3: Overlay 1 Texture (R8Uint)
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D3,
                    sample_type: wgpu::TextureSampleType::Uint,
                },
                count: None,
            },
            // 4: Overlay 1 LUT (RGBA8)
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D1,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            // 5: Overlay 2 Texture (R8Uint)
            wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D3,
                    sample_type: wgpu::TextureSampleType::Uint,
                },
                count: None,
            },
            // 6: Overlay 2 LUT (RGBA8)
            wgpu::BindGroupLayoutEntry {
                binding: 6,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D1,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            // 7: Overlay Primitives Storage Buffer
            wgpu::BindGroupLayoutEntry {
                binding: 7,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
        label: Some("texture_bind_group_layout"),
    })
}

const MAX_OVERLAY_PRIMITIVES: usize = 64;

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
        label: Some("Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/shader.wgsl").into()),
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

/// Create the uniform buffer with proper alignment for 4 viewports.
pub fn create_uniform_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    let uniform_alignment = 256;
    let uniform_buffer_size = (uniform_alignment * 4) as u64;
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

/// Create a bind group for the scene with volume and overlay textures.
pub fn create_scene_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    volume_view: &wgpu::TextureView,
    volume_sampler: &wgpu::Sampler,
    uniform_buffer: &wgpu::Buffer,
    overlay1_view: &wgpu::TextureView,
    overlay1_lut: &wgpu::TextureView,
    overlay2_view: &wgpu::TextureView,
    overlay2_lut: &wgpu::TextureView,
    overlay_buffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(volume_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(volume_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: uniform_buffer,
                    offset: 0,
                    size: Some(
                        std::num::NonZeroU64::new(std::mem::size_of::<Uniforms>() as u64).unwrap(),
                    ),
                }),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(overlay1_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(overlay1_lut),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(overlay2_view),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(overlay2_lut),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: overlay_buffer,
                    offset: 0,
                    size: None,
                }),
            },
        ],
        label: Some("diffuse_bind_group"),
    })
}

/// Render a complete frame with all 4 viewports.
pub fn render_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    surface: &wgpu::Surface,
    config: &wgpu::SurfaceConfiguration,
    render_pipeline: &wgpu::RenderPipeline,
    uniform_buffer: &wgpu::Buffer,
    vertex_buffer: &wgpu::Buffer,
    index_buffer: &wgpu::Buffer,
    num_indices: u32,
    overlay_buffer: &wgpu::Buffer,
    world: &mut World,
    entities: &AppEntities,
    gui: &mut gui::Gui,
    window: &Arc<Window>,
    event_proxy: winit::event_loop::EventLoopProxy<crate::AppEvent>,
) -> std::time::Duration {
    if config.width == 0 || config.height == 0 {
        return std::time::Duration::MAX;
    }

    // 0. Process interaction and updates once per frame, right before rendering
    systems::sys_handle_mouse_drag(world, entities);
    systems::sys_paint(world, entities, queue);

    // 1. Run GUI first so it can process interactions and update ECS state for THIS frame
    gui.prepare(window, world, entities, event_proxy);

    // 2. Sync annotations to overlay primitives (using the potentially updated ECS state)
    systems::sys_sync_annotations_to_overlay(world, entities);

    // 3. Get overlay data for GPU buffer
    let (overlay_bytes, overlay_count, dragging_idx, overlay_mouse_uv) =
        systems::get_overlay_render_data(world, entities);

    // Write overlay primitives to storage buffer
    if !overlay_bytes.is_empty() {
        queue.write_buffer(overlay_buffer, 0, &overlay_bytes);
    }

    // 4. Prepare uniforms for all viewports
    let mut viewports = Vec::new();
    for (e, vp) in world.query::<&Viewport>().iter() {
        viewports.push((e, vp.rect, vp.uniform_index));
    }

    for (e, _, u_idx) in &viewports {
        let mut u = systems::sys_prepare_render_data(world, entities, *e);
        // Inject overlay data into uniforms
        u.overlay_primitive_count = overlay_count;
        u.overlay_dragging_idx = dragging_idx;
        u.overlay_mouse_uv = overlay_mouse_uv;

        let offset = *u_idx as u64 * 256;
        queue.write_buffer(uniform_buffer, offset, bytemuck::cast_slice(&[u]));
    }

    let frame = surface.get_current_texture().unwrap();
    let view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Render Encoder"),
    });

    {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
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
        render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint16);

        // Query the bind group from ECS and render
        {
            let mut query = world
                .query::<&GpuVolumeResources>()
                .with::<&MainVolumeTag>();
            if let Some((_, res)) = query.iter().next() {
                let bg = &res.bind_group;
                for (_, rect, u_idx) in &viewports {
                    render_pass.set_viewport(rect[0], rect[1], rect[2], rect[3], 0.0, 1.0);
                    render_pass.set_bind_group(0, bg, &[*u_idx * 256]);
                    render_pass.draw_indexed(0..num_indices, 0, 0..1);
                }
            }
        }
    }

    let screen_descriptor = egui_wgpu::ScreenDescriptor {
        size_in_pixels: [config.width, config.height],
        pixels_per_point: window.scale_factor() as f32,
    };
    let repaint_after = gui.render(device, queue, &mut encoder, &view, &screen_descriptor);

    queue.submit(std::iter::once(encoder.finish()));
    frame.present();

    repaint_after
}
