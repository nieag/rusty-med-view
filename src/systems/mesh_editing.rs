use crate::app::roi;
use crate::components::{
    AppEntities, EditorState, EditorTool, MeshData, MeshSelection, Roi, RoiAuthoritativeData,
    ViewMode, Viewport, ViewportState,
};
use crate::render::geometry::{
    build_display_projection_context, project_world_mm_to_viewport_uv_3d, DisplayProjectionContext,
};
use glam::Vec3;
use hecs::World;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshEditInteractionError {
    ToolNotActive,
    MissingActiveRoi,
    MissingViewport,
    ViewportNotThreeD,
    ActiveRoiNotMesh,
    ActiveRoiLocked,
    MissingSelection,
    InvalidBrush,
    ProjectionFailed,
}

pub fn select_mesh_vertex(
    world: &mut World,
    entities: &AppEntities,
    viewport_uv: [f32; 2],
) -> Result<Option<MeshSelection>, MeshEditInteractionError> {
    let (active_tool, roi_entity) = world
        .get::<&EditorState>(entities.editor)
        .map(|editor| (editor.active_tool, editor.active_roi))
        .map_err(|_| MeshEditInteractionError::ToolNotActive)?;
    if active_tool != EditorTool::MeshDeform {
        return Err(MeshEditInteractionError::ToolNotActive);
    }
    let roi_entity = roi_entity.ok_or(MeshEditInteractionError::MissingActiveRoi)?;
    let viewport_entity = world
        .get::<&crate::components::InputState>(entities.input)
        .ok()
        .and_then(|input| input.active_viewport)
        .ok_or(MeshEditInteractionError::MissingViewport)?;
    let viewport = world
        .get::<&Viewport>(viewport_entity)
        .map_err(|_| MeshEditInteractionError::MissingViewport)?;
    if viewport.mode != ViewMode::ThreeD {
        return Err(MeshEditInteractionError::ViewportNotThreeD);
    }
    let viewport_state = world
        .get::<&ViewportState>(viewport_entity)
        .map_err(|_| MeshEditInteractionError::MissingViewport)?;
    let projection = build_display_projection_context(world, entities, &viewport, &viewport_state)
        .ok_or(MeshEditInteractionError::ProjectionFailed)?;
    drop(viewport_state);
    drop(viewport);

    let roi = world
        .get::<&Roi>(roi_entity)
        .map_err(|_| MeshEditInteractionError::MissingActiveRoi)?;
    if roi.metadata.is_locked {
        return Err(MeshEditInteractionError::ActiveRoiLocked);
    }
    let mesh = match &roi.authoritative_data {
        RoiAuthoritativeData::Mesh(mesh) => mesh,
        _ => return Err(MeshEditInteractionError::ActiveRoiNotMesh),
    };
    let selection =
        nearest_mesh_surface_hit(mesh, viewport_uv, projection).map(|hit| MeshSelection {
            roi_entity,
            vertex_index: hit.vertex_index,
            triangle_vertex_indices: hit.triangle_vertex_indices,
            anchor_world_mm: hit.anchor_world_mm,
        });
    drop(roi);
    if let Ok(mut editor) = world.get::<&mut EditorState>(entities.editor) {
        editor.mesh_selection = selection;
    }
    Ok(selection)
}

pub fn update_selected_mesh_deform_preview(
    world: &mut World,
    entities: &AppEntities,
    current_uv: [f32; 2],
) -> Result<u64, MeshEditInteractionError> {
    let (selection, radius_mm, strength) = world
        .get::<&EditorState>(entities.editor)
        .map(|editor| {
            (
                editor.mesh_selection,
                editor.mesh_brush_radius_mm,
                editor.mesh_brush_strength,
            )
        })
        .map_err(|_| MeshEditInteractionError::MissingSelection)?;
    let selection = selection.ok_or(MeshEditInteractionError::MissingSelection)?;
    if !radius_mm.is_finite() || radius_mm <= 0.0 || !strength.is_finite() || strength <= 0.0 {
        return Err(MeshEditInteractionError::InvalidBrush);
    }
    let start_uv = world
        .get::<&crate::components::InputState>(entities.input)
        .map(|input| input.drag_start_pos)
        .map_err(|_| MeshEditInteractionError::MissingViewport)?;
    let viewport_entity = world
        .get::<&crate::components::InputState>(entities.input)
        .ok()
        .and_then(|input| input.active_viewport)
        .ok_or(MeshEditInteractionError::MissingViewport)?;
    let viewport = world
        .get::<&Viewport>(viewport_entity)
        .map_err(|_| MeshEditInteractionError::MissingViewport)?;
    if viewport.mode != ViewMode::ThreeD {
        return Err(MeshEditInteractionError::ViewportNotThreeD);
    }
    let viewport_state = world
        .get::<&ViewportState>(viewport_entity)
        .map_err(|_| MeshEditInteractionError::MissingViewport)?;
    let projection = build_display_projection_context(world, entities, &viewport, &viewport_state)
        .ok_or(MeshEditInteractionError::ProjectionFailed)?;
    drop(viewport_state);
    drop(viewport);

    let mesh = {
        let roi = world
            .get::<&Roi>(selection.roi_entity)
            .map_err(|_| MeshEditInteractionError::MissingActiveRoi)?;
        if roi.metadata.is_locked {
            return Err(MeshEditInteractionError::ActiveRoiLocked);
        }
        let mesh = roi
            .mesh_data()
            .ok_or(MeshEditInteractionError::ActiveRoiNotMesh)?;
        mesh.clone()
    };
    let anchor_world_mm = selection.anchor_world_mm;
    let delta_world_mm = drag_delta_world_mm(start_uv, current_uv, anchor_world_mm, projection)
        .ok_or(MeshEditInteractionError::ProjectionFailed)?;
    let mesh_data = deform_mesh_surface_brush(
        &mesh,
        selection.triangle_vertex_indices,
        anchor_world_mm,
        delta_world_mm,
        radius_mm,
        strength,
    );
    roi::begin_mesh_edit_preview(world, entities.editor, selection.roi_entity, mesh_data)
        .map_err(|_| MeshEditInteractionError::MissingActiveRoi)
}

struct MeshSurfaceHit {
    vertex_index: usize,
    triangle_vertex_indices: [u32; 3],
    anchor_world_mm: [f32; 3],
}

fn nearest_mesh_surface_hit(
    mesh: &MeshData,
    viewport_uv: [f32; 2],
    projection: DisplayProjectionContext,
) -> Option<MeshSurfaceHit> {
    let (_, ray_direction) = crate::util::orientation::screen_to_ray_3d(
        viewport_uv,
        projection.composed_rotation_3d,
        projection.zoom,
        projection.pan,
        projection.screen_aspect,
    );
    let ray_direction = Vec3::from_array(ray_direction);
    let aspect = Vec3::from_array(projection.display_aspect_ratios);
    if aspect
        .to_array()
        .iter()
        .any(|value| value.abs() <= f32::EPSILON)
    {
        return None;
    }
    mesh.faces
        .iter()
        .filter_map(|face| {
            let indices = face.vertex_indices;
            let vertices = indices.map(|index| mesh.vertices.get(index as usize));
            let [Some(a), Some(b), Some(c)] = vertices else {
                return None;
            };
            let projected = [
                project_world_mm_to_viewport_uv_3d(a.world_mm, projection)?,
                project_world_mm_to_viewport_uv_3d(b.world_mm, projection)?,
                project_world_mm_to_viewport_uv_3d(c.world_mm, projection)?,
            ];
            let weights = barycentric_weights(viewport_uv, projected)?;
            if weights.iter().any(|weight| *weight < -1e-4) {
                return None;
            }
            let anchor = Vec3::from_array(a.world_mm) * weights[0]
                + Vec3::from_array(b.world_mm) * weights[1]
                + Vec3::from_array(c.world_mm) * weights[2];
            let anchor_uv =
                crate::convert::world_mm_to_volume_uv(anchor.to_array(), projection.main_geometry);
            let depth =
                ((Vec3::from_array(anchor_uv) - Vec3::splat(0.5)) * aspect).dot(ray_direction);
            let nearest = weights
                .iter()
                .enumerate()
                .max_by(|left, right| left.1.total_cmp(right.1))?
                .0;
            Some((
                MeshSurfaceHit {
                    vertex_index: indices[nearest] as usize,
                    triangle_vertex_indices: indices,
                    anchor_world_mm: anchor.to_array(),
                },
                depth,
            ))
        })
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .map(|(hit, _)| hit)
}

fn barycentric_weights(point: [f32; 2], triangle: [[f32; 2]; 3]) -> Option<[f32; 3]> {
    let [a, b, c] = triangle;
    let denominator = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    if denominator.abs() <= f32::EPSILON {
        return None;
    }
    let first =
        ((b[1] - c[1]) * (point[0] - c[0]) + (c[0] - b[0]) * (point[1] - c[1])) / denominator;
    let second =
        ((c[1] - a[1]) * (point[0] - c[0]) + (a[0] - c[0]) * (point[1] - c[1])) / denominator;
    Some([first, second, 1.0 - first - second])
}

fn drag_delta_world_mm(
    start_uv: [f32; 2],
    current_uv: [f32; 2],
    anchor_world_mm: [f32; 3],
    projection: DisplayProjectionContext,
) -> Option<[f32; 3]> {
    let (start_origin, start_direction) = crate::util::orientation::screen_to_ray_3d(
        start_uv,
        projection.composed_rotation_3d,
        projection.zoom,
        projection.pan,
        projection.screen_aspect,
    );
    let (current_origin, current_direction) = crate::util::orientation::screen_to_ray_3d(
        current_uv,
        projection.composed_rotation_3d,
        projection.zoom,
        projection.pan,
        projection.screen_aspect,
    );
    let anchor_uv =
        crate::convert::world_mm_to_volume_uv(anchor_world_mm, projection.main_geometry);
    let aspect = Vec3::from_array(projection.display_aspect_ratios);
    if aspect
        .to_array()
        .iter()
        .any(|value| value.abs() <= f32::EPSILON)
    {
        return None;
    }
    let anchor_object = (Vec3::from_array(anchor_uv) - Vec3::splat(0.5)) * aspect;
    let normal = Vec3::from_array(start_direction).normalize_or_zero();
    if normal == Vec3::ZERO {
        return None;
    }
    let start_hit = ray_plane_intersection(
        Vec3::from_array(start_origin),
        Vec3::from_array(start_direction),
        anchor_object,
        normal,
    )?;
    let current_hit = ray_plane_intersection(
        Vec3::from_array(current_origin),
        Vec3::from_array(current_direction),
        anchor_object,
        normal,
    )?;
    let target_uv = Vec3::from_array(anchor_uv) + (current_hit - start_hit) / aspect;
    let target_world_mm =
        crate::convert::volume_uv_to_world_mm(target_uv.to_array(), projection.main_geometry);
    Some((Vec3::from_array(target_world_mm) - Vec3::from_array(anchor_world_mm)).to_array())
}

fn ray_plane_intersection(
    origin: Vec3,
    direction: Vec3,
    point_on_plane: Vec3,
    plane_normal: Vec3,
) -> Option<Vec3> {
    let denominator = direction.dot(plane_normal);
    if denominator.abs() <= 1e-6 {
        return None;
    }
    let t = (point_on_plane - origin).dot(plane_normal) / denominator;
    t.is_finite().then_some(origin + direction * t)
}

pub fn deform_mesh_with_brush(
    mesh: &MeshData,
    anchor_world_mm: [f32; 3],
    delta_world_mm: [f32; 3],
    radius_mm: f32,
    strength: f32,
) -> MeshData {
    let mut deformed = mesh.clone();
    if !radius_mm.is_finite()
        || radius_mm <= 0.0
        || !strength.is_finite()
        || delta_world_mm.iter().any(|value| !value.is_finite())
    {
        return deformed;
    }
    let anchor = Vec3::from_array(anchor_world_mm);
    let delta = Vec3::from_array(delta_world_mm) * strength.max(0.0);
    for vertex in &mut deformed.vertices {
        let distance = Vec3::from_array(vertex.world_mm).distance(anchor);
        if distance > radius_mm {
            continue;
        }
        let normalized = 1.0 - distance / radius_mm;
        let weight = normalized * normalized * (3.0 - 2.0 * normalized);
        vertex.world_mm = (Vec3::from_array(vertex.world_mm) + delta * weight).to_array();
    }
    deformed
}

pub fn deform_mesh_surface_brush(
    mesh: &MeshData,
    seed_indices: [u32; 3],
    anchor_world_mm: [f32; 3],
    delta_world_mm: [f32; 3],
    radius_mm: f32,
    strength: f32,
) -> MeshData {
    let mut deformed = mesh.clone();
    if !radius_mm.is_finite()
        || radius_mm <= 0.0
        || !strength.is_finite()
        || delta_world_mm.iter().any(|value| !value.is_finite())
    {
        return deformed;
    }
    let mut adjacency = vec![Vec::new(); mesh.vertices.len()];
    for face in &mesh.faces {
        let [a, b, c] = face.vertex_indices.map(|index| index as usize);
        if [a, b, c].iter().any(|index| *index >= mesh.vertices.len()) {
            return deformed;
        }
        for (from, to) in [(a, b), (b, c), (c, a)] {
            adjacency[from].push(to);
            adjacency[to].push(from);
        }
    }
    let anchor = Vec3::from_array(anchor_world_mm);
    let mut distances = vec![f32::INFINITY; mesh.vertices.len()];
    let mut queue = BinaryHeap::new();
    for index in seed_indices.map(|index| index as usize) {
        let Some(vertex) = mesh.vertices.get(index) else {
            return deformed;
        };
        let distance = Vec3::from_array(vertex.world_mm).distance(anchor);
        if distance <= radius_mm && distance < distances[index] {
            distances[index] = distance;
            queue.push(MeshBrushQueueEntry { distance, index });
        }
    }
    while let Some(MeshBrushQueueEntry { distance, index }) = queue.pop() {
        if distance != distances[index] {
            continue;
        }
        for &next in &adjacency[index] {
            let edge_length = Vec3::from_array(mesh.vertices[index].world_mm)
                .distance(Vec3::from_array(mesh.vertices[next].world_mm));
            let next_distance = distance + edge_length;
            if next_distance <= radius_mm && next_distance < distances[next] {
                distances[next] = next_distance;
                queue.push(MeshBrushQueueEntry {
                    distance: next_distance,
                    index: next,
                });
            }
        }
    }
    let delta = Vec3::from_array(delta_world_mm) * strength.max(0.0);
    for (vertex, distance) in deformed.vertices.iter_mut().zip(distances) {
        if !distance.is_finite() {
            continue;
        }
        let normalized = 1.0 - distance / radius_mm;
        let weight = normalized * normalized * (3.0 - 2.0 * normalized);
        vertex.world_mm = (Vec3::from_array(vertex.world_mm) + delta * weight).to_array();
    }
    deformed
}

#[derive(Debug, Clone, Copy)]
struct MeshBrushQueueEntry {
    distance: f32,
    index: usize,
}

impl PartialEq for MeshBrushQueueEntry {
    fn eq(&self, other: &Self) -> bool {
        self.distance.to_bits() == other.distance.to_bits() && self.index == other.index
    }
}

impl Eq for MeshBrushQueueEntry {}

impl Ord for MeshBrushQueueEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .distance
            .total_cmp(&self.distance)
            .then_with(|| other.index.cmp(&self.index))
    }
}

impl PartialOrd for MeshBrushQueueEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{
        AnnotationState, GuiState, InputState, MainVolumeTag, MeshFace, MeshVertex, ProtocolState,
        RoiId, Transform, VolumeData, VolumeWindowing, WindowSettings,
    };

    fn spawn_mesh_edit_world() -> (World, AppEntities, hecs::Entity) {
        let mut world = World::new();
        world.spawn((
            VolumeData {
                dimensions: [10, 10, 10],
                spacing: [1.0, 1.0, 1.0],
                origin: [0.0, 0.0, 0.0],
                intensities: Vec::new(),
                intensity_range: [0.0, 1.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            MainVolumeTag,
        ));
        let viewport = world.spawn((
            Viewport {
                mode: ViewMode::ThreeD,
                rect: [0.0, 0.0, 800.0, 600.0],
                uniform_index: 0,
            },
            ViewportState::default(),
        ));
        let mesh = MeshData {
            vertices: vec![
                MeshVertex {
                    world_mm: [4.0, 4.0, 4.0],
                },
                MeshVertex {
                    world_mm: [5.0, 4.0, 4.0],
                },
                MeshVertex {
                    world_mm: [4.0, 5.0, 4.0],
                },
                MeshVertex {
                    world_mm: [4.0, 4.0, 5.0],
                },
            ],
            faces: vec![
                MeshFace {
                    vertex_indices: [0, 2, 1],
                },
                MeshFace {
                    vertex_indices: [0, 1, 3],
                },
                MeshFace {
                    vertex_indices: [0, 3, 2],
                },
                MeshFace {
                    vertex_indices: [1, 2, 3],
                },
            ],
        };
        let roi_entity = world.spawn((Roi::new_mesh(RoiId(1), "Mesh".to_string(), mesh),));
        let cursor = world.spawn((Transform {
            position: [0.5, 0.5, 0.5],
        },));
        let editor = world.spawn((EditorState {
            active_roi: Some(roi_entity),
            active_tool: EditorTool::MeshDeform,
            ..EditorState::default()
        },));
        let input = world.spawn((InputState {
            active_viewport: Some(viewport),
            ..InputState::default()
        },));
        let gui_state = world.spawn((GuiState {
            status_message: None,
        },));
        let volume_windowing = world.spawn((VolumeWindowing::default(),));
        let annotations = world.spawn((AnnotationState::default(),));
        let overlay = world.spawn((crate::overlay::OverlayManager::default(),));
        let protocol = world.spawn((ProtocolState::default(),));
        let window_settings = world.spawn((WindowSettings {
            width: 800,
            height: 600,
            viewport_rect: [0.0, 0.0, 800.0, 600.0],
        },));
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
                window_settings,
            },
            roi_entity,
        )
    }

    #[test]
    fn test_mesh_brush_moves_anchor_and_preserves_outside_vertex() {
        let mesh = MeshData {
            vertices: vec![
                MeshVertex {
                    world_mm: [0.0, 0.0, 0.0],
                },
                MeshVertex {
                    world_mm: [5.0, 0.0, 0.0],
                },
            ],
            faces: Vec::new(),
        };

        let deformed = deform_mesh_with_brush(&mesh, [0.0; 3], [0.0, 2.0, 0.0], 2.0, 1.0);

        assert_eq!(deformed.vertices[0].world_mm, [0.0, 2.0, 0.0]);
        assert_eq!(deformed.vertices[1].world_mm, [5.0, 0.0, 0.0]);
    }

    #[test]
    fn test_mesh_brush_moves_duplicate_surface_vertices_identically() {
        let mesh = MeshData {
            vertices: vec![
                MeshVertex {
                    world_mm: [1.0, 1.0, 1.0],
                },
                MeshVertex {
                    world_mm: [1.0, 1.0, 1.0],
                },
            ],
            faces: Vec::new(),
        };

        let deformed = deform_mesh_with_brush(&mesh, [1.0; 3], [1.0, 0.0, 0.0], 4.0, 0.5);

        assert_eq!(deformed.vertices[0], deformed.vertices[1]);
        assert_eq!(deformed.vertices[0].world_mm, [1.5, 1.0, 1.0]);
    }

    #[test]
    fn test_surface_brush_does_not_cross_disconnected_nearby_surface() {
        let mesh = MeshData {
            vertices: vec![
                MeshVertex {
                    world_mm: [0.0, 0.0, 0.0],
                },
                MeshVertex {
                    world_mm: [1.0, 0.0, 0.0],
                },
                MeshVertex {
                    world_mm: [0.0, 1.0, 0.0],
                },
                MeshVertex {
                    world_mm: [0.0, 0.0, 0.2],
                },
                MeshVertex {
                    world_mm: [1.0, 0.0, 0.2],
                },
                MeshVertex {
                    world_mm: [0.0, 1.0, 0.2],
                },
            ],
            faces: vec![
                MeshFace {
                    vertex_indices: [0, 1, 2],
                },
                MeshFace {
                    vertex_indices: [3, 4, 5],
                },
            ],
        };

        let deformed =
            deform_mesh_surface_brush(&mesh, [0, 1, 2], [0.0, 0.0, 0.0], [0.0, 0.0, 1.0], 2.0, 1.0);

        assert!(deformed.vertices[0].world_mm[2] > mesh.vertices[0].world_mm[2]);
        assert_eq!(deformed.vertices[3], mesh.vertices[3]);
        assert_eq!(deformed.vertices[4], mesh.vertices[4]);
        assert_eq!(deformed.vertices[5], mesh.vertices[5]);
    }

    #[test]
    fn test_projected_mesh_drag_creates_preview_without_mutating_authority() {
        let (mut world, entities, roi_entity) = spawn_mesh_edit_world();
        let viewport_entity = world
            .get::<&InputState>(entities.input)
            .unwrap()
            .active_viewport
            .unwrap();
        let viewport = world.get::<&Viewport>(viewport_entity).unwrap();
        let viewport_state = world.get::<&ViewportState>(viewport_entity).unwrap();
        let projection =
            build_display_projection_context(&world, &entities, &viewport, &viewport_state)
                .expect("display projection");
        // The default view looks along +Y, so this point is inside the
        // front-facing tetrahedron triangle rather than an occluded face.
        let surface_point = [4.25, 4.75, 4.0];
        let click_uv = project_world_mm_to_viewport_uv_3d(surface_point, projection)
            .expect("projected anchor");
        drop(viewport_state);
        drop(viewport);

        let selection = select_mesh_vertex(&mut world, &entities, click_uv)
            .expect("mesh selection")
            .expect("selected surface");
        assert_eq!(selection.roi_entity, roi_entity);
        assert!(
            Vec3::from_array(selection.anchor_world_mm).distance(Vec3::from_array(surface_point))
                < 1e-5
        );
        let authoritative_before = world
            .get::<&Roi>(roi_entity)
            .unwrap()
            .mesh_data()
            .unwrap()
            .clone();
        world
            .get::<&mut InputState>(entities.input)
            .unwrap()
            .drag_start_pos = click_uv;

        let revision = update_selected_mesh_deform_preview(
            &mut world,
            &entities,
            [click_uv[0] + 0.05, click_uv[1]],
        )
        .expect("mesh preview");

        assert_eq!(revision, 1);
        let authoritative_after = world
            .get::<&Roi>(roi_entity)
            .unwrap()
            .mesh_data()
            .unwrap()
            .clone();
        assert_eq!(authoritative_after, authoritative_before);
        let editor = world.get::<&EditorState>(entities.editor).unwrap();
        let preview = editor.mesh_edit_preview().expect("mesh preview data");
        assert_ne!(
            preview.mesh_data.vertices[selection.vertex_index].world_mm,
            authoritative_before.vertices[selection.vertex_index].world_mm
        );
    }
}
