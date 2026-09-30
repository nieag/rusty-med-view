use crate::components::{MeshData, Roi, Session, ViewMode, Viewport, ViewportState};
use crate::convert::{ChunkedMeshData, MeshChunkKey};
use crate::render::geometry::{
    build_display_projection_context, project_world_mm_to_viewport_uv_3d, DisplayProjectionContext,
};
use crate::render::roi_views::{RenderRepresentationRequest, RoiRenderViews};
use hecs::{Entity, World};
use std::collections::HashMap;

const INITIAL_VERTEX_CAPACITY: usize = 256;
const INITIAL_INDEX_CAPACITY: usize = 768;
/// Bytes between per-draw uniform slots (the WebGPU minimum uniform offset alignment).
const DRAW_UNIFORM_STRIDE: u64 = 256;
/// Distance in millimetres used to read the affine screen projection back out of the
/// (exactly affine) orthographic projection.
const PROJECTION_PROBE_MM: f32 = 100.0;
const HANDLE_COLOR: [f32; 4] = [1.0, 0.85, 0.1, 1.0];
/// The 3D mesh is a translucent shell so the volume stays readable through it.
const MESH_ALPHA_SCALE: f32 = 0.35;

/// One uniform block per draw. `row0` and `row1` map a vertex position `(x, y, z, 1)` to the
/// x and y of the full-window NDC position, so a camera move only rewrites 48 bytes per draw
/// instead of re-projecting and re-uploading the mesh.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, PartialEq)]
pub struct MeshDrawUniform {
    pub row0: [f32; 4],
    pub row1: [f32; 4],
    pub color: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MeshRenderPartKey {
    Full,
    Chunk(MeshChunkKey),
    /// The selected-vertex marker. It is stored in NDC of one viewport, so it is per viewport.
    Handle(Entity),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MeshPartKey {
    pub roi_entity: Entity,
    pub part: MeshRenderPartKey,
}

/// Indexed triangles of one part, in the space its draws' transforms expect.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshGeometry {
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
}

/// A part whose geometry changed since the renderer last saw it. Parts that did not change are
/// not listed with geometry and are reused from the GPU.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshPartUpdate {
    pub key: MeshPartKey,
    pub fingerprint: u64,
    pub geometry: Option<MeshGeometry>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshDraw {
    pub key: MeshPartKey,
    pub viewport_entity: Entity,
    pub uniform: MeshDrawUniform,
    pub scissor_rect: [u32; 4],
}

#[derive(Default, Debug, Clone)]
pub struct MeshRenderData {
    pub parts: Vec<MeshPartUpdate>,
    pub draws: Vec<MeshDraw>,
}

impl MeshRenderData {
    pub fn batch_count(&self) -> usize {
        self.draws.len()
    }
}

struct GpuMeshPart {
    fingerprint: u64,
    vertex_buffer: wgpu::Buffer,
    vertex_capacity: usize,
    index_buffer: wgpu::Buffer,
    index_capacity: usize,
    index_count: u32,
}

pub struct MeshRenderer {
    pub pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
    uniform_capacity: usize,
    bind_group: wgpu::BindGroup,
    parts: HashMap<MeshPartKey, GpuMeshPart>,
    draws: Vec<MeshDraw>,
    pub uploaded_chunks_last_frame: u32,
    pub reused_chunks_last_frame: u32,
}

impl MeshRenderer {
    /// What the renderer already holds for each part, so a frame only builds changed geometry.
    pub fn fingerprints(&self) -> HashMap<MeshPartKey, u64> {
        self.parts
            .iter()
            .map(|(key, part)| (*key, part.fingerprint))
            .collect()
    }
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

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Mesh Overlay Draw Layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(
                    std::mem::size_of::<MeshDrawUniform>() as u64
                ),
            },
            count: None,
        }],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Mesh Overlay Pipeline Layout"),
        bind_group_layouts: &[&bind_group_layout],
        push_constant_ranges: &[],
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Mesh Overlay Pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                }],
            }],
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

    let uniform_capacity = 64;
    let uniform_buffer = create_uniform_buffer(device, uniform_capacity);
    let bind_group = create_draw_bind_group(device, &bind_group_layout, &uniform_buffer);
    MeshRenderer {
        pipeline,
        bind_group_layout,
        uniform_buffer,
        uniform_capacity,
        bind_group,
        parts: HashMap::new(),
        draws: Vec::new(),
        uploaded_chunks_last_frame: 0,
        reused_chunks_last_frame: 0,
    }
}

fn create_uniform_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Mesh Overlay Draw Uniforms"),
        size: capacity as u64 * DRAW_UNIFORM_STRIDE,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn create_draw_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Mesh Overlay Draw Bind Group"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer,
                offset: 0,
                size: wgpu::BufferSize::new(std::mem::size_of::<MeshDrawUniform>() as u64),
            }),
        }],
    })
}

/// Collects what to draw this frame. Geometry is built only for parts whose fingerprint differs
/// from `known` (what the renderer already holds); everything else is one uniform per viewport.
pub fn prepare_mesh_render_data(
    world: &World,
    session: &Session,
    known: &HashMap<MeshPartKey, u64>,
) -> MeshRenderData {
    let mut data = MeshRenderData::default();
    let roi_views = RoiRenderViews::for_world(world, RenderRepresentationRequest::default());
    let (mesh_selection, active_tool) = (session.editor.mesh_selection, session.editor.active_tool);

    let mut viewports = Vec::new();
    for (viewport_entity, (viewport, viewport_state)) in
        world.query::<(&Viewport, &ViewportState)>().iter()
    {
        if viewport.mode != ViewMode::ThreeD {
            continue;
        }
        let Some(projection_ctx) =
            build_display_projection_context(world, session, viewport, viewport_state)
        else {
            continue;
        };
        let scissor =
            viewport_scissor_rect(projection_ctx.viewport_rect, projection_ctx.window_size);
        if scissor[2] == 0 || scissor[3] == 0 {
            continue;
        }
        let Some(transform) = screen_transform(projection_ctx) else {
            continue;
        };
        viewports.push((viewport_entity, projection_ctx, scissor, transform));
    }
    if viewports.is_empty() {
        return data;
    }

    for mesh_view in &roi_views.mesh_overlays {
        let Ok(roi) = world.get::<&Roi>(mesh_view.entity) else {
            continue;
        };
        let color = {
            let color = roi.metadata.color;
            [color[0], color[1], color[2], color[3] * MESH_ALPHA_SCALE]
        };
        let mut parts: Vec<(MeshRenderPartKey, &MeshData)> = Vec::new();
        let chunked;
        if let Some(preview) = roi.mesh_edit_preview() {
            parts.push((MeshRenderPartKey::Full, &preview.mesh_data));
        } else if let Some(found) = chunked_mesh_for_render(&roi) {
            chunked = found;
            for chunk in &chunked.chunks {
                parts.push((MeshRenderPartKey::Chunk(chunk.key), &chunk.data));
            }
        } else if let Some(mesh) = mesh_data_for_render(&roi) {
            parts.push((MeshRenderPartKey::Full, mesh));
        }
        // A part of another kind that is no longer current is dropped by the renderer, which
        // keeps only the parts listed in `draws`.
        for (part, mesh) in parts {
            let key = MeshPartKey {
                roi_entity: mesh_view.entity,
                part,
            };
            let fingerprint = mesh_fingerprint(mesh);
            let geometry = if known.get(&key) == Some(&fingerprint) {
                None
            } else {
                match indexed_geometry(mesh) {
                    Some(geometry) => Some(geometry),
                    None => continue,
                }
            };
            if geometry.is_none() && !known.contains_key(&key) {
                continue;
            }
            data.parts.push(MeshPartUpdate {
                key,
                fingerprint,
                geometry,
            });
            for (viewport_entity, _, scissor, transform) in &viewports {
                data.draws.push(MeshDraw {
                    key,
                    viewport_entity: *viewport_entity,
                    uniform: MeshDrawUniform {
                        row0: transform[0],
                        row1: transform[1],
                        color,
                    },
                    scissor_rect: *scissor,
                });
            }
        }
    }

    if active_tool == crate::components::EditorTool::MeshDeform {
        if let Some(selection) = mesh_selection {
            if let Ok(roi) = world.get::<&Roi>(selection.roi_entity) {
                if roi.metadata.is_visible {
                    let mesh = roi
                        .mesh_edit_preview()
                        .map(|preview| &preview.mesh_data)
                        .or_else(|| roi.mesh_data());
                    let world_mm = mesh.and_then(|mesh| {
                        mesh.vertices
                            .get(selection.vertex_index)
                            .map(|vertex| vertex.world_mm)
                    });
                    if let Some(world_mm) = world_mm {
                        for (viewport_entity, ctx, scissor, _) in &viewports {
                            append_mesh_handle(
                                selection.roi_entity,
                                *viewport_entity,
                                world_mm,
                                *ctx,
                                *scissor,
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

/// The screen projection of one 3D viewport as two rows mapping millimetres to window NDC.
///
/// The 3D view is an orthographic projection of an affine volume mapping, so it is exactly
/// affine; reading it back from four probe points keeps one definition of the projection.
fn screen_transform(ctx: DisplayProjectionContext) -> Option<[[f32; 4]; 2]> {
    let origin = project_world_vertex([0.0; 3], ctx)?;
    let mut rows = [[0.0_f32; 4]; 2];
    for axis in 0..3 {
        let mut probe = [0.0_f32; 3];
        probe[axis] = PROJECTION_PROBE_MM;
        let projected = project_world_vertex(probe, ctx)?;
        rows[0][axis] = (projected[0] - origin[0]) / PROJECTION_PROBE_MM;
        rows[1][axis] = (projected[1] - origin[1]) / PROJECTION_PROBE_MM;
    }
    rows[0][3] = origin[0];
    rows[1][3] = origin[1];
    Some(rows)
}

/// Cheap change detector: every vertex position and a sample of the faces. Face topology only
/// changes together with the vertices (a rebuild), so sampling faces cannot miss an edit.
fn mesh_fingerprint(mesh: &MeshData) -> u64 {
    const K: u64 = 0x517c_c1b7_2722_0a95;
    let mut hash = (mesh.vertices.len() as u64) ^ ((mesh.faces.len() as u64) << 32);
    for vertex in &mesh.vertices {
        for coordinate in vertex.world_mm {
            hash = (hash.rotate_left(5) ^ u64::from(coordinate.to_bits())).wrapping_mul(K);
        }
    }
    for face in mesh.faces.iter().step_by(7) {
        for index in face.vertex_indices {
            hash = (hash.rotate_left(5) ^ u64::from(index)).wrapping_mul(K);
        }
    }
    hash
}

/// Positions and triangle indices, skipping faces that reference missing vertices.
fn indexed_geometry(mesh: &MeshData) -> Option<MeshGeometry> {
    let vertex_count = mesh.vertices.len();
    let mut indices = Vec::with_capacity(mesh.faces.len() * 3);
    for face in &mesh.faces {
        if face
            .vertex_indices
            .iter()
            .all(|index| (*index as usize) < vertex_count)
        {
            indices.extend_from_slice(&face.vertex_indices);
        }
    }
    if indices.is_empty() {
        return None;
    }
    Some(MeshGeometry {
        positions: mesh.vertices.iter().map(|vertex| vertex.world_mm).collect(),
        indices,
    })
}

fn mesh_data_for_render(roi: &Roi) -> Option<&MeshData> {
    crate::render::roi_views::mesh_data_for_adapter(roi)
}

fn chunked_mesh_for_render(roi: &Roi) -> Option<&ChunkedMeshData> {
    if roi.preview_state.active {
        if let Some(preview) = roi.session_caches.preview_mesh.as_ref().filter(|cache| {
            cache.source_generation == roi.dirty_state.authoritative.shape
                && cache.preview_revision == roi.preview_state.revision
        }) {
            return preview.chunks.as_ref();
        }
    }
    if matches!(roi.body, crate::components::RoiBody::Mesh(_)) {
        return None;
    }
    roi.is_cache_current(crate::components::RoiCacheKind::Mesh)
        .then(|| roi.mesh_cache().and_then(|cache| cache.chunks.as_ref()))
        .flatten()
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
    let [dx, dy] = [12.0 / window_width, 12.0 / window_height];
    let corners = [
        [center[0] - dx, center[1] - dy, 0.0],
        [center[0] + dx, center[1] - dy, 0.0],
        [center[0] + dx, center[1] + dy, 0.0],
        [center[0] - dx, center[1] + dy, 0.0],
    ];
    let geometry = MeshGeometry {
        positions: corners.to_vec(),
        indices: vec![0, 1, 2, 0, 2, 3],
    };
    let key = MeshPartKey {
        roi_entity,
        part: MeshRenderPartKey::Handle(viewport_entity),
    };
    // The handle is already in window NDC, so its transform is the identity.
    data.parts.push(MeshPartUpdate {
        key,
        fingerprint: mesh_positions_fingerprint(&geometry.positions),
        geometry: Some(geometry),
    });
    data.draws.push(MeshDraw {
        key,
        viewport_entity,
        uniform: MeshDrawUniform {
            row0: [1.0, 0.0, 0.0, 0.0],
            row1: [0.0, 1.0, 0.0, 0.0],
            color: HANDLE_COLOR,
        },
        scissor_rect,
    });
}

fn mesh_positions_fingerprint(positions: &[[f32; 3]]) -> u64 {
    positions.iter().flatten().fold(0, |hash, coordinate| {
        (u64::rotate_left(hash, 5) ^ u64::from(coordinate.to_bits()))
            .wrapping_mul(0x517c_c1b7_2722_0a95)
    })
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

/// Uploads changed geometry and this frame's per-draw uniforms.
pub fn upload_mesh_render_data(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut MeshRenderer,
    data: &MeshRenderData,
) {
    renderer.uploaded_chunks_last_frame = 0;
    renderer.reused_chunks_last_frame = 0;

    let live: std::collections::HashSet<MeshPartKey> =
        data.parts.iter().map(|part| part.key).collect();
    for update in &data.parts {
        match &update.geometry {
            Some(geometry) => {
                upload_part(
                    device,
                    queue,
                    renderer,
                    update.key,
                    update.fingerprint,
                    geometry,
                );
                renderer.uploaded_chunks_last_frame += 1;
            }
            None => renderer.reused_chunks_last_frame += 1,
        }
    }
    renderer.parts.retain(|key, _| live.contains(key));

    if data.draws.len() > renderer.uniform_capacity {
        renderer.uniform_capacity = data.draws.len().next_power_of_two();
        renderer.uniform_buffer = create_uniform_buffer(device, renderer.uniform_capacity);
        renderer.bind_group = create_draw_bind_group(
            device,
            &renderer.bind_group_layout,
            &renderer.uniform_buffer,
        );
    }
    let mut block = vec![0_u8; data.draws.len() * DRAW_UNIFORM_STRIDE as usize];
    for (index, draw) in data.draws.iter().enumerate() {
        let offset = index * DRAW_UNIFORM_STRIDE as usize;
        block[offset..offset + std::mem::size_of::<MeshDrawUniform>()]
            .copy_from_slice(bytemuck::bytes_of(&draw.uniform));
    }
    if !block.is_empty() {
        queue.write_buffer(&renderer.uniform_buffer, 0, &block);
    }
    renderer.draws.clone_from(&data.draws);
}

fn upload_part(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut MeshRenderer,
    key: MeshPartKey,
    fingerprint: u64,
    geometry: &MeshGeometry,
) {
    let vertex_capacity = geometry
        .positions
        .len()
        .max(INITIAL_VERTEX_CAPACITY)
        .next_power_of_two();
    let index_capacity = geometry
        .indices
        .len()
        .max(INITIAL_INDEX_CAPACITY)
        .next_power_of_two();
    let part = renderer.parts.entry(key).or_insert_with(|| GpuMeshPart {
        fingerprint,
        vertex_buffer: create_buffer(device, vertex_capacity * 12, wgpu::BufferUsages::VERTEX),
        vertex_capacity,
        index_buffer: create_buffer(device, index_capacity * 4, wgpu::BufferUsages::INDEX),
        index_capacity,
        index_count: 0,
    });
    if geometry.positions.len() > part.vertex_capacity {
        part.vertex_capacity = vertex_capacity;
        part.vertex_buffer =
            create_buffer(device, vertex_capacity * 12, wgpu::BufferUsages::VERTEX);
    }
    if geometry.indices.len() > part.index_capacity {
        part.index_capacity = index_capacity;
        part.index_buffer = create_buffer(device, index_capacity * 4, wgpu::BufferUsages::INDEX);
    }
    queue.write_buffer(
        &part.vertex_buffer,
        0,
        bytemuck::cast_slice(&geometry.positions),
    );
    queue.write_buffer(
        &part.index_buffer,
        0,
        bytemuck::cast_slice(&geometry.indices),
    );
    part.index_count = geometry.indices.len() as u32;
    part.fingerprint = fingerprint;
}

fn create_buffer(device: &wgpu::Device, size: usize, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Mesh Overlay Buffer"),
        size: size as u64,
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

pub fn render_meshes(
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    renderer: &MeshRenderer,
) {
    if renderer.draws.is_empty() {
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
    for (index, draw) in renderer.draws.iter().enumerate() {
        if draw.scissor_rect[2] == 0 || draw.scissor_rect[3] == 0 {
            continue;
        }
        let Some(part) = renderer.parts.get(&draw.key) else {
            continue;
        };
        if part.index_count == 0 {
            continue;
        }
        pass.set_bind_group(
            0,
            &renderer.bind_group,
            &[(index as u64 * DRAW_UNIFORM_STRIDE) as u32],
        );
        pass.set_vertex_buffer(0, part.vertex_buffer.slice(..));
        pass.set_index_buffer(part.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.set_scissor_rect(
            draw.scissor_rect[0],
            draw.scissor_rect[1],
            draw.scissor_rect[2],
            draw.scissor_rect[3],
        );
        pass.draw_indexed(0..part.index_count, 0, 0..1);
    }
}

#[cfg(test)]
mod tests;
