use crate::components::{
    AppEntities, MeshData, Roi, RoiAuthoritativeData, ViewMode, Viewport, ViewportState,
};
use crate::render::geometry::{
    build_display_projection_context, world_to_ndc, DisplayProjectionContext,
};
use glam::Vec3;
use hecs::World;
use wgpu::util::DeviceExt;

const INITIAL_VERTEX_CAPACITY: usize = 256;

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, PartialEq)]
pub struct MeshVertex2d {
    pub position_ndc: [f32; 2],
    pub color: [f32; 4],
}

impl MeshVertex2d {
    pub fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<MeshVertex2d>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeshRenderBatch {
    pub start_vertex: u32,
    pub vertex_count: u32,
    pub scissor_rect: [u32; 4],
}

#[derive(Default, Debug, Clone)]
pub struct MeshRenderData {
    pub vertices: Vec<MeshVertex2d>,
    pub batches: Vec<MeshRenderBatch>,
}

pub struct MeshRenderer {
    pub pipeline: wgpu::RenderPipeline,
    pub vertex_buffer: wgpu::Buffer,
    pub vertex_capacity: usize,
    pub vertex_count: u32,
    pub batches: Vec<MeshRenderBatch>,
}

pub fn create_mesh_renderer(
    device: &wgpu::Device,
    surface_format: wgpu::TextureFormat,
) -> MeshRenderer {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Mesh Overlay Shader"),
        source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
            "../shaders/mesh_overlay.wgsl"
        ))),
    });

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Mesh Overlay Pipeline Layout"),
        bind_group_layouts: &[],
        push_constant_ranges: &[],
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Mesh Overlay Pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[MeshVertex2d::desc()],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    });

    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Mesh Overlay Vertex Buffer"),
        contents: bytemuck::cast_slice(&vec![
            MeshVertex2d {
                position_ndc: [0.0, 0.0],
                color: [0.0, 0.0, 0.0, 0.0],
            };
            INITIAL_VERTEX_CAPACITY
        ]),
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
    });

    MeshRenderer {
        pipeline,
        vertex_buffer,
        vertex_capacity: INITIAL_VERTEX_CAPACITY,
        vertex_count: 0,
        batches: Vec::new(),
    }
}

pub fn prepare_mesh_render_data(world: &World, entities: &AppEntities) -> MeshRenderData {
    let mut data = MeshRenderData::default();
    for (_, (viewport, viewport_state)) in world.query::<(&Viewport, &ViewportState)>().iter() {
        if viewport.mode != ViewMode::ThreeD {
            continue;
        }
        let Some(projection_ctx) =
            build_display_projection_context(world, entities, viewport, viewport_state)
        else {
            continue;
        };

        let batch_start = data.vertices.len() as u32;

        for (_, roi) in world.query::<&Roi>().iter() {
            if !roi.metadata.is_visible {
                continue;
            }
            let Some(mesh) = mesh_data_for_render(roi) else {
                continue;
            };
            append_projected_mesh(mesh, roi.metadata.color, projection_ctx, &mut data.vertices);
        }

        let batch_count = data.vertices.len() as u32 - batch_start;
        if batch_count == 0 {
            continue;
        }
        let scissor =
            viewport_scissor_rect(projection_ctx.viewport_rect, projection_ctx.window_size);
        if scissor[2] == 0 || scissor[3] == 0 {
            continue;
        }
        data.batches.push(MeshRenderBatch {
            start_vertex: batch_start,
            vertex_count: batch_count,
            scissor_rect: scissor,
        });
    }

    data
}

fn mesh_data_for_render(roi: &Roi) -> Option<&MeshData> {
    match &roi.authoritative_data {
        RoiAuthoritativeData::Mesh(mesh) => Some(mesh),
        RoiAuthoritativeData::Voxel(_) | RoiAuthoritativeData::Contour(_) => None,
    }
}

fn append_projected_mesh(
    mesh: &MeshData,
    color: [f32; 4],
    projection_ctx: DisplayProjectionContext,
    out: &mut Vec<MeshVertex2d>,
) {
    let color = [color[0], color[1], color[2], color[3] * 0.35];
    for face in &mesh.faces {
        let [a, b, c] = face.vertex_indices;
        let (Some(va), Some(vb), Some(vc)) = (
            mesh.vertices.get(a as usize),
            mesh.vertices.get(b as usize),
            mesh.vertices.get(c as usize),
        ) else {
            continue;
        };

        let pa = project_world_vertex(va.world_mm, projection_ctx);
        let pb = project_world_vertex(vb.world_mm, projection_ctx);
        let pc = project_world_vertex(vc.world_mm, projection_ctx);
        let (Some(pa), Some(pb), Some(pc)) = (pa, pb, pc) else {
            continue;
        };

        out.push(MeshVertex2d {
            position_ndc: pa,
            color,
        });
        out.push(MeshVertex2d {
            position_ndc: pb,
            color,
        });
        out.push(MeshVertex2d {
            position_ndc: pc,
            color,
        });
    }
}

fn project_world_vertex(
    world_mm: [f32; 3],
    projection_ctx: DisplayProjectionContext,
) -> Option<[f32; 2]> {
    let uv = crate::convert::world_mm_to_volume_uv(world_mm, projection_ctx.main_geometry);
    let projection = projection_ctx.view_projection_3d();
    let viewport_uv = world_to_ndc(
        Vec3::from_array(uv),
        ViewMode::ThreeD,
        &projection,
        projection_ctx.screen_aspect,
    )?;
    viewport_uv_to_full_ndc(
        viewport_uv,
        projection_ctx.viewport_rect,
        projection_ctx.window_size,
    )
}

fn viewport_uv_to_full_ndc(
    viewport_uv: [f32; 2],
    viewport_rect: [f32; 4],
    window_size: [f32; 2],
) -> Option<[f32; 2]> {
    let [window_w, window_h] = window_size;
    if window_w <= 0.0 || window_h <= 0.0 {
        return None;
    }
    if !viewport_uv[0].is_finite() || !viewport_uv[1].is_finite() {
        return None;
    }
    let screen_x = viewport_rect[0] + viewport_uv[0] * viewport_rect[2];
    let screen_y = viewport_rect[1] + viewport_uv[1] * viewport_rect[3];
    if !screen_x.is_finite() || !screen_y.is_finite() {
        return None;
    }
    Some([
        (screen_x / window_w) * 2.0 - 1.0,
        1.0 - (screen_y / window_h) * 2.0,
    ])
}

fn viewport_scissor_rect(viewport_rect: [f32; 4], window_size: [f32; 2]) -> [u32; 4] {
    let [window_w, window_h] = window_size;
    let vx0 = viewport_rect[0].max(0.0).min(window_w);
    let vy0 = viewport_rect[1].max(0.0).min(window_h);
    let vx1 = (viewport_rect[0] + viewport_rect[2]).max(0.0).min(window_w);
    let vy1 = (viewport_rect[1] + viewport_rect[3]).max(0.0).min(window_h);
    let w = (vx1 - vx0).max(0.0);
    let h = (vy1 - vy0).max(0.0);
    [vx0 as u32, vy0 as u32, w as u32, h as u32]
}

pub fn upload_mesh_render_data(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut MeshRenderer,
    data: &MeshRenderData,
) {
    renderer.vertex_count = data.vertices.len() as u32;
    renderer.batches = data.batches.clone();
    if data.vertices.is_empty() {
        return;
    }

    if data.vertices.len() > renderer.vertex_capacity {
        renderer.vertex_capacity = data.vertices.len().next_power_of_two();
        renderer.vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Mesh Overlay Vertex Buffer"),
            size: (renderer.vertex_capacity * std::mem::size_of::<MeshVertex2d>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
    }

    queue.write_buffer(
        &renderer.vertex_buffer,
        0,
        bytemuck::cast_slice(&data.vertices),
    );
}

pub fn render_meshes(
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    renderer: &MeshRenderer,
) {
    if renderer.vertex_count == 0 || renderer.batches.is_empty() {
        return;
    }

    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Mesh Overlay Pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
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

    pass.set_pipeline(&renderer.pipeline);
    pass.set_vertex_buffer(0, renderer.vertex_buffer.slice(..));
    for batch in &renderer.batches {
        if batch.vertex_count == 0 || batch.scissor_rect[2] == 0 || batch.scissor_rect[3] == 0 {
            continue;
        }
        pass.set_scissor_rect(
            batch.scissor_rect[0],
            batch.scissor_rect[1],
            batch.scissor_rect[2],
            batch.scissor_rect[3],
        );
        pass.draw(
            batch.start_vertex..(batch.start_vertex + batch.vertex_count),
            0..1,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{
        MainVolumeTag, MeshFace, MeshVertex, RoiId, Transform, VolumeData, WindowSettings,
    };
    use crate::render::geometry::ViewProjection;

    fn spawn_world_base() -> (World, AppEntities) {
        let mut world = World::new();
        world.spawn((
            VolumeData {
                dimensions: [10, 10, 10],
                spacing: [1.0, 1.0, 1.0],
                origin: [0.0, 0.0, 0.0],
                intensities: vec![],
                intensity_range: [0.0, 1.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            MainVolumeTag,
        ));
        let cursor = world.spawn((Transform {
            position: [0.5, 0.5, 0.5],
        },));
        let ws = world.spawn((WindowSettings {
            width: 800,
            height: 600,
            viewport_rect: [0.0, 0.0, 800.0, 600.0],
        },));
        let editor = world.spawn((crate::components::EditorState::default(),));
        let input = world.spawn((crate::components::InputState::default(),));
        let gui_state = world.spawn((crate::components::GuiState {
            status_message: None,
        },));
        let volume_windowing = world.spawn((crate::components::VolumeWindowing::default(),));
        let annotations = world.spawn((crate::components::AnnotationState::default(),));
        let overlay = world.spawn((crate::overlay::OverlayManager::default(),));
        let protocol = world.spawn((crate::components::ProtocolState::default(),));
        (
            world,
            AppEntities {
                input,
                editor,
                gui_state,
                volume_windowing,
                annotations,
                overlay,
                protocol,
                cursor,
                window_settings: ws,
            },
        )
    }

    #[test]
    fn test_prepare_mesh_render_data_empty_without_main_volume() {
        let world = World::new();
        let entities = AppEntities {
            input: hecs::Entity::DANGLING,
            editor: hecs::Entity::DANGLING,
            gui_state: hecs::Entity::DANGLING,
            volume_windowing: hecs::Entity::DANGLING,
            annotations: hecs::Entity::DANGLING,
            overlay: hecs::Entity::DANGLING,
            protocol: hecs::Entity::DANGLING,
            cursor: hecs::Entity::DANGLING,
            window_settings: hecs::Entity::DANGLING,
        };
        let data = prepare_mesh_render_data(&world, &entities);
        assert!(data.vertices.is_empty());
        assert!(data.batches.is_empty());
    }

    #[test]
    fn test_prepare_mesh_render_data_skips_non_3d_viewports() {
        let (mut world, entities) = spawn_world_base();
        world.spawn((
            Viewport {
                mode: ViewMode::Axial,
                rect: [0.0, 0.0, 400.0, 300.0],
                uniform_index: 0,
            },
            ViewportState::default(),
        ));
        world.spawn((Roi::new_mesh(
            RoiId(1),
            "Mesh".to_string(),
            MeshData {
                vertices: vec![
                    MeshVertex {
                        world_mm: [1.0, 1.0, 1.0],
                    },
                    MeshVertex {
                        world_mm: [2.0, 1.0, 1.0],
                    },
                    MeshVertex {
                        world_mm: [1.0, 2.0, 1.0],
                    },
                ],
                faces: vec![MeshFace {
                    vertex_indices: [0, 1, 2],
                }],
            },
        ),));
        let data = prepare_mesh_render_data(&world, &entities);
        assert!(data.vertices.is_empty());
        assert!(data.batches.is_empty());
    }

    #[test]
    fn test_prepare_mesh_render_data_uses_main_volume_geometry_for_projection() {
        let (mut world, entities) = spawn_world_base();
        {
            let mut query = world.query::<&mut VolumeData>().with::<&MainVolumeTag>();
            let (_, vol) = query.iter().next().expect("main volume");
            vol.orientation = glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2).to_array();
        }
        let user_rotation = glam::Quat::from_rotation_x(std::f32::consts::FRAC_PI_4).to_array();
        world.spawn((
            Viewport {
                mode: ViewMode::ThreeD,
                rect: [100.0, 50.0, 400.0, 300.0],
                uniform_index: 0,
            },
            ViewportState {
                user_rotation,
                ..ViewportState::default()
            },
        ));
        let world_vertex = [2.0, 2.0, 2.0];
        world.spawn((Roi::new_mesh(
            RoiId(2),
            "Mesh".to_string(),
            MeshData {
                vertices: vec![
                    MeshVertex {
                        world_mm: world_vertex,
                    },
                    MeshVertex {
                        world_mm: [3.0, 2.0, 2.0],
                    },
                    MeshVertex {
                        world_mm: [2.0, 3.0, 2.0],
                    },
                ],
                faces: vec![MeshFace {
                    vertex_indices: [0, 1, 2],
                }],
            },
        ),));
        let data = prepare_mesh_render_data(&world, &entities);
        assert_eq!(data.batches.len(), 1);
        assert_eq!(data.batches[0].vertex_count, 3);

        let main_geometry = crate::app::roi_runtime::main_volume_geometry(&world).unwrap();
        let viewport_rect = {
            let mut q = world.query::<&Viewport>();
            q.iter().next().unwrap().1.rect
        };
        let viewport_state = {
            let mut q = world.query::<&ViewportState>();
            *q.iter().next().unwrap().1
        };
        let (main_orientation, main_aspect_ratios) = {
            let mut q = world.query::<&VolumeData>().with::<&MainVolumeTag>();
            let vol = q.iter().next().unwrap().1;
            (vol.orientation, vol.aspect_ratios())
        };
        let uv = crate::convert::world_mm_to_volume_uv(world_vertex, main_geometry);
        let screen_aspect = viewport_rect[2] / viewport_rect[3];

        let composed_projection = ViewProjection {
            zoom: viewport_state.zoom,
            pan: viewport_state.pan,
            pivot: viewport_state.pivot,
            rotation: crate::util::orientation::compose_view_rotation(
                main_orientation,
                viewport_state.user_rotation,
            ),
            aspect_ratios: main_aspect_ratios,
            geometry: main_geometry,
            cursor_pos: [0.5, 0.5, 0.5],
        };
        let composed_uv = world_to_ndc(
            Vec3::from_array(uv),
            ViewMode::ThreeD,
            &composed_projection,
            screen_aspect,
        )
        .unwrap();
        let composed_ndc =
            viewport_uv_to_full_ndc(composed_uv, viewport_rect, [800.0, 600.0]).unwrap();
        assert_eq!(data.vertices[0].position_ndc, composed_ndc);

        let raw_projection = ViewProjection {
            rotation: viewport_state.user_rotation,
            ..composed_projection
        };
        let raw_uv = world_to_ndc(
            Vec3::from_array(uv),
            ViewMode::ThreeD,
            &raw_projection,
            screen_aspect,
        )
        .unwrap();
        let raw_ndc = viewport_uv_to_full_ndc(raw_uv, viewport_rect, [800.0, 600.0]).unwrap();
        assert_ne!(data.vertices[0].position_ndc, raw_ndc);
    }

    #[test]
    fn test_prepare_mesh_render_data_skips_invalid_face_indices() {
        let (mut world, entities) = spawn_world_base();
        world.spawn((
            Viewport {
                mode: ViewMode::ThreeD,
                rect: [0.0, 0.0, 400.0, 300.0],
                uniform_index: 0,
            },
            ViewportState::default(),
        ));
        world.spawn((Roi::new_mesh(
            RoiId(3),
            "Mesh".to_string(),
            MeshData {
                vertices: vec![MeshVertex {
                    world_mm: [1.0, 1.0, 1.0],
                }],
                faces: vec![MeshFace {
                    vertex_indices: [0, 1, 2],
                }],
            },
        ),));
        let data = prepare_mesh_render_data(&world, &entities);
        assert!(data.vertices.is_empty());
    }
}
