use crate::components::AppEntities;
use hecs::World;
use wgpu::util::DeviceExt;

const INITIAL_VERTEX_CAPACITY: usize = 128;

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, PartialEq)]
pub struct ContourVertex {
    pub position_ndc: [f32; 2],
    pub color: [f32; 4],
}

impl ContourVertex {
    pub fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<ContourVertex>() as wgpu::BufferAddress,
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

#[derive(Default, Debug, Clone)]
pub struct ContourRenderData {
    pub vertices: Vec<ContourVertex>,
}

pub struct ContourRenderer {
    pub pipeline: wgpu::RenderPipeline,
    pub vertex_buffer: wgpu::Buffer,
    pub vertex_capacity: usize,
    pub vertex_count: u32,
}

pub fn create_contour_renderer(
    device: &wgpu::Device,
    surface_format: wgpu::TextureFormat,
) -> ContourRenderer {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Contour Overlay Shader"),
        source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
            "../shaders/contour_overlay.wgsl"
        ))),
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Contour Overlay Pipeline Layout"),
        bind_group_layouts: &[],
        push_constant_ranges: &[],
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Contour Overlay Pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[ContourVertex::desc()],
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
        label: Some("Contour Overlay Vertex Buffer"),
        contents: bytemuck::cast_slice(&vec![
            ContourVertex {
                position_ndc: [0.0, 0.0],
                color: [0.0, 0.0, 0.0, 0.0],
            };
            INITIAL_VERTEX_CAPACITY
        ]),
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
    });

    ContourRenderer {
        pipeline,
        vertex_buffer,
        vertex_capacity: INITIAL_VERTEX_CAPACITY,
        vertex_count: 0,
    }
}

pub fn build_polyline_triangles_ndc(
    points: &[[f32; 2]],
    thickness_ndc: f32,
    color: [f32; 4],
) -> Vec<ContourVertex> {
    if points.len() < 2 || thickness_ndc <= 0.0 {
        return Vec::new();
    }

    let mut out = Vec::with_capacity((points.len() - 1) * 6);
    let half = thickness_ndc * 0.5;

    for segment in points.windows(2) {
        let p0 = glam::Vec2::from_array(segment[0]);
        let p1 = glam::Vec2::from_array(segment[1]);
        let dir = p1 - p0;
        let len_sq = dir.length_squared();
        if len_sq <= 1e-12 {
            continue;
        }
        let tangent = dir / len_sq.sqrt();
        let normal = glam::Vec2::new(-tangent.y, tangent.x) * half;

        let v0 = p0 + normal;
        let v1 = p0 - normal;
        let v2 = p1 + normal;
        let v3 = p1 - normal;

        out.push(ContourVertex {
            position_ndc: v0.to_array(),
            color,
        });
        out.push(ContourVertex {
            position_ndc: v1.to_array(),
            color,
        });
        out.push(ContourVertex {
            position_ndc: v2.to_array(),
            color,
        });

        out.push(ContourVertex {
            position_ndc: v2.to_array(),
            color,
        });
        out.push(ContourVertex {
            position_ndc: v1.to_array(),
            color,
        });
        out.push(ContourVertex {
            position_ndc: v3.to_array(),
            color,
        });
    }

    out
}

pub fn prepare_contour_render_data(_world: &World, _entities: &AppEntities) -> ContourRenderData {
    // Step 6C only wires native renderer infrastructure.
    // Semantic contour-to-vertex preparation starts in Step 6D.
    ContourRenderData::default()
}

pub fn upload_contour_render_data(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut ContourRenderer,
    data: &ContourRenderData,
) {
    renderer.vertex_count = data.vertices.len() as u32;
    if data.vertices.is_empty() {
        return;
    }

    if data.vertices.len() > renderer.vertex_capacity {
        renderer.vertex_capacity = data.vertices.len().next_power_of_two();
        renderer.vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Contour Overlay Vertex Buffer"),
            size: (renderer.vertex_capacity * std::mem::size_of::<ContourVertex>()) as u64,
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

pub fn render_contours(
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    renderer: &ContourRenderer,
) {
    if renderer.vertex_count == 0 {
        return;
    }

    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Contour Overlay Pass"),
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
    pass.draw(0..renderer.vertex_count, 0..1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_polyline_triangles_produces_expected_geometry_for_horizontal_segment() {
        let points = [[0.0, 0.0], [1.0, 0.0]];
        let color = [0.9, 0.2, 0.2, 0.8];
        let vertices = build_polyline_triangles_ndc(&points, 0.2, color);

        assert_eq!(vertices.len(), 6);
        assert_eq!(vertices[0].position_ndc, [0.0, 0.1]);
        assert_eq!(vertices[1].position_ndc, [0.0, -0.1]);
        assert_eq!(vertices[2].position_ndc, [1.0, 0.1]);
        assert_eq!(vertices[3].position_ndc, [1.0, 0.1]);
        assert_eq!(vertices[4].position_ndc, [0.0, -0.1]);
        assert_eq!(vertices[5].position_ndc, [1.0, -0.1]);
        assert!(vertices.iter().all(|vertex| vertex.color == color));
    }

    #[test]
    fn test_prepare_contour_render_data_is_empty_noop_for_step_6c() {
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

        let data = prepare_contour_render_data(&world, &entities);
        assert!(data.vertices.is_empty());
    }
}
