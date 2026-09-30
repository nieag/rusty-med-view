use crate::components::*;
use crate::convert::{
    plane_local_mm_to_world_mm, volume_uv_to_viewport_uv, world_mm_to_volume_uv, PlaneDefinition,
    ViewportMapping,
};
use hecs::World;
use wgpu::util::DeviceExt;

const INITIAL_VERTEX_CAPACITY: usize = 128;
const LINE_WIDTH_PX: f32 = 2.0;
const POINT_MARKER_SIZE_PX: f32 = 6.0;

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
    pub batches: Vec<ContourRenderBatch>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContourRenderBatch {
    pub start_vertex: u32,
    pub vertex_count: u32,
    pub scissor_rect: [u32; 4],
}

pub struct ContourRenderer {
    pub pipeline: wgpu::RenderPipeline,
    pub vertex_buffer: wgpu::Buffer,
    pub vertex_capacity: usize,
    pub vertex_count: u32,
    pub batches: Vec<ContourRenderBatch>,
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
        batches: Vec::new(),
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

fn viewport_scissor_rect(viewport_rect: [f32; 4], window_size: [f32; 2]) -> [u32; 4] {
    let [window_w, window_h] = window_size;
    let x0 = viewport_rect[0].clamp(0.0, window_w);
    let y0 = viewport_rect[1].clamp(0.0, window_h);
    let x1 = (viewport_rect[0] + viewport_rect[2]).clamp(0.0, window_w);
    let y1 = (viewport_rect[1] + viewport_rect[3]).clamp(0.0, window_h);
    [
        x0 as u32,
        y0 as u32,
        (x1 - x0).max(0.0) as u32,
        (y1 - y0).max(0.0) as u32,
    ]
}

struct ContourInteraction<'a> {
    active_roi: Option<hecs::Entity>,
    active_tool: EditorTool,
    draft: Option<&'a ContourDraft>,
    selection: Option<&'a ContourSelection>,
}

struct ContourViewportContext<'a> {
    viewport: &'a Viewport,
    viewport_state: &'a ViewportState,
    displayed_plane: PlaneDefinition,
    geometry: VoxelGeometry,
    window_size: [f32; 2],
    line_width_ndc: f32,
    point_size_ndc: f32,
}

fn append_roi_contour_vertices(
    vertices: &mut Vec<ContourVertex>,
    roi_entity: hecs::Entity,
    roi: &Roi,
    roi_color: [f32; 4],
    interaction: &ContourInteraction<'_>,
    context: &ContourViewportContext<'_>,
) {
    let displayed_plane = context.displayed_plane;
    let view_key = ContourViewKey::from_plane(displayed_plane);
    let is_active = interaction.active_roi == Some(roi_entity);
    let preview_contour_data = roi
        .contour_move_preview()
        .filter(|_| is_active)
        .filter(|_| {
            roi.contour_data()
                .is_some_and(|contour| contour.active_plane_family == displayed_plane.family)
        })
        .map(|preview| preview.contour_data.slices.as_slice());
    let contour_data = preview_contour_data.or_else(|| roi.contour_view_data_for_render(&view_key));
    let editable_view = is_active
        && roi
            .contour_data()
            .is_some_and(|contour| contour.active_plane_family == displayed_plane.family);
    let mapping = ViewportMapping {
        zoom: context.viewport_state.zoom,
        pan: context.viewport_state.pan,
        pivot: context.viewport_state.pivot,
        screen_aspect: if context.viewport.rect[3] > 0.0 {
            context.viewport.rect[2] / context.viewport.rect[3]
        } else {
            1.0
        },
    };

    if let Some(contour_slices) = contour_data {
        for (slice_idx, contour_slice) in contour_slices.iter().enumerate() {
            if !crate::convert::planes_are_same_slice(
                displayed_plane,
                contour_slice.plane,
                context.geometry,
            ) {
                continue;
            }

            for (loop_idx, contour_loop) in contour_slice.loops.iter().enumerate() {
                let mut loop_ndc_points = Vec::with_capacity(contour_loop.points.len());
                for point in &contour_loop.points {
                    let world_mm = plane_local_mm_to_world_mm(point.local_mm, contour_slice.plane);
                    let volume_uv = world_mm_to_volume_uv(world_mm, context.geometry);
                    let Some(viewport_uv) = volume_uv_to_viewport_uv(
                        volume_uv,
                        displayed_plane,
                        context.geometry,
                        mapping,
                    ) else {
                        continue;
                    };
                    let Some(ndc) =
                        viewport_uv_to_ndc(viewport_uv, context.viewport.rect, context.window_size)
                    else {
                        continue;
                    };
                    if ndc[0].is_finite() && ndc[1].is_finite() {
                        loop_ndc_points.push(ndc);
                    }
                }

                if loop_ndc_points.len() < 2 {
                    continue;
                }

                let is_selected_loop = editable_view
                    && interaction.selection.is_some_and(|selection| {
                        selection.roi_entity == roi_entity
                            && selection.slice_index == slice_idx
                            && selection.loop_index == loop_idx
                    });
                let loop_color = if is_selected_loop {
                    [1.0, 0.9, 0.2, 1.0]
                } else {
                    roi_color
                };

                vertices.extend(build_polyline_triangles_ndc(
                    &loop_ndc_points,
                    context.line_width_ndc,
                    loop_color,
                ));
                if contour_loop.is_closed {
                    vertices.extend(build_polyline_triangles_ndc(
                        &[
                            loop_ndc_points[loop_ndc_points.len() - 1],
                            loop_ndc_points[0],
                        ],
                        context.line_width_ndc,
                        loop_color,
                    ));
                }

                if editable_view && interaction.active_tool == EditorTool::ContourSelect {
                    for (point_idx, point_ndc) in loop_ndc_points.into_iter().enumerate() {
                        let is_selected_point = interaction.selection.is_some_and(|selection| {
                            selection.roi_entity == roi_entity
                                && selection.slice_index == slice_idx
                                && selection.loop_index == loop_idx
                                && selection.point_index == Some(point_idx)
                        });
                        vertices.extend(build_point_marker_triangles_ndc(
                            point_ndc,
                            if is_selected_point {
                                context.point_size_ndc * 1.4
                            } else {
                                context.point_size_ndc
                            },
                            if is_selected_point {
                                [1.0, 1.0, 0.0, 1.0]
                            } else {
                                loop_color
                            },
                        ));
                    }
                }
            }
        }
    }

    if let Some(draft) = interaction.draft.filter(|draft| {
        editable_view
            && draft.roi_entity == roi_entity
            && draft.plane.family == displayed_plane.family
    }) {
        let mut draft_points_ndc = Vec::with_capacity(draft.points.len());
        for point in &draft.points {
            let world_mm = plane_local_mm_to_world_mm(point.local_mm, draft.plane);
            let volume_uv = world_mm_to_volume_uv(world_mm, context.geometry);
            let Some(viewport_uv) =
                volume_uv_to_viewport_uv(volume_uv, displayed_plane, context.geometry, mapping)
            else {
                continue;
            };
            let Some(ndc) =
                viewport_uv_to_ndc(viewport_uv, context.viewport.rect, context.window_size)
            else {
                continue;
            };
            draft_points_ndc.push(ndc);
        }

        let draft_color = [
            roi_color[0],
            roi_color[1],
            roi_color[2],
            roi_color[3] * 0.85,
        ];
        vertices.extend(build_polyline_triangles_ndc(
            &draft_points_ndc,
            context.line_width_ndc,
            draft_color,
        ));
        for point_ndc in draft_points_ndc {
            vertices.extend(build_point_marker_triangles_ndc(
                point_ndc,
                context.point_size_ndc,
                draft_color,
            ));
        }
    }
}

pub fn prepare_contour_render_data(world: &World, entities: &AppEntities) -> ContourRenderData {
    let (active_roi, active_tool, contour_draft, contour_selection) = world
        .get::<&EditorState>(entities.editor)
        .map(|editor| {
            (
                editor.active_roi,
                editor.active_tool,
                editor.contour_draft.clone(),
                editor.contour_selection.clone(),
            )
        })
        .unwrap_or((None, EditorTool::Navigation, None, None));
    let main_geometry = {
        let mut volume_query = world.query::<&VolumeData>().with::<&MainVolumeTag>();
        volume_query
            .iter()
            .next()
            .and_then(|(_, volume)| volume.geometry)
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
    let mut batches = Vec::new();
    let roi_views = crate::render::roi_views::RoiRenderViews::for_world(
        world,
        crate::render::roi_views::RenderRepresentationRequest {
            active_roi,
            contour_active_only: false,
            ..Default::default()
        },
    );
    let mut contour_entities: Vec<_> = roi_views
        .contour_overlays
        .into_iter()
        .map(|view| view.entity)
        .collect();
    if let Some(active_roi) = active_roi {
        let needs_active_preview = !contour_entities.contains(&active_roi)
            && world
                .get::<&Roi>(active_roi)
                .is_ok_and(|roi| roi.metadata.is_visible)
            && (contour_draft.is_some()
                || world
                    .get::<&Roi>(active_roi)
                    .is_ok_and(|roi| roi.contour_move_preview().is_some()));
        if needs_active_preview {
            contour_entities.insert(0, active_roi);
        }
    }
    let interaction = ContourInteraction {
        active_roi,
        active_tool,
        draft: contour_draft.as_ref(),
        selection: contour_selection.as_ref(),
    };

    for (_, (viewport, viewport_state)) in world.query::<(&Viewport, &ViewportState)>().iter() {
        if viewport.mode == ViewMode::ThreeD {
            continue;
        }

        let batch_start = vertices.len();
        for roi_entity in &contour_entities {
            let Ok(roi) = world.get::<&Roi>(*roi_entity) else {
                continue;
            };
            let Some(geometry) =
                main_geometry.or_else(|| roi.voxel_cache().map(|cache| cache.data.geometry))
            else {
                continue;
            };
            let Some(displayed_plane) = crate::render::roi_views::displayed_plane_for_viewport(
                viewport.mode,
                cursor_uv,
                viewport_state.user_rotation,
                geometry,
            ) else {
                continue;
            };
            // Outlines are opaque: layer opacity controls the voxel fill, not the contour.
            let roi_color = roi.metadata.color;
            append_roi_contour_vertices(
                &mut vertices,
                *roi_entity,
                &roi,
                roi_color,
                &interaction,
                &ContourViewportContext {
                    viewport,
                    viewport_state,
                    displayed_plane,
                    geometry,
                    window_size,
                    line_width_ndc,
                    point_size_ndc,
                },
            );
        }

        let batch_count = vertices.len() - batch_start;
        if batch_count > 0 {
            let scissor_rect = viewport_scissor_rect(viewport.rect, window_size);
            if scissor_rect[2] > 0 && scissor_rect[3] > 0 {
                batches.push(ContourRenderBatch {
                    start_vertex: batch_start as u32,
                    vertex_count: batch_count as u32,
                    scissor_rect,
                });
            } else {
                vertices.truncate(batch_start);
            }
        }
    }

    ContourRenderData { vertices, batches }
}

pub fn upload_contour_render_data(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut ContourRenderer,
    data: &ContourRenderData,
) {
    renderer.vertex_count = data.vertices.len() as u32;
    renderer.batches = data.batches.clone();
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
    if renderer.vertex_count == 0 || renderer.batches.is_empty() {
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
    for batch in &renderer.batches {
        pass.set_scissor_rect(
            batch.scissor_rect[0],
            batch.scissor_rect[1],
            batch.scissor_rect[2],
            batch.scissor_rect[3],
        );
        pass.draw(
            batch.start_vertex..batch.start_vertex + batch.vertex_count,
            0..1,
        );
    }
}

#[cfg(test)]
mod tests;
