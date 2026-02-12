// src/render.rs
//
// Rendering infrastructure: pipeline setup, bind group creation, and frame rendering.

use crate::components::*;
use crate::gui;
use crate::overlay::OverlayPrimitive;
use crate::render::contour_pipeline::{ContourLineGpu, ContourPipeline, ContourUniforms};
use crate::render::geometry;
use crate::render::mesh_pipeline::{MeshPipeline, MeshResources, MeshUniforms};
use crate::render::sdf_preview_pipeline::{SdfPreviewPipeline, SdfPreviewUniforms};
use crate::systems::{self, ContourDrawState, SegmentManager};
use crate::util::orientation::SlicePlane;
use hecs::World;
use std::collections::{HashMap, HashSet};
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
    contour_pipeline: &mut ContourPipeline,
    sdf_preview_pipeline: &mut SdfPreviewPipeline,
    mesh_pipeline: &MeshPipeline,
    segment_mesh_gpu_cache: &mut HashMap<uuid::Uuid, (u64, MeshResources)>,
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
    systems::sys_update_segment_derivatives(world, entities);

    // 1. Run GUI first so it can process interactions and update ECS state for THIS frame
    gui.prepare(window, world, entities, event_proxy);

    // 2. Sync annotations to overlay primitives (using the potentially updated ECS state)
    systems::sys_sync_annotations_to_overlay(world, entities);

    // 2c. Rebuild overlay primitives after all syncs
    if let Ok(mut overlay) = world.get::<&mut crate::overlay::OverlayManager>(entities.overlay) {
        overlay.rebuild_primitives();
    }

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

    let frame = match surface.get_current_texture() {
        Ok(frame) => frame,
        Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
            log::warn!("Surface lost/outdated; reconfiguring surface");
            surface.configure(device, config);
            match surface.get_current_texture() {
                Ok(frame) => frame,
                Err(wgpu::SurfaceError::Timeout) => {
                    log::warn!("Surface timeout after reconfigure; skipping frame");
                    return std::time::Duration::from_millis(16);
                }
                Err(wgpu::SurfaceError::OutOfMemory) => {
                    panic!("Surface out of memory after reconfigure");
                }
                Err(err) => {
                    log::warn!("Surface error after reconfigure: {err:?}; skipping frame");
                    return std::time::Duration::from_millis(16);
                }
            }
        }
        Err(wgpu::SurfaceError::Timeout) => {
            log::warn!("Surface timeout; skipping frame");
            return std::time::Duration::from_millis(16);
        }
        Err(wgpu::SurfaceError::OutOfMemory) => {
            panic!("Surface out of memory");
        }
        Err(err) => {
            log::warn!("Surface error: {err:?}; skipping frame");
            return std::time::Duration::from_millis(16);
        }
    };
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

    // --- Contour Rendering Pass (Step 2: real uniforms + test triangle) ---
    {
        // Get volume info
        let mut volume_dims = [0u32; 3];
        let mut volume_spacing = [1.0f32; 3];
        for (_, vol) in world.query::<&VolumeData>().iter() {
            volume_dims = vol.dimensions;
            volume_spacing = vol.spacing;
        }

        // Get cursor for slice calculation
        let cursor_pos = world
            .get::<&Transform>(entities.cursor)
            .map(|t| t.position)
            .unwrap_or([0.5, 0.5, 0.5]);

        // Create GPU lines from SegmentManager (Step 3: Real Data)
        let mut gpu_lines = Vec::new();

        if let Ok(manager) = world.get::<&SegmentManager>(entities.segments) {
            // 1. Collect lines from existing segments
            for segment in &manager.segments {
                if !segment.visible {
                    continue;
                }

                // Volume UV normalization factor: 1.0 / ( (dims-1) * spacing )
                let denoms = [
                    ((volume_dims[0] as f32 - 1.0) * volume_spacing[0]).max(0.001),
                    ((volume_dims[1] as f32 - 1.0) * volume_spacing[1]).max(0.001),
                    ((volume_dims[2] as f32 - 1.0) * volume_spacing[2]).max(0.001),
                ];

                for contour in segment.contours.all_contours() {
                    if contour.points.len() < 2 {
                        continue;
                    }

                    // Which axis-aligned plane is this on? (heuristic check)
                    let view_mode = if contour.plane.normal[2].abs() > 0.9 {
                        1u32 // Axial
                    } else if contour.plane.normal[1].abs() > 0.9 {
                        2u32 // Coronal
                    } else if contour.plane.normal[0].abs() > 0.9 {
                        3u32 // Sagittal
                    } else {
                        0u32 // 3D/Oblique
                    };

                    if view_mode == 0 {
                        continue; // Skip oblique for now
                    }

                    // Slice index from plane distance
                    let depth_axis = match view_mode {
                        1 => 2,
                        2 => 1,
                        3 => 0,
                        _ => 2,
                    };
                    let slice =
                        (contour.plane.distance / volume_spacing[depth_axis]).floor() as i32;

                    let segment_count = if contour.is_closed {
                        contour.points.len()
                    } else {
                        contour.points.len().saturating_sub(1)
                    };

                    for i in 0..segment_count {
                        let j = (i + 1) % contour.points.len();
                        let p_i = contour.points[i];
                        let p_j = contour.points[j];

                        let p0 = [p_i[0] / denoms[0], p_i[1] / denoms[1], p_i[2] / denoms[2]];
                        let p1 = [p_j[0] / denoms[0], p_j[1] / denoms[1], p_j[2] / denoms[2]];

                        gpu_lines.push(ContourLineGpu::new(
                            p0,
                            p1,
                            segment.color,
                            view_mode,
                            slice,
                        ));
                    }
                }
            }

            // 2. Add current drawing preview line
            if let ContourDrawState::Drawing {
                points,
                slice_plane,
                slice_index,
            } = &manager.draw_state
            {
                let view_mode = match slice_plane {
                    SlicePlane::Axial => 1u32,
                    SlicePlane::Coronal => 2u32,
                    SlicePlane::Sagittal => 3u32,
                };

                let depth_idx = slice_plane.depth_axis();
                let depth_uv = (*slice_index as f32 + 0.5) / volume_dims[depth_idx].max(1) as f32;

                for i in 0..points.len().saturating_sub(1) {
                    let p0_uv = slice_plane.screen_uv_to_volume(points[i], depth_uv);
                    let p1_uv = slice_plane.screen_uv_to_volume(points[i + 1], depth_uv);

                    gpu_lines.push(ContourLineGpu::new(
                        p0_uv,
                        p1_uv,
                        [1.0, 1.0, 0.0, 1.0], // Yellow preview
                        view_mode,
                        *slice_index,
                    ));
                }
            }
        }

        contour_pipeline.update_lines(queue, &gpu_lines);

        for (_e, rect, u_idx) in &viewports {
            if *u_idx == 0 {
                continue; // Skip 3D viewport
            }

            let view_mode = *u_idx;

            // Calculate current slice index for this viewport
            let current_slice = match view_mode {
                1 => (cursor_pos[2] * volume_dims[2] as f32) as i32,
                2 => (cursor_pos[1] * volume_dims[1] as f32) as i32,
                3 => (cursor_pos[0] * volume_dims[0] as f32) as i32,
                _ => 0,
            };

            // Read real zoom/pan from ViewportState
            let (zoom, pan) = world
                .get::<&ViewportState>(*_e)
                .map(|vs| (vs.zoom, vs.pan))
                .unwrap_or((1.0, [0.0, 0.0]));

            let uniforms = ContourUniforms {
                zoom,
                _pad0: 0.0,
                pan,
                pivot: [0.5, 0.5],
                view_mode,
                _pad1: 0,
                resolution: [rect[2], rect[3]],
                _pad2: [0.0; 2],
                volume_dims,
                _pad3: 0,
                volume_spacing,
                current_slice,
                _padding: [0.0; 4],
            };

            contour_pipeline.update_uniforms(queue, &uniforms, view_mode);

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Contour Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            pass.set_viewport(rect[0], rect[1], rect[2], rect[3], 0.0, 1.0);
            contour_pipeline.render(&mut pass, view_mode);
        }
    }

    // --- SDF Preview Pass (2D viewports only, active segment only) ---
    {
        let mut volume_dims = [0u32; 3];
        let mut volume_spacing = [1.0f32; 3];
        for (_, vol) in world.query::<&VolumeData>().iter() {
            volume_dims = vol.dimensions;
            volume_spacing = vol.spacing;
            break;
        }

        let preview_state = world
            .get::<&SdfPreviewState>(entities.sdf_preview)
            .ok()
            .map(|s| *s)
            .unwrap_or_default();

        if let Ok(manager) = world.get::<&SegmentManager>(entities.segments) {
            sdf_preview_pipeline.update_from_active_segment(device, queue, manager.active_segment());
        } else {
            sdf_preview_pipeline.update_from_active_segment(device, queue, None);
        }

        if preview_state.enabled {
            let cursor_pos = world
                .get::<&Transform>(entities.cursor)
                .map(|t| t.position)
                .unwrap_or([0.5, 0.5, 0.5]);

            let sdf_dims = sdf_preview_pipeline.sdf_dims();
            for (_e, rect, u_idx) in &viewports {
                if *u_idx == 0 {
                    continue;
                }
                let view_mode = *u_idx;
                let current_slice = match view_mode {
                    1 => (cursor_pos[2] * volume_dims[2] as f32) as i32,
                    2 => (cursor_pos[1] * volume_dims[1] as f32) as i32,
                    3 => (cursor_pos[0] * volume_dims[0] as f32) as i32,
                    _ => 0,
                };

                let (zoom, pan) = world
                    .get::<&ViewportState>(*_e)
                    .map(|vs| (vs.zoom, vs.pan))
                    .unwrap_or((1.0, [0.0, 0.0]));

                let uniforms = SdfPreviewUniforms {
                    zoom,
                    _pad0: 0.0,
                    pan,
                    pivot: [0.5, 0.5],
                    view_mode,
                    show_zero_isoline: if preview_state.show_zero_isoline { 1 } else { 0 },
                    resolution: [rect[2], rect[3]],
                    _pad_align0: [0; 2],
                    volume_dims,
                    current_slice,
                    volume_spacing,
                    opacity: preview_state.opacity,
                    sdf_dims,
                    _pad1: 0,
                    value_window_mm: preview_state.value_window_mm,
                    _pad_align1: [0.0; 3],
                    _pad2: [0.0; 4],
                };
                sdf_preview_pipeline.update_uniforms(queue, &uniforms, view_mode);

                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("SDF Preview Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_viewport(rect[0], rect[1], rect[2], rect[3], 0.0, 1.0);
                sdf_preview_pipeline.render(&mut pass, view_mode);
            }
        }
    }

    // --- 3D SDF Surface Preview Pass (active segment mesh in 3D viewport) ---
    {
        let preview_state = world
            .get::<&SdfPreviewState>(entities.sdf_preview)
            .ok()
            .map(|s| *s)
            .unwrap_or_default();

        if preview_state.show_3d_surface {
            let mut volume_dims = [0u32; 3];
            let mut volume_spacing = [1.0f32; 3];
            let mut data_orientation = [0.0f32; 4];
            for (_, vol) in world.query::<&VolumeData>().iter() {
                volume_dims = vol.dimensions;
                volume_spacing = vol.spacing;
                data_orientation = vol.orientation;
                break;
            }

            if let Ok(manager) = world.get::<&SegmentManager>(entities.segments) {
                let live_ids: HashSet<uuid::Uuid> =
                    manager.segments.iter().map(|s| s.id).collect();
                segment_mesh_gpu_cache.retain(|id, _| live_ids.contains(id));

                if let Some(segment) = manager.active_segment() {
                    if let Some(mesh_data) = segment.mesh.as_ref() {
                        let segment_color = segment.color;
                        let mut mesh_ready = false;
                        if let Some((rev, _)) = segment_mesh_gpu_cache.get(&segment.id) {
                            if *rev == segment.mesh_revision {
                                mesh_ready = true;
                            }
                        }
                        if !mesh_ready {
                            if let Some(mesh_res) = MeshResources::from_mesh_data(device, mesh_data) {
                                segment_mesh_gpu_cache
                                    .insert(segment.id, (segment.mesh_revision, mesh_res));
                                mesh_ready = true;
                            } else {
                                segment_mesh_gpu_cache.remove(&segment.id);
                            }
                        }

                        if mesh_ready {
                            if let Some((_, mesh_res)) = segment_mesh_gpu_cache.get(&segment.id) {
                                for (e, rect, u_idx) in &viewports {
                                    if *u_idx != 0 {
                                        continue;
                                    }

                                    let vs = world
                                        .get::<&ViewportState>(*e)
                                        .ok()
                                        .map(|r| *r)
                                        .unwrap_or_default();
                                    let composed_rotation = crate::util::orientation::compose_view_rotation(
                                        data_orientation,
                                        vs.user_rotation,
                                    );

                                let phys = [
                                    (volume_dims[0].saturating_sub(1) as f32 * volume_spacing[0]).max(1e-3),
                                    (volume_dims[1].saturating_sub(1) as f32 * volume_spacing[1]).max(1e-3),
                                    (volume_dims[2].saturating_sub(1) as f32 * volume_spacing[2]).max(1e-3),
                                ];
                                let max_phys = phys[0].max(phys[1]).max(phys[2]).max(1e-3);

                                let t_center = glam::Mat4::from_translation(glam::Vec3::new(
                                    -0.5 * phys[0],
                                    -0.5 * phys[1],
                                    -0.5 * phys[2],
                                ));
                                let s_norm = glam::Mat4::from_scale(glam::Vec3::splat(1.0 / max_phys));
                                let r = glam::Mat4::from_quat(glam::Quat::from_array(composed_rotation));
                                let model = r * s_norm * t_center;

                                let aspect = (rect[2] / rect[3].max(1.0)).max(1e-3);
                                let view_mat = glam::Mat4::look_at_rh(
                                    glam::Vec3::new(0.0, 0.0, -3.5),
                                    glam::Vec3::ZERO,
                                    glam::Vec3::Y,
                                );
                                // Match volume ray shader camera model:
                                // ray_dir = normalize(forward + right*x + up*y) where y spans [-0.5, 0.5].
                                // Equivalent vertical FOV = 2 * atan(0.5) ~= 53.13 deg.
                                let proj = glam::Mat4::perspective_rh_gl(
                                    2.0_f32 * 0.5_f32.atan(),
                                    aspect,
                                    0.01,
                                    20.0,
                                );
                                // Match volume viewport 3D interaction by applying the same
                                // screen-space zoom/pan transform in clip space.
                                let pan_zoom = glam::Mat4::from_cols(
                                    glam::Vec4::new(vs.zoom, 0.0, 0.0, 0.0),
                                    glam::Vec4::new(0.0, vs.zoom, 0.0, 0.0),
                                    glam::Vec4::new(0.0, 0.0, 1.0, 0.0),
                                    glam::Vec4::new(
                                        -2.0 * vs.pan[0] * vs.zoom,
                                        2.0 * vs.pan[1] * vs.zoom,
                                        0.0,
                                        1.0,
                                    ),
                                );
                                let view_proj = pan_zoom * proj * view_mat;
                                let normal = model.inverse().transpose();

                                let mut color = segment_color;
                                // Render 3D preview as a solid mesh; opacity slider is for 2D SDF heatmap.
                                color[3] = 1.0;
                                let mesh_uniforms = MeshUniforms {
                                    model_matrix: model.to_cols_array_2d(),
                                    view_proj_matrix: view_proj.to_cols_array_2d(),
                                    normal_matrix: normal.to_cols_array_2d(),
                                    color,
                                    camera_pos: [0.0, 0.0, -3.5, 1.0],
                                    light_dir: [0.45, -1.0, 0.35, 0.0],
                                    light_color: [1.0, 1.0, 1.0, 1.0],
                                    ambient: [0.16, 0.16, 0.16, 1.0],
                                };
                                mesh_pipeline.update_uniforms(queue, &mesh_uniforms);

                                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                        label: Some("SDF 3D Surface Pass"),
                                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                            view: &view,
                                            resolve_target: None,
                                            ops: wgpu::Operations {
                                                load: wgpu::LoadOp::Load,
                                                store: wgpu::StoreOp::Store,
                                            },
                                            depth_slice: None,
                                        })],
                                        depth_stencil_attachment: Some(
                                            wgpu::RenderPassDepthStencilAttachment {
                                                view: &mesh_pipeline.depth_texture,
                                                depth_ops: Some(wgpu::Operations {
                                                    load: wgpu::LoadOp::Clear(1.0),
                                                    store: wgpu::StoreOp::Store,
                                                }),
                                                stencil_ops: None,
                                            },
                                        ),
                                        timestamp_writes: None,
                                        occlusion_query_set: None,
                                    });
                                    pass.set_viewport(rect[0], rect[1], rect[2], rect[3], 0.0, 1.0);
                                    mesh_pipeline.render(&mut pass, mesh_res);
                                }
                            }
                        }
                    }
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
