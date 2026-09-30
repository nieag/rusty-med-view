use crate::app::roi;
use crate::components::Session;
use crate::components::{
    EditorTool, MeshBody, MeshData, MeshSelection, Roi, RoiBody, ViewMode, Viewport, ViewportState,
};
#[cfg(test)]
use crate::convert::deform_mesh_surface_brush;
use crate::convert::{deform_mesh_surface_brush_limited, MeshDeformBase};
use crate::render::geometry::{
    build_display_projection_context, project_world_mm_to_viewport_uv_3d, DisplayProjectionContext,
};
use glam::Vec3;
use hecs::World;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshEditInteractionError {
    ToolNotActive,
    MissingActiveRoi,
    MissingViewport,
    ViewportNotThreeD,
    ActiveRoiNotMesh,
    ActiveRoiLocked,
    /// The ROI is being prepared for mesh editing; the click is ignored.
    SwitchPending,
    Switch(roi::SwitchError),
    MissingSelection,
    InvalidBrush,
    ProjectionFailed,
}

pub fn select_mesh_vertex(
    world: &mut World,
    session: &mut Session,
    viewport_uv: [f32; 2],
) -> Result<Option<MeshSelection>, MeshEditInteractionError> {
    let (active_tool, roi_entity) = {
        let editor = &session.editor;
        (editor.active_tool, editor.active_roi)
    };
    if active_tool != EditorTool::MeshDeform {
        return Err(MeshEditInteractionError::ToolNotActive);
    }
    let roi_entity = roi_entity.ok_or(MeshEditInteractionError::MissingActiveRoi)?;
    let viewport_entity = {
        let input = &session.input;
        input.active_viewport
    }
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
    let projection = build_display_projection_context(world, session, &viewport, &viewport_state)
        .ok_or(MeshEditInteractionError::ProjectionFailed)?;
    drop(viewport_state);
    drop(viewport);

    match roi::ensure_editable(world, roi_entity, roi::EditTarget::Mesh) {
        Ok(roi::Readiness::Ready) => {}
        Ok(roi::Readiness::Switched(report)) => {
            crate::app::handlers::set_status_message(session, report.message());
        }
        Ok(roi::Readiness::Pending) => return Err(MeshEditInteractionError::SwitchPending),
        Err(roi::SwitchError::Locked) => return Err(MeshEditInteractionError::ActiveRoiLocked),
        Err(error) => return Err(MeshEditInteractionError::Switch(error)),
    }

    let roi = world
        .get::<&Roi>(roi_entity)
        .map_err(|_| MeshEditInteractionError::MissingActiveRoi)?;
    if roi.metadata.is_locked {
        return Err(MeshEditInteractionError::ActiveRoiLocked);
    }
    let mesh = match &roi.body {
        RoiBody::Mesh(MeshBody { data: mesh, .. }) => mesh,
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
    {
        let editor = &mut session.editor;
        editor.mesh_selection = selection;
    }
    Ok(selection)
}

pub fn update_selected_mesh_deform_preview(
    world: &mut World,
    session: &Session,
    current_uv: [f32; 2],
) -> Result<u64, MeshEditInteractionError> {
    let (selection, radius_mm, strength) = {
        let editor = &session.editor;
        {
            (
                editor.mesh_selection,
                editor.mesh_brush_radius_mm,
                editor.mesh_brush_strength,
            )
        }
    };
    let selection = selection.ok_or(MeshEditInteractionError::MissingSelection)?;
    if !radius_mm.is_finite() || radius_mm <= 0.0 || !strength.is_finite() || strength <= 0.0 {
        return Err(MeshEditInteractionError::InvalidBrush);
    }
    let start_uv = {
        let input = &session.input;
        input.drag_start_pos
    };
    let viewport_entity = {
        let input = &session.input;
        input.active_viewport
    }
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
    let projection = build_display_projection_context(world, session, &viewport, &viewport_state)
        .ok_or(MeshEditInteractionError::ProjectionFailed)?;
    drop(viewport_state);
    drop(viewport);

    let (mesh, existing_base) = {
        let roi = world
            .get::<&Roi>(selection.roi_entity)
            .map_err(|_| MeshEditInteractionError::MissingActiveRoi)?;
        if roi.metadata.is_locked {
            return Err(MeshEditInteractionError::ActiveRoiLocked);
        }
        let mesh = roi
            .mesh_data()
            .ok_or(MeshEditInteractionError::ActiveRoiNotMesh)?;
        (
            mesh.clone(),
            roi.mesh_edit_preview()
                .and_then(|preview| preview.deform_base.clone()),
        )
    };
    // The topology is built once per drag; the preview carries it between updates.
    let base = match existing_base {
        Some(base) => base,
        None => {
            Arc::new(MeshDeformBase::new(&mesh).ok_or(MeshEditInteractionError::ActiveRoiNotMesh)?)
        }
    };
    let anchor_world_mm = selection.anchor_world_mm;
    let delta_world_mm = drag_delta_world_mm(start_uv, current_uv, anchor_world_mm, projection)
        .ok_or(MeshEditInteractionError::ProjectionFailed)?;
    let (mesh_data, _applied_fraction) = deform_mesh_surface_brush_limited(
        &base,
        &mesh,
        selection.triangle_vertex_indices,
        anchor_world_mm,
        delta_world_mm,
        radius_mm,
        strength,
    );
    roi::set_mesh_edit_preview(world, selection.roi_entity, mesh_data, Some(base))
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

#[cfg(test)]
mod tests;
