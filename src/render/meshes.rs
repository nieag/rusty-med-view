use crate::components::{AppEntities, MeshData, Roi, ViewMode, Viewport, ViewportState};
use crate::convert::{ChunkedMeshData, MeshChunkKey};
use crate::render::geometry::{
    build_display_projection_context, project_world_mm_to_viewport_uv_3d, DisplayProjectionContext,
};
use crate::render::roi_views::{RenderRepresentationRequest, RoiRenderViews};
use hecs::{Entity, World};
use std::collections::{HashMap, HashSet};

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
    pub key: MeshRenderChunkKey,
    pub vertex_count: u32,
    pub scissor_rect: [u32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MeshRenderPartKey {
    Full,
    Chunk(MeshChunkKey),
    Handle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MeshRenderChunkKey {
    pub roi_entity: Entity,
    pub viewport_entity: Entity,
    pub part: MeshRenderPartKey,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MeshRenderChunkData {
    pub key: MeshRenderChunkKey,
    pub vertices: Vec<MeshVertex2d>,
    pub scissor_rect: [u32; 4],
}

#[derive(Default, Debug, Clone)]
pub struct MeshRenderData {
    pub chunks: Vec<MeshRenderChunkData>,
}

impl MeshRenderData {
    pub fn batch_count(&self) -> usize {
        self.chunks.len()
    }
}

struct GpuMeshChunk {
    vertex_buffer: wgpu::Buffer,
    vertex_capacity: usize,
    vertices: Vec<MeshVertex2d>,
}

pub struct MeshRenderer {
    pub pipeline: wgpu::RenderPipeline,
    chunks: HashMap<MeshRenderChunkKey, GpuMeshChunk>,
    pub batches: Vec<MeshRenderBatch>,
    pub uploaded_chunks_last_frame: u32,
    pub reused_chunks_last_frame: u32,
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

    MeshRenderer {
        pipeline,
        chunks: HashMap::new(),
        batches: Vec::new(),
        uploaded_chunks_last_frame: 0,
        reused_chunks_last_frame: 0,
    }
}

pub fn prepare_mesh_render_data(world: &World, entities: &AppEntities) -> MeshRenderData {
    let mut data = MeshRenderData::default();
    let roi_views = RoiRenderViews::for_world(world, RenderRepresentationRequest::default());
    let (mesh_preview, mesh_selection, active_tool) = world
        .get::<&crate::components::EditorState>(entities.editor)
        .ok()
        .map(|editor| {
            (
                editor.mesh_edit_preview().cloned(),
                editor.mesh_selection,
                editor.active_tool,
            )
        })
        .unwrap_or((None, None, crate::components::EditorTool::Navigation));
    for (viewport_entity, (viewport, viewport_state)) in
        world.query::<(&Viewport, &ViewportState)>().iter()
    {
        if viewport.mode != ViewMode::ThreeD {
            continue;
        }
        let Some(projection_ctx) =
            build_display_projection_context(world, entities, viewport, viewport_state)
        else {
            continue;
        };

        let scissor =
            viewport_scissor_rect(projection_ctx.viewport_rect, projection_ctx.window_size);
        if scissor[2] == 0 || scissor[3] == 0 {
            continue;
        }

        for mesh_view in &roi_views.mesh_overlays {
            let Ok(roi) = world.get::<&Roi>(mesh_view.entity) else {
                continue;
            };
            if let Some(preview) = mesh_preview
                .as_ref()
                .filter(|preview| preview.roi_entity == mesh_view.entity)
            {
                append_render_chunk(
                    mesh_view.entity,
                    viewport_entity,
                    MeshRenderPartKey::Full,
                    &preview.mesh_data,
                    roi.metadata.color,
                    projection_ctx,
                    scissor,
                    &mut data,
                );
            } else if let Some(chunked) = chunked_mesh_for_render(&roi) {
                for chunk in &chunked.chunks {
                    append_render_chunk(
                        mesh_view.entity,
                        viewport_entity,
                        MeshRenderPartKey::Chunk(chunk.key),
                        &chunk.data,
                        roi.metadata.color,
                        projection_ctx,
                        scissor,
                        &mut data,
                    );
                }
            } else if let Some(mesh) = mesh_data_for_render(&roi) {
                append_render_chunk(
                    mesh_view.entity,
                    viewport_entity,
                    MeshRenderPartKey::Full,
                    mesh,
                    roi.metadata.color,
                    projection_ctx,
                    scissor,
                    &mut data,
                );
            }
        }
        if active_tool == crate::components::EditorTool::MeshDeform {
            if let Some(selection) = mesh_selection {
                if let Ok(roi) = world.get::<&Roi>(selection.roi_entity) {
                    if roi.metadata.is_visible {
                        let mesh = mesh_preview
                            .as_ref()
                            .filter(|preview| preview.roi_entity == selection.roi_entity)
                            .map(|preview| &preview.mesh_data)
                            .or_else(|| roi.mesh_data());
                        if let Some(world_mm) = mesh.and_then(|mesh| {
                            mesh.vertices
                                .get(selection.vertex_index)
                                .map(|vertex| vertex.world_mm)
                        }) {
                            append_mesh_handle(
                                selection.roi_entity,
                                viewport_entity,
                                world_mm,
                                projection_ctx,
                                scissor,
                                &mut data,
                            );
                        }
                    }
                }
            }
        }
    }

    data
}

fn mesh_data_for_render(roi: &Roi) -> Option<&MeshData> {
    crate::render::roi_views::mesh_data_for_adapter(roi)
}

fn chunked_mesh_for_render(roi: &Roi) -> Option<&ChunkedMeshData> {
    if roi.preview_state.active {
        if let Some(preview) = roi.session_caches.preview_mesh.as_ref().filter(|cache| {
            cache.source_generation == roi.dirty_state.generations.authoritative
                && cache.preview_revision == roi.preview_state.revision
        }) {
            return preview.chunks.as_ref();
        }
    }
    if matches!(
        roi.authoritative_data,
        crate::components::RoiAuthoritativeData::Mesh(_)
    ) {
        return None;
    }
    roi.is_cache_current(crate::components::RoiCacheKind::Mesh)
        .then(|| roi.mesh_cache().and_then(|cache| cache.chunks.as_ref()))
        .flatten()
}

#[allow(clippy::too_many_arguments)]
fn append_render_chunk(
    roi_entity: Entity,
    viewport_entity: Entity,
    part: MeshRenderPartKey,
    mesh: &MeshData,
    color: [f32; 4],
    projection_ctx: DisplayProjectionContext,
    scissor_rect: [u32; 4],
    data: &mut MeshRenderData,
) {
    let mut vertices = Vec::with_capacity(mesh.faces.len().saturating_mul(3));
    append_projected_mesh(mesh, color, projection_ctx, &mut vertices);
    if vertices.is_empty() {
        return;
    }
    data.chunks.push(MeshRenderChunkData {
        key: MeshRenderChunkKey {
            roi_entity,
            viewport_entity,
            part,
        },
        vertices,
        scissor_rect,
    });
}

fn append_mesh_handle(
    roi_entity: Entity,
    viewport_entity: Entity,
    world_mm: [f32; 3],
    projection_ctx: DisplayProjectionContext,
    scissor_rect: [u32; 4],
    data: &mut MeshRenderData,
) {
    let Some(center) = project_world_vertex(world_mm, projection_ctx) else {
        return;
    };
    let [window_width, window_height] = projection_ctx.window_size;
    if window_width <= 0.0 || window_height <= 0.0 {
        return;
    }
    let half_size_ndc = [12.0 / window_width, 12.0 / window_height];
    let [dx, dy] = half_size_ndc;
    let color = [1.0, 0.85, 0.1, 1.0];
    let corners = [
        [center[0] - dx, center[1] - dy],
        [center[0] + dx, center[1] - dy],
        [center[0] + dx, center[1] + dy],
        [center[0] - dx, center[1] + dy],
    ];
    let vertices = [0, 1, 2, 0, 2, 3]
        .into_iter()
        .map(|index| MeshVertex2d {
            position_ndc: corners[index],
            color,
        })
        .collect();
    data.chunks.push(MeshRenderChunkData {
        key: MeshRenderChunkKey {
            roi_entity,
            viewport_entity,
            part: MeshRenderPartKey::Handle,
        },
        vertices,
        scissor_rect,
    });
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
    let viewport_uv = project_world_mm_to_viewport_uv_3d(world_mm, projection_ctx)?;
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
    renderer.batches.clear();
    renderer.uploaded_chunks_last_frame = 0;
    renderer.reused_chunks_last_frame = 0;
    let mut live_keys = HashSet::with_capacity(data.chunks.len());

    for chunk in &data.chunks {
        live_keys.insert(chunk.key);
        let required_capacity = mesh_vertex_capacity(chunk.vertices.len());
        let gpu_chunk = renderer
            .chunks
            .entry(chunk.key)
            .or_insert_with(|| GpuMeshChunk {
                vertex_buffer: create_mesh_vertex_buffer(device, required_capacity),
                vertex_capacity: required_capacity,
                vertices: Vec::new(),
            });
        if chunk.vertices.len() > gpu_chunk.vertex_capacity {
            gpu_chunk.vertex_capacity = required_capacity;
            gpu_chunk.vertex_buffer = create_mesh_vertex_buffer(device, required_capacity);
        }
        if gpu_chunk.vertices != chunk.vertices {
            queue.write_buffer(
                &gpu_chunk.vertex_buffer,
                0,
                bytemuck::cast_slice(&chunk.vertices),
            );
            gpu_chunk.vertices.clone_from(&chunk.vertices);
            renderer.uploaded_chunks_last_frame += 1;
        } else {
            renderer.reused_chunks_last_frame += 1;
        }
        renderer.batches.push(MeshRenderBatch {
            key: chunk.key,
            vertex_count: chunk.vertices.len() as u32,
            scissor_rect: chunk.scissor_rect,
        });
    }
    renderer.chunks.retain(|key, _| live_keys.contains(key));
}

fn create_mesh_vertex_buffer(device: &wgpu::Device, vertex_capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Mesh Overlay Chunk Vertex Buffer"),
        size: (vertex_capacity * std::mem::size_of::<MeshVertex2d>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn mesh_vertex_capacity(vertex_count: usize) -> usize {
    vertex_count
        .max(INITIAL_VERTEX_CAPACITY)
        .next_power_of_two()
}

pub fn render_meshes(
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    renderer: &MeshRenderer,
) {
    if renderer.batches.is_empty() {
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
    for batch in &renderer.batches {
        if batch.vertex_count == 0 || batch.scissor_rect[2] == 0 || batch.scissor_rect[3] == 0 {
            continue;
        }
        let Some(chunk) = renderer.chunks.get(&batch.key) else {
            continue;
        };
        pass.set_vertex_buffer(0, chunk.vertex_buffer.slice(..));
        pass.set_scissor_rect(
            batch.scissor_rect[0],
            batch.scissor_rect[1],
            batch.scissor_rect[2],
            batch.scissor_rect[3],
        );
        pass.draw(0..batch.vertex_count, 0..1);
    }
}

#[cfg(test)]
mod tests;
