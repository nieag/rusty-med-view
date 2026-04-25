use crate::components::*;
use crate::convert::{
    oblique_plane_from_view_rotation, orthogonal_plane_from_volume_uv, plane_local_mm_to_world_mm,
    volume_uv_to_viewport_uv, world_mm_to_volume_uv, PlaneDefinition, PlaneFamily, ViewportMapping,
};
use hecs::World;
use wgpu::util::DeviceExt;

const INITIAL_VERTEX_CAPACITY: usize = 128;
const LINE_WIDTH_PX: f32 = 2.0;
const POINT_MARKER_SIZE_PX: f32 = 6.0;
const PLANE_ORIGIN_TOLERANCE_MM: f32 = 0.5;
const PLANE_NORMAL_ALIGNMENT_COS: f32 = 0.999;

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

fn build_point_marker_triangles_ndc(
    center_ndc: [f32; 2],
    size_ndc: f32,
    color: [f32; 4],
) -> Vec<ContourVertex> {
    if size_ndc <= 0.0 {
        return Vec::new();
    }

    let half = size_ndc * 0.5;
    let [x, y] = center_ndc;
    let v0 = [x - half, y - half];
    let v1 = [x + half, y - half];
    let v2 = [x + half, y + half];
    let v3 = [x - half, y + half];

    vec![
        ContourVertex {
            position_ndc: v0,
            color,
        },
        ContourVertex {
            position_ndc: v1,
            color,
        },
        ContourVertex {
            position_ndc: v2,
            color,
        },
        ContourVertex {
            position_ndc: v2,
            color,
        },
        ContourVertex {
            position_ndc: v3,
            color,
        },
        ContourVertex {
            position_ndc: v0,
            color,
        },
    ]
}

fn normalized(v: [f32; 3]) -> Option<glam::Vec3> {
    let vec = glam::Vec3::from_array(v);
    let len_sq = vec.length_squared();
    if !len_sq.is_finite() || len_sq <= 1e-12 {
        None
    } else {
        Some(vec / len_sq.sqrt())
    }
}

fn planes_are_slice_compatible(displayed: PlaneDefinition, stored: PlaneDefinition) -> bool {
    if displayed.family != stored.family {
        return false;
    }

    let Some(displayed_normal) = normalized(displayed.normal_mm) else {
        return false;
    };
    let Some(stored_normal) = normalized(stored.normal_mm) else {
        return false;
    };

    if displayed_normal.dot(stored_normal).abs() < PLANE_NORMAL_ALIGNMENT_COS {
        return false;
    }

    let displayed_origin = glam::Vec3::from_array(displayed.origin_mm);
    let stored_origin = glam::Vec3::from_array(stored.origin_mm);
    let signed_distance = (stored_origin - displayed_origin)
        .dot(displayed_normal)
        .abs();
    signed_distance <= PLANE_ORIGIN_TOLERANCE_MM
}

fn displayed_plane_for_viewport(
    mode: ViewMode,
    cursor_uv: [f32; 3],
    user_rotation: [f32; 4],
    geometry: VoxelGeometry,
) -> Option<PlaneDefinition> {
    match mode {
        ViewMode::Axial => orthogonal_plane_from_volume_uv(PlaneFamily::Axial, cursor_uv, geometry),
        ViewMode::Coronal => {
            orthogonal_plane_from_volume_uv(PlaneFamily::Coronal, cursor_uv, geometry)
        }
        ViewMode::Sagittal => {
            orthogonal_plane_from_volume_uv(PlaneFamily::Sagittal, cursor_uv, geometry)
        }
        ViewMode::Oblique => oblique_plane_from_view_rotation(cursor_uv, user_rotation, geometry),
        ViewMode::ThreeD => None,
    }
}

fn viewport_uv_to_ndc(
    viewport_uv: [f32; 2],
    viewport_rect: [f32; 4],
    window_size: [f32; 2],
) -> Option<[f32; 2]> {
    let [window_w, window_h] = window_size;
    if window_w <= 0.0 || window_h <= 0.0 {
        return None;
    }

    let screen_x = viewport_rect[0] + viewport_uv[0] * viewport_rect[2];
    let screen_y = viewport_rect[1] + viewport_uv[1] * viewport_rect[3];
    Some([
        (screen_x / window_w) * 2.0 - 1.0,
        1.0 - (screen_y / window_h) * 2.0,
    ])
}

pub fn prepare_contour_render_data(world: &World, entities: &AppEntities) -> ContourRenderData {
    let active_roi = match world
        .get::<&EditorState>(entities.editor)
        .ok()
        .and_then(|editor| editor.active_roi)
    {
        Some(active_roi) => active_roi,
        None => return ContourRenderData::default(),
    };

    let roi = match world.get::<&Roi>(active_roi) {
        Ok(roi) => roi,
        Err(_) => return ContourRenderData::default(),
    };
    if !roi.metadata.is_visible {
        return ContourRenderData::default();
    }
    let Some(contour_data) = roi.contour_data() else {
        return ContourRenderData::default();
    };

    let geometry = {
        let mut volume_query = world.query::<&VolumeData>().with::<&MainVolumeTag>();
        let Some((_, volume)) = volume_query.iter().next() else {
            return ContourRenderData::default();
        };
        VoxelGeometry {
            dimensions: volume.dimensions,
            spacing: volume.spacing,
            origin: volume.origin,
            orientation: volume.orientation,
        }
    };

    let cursor_uv = world
        .get::<&Transform>(entities.cursor)
        .map(|cursor| cursor.position)
        .unwrap_or([0.5, 0.5, 0.5]);

    let window_size = world
        .get::<&WindowSettings>(entities.window_settings)
        .map(|settings| [settings.width as f32, settings.height as f32])
        .unwrap_or([1.0, 1.0]);

    let px_to_ndc = (2.0 / window_size[0].max(1.0)).min(2.0 / window_size[1].max(1.0));
    let line_width_ndc = LINE_WIDTH_PX * px_to_ndc;
    let point_size_ndc = POINT_MARKER_SIZE_PX * px_to_ndc;

    let mut vertices = Vec::new();
    let roi_color = roi.metadata.color;

    for (_, (viewport, viewport_state)) in world.query::<(&Viewport, &ViewportState)>().iter() {
        if viewport.mode == ViewMode::ThreeD {
            continue;
        }

        let Some(displayed_plane) = displayed_plane_for_viewport(
            viewport.mode,
            cursor_uv,
            viewport_state.user_rotation,
            geometry,
        ) else {
            continue;
        };
        if displayed_plane.family != contour_data.active_plane_family {
            continue;
        }

        let mapping = ViewportMapping {
            zoom: viewport_state.zoom,
            pan: viewport_state.pan,
            pivot: viewport_state.pivot,
            screen_aspect: if viewport.rect[3] > 0.0 {
                viewport.rect[2] / viewport.rect[3]
            } else {
                1.0
            },
        };

        for contour_slice in &contour_data.slices {
            if !planes_are_slice_compatible(displayed_plane, contour_slice.plane) {
                continue;
            }

            for contour_loop in &contour_slice.loops {
                let mut loop_ndc_points = Vec::with_capacity(contour_loop.points.len());
                for point in &contour_loop.points {
                    let world_mm = plane_local_mm_to_world_mm(point.local_mm, contour_slice.plane);
                    let volume_uv = world_mm_to_volume_uv(world_mm, geometry);
                    let Some(viewport_uv) =
                        volume_uv_to_viewport_uv(volume_uv, displayed_plane, geometry, mapping)
                    else {
                        continue;
                    };
                    let Some(ndc) = viewport_uv_to_ndc(viewport_uv, viewport.rect, window_size)
                    else {
                        continue;
                    };
                    if !ndc[0].is_finite() || !ndc[1].is_finite() {
                        continue;
                    }
                    loop_ndc_points.push(ndc);
                }

                if loop_ndc_points.len() < 2 {
                    continue;
                }

                vertices.extend(build_polyline_triangles_ndc(
                    &loop_ndc_points,
                    line_width_ndc,
                    roi_color,
                ));
                if contour_loop.is_closed {
                    let closing = [
                        loop_ndc_points[loop_ndc_points.len() - 1],
                        loop_ndc_points[0],
                    ];
                    vertices.extend(build_polyline_triangles_ndc(
                        &closing,
                        line_width_ndc,
                        roi_color,
                    ));
                }

                for point_ndc in loop_ndc_points {
                    vertices.extend(build_point_marker_triangles_ndc(
                        point_ndc,
                        point_size_ndc,
                        roi_color,
                    ));
                }
            }
        }
    }

    ContourRenderData { vertices }
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
    use crate::convert::orthogonal_plane_from_volume_uv;

    fn approx_eq(a: [f32; 2], b: [f32; 2], epsilon: f32) -> bool {
        (a[0] - b[0]).abs() <= epsilon && (a[1] - b[1]).abs() <= epsilon
    }

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
    fn test_build_point_marker_triangles_produces_quad_geometry() {
        let color = [0.2, 0.8, 0.4, 1.0];
        let vertices = build_point_marker_triangles_ndc([0.0, 0.0], 0.2, color);
        assert_eq!(vertices.len(), 6);
        assert_eq!(vertices[0].position_ndc, [-0.1, -0.1]);
        assert_eq!(vertices[2].position_ndc, [0.1, 0.1]);
        assert!(vertices.iter().all(|vertex| vertex.color == color));
    }

    #[test]
    fn test_planes_are_slice_compatible_respects_origin_tolerance() {
        let displayed = PlaneDefinition {
            family: PlaneFamily::Axial,
            origin_mm: [0.0, 0.0, 10.0],
            u_axis_mm: [1.0, 0.0, 0.0],
            v_axis_mm: [0.0, 1.0, 0.0],
            normal_mm: [0.0, 0.0, 1.0],
        };
        let near = PlaneDefinition {
            origin_mm: [0.0, 0.0, 10.4],
            ..displayed
        };
        let far = PlaneDefinition {
            origin_mm: [0.0, 0.0, 10.8],
            ..displayed
        };

        assert!(planes_are_slice_compatible(displayed, near));
        assert!(!planes_are_slice_compatible(displayed, far));
    }

    #[test]
    fn test_projection_helper_local_world_viewport_roundtrip_stays_stable() {
        let geometry = VoxelGeometry {
            dimensions: [16, 16, 16],
            spacing: [1.0, 1.0, 1.0],
            origin: [0.0, 0.0, 0.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let plane =
            orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 0.5], geometry).unwrap();
        let mapping = ViewportMapping {
            zoom: 1.0,
            pan: [0.0, 0.0],
            pivot: [0.5, 0.5],
            screen_aspect: 1.0,
        };
        let local = [2.0, -1.5];
        let world = plane_local_mm_to_world_mm(local, plane);
        let uv = world_mm_to_volume_uv(world, geometry);
        let viewport_uv = volume_uv_to_viewport_uv(uv, plane, geometry, mapping).unwrap();
        let world_roundtrip = plane_local_mm_to_world_mm(local, plane);
        let uv_roundtrip = world_mm_to_volume_uv(world_roundtrip, geometry);
        let viewport_uv_roundtrip =
            volume_uv_to_viewport_uv(uv_roundtrip, plane, geometry, mapping).unwrap();
        assert!(approx_eq(viewport_uv, viewport_uv_roundtrip, 1e-6));
    }

    #[test]
    fn test_prepare_contour_render_data_is_empty_when_no_active_contour_roi() {
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

    #[test]
    fn test_prepare_contour_render_data_emits_vertices_for_matching_slice() {
        let mut world = World::new();
        let cursor = world.spawn((Transform {
            position: [0.5, 0.5, 0.5],
        },));
        let window_settings = world.spawn((WindowSettings {
            width: 800,
            height: 600,
            viewport_rect: [0.0, 0.0, 800.0, 600.0],
        },));
        let editor = world.spawn((EditorState {
            active_roi: None,
            active_tool: EditorTool::Navigation,
        },));
        let viewport = world.spawn((
            Viewport {
                mode: ViewMode::Axial,
                rect: [0.0, 0.0, 800.0, 600.0],
                uniform_index: 0,
            },
            ViewportState {
                zoom: 1.0,
                pan: [0.0, 0.0],
                pivot: [0.5, 0.5],
                user_rotation: [0.0, 0.0, 0.0, 1.0],
            },
        ));
        let _ = viewport;

        world.spawn((
            VolumeData {
                dimensions: [32, 32, 32],
                spacing: [1.0, 1.0, 1.0],
                origin: [0.0, 0.0, 0.0],
                intensities: vec![],
                intensity_range: [0.0, 1.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            MainVolumeTag,
        ));

        let geometry = VoxelGeometry {
            dimensions: [32, 32, 32],
            spacing: [1.0, 1.0, 1.0],
            origin: [0.0, 0.0, 0.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let plane =
            orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 0.5], geometry).unwrap();
        let roi = Roi::new_contour(
            RoiId(1),
            "Contour".to_string(),
            ContourData {
                active_plane_family: PlaneFamily::Axial,
                slices: vec![ContourSlice {
                    plane,
                    loops: vec![ContourLoop {
                        points: vec![
                            ContourPoint {
                                local_mm: [0.0, 0.0],
                            },
                            ContourPoint {
                                local_mm: [3.0, 0.0],
                            },
                            ContourPoint {
                                local_mm: [3.0, 3.0],
                            },
                        ],
                        is_closed: true,
                    }],
                }],
            },
        );
        let roi_entity = world.spawn((roi, LayerSettings { opacity: 1.0 }, RoiTag));
        world.get::<&mut EditorState>(editor).unwrap().active_roi = Some(roi_entity);

        let entities = AppEntities {
            input: hecs::Entity::DANGLING,
            editor,
            gui_state: hecs::Entity::DANGLING,
            volume_windowing: hecs::Entity::DANGLING,
            annotations: hecs::Entity::DANGLING,
            overlay: hecs::Entity::DANGLING,
            protocol: hecs::Entity::DANGLING,
            cursor,
            window_settings,
        };

        let data = prepare_contour_render_data(&world, &entities);
        assert!(!data.vertices.is_empty());
    }
}
