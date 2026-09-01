use crate::app::roi;
#[cfg(test)]
use crate::app::roi_runtime;
#[cfg(test)]
use crate::components::VoxelData;
use crate::components::{
    AppEntities, ContourData, ContourDraft, ContourLoop, ContourPoint, ContourSelection,
    ContourSlice, EditorState, EditorTool, InputState, MainVolumeTag, Roi, Transform, ViewMode,
    Viewport, VoxelGeometry,
};
use crate::convert::{
    contour_slice_contains_point, oblique_plane_from_view_rotation,
    orthogonal_plane_from_volume_uv, plane_local_mm_to_viewport_uv, plane_local_mm_to_world_mm,
    reproject_plane_local_mm, union_contour_slice_with_loop, viewport_uv_to_plane_local_mm,
    volume_uv_to_viewport_uv, world_mm_to_volume_uv, PlaneDefinition, PlaneFamily, ViewportMapping,
};
use hecs::World;

const LOOP_CLOSE_RADIUS_PX: f32 = 10.0;
const SELECTION_RADIUS_PX: f32 = 10.0;
const SLICE_MATCH_DISTANCE_MM: f32 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContourEditMappingError {
    MissingActiveViewport,
    MissingViewport,
    MissingViewportState,
    MissingCursor,
    MissingMainVolume,
    InvalidMainVolumeGeometry,
    UnsupportedViewportMode,
    PlaneUnavailable,
    PlaneFamilyMismatch {
        contour_family: PlaneFamily,
        viewport_family: PlaneFamily,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContourEditViewport {
    pub viewport_entity: hecs::Entity,
    pub plane: PlaneDefinition,
    pub mapping: ViewportMapping,
    pub geometry: VoxelGeometry,
}

pub fn resolve_active_contour_edit_viewport(
    world: &World,
    entities: &AppEntities,
    contour_data: &ContourData,
) -> Result<ContourEditViewport, ContourEditMappingError> {
    let input = world
        .get::<&InputState>(entities.input)
        .map_err(|_| ContourEditMappingError::MissingActiveViewport)?;
    let viewport_entity = input
        .active_viewport
        .ok_or(ContourEditMappingError::MissingActiveViewport)?;
    drop(input);

    let viewport = world
        .get::<&Viewport>(viewport_entity)
        .map_err(|_| ContourEditMappingError::MissingViewport)?;
    let viewport_state = world
        .get::<&crate::components::ViewportState>(viewport_entity)
        .map_err(|_| ContourEditMappingError::MissingViewportState)?;

    let has_main_volume = world
        .query::<&crate::components::VolumeData>()
        .with::<&MainVolumeTag>()
        .iter()
        .next()
        .is_some();
    let geometry =
        crate::app::roi_runtime::main_volume_voxel_geometry(world).ok_or(if has_main_volume {
            ContourEditMappingError::InvalidMainVolumeGeometry
        } else {
            ContourEditMappingError::MissingMainVolume
        })?;

    let cursor = world
        .get::<&Transform>(entities.cursor)
        .map_err(|_| ContourEditMappingError::MissingCursor)?;

    let plane = match viewport.mode {
        ViewMode::Axial => {
            orthogonal_plane_from_volume_uv(PlaneFamily::Axial, cursor.position, geometry)
        }
        ViewMode::Coronal => {
            orthogonal_plane_from_volume_uv(PlaneFamily::Coronal, cursor.position, geometry)
        }
        ViewMode::Sagittal => {
            orthogonal_plane_from_volume_uv(PlaneFamily::Sagittal, cursor.position, geometry)
        }
        ViewMode::Oblique => oblique_plane_from_view_rotation(
            cursor.position,
            viewport_state.user_rotation,
            geometry,
        ),
        ViewMode::ThreeD => return Err(ContourEditMappingError::UnsupportedViewportMode),
    }
    .ok_or(ContourEditMappingError::PlaneUnavailable)?;

    if plane.family != contour_data.active_plane_family {
        return Err(ContourEditMappingError::PlaneFamilyMismatch {
            contour_family: contour_data.active_plane_family,
            viewport_family: plane.family,
        });
    }

    let screen_aspect = if viewport.rect[3] > 0.0 {
        viewport.rect[2] / viewport.rect[3]
    } else {
        1.0
    };
    Ok(ContourEditViewport {
        viewport_entity,
        plane,
        mapping: ViewportMapping {
            zoom: viewport_state.zoom,
            pan: viewport_state.pan,
            pivot: viewport_state.pivot,
            screen_aspect,
        },
        geometry,
    })
}

pub fn viewport_uv_to_contour_plane_local_mm(
    viewport_uv: [f32; 2],
    viewport: ContourEditViewport,
) -> Option<[f32; 2]> {
    viewport_uv_to_plane_local_mm(
        viewport_uv,
        viewport.plane,
        viewport.geometry,
        viewport.mapping,
    )
}

pub fn contour_plane_local_mm_to_viewport_uv(
    local_mm: [f32; 2],
    viewport: ContourEditViewport,
) -> Option<[f32; 2]> {
    plane_local_mm_to_viewport_uv(
        local_mm,
        viewport.plane,
        viewport.geometry,
        viewport.mapping,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContourDrawClickOutcome {
    PointAdded,
    LoopCommitted,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ContourDrawClickError {
    ToolNotActive,
    MissingActiveRoi,
    ActiveRoiNotContour,
    Mapping(ContourEditMappingError),
    ProjectionFailed,
    LoopNeedsThreePoints,
    CommitFailed,
}

impl From<ContourEditMappingError> for ContourDrawClickError {
    fn from(value: ContourEditMappingError) -> Self {
        ContourDrawClickError::Mapping(value)
    }
}

fn planes_match_for_slice(lhs: PlaneDefinition, rhs: PlaneDefinition) -> bool {
    if lhs.family != rhs.family {
        return false;
    }
    let lhs_n = glam::Vec3::from_array(lhs.normal_mm).normalize_or_zero();
    let rhs_n = glam::Vec3::from_array(rhs.normal_mm).normalize_or_zero();
    if lhs_n.length_squared() <= 1e-12 || rhs_n.length_squared() <= 1e-12 {
        return false;
    }
    if lhs_n.dot(rhs_n).abs() < 0.999 {
        return false;
    }

    let lhs_o = glam::Vec3::from_array(lhs.origin_mm);
    let rhs_o = glam::Vec3::from_array(rhs.origin_mm);
    (lhs_o - rhs_o).dot(lhs_n).abs() <= SLICE_MATCH_DISTANCE_MM
}

fn contour_data_for_active_roi(world: &World, roi_entity: hecs::Entity) -> Option<ContourData> {
    world
        .get::<&Roi>(roi_entity)
        .ok()
        .and_then(|roi| roi.contour_data().cloned())
}

pub fn clear_contour_draft_if_inactive(world: &mut World, editor_entity: hecs::Entity) {
    if let Ok(mut editor) = world.get::<&mut EditorState>(editor_entity) {
        if editor.active_tool != EditorTool::ContourDraw {
            editor.contour_draft = None;
        }
    }
}

pub fn clear_contour_draft_for_roi_change(
    world: &mut World,
    editor_entity: hecs::Entity,
    new_active_roi: Option<hecs::Entity>,
) {
    let mut ended_preview_roi = None;
    let mut ended_mesh_preview_roi = None;
    if let Ok(mut editor) = world.get::<&mut EditorState>(editor_entity) {
        if editor.active_roi != new_active_roi {
            editor.contour_draft = None;
            editor.mesh_selection = None;
            ended_preview_roi = editor
                .take_contour_move_preview()
                .map(|preview| preview.roi_entity);
            ended_mesh_preview_roi = editor
                .take_mesh_edit_preview()
                .map(|preview| preview.roi_entity);
        }
    }
    if let Some(roi_entity) = ended_preview_roi {
        roi::end_roi_preview(world, roi_entity);
    }
    if let Some(roi_entity) = ended_mesh_preview_roi {
        roi::end_roi_preview(world, roi_entity);
    }
}

pub fn clear_contour_selection_for_roi_change(
    world: &mut World,
    editor_entity: hecs::Entity,
    new_active_roi: Option<hecs::Entity>,
) {
    let mut ended_preview_roi = None;
    if let Ok(mut editor) = world.get::<&mut EditorState>(editor_entity) {
        if editor.active_roi != new_active_roi {
            editor.contour_selection = None;
            editor.mesh_selection = None;
            ended_preview_roi = editor
                .take_contour_move_preview()
                .map(|preview| preview.roi_entity);
        }
    }
    if let Some(roi_entity) = ended_preview_roi {
        roi::end_roi_preview(world, roi_entity);
    }
}

pub fn clear_contour_selection_if_inactive(world: &mut World, editor_entity: hecs::Entity) {
    let mut ended_preview_roi = None;
    if let Ok(mut editor) = world.get::<&mut EditorState>(editor_entity) {
        if editor.active_tool != EditorTool::ContourSelect {
            editor.contour_selection = None;
            ended_preview_roi = editor
                .take_contour_move_preview()
                .map(|preview| preview.roi_entity);
        }
    }
    if let Some(roi_entity) = ended_preview_roi {
        roi::end_roi_preview(world, roi_entity);
    }
}

pub fn handle_contour_draw_click(
    world: &mut World,
    entities: &AppEntities,
    viewport_uv: [f32; 2],
) -> Result<ContourDrawClickOutcome, ContourDrawClickError> {
    let (active_tool, active_roi) = world
        .get::<&EditorState>(entities.editor)
        .map(|editor| (editor.active_tool, editor.active_roi))
        .map_err(|_| ContourDrawClickError::ToolNotActive)?;
    if active_tool != EditorTool::ContourDraw {
        return Err(ContourDrawClickError::ToolNotActive);
    }
    let roi_entity = active_roi.ok_or(ContourDrawClickError::MissingActiveRoi)?;

    let contour_data = contour_data_for_active_roi(world, roi_entity)
        .ok_or(ContourDrawClickError::ActiveRoiNotContour)?;
    let viewport = resolve_active_contour_edit_viewport(world, entities, &contour_data)?;
    let point_local_mm = viewport_uv_to_contour_plane_local_mm(viewport_uv, viewport)
        .ok_or(ContourDrawClickError::ProjectionFailed)?;
    let existing_slice = contour_data
        .slices
        .iter()
        .find(|slice| planes_match_for_slice(slice.plane, viewport.plane))
        .cloned();

    let viewport_rect = world
        .get::<&Viewport>(viewport.viewport_entity)
        .map(|vp| vp.rect)
        .map_err(|_| ContourDrawClickError::ProjectionFailed)?;

    let loop_points_to_commit = {
        let mut editor = world
            .get::<&mut EditorState>(entities.editor)
            .map_err(|_| ContourDrawClickError::ToolNotActive)?;
        let reset_draft = editor
            .contour_draft
            .as_ref()
            .map(|draft| {
                draft.roi_entity != roi_entity
                    || !planes_match_for_slice(draft.plane, viewport.plane)
            })
            .unwrap_or(true);
        if reset_draft {
            editor.contour_draft = Some(ContourDraft {
                roi_entity,
                plane: viewport.plane,
                points: Vec::new(),
            });
        }

        let draft = editor
            .contour_draft
            .as_mut()
            .ok_or(ContourDrawClickError::ToolNotActive)?;
        let near_first = if let Some(first) = draft.points.first() {
            let first_world = plane_local_mm_to_world_mm(first.local_mm, draft.plane);
            let first_volume_uv = world_mm_to_volume_uv(first_world, viewport.geometry);
            if let Some(first_uv) = volume_uv_to_viewport_uv(
                first_volume_uv,
                viewport.plane,
                viewport.geometry,
                viewport.mapping,
            ) {
                let dx = (viewport_uv[0] - first_uv[0]) * viewport_rect[2];
                let dy = (viewport_uv[1] - first_uv[1]) * viewport_rect[3];
                (dx * dx + dy * dy).sqrt() <= LOOP_CLOSE_RADIUS_PX
            } else {
                false
            }
        } else {
            false
        };

        let closes_through_existing = existing_slice.as_ref().is_some_and(|slice| {
            let contains_display_point = |local_mm| {
                contour_slice_contains_point(
                    slice,
                    reproject_plane_local_mm(local_mm, viewport.plane, slice.plane),
                )
            };
            draft.points.len() >= 2
                && draft
                    .points
                    .first()
                    .is_some_and(|first| contains_display_point(first.local_mm))
                && draft
                    .points
                    .iter()
                    .skip(1)
                    .any(|point| !contains_display_point(point.local_mm))
                && contains_display_point(point_local_mm)
        });

        if near_first && !draft.points.is_empty() {
            if draft.points.len() < 3 {
                return Err(ContourDrawClickError::LoopNeedsThreePoints);
            }
            Some(draft.points.clone())
        } else if closes_through_existing {
            let mut points = draft.points.clone();
            points.push(ContourPoint {
                local_mm: point_local_mm,
            });
            Some(points)
        } else {
            draft.points.push(ContourPoint {
                local_mm: point_local_mm,
            });
            return Ok(ContourDrawClickOutcome::PointAdded);
        }
    };

    if let Some(loop_points) = loop_points_to_commit {
        let mut next_contour_data = contour_data_for_active_roi(world, roi_entity)
            .ok_or(ContourDrawClickError::ActiveRoiNotContour)?;
        let mut loop_to_commit = ContourLoop {
            points: loop_points,
            is_closed: true,
        };
        let committed_plane;
        if let Some(existing_slice) = next_contour_data
            .slices
            .iter_mut()
            .find(|slice| planes_match_for_slice(slice.plane, viewport.plane))
        {
            committed_plane = existing_slice.plane;
            for point in &mut loop_to_commit.points {
                point.local_mm =
                    reproject_plane_local_mm(point.local_mm, viewport.plane, existing_slice.plane);
            }
            *existing_slice = union_contour_slice_with_loop(existing_slice, &loop_to_commit)
                .map_err(|_| ContourDrawClickError::CommitFailed)?;
        } else {
            committed_plane = viewport.plane;
            next_contour_data.slices.push(ContourSlice {
                plane: viewport.plane,
                loops: vec![loop_to_commit],
            });
        }

        roi::replace_contour_data_for_slice_with_history(
            world,
            entities.editor,
            roi_entity,
            next_contour_data,
            committed_plane,
        )
        .map_err(|_| ContourDrawClickError::CommitFailed)?;
        if let Ok(mut editor) = world.get::<&mut EditorState>(entities.editor) {
            editor.contour_draft = None;
        }
        Ok(ContourDrawClickOutcome::LoopCommitted)
    } else {
        Err(ContourDrawClickError::ToolNotActive)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContourSelectClickError {
    ToolNotActive,
    MissingActiveRoi,
    ActiveRoiNotContour,
    Mapping(ContourEditMappingError),
}

impl From<ContourEditMappingError> for ContourSelectClickError {
    fn from(value: ContourEditMappingError) -> Self {
        ContourSelectClickError::Mapping(value)
    }
}

fn nearest_point_hit(
    candidate_points_uv: &[(usize, usize, usize, [f32; 2])],
    click_uv: [f32; 2],
    viewport_px: [f32; 2],
    threshold_px: f32,
) -> Option<(usize, usize, usize)> {
    let mut best: Option<(usize, usize, usize, f32)> = None;
    for (slice_idx, loop_idx, point_idx, uv) in candidate_points_uv {
        let dx = (click_uv[0] - uv[0]) * viewport_px[0];
        let dy = (click_uv[1] - uv[1]) * viewport_px[1];
        let dist = (dx * dx + dy * dy).sqrt();
        if dist > threshold_px {
            continue;
        }
        match best {
            Some((_, _, _, best_dist)) if dist >= best_dist => {}
            _ => best = Some((*slice_idx, *loop_idx, *point_idx, dist)),
        }
    }
    best.map(|(slice, loop_idx, point, _)| (slice, loop_idx, point))
}

fn nearest_loop_hit(
    candidate_loops_uv: &[(usize, usize, Vec<[f32; 2]>, bool)],
    click_uv: [f32; 2],
    viewport_px: [f32; 2],
    threshold_px: f32,
) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize, f32)> = None;
    for (slice_idx, loop_idx, points, is_closed) in candidate_loops_uv {
        if points.len() < 2 {
            continue;
        }
        let mut local_best: Option<f32> = None;
        for window in points.windows(2) {
            let dist = point_to_segment_distance_px(click_uv, window[0], window[1], viewport_px);
            if dist <= threshold_px {
                local_best = Some(local_best.map_or(dist, |b| b.min(dist)));
            }
        }
        if *is_closed {
            let dist = point_to_segment_distance_px(
                click_uv,
                points[points.len() - 1],
                points[0],
                viewport_px,
            );
            if dist <= threshold_px {
                local_best = Some(local_best.map_or(dist, |b| b.min(dist)));
            }
        }
        if let Some(local_dist) = local_best {
            match best {
                Some((_, _, best_dist)) if local_dist >= best_dist => {}
                _ => best = Some((*slice_idx, *loop_idx, local_dist)),
            }
        }
    }
    best.map(|(slice, loop_idx, _)| (slice, loop_idx))
}

fn point_to_segment_distance_px(
    p_uv: [f32; 2],
    a_uv: [f32; 2],
    b_uv: [f32; 2],
    viewport_px: [f32; 2],
) -> f32 {
    let p = glam::Vec2::new(p_uv[0] * viewport_px[0], p_uv[1] * viewport_px[1]);
    let a = glam::Vec2::new(a_uv[0] * viewport_px[0], a_uv[1] * viewport_px[1]);
    let b = glam::Vec2::new(b_uv[0] * viewport_px[0], b_uv[1] * viewport_px[1]);
    let ab = b - a;
    let ab_len_sq = ab.length_squared();
    if ab_len_sq <= 1e-12 {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / ab_len_sq).clamp(0.0, 1.0);
    let closest = a + ab * t;
    (p - closest).length()
}

pub fn handle_contour_select_click(
    world: &mut World,
    entities: &AppEntities,
    viewport_uv: [f32; 2],
) -> Result<Option<ContourSelection>, ContourSelectClickError> {
    let (active_tool, active_roi) = world
        .get::<&EditorState>(entities.editor)
        .map(|editor| (editor.active_tool, editor.active_roi))
        .map_err(|_| ContourSelectClickError::ToolNotActive)?;
    if active_tool != EditorTool::ContourSelect {
        return Err(ContourSelectClickError::ToolNotActive);
    }
    let roi_entity = active_roi.ok_or(ContourSelectClickError::MissingActiveRoi)?;

    let contour_data = contour_data_for_active_roi(world, roi_entity)
        .ok_or(ContourSelectClickError::ActiveRoiNotContour)?;
    let viewport = resolve_active_contour_edit_viewport(world, entities, &contour_data)?;
    let viewport_rect = world
        .get::<&Viewport>(viewport.viewport_entity)
        .map(|vp| vp.rect)
        .map_err(|_| ContourSelectClickError::Mapping(ContourEditMappingError::MissingViewport))?;
    let viewport_px = [viewport_rect[2], viewport_rect[3]];

    let mut candidate_points = Vec::new();
    let mut candidate_loops = Vec::new();
    for (slice_idx, slice) in contour_data.slices.iter().enumerate() {
        if !planes_match_for_slice(slice.plane, viewport.plane) {
            continue;
        }
        for (loop_idx, contour_loop) in slice.loops.iter().enumerate() {
            let mut loop_points_uv = Vec::new();
            for (point_idx, point) in contour_loop.points.iter().enumerate() {
                let world_mm = plane_local_mm_to_world_mm(point.local_mm, slice.plane);
                let volume_uv = world_mm_to_volume_uv(world_mm, viewport.geometry);
                let Some(point_uv) = volume_uv_to_viewport_uv(
                    volume_uv,
                    viewport.plane,
                    viewport.geometry,
                    viewport.mapping,
                ) else {
                    continue;
                };
                candidate_points.push((slice_idx, loop_idx, point_idx, point_uv));
                loop_points_uv.push(point_uv);
            }
            candidate_loops.push((slice_idx, loop_idx, loop_points_uv, contour_loop.is_closed));
        }
    }

    let selection = if let Some((slice_idx, loop_idx, point_idx)) = nearest_point_hit(
        &candidate_points,
        viewport_uv,
        viewport_px,
        SELECTION_RADIUS_PX,
    ) {
        Some(ContourSelection {
            roi_entity,
            slice_index: slice_idx,
            loop_index: loop_idx,
            point_index: Some(point_idx),
        })
    } else if let Some((slice_idx, loop_idx)) = nearest_loop_hit(
        &candidate_loops,
        viewport_uv,
        viewport_px,
        SELECTION_RADIUS_PX,
    ) {
        Some(ContourSelection {
            roi_entity,
            slice_index: slice_idx,
            loop_index: loop_idx,
            point_index: None,
        })
    } else {
        None
    };

    if let Ok(mut editor) = world.get::<&mut EditorState>(entities.editor) {
        editor.contour_selection = selection.clone();
    }
    Ok(selection)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContourEditOperationError {
    MissingSelection,
    ActiveRoiMismatch,
    ActiveRoiNotContour,
    InvalidSelection,
    Mapping(ContourEditMappingError),
    ProjectionFailed,
    ReplaceFailed,
}

impl From<ContourEditMappingError> for ContourEditOperationError {
    fn from(value: ContourEditMappingError) -> Self {
        ContourEditOperationError::Mapping(value)
    }
}

fn selected_context(
    world: &World,
    entities: &AppEntities,
) -> Result<(ContourSelection, ContourData, ContourEditViewport), ContourEditOperationError> {
    let editor = world
        .get::<&EditorState>(entities.editor)
        .map_err(|_| ContourEditOperationError::MissingSelection)?;
    let selection = editor
        .contour_selection
        .clone()
        .ok_or(ContourEditOperationError::MissingSelection)?;
    let active_roi = editor
        .active_roi
        .ok_or(ContourEditOperationError::ActiveRoiMismatch)?;
    if active_roi != selection.roi_entity {
        return Err(ContourEditOperationError::ActiveRoiMismatch);
    }

    let contour_data = contour_data_for_active_roi(world, selection.roi_entity)
        .ok_or(ContourEditOperationError::ActiveRoiNotContour)?;
    let viewport = resolve_active_contour_edit_viewport(world, entities, &contour_data)?;
    Ok((selection, contour_data, viewport))
}

fn nearest_segment_index_in_loop(
    contour_loop: &ContourLoop,
    slice_plane: PlaneDefinition,
    viewport: ContourEditViewport,
    click_uv: [f32; 2],
    viewport_px: [f32; 2],
) -> Option<usize> {
    if contour_loop.points.len() < 2 {
        return None;
    }

    let mut projected = Vec::with_capacity(contour_loop.points.len());
    for point in &contour_loop.points {
        let world_mm = plane_local_mm_to_world_mm(point.local_mm, slice_plane);
        let volume_uv = world_mm_to_volume_uv(world_mm, viewport.geometry);
        let uv = volume_uv_to_viewport_uv(
            volume_uv,
            viewport.plane,
            viewport.geometry,
            viewport.mapping,
        )?;
        projected.push(uv);
    }

    let mut best: Option<(usize, f32)> = None;
    for idx in 0..projected.len() {
        let next = if idx + 1 < projected.len() {
            idx + 1
        } else if contour_loop.is_closed {
            0
        } else {
            continue;
        };
        let distance =
            point_to_segment_distance_px(click_uv, projected[idx], projected[next], viewport_px);
        match best {
            Some((_, best_distance)) if distance >= best_distance => {}
            _ => best = Some((idx, distance)),
        }
    }

    best.map(|(idx, _)| idx)
}

pub fn move_selected_point(
    world: &mut World,
    entities: &AppEntities,
    viewport_uv: [f32; 2],
) -> Result<(), ContourEditOperationError> {
    let (selection, mut contour_data, viewport) = selected_context(world, entities)?;
    let dirty_plane =
        update_selected_point_in_data(&selection, &mut contour_data, viewport, viewport_uv)?;
    roi::replace_contour_data_for_slice_with_history(
        world,
        entities.editor,
        selection.roi_entity,
        contour_data,
        dirty_plane,
    )
    .map_err(|_| ContourEditOperationError::ReplaceFailed)
}

pub fn move_selected_point_preview(
    world: &mut World,
    entities: &AppEntities,
    viewport_uv: [f32; 2],
) -> Result<(), ContourEditOperationError> {
    let (selection, mut contour_data, viewport) = selected_context(world, entities)?;
    let dirty_plane =
        update_selected_point_in_data(&selection, &mut contour_data, viewport, viewport_uv)?;
    roi::begin_contour_move_preview(
        world,
        entities.editor,
        selection.roi_entity,
        contour_data,
        dirty_plane,
    )
    .map(|_| ())
    .map_err(|_| ContourEditOperationError::ReplaceFailed)
}

fn update_selected_point_in_data(
    selection: &ContourSelection,
    contour_data: &mut ContourData,
    viewport: ContourEditViewport,
    viewport_uv: [f32; 2],
) -> Result<PlaneDefinition, ContourEditOperationError> {
    let point_index = selection
        .point_index
        .ok_or(ContourEditOperationError::InvalidSelection)?;
    let viewport_local_mm = viewport_uv_to_contour_plane_local_mm(viewport_uv, viewport)
        .ok_or(ContourEditOperationError::ProjectionFailed)?;

    let slice = contour_data
        .slices
        .get_mut(selection.slice_index)
        .ok_or(ContourEditOperationError::InvalidSelection)?;
    let dirty_plane = slice.plane;
    let local_mm = reproject_plane_local_mm(viewport_local_mm, viewport.plane, slice.plane);
    let contour_loop = slice
        .loops
        .get_mut(selection.loop_index)
        .ok_or(ContourEditOperationError::InvalidSelection)?;
    let point = contour_loop
        .points
        .get_mut(point_index)
        .ok_or(ContourEditOperationError::InvalidSelection)?;
    point.local_mm = local_mm;
    Ok(dirty_plane)
}

pub fn finalize_selected_point_move(
    world: &mut World,
    entities: &AppEntities,
) -> Result<(), ContourEditOperationError> {
    let dirty_plane = {
        let editor = world
            .get::<&EditorState>(entities.editor)
            .map_err(|_| ContourEditOperationError::MissingSelection)?;
        let preview = editor
            .contour_move_preview()
            .ok_or(ContourEditOperationError::MissingSelection)?;
        let selection = editor
            .contour_selection
            .as_ref()
            .filter(|selection| selection.roi_entity == preview.roi_entity)
            .ok_or(ContourEditOperationError::InvalidSelection)?;
        preview
            .contour_data
            .slices
            .get(selection.slice_index)
            .map(|slice| slice.plane)
            .ok_or(ContourEditOperationError::InvalidSelection)?
    };
    roi::commit_contour_move_preview(world, entities.editor, dirty_plane)
        .map_err(|_| ContourEditOperationError::ReplaceFailed)
}

pub fn insert_point_into_selected_loop(
    world: &mut World,
    entities: &AppEntities,
    viewport_uv: [f32; 2],
) -> Result<(), ContourEditOperationError> {
    let (selection, mut contour_data, viewport) = selected_context(world, entities)?;
    let viewport_rect = world
        .get::<&Viewport>(viewport.viewport_entity)
        .map(|vp| vp.rect)
        .map_err(|_| ContourEditOperationError::ProjectionFailed)?;
    let viewport_px = [viewport_rect[2], viewport_rect[3]];

    let slice = contour_data
        .slices
        .get_mut(selection.slice_index)
        .ok_or(ContourEditOperationError::InvalidSelection)?;
    let contour_loop = slice
        .loops
        .get_mut(selection.loop_index)
        .ok_or(ContourEditOperationError::InvalidSelection)?;
    if contour_loop.points.len() < 2 {
        return Err(ContourEditOperationError::InvalidSelection);
    }

    let segment_start_index = if let Some(point_idx) = selection.point_index {
        point_idx.min(contour_loop.points.len() - 1)
    } else {
        nearest_segment_index_in_loop(
            contour_loop,
            slice.plane,
            viewport,
            viewport_uv,
            viewport_px,
        )
        .ok_or(ContourEditOperationError::ProjectionFailed)?
    };
    let segment_end_index = if segment_start_index + 1 < contour_loop.points.len() {
        segment_start_index + 1
    } else if contour_loop.is_closed {
        0
    } else {
        return Err(ContourEditOperationError::InvalidSelection);
    };

    let inserted_local = viewport_uv_to_contour_plane_local_mm(viewport_uv, viewport)
        .map(|local_mm| reproject_plane_local_mm(local_mm, viewport.plane, slice.plane))
        .unwrap_or_else(|| {
            let start = contour_loop.points[segment_start_index].local_mm;
            let end = contour_loop.points[segment_end_index].local_mm;
            [(start[0] + end[0]) * 0.5, (start[1] + end[1]) * 0.5]
        });
    let dirty_plane = slice.plane;

    let insert_index = segment_start_index + 1;
    contour_loop.points.insert(
        insert_index,
        ContourPoint {
            local_mm: inserted_local,
        },
    );

    roi::replace_contour_data_for_slice_with_history(
        world,
        entities.editor,
        selection.roi_entity,
        contour_data,
        dirty_plane,
    )
    .map_err(|_| ContourEditOperationError::ReplaceFailed)?;
    if let Ok(mut editor) = world.get::<&mut EditorState>(entities.editor) {
        editor.contour_selection = Some(ContourSelection {
            roi_entity: selection.roi_entity,
            slice_index: selection.slice_index,
            loop_index: selection.loop_index,
            point_index: Some(insert_index),
        });
    }
    Ok(())
}

pub fn delete_selected_contour_element(
    world: &mut World,
    entities: &AppEntities,
) -> Result<(), ContourEditOperationError> {
    let (selection, mut contour_data, _) = selected_context(world, entities)?;
    let Some(slice) = contour_data.slices.get_mut(selection.slice_index) else {
        return Err(ContourEditOperationError::InvalidSelection);
    };
    if selection.loop_index >= slice.loops.len() {
        return Err(ContourEditOperationError::InvalidSelection);
    }
    let mut clear_selection = false;
    if let Some(point_idx) = selection.point_index {
        let contour_loop = &mut slice.loops[selection.loop_index];
        if point_idx >= contour_loop.points.len() {
            return Err(ContourEditOperationError::InvalidSelection);
        }
        contour_loop.points.remove(point_idx);
        if contour_loop.points.len() < 3 {
            slice.loops.remove(selection.loop_index);
            clear_selection = true;
        }
    } else {
        slice.loops.remove(selection.loop_index);
        clear_selection = true;
    }

    if slice.loops.is_empty() {
        contour_data.slices.remove(selection.slice_index);
        clear_selection = true;
    }

    roi::replace_contour_data_with_history(
        world,
        entities.editor,
        selection.roi_entity,
        contour_data,
    )
    .map_err(|_| ContourEditOperationError::ReplaceFailed)?;

    if let Ok(mut editor) = world.get::<&mut EditorState>(entities.editor) {
        if clear_selection {
            editor.contour_selection = None;
        } else if let Some(point_idx) = selection.point_index {
            let next_selection = world
                .get::<&Roi>(selection.roi_entity)
                .ok()
                .and_then(|roi| roi.contour_data().cloned())
                .and_then(|data| data.slices.get(selection.slice_index).cloned())
                .and_then(|slice_after| slice_after.loops.get(selection.loop_index).cloned())
                .map(|loop_after| ContourSelection {
                    roi_entity: selection.roi_entity,
                    slice_index: selection.slice_index,
                    loop_index: selection.loop_index,
                    point_index: Some(point_idx.min(loop_after.points.len().saturating_sub(1))),
                });
            editor.contour_selection = next_selection;
        } else {
            editor.contour_selection = None;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{
        ContourData, InputState, RoiAuthoritativeData, RoiCacheKind, RoiJobKind, ViewportState,
        VoxelCache, WindowSettings,
    };

    fn test_geometry() -> VoxelGeometry {
        VoxelGeometry {
            dimensions: [64, 48, 32],
            spacing: [1.0, 1.0, 1.0],
            origin: [0.0, 0.0, 0.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        }
    }

    fn spawn_test_entities(
        world: &mut World,
        mode: ViewMode,
        user_rotation: [f32; 4],
        active_viewport: Option<hecs::Entity>,
    ) -> AppEntities {
        let cursor = world.spawn((Transform {
            position: [0.4, 0.55, 0.2],
        },));
        let viewport = world.spawn((
            Viewport {
                mode,
                rect: [0.0, 0.0, 800.0, 600.0],
                uniform_index: 0,
            },
            ViewportState {
                zoom: 1.1,
                pan: [0.02, -0.01],
                pivot: [0.5, 0.5],
                user_rotation,
            },
        ));
        let input = world.spawn((InputState {
            active_viewport: active_viewport.or(Some(viewport)),
            ..InputState::default()
        },));
        world.spawn((
            crate::components::VolumeData {
                dimensions: test_geometry().dimensions,
                spacing: test_geometry().spacing,
                origin: test_geometry().origin,
                intensities: vec![],
                intensity_range: [0.0, 1.0],
                orientation: test_geometry().orientation,
            },
            MainVolumeTag,
        ));

        let editor = world.spawn((crate::components::EditorState::default(),));
        let gui_state = world.spawn((crate::components::GuiState {
            status_message: None,
        },));
        let volume_windowing = world.spawn((crate::components::VolumeWindowing::default(),));
        let annotations = world.spawn((crate::components::AnnotationState::default(),));
        let overlay = world.spawn((crate::overlay::OverlayManager::default(),));
        let protocol = world.spawn((crate::components::ProtocolState::default(),));
        let window_settings = world.spawn((WindowSettings {
            width: 800,
            height: 600,
            viewport_rect: [0.0, 0.0, 800.0, 600.0],
        },));

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
        }
    }

    fn spawn_test_contour_roi(world: &mut World, family: PlaneFamily) -> hecs::Entity {
        world.spawn((Roi::new_contour(
            crate::components::RoiId(100),
            "Contour".to_string(),
            ContourData {
                active_plane_family: family,
                slices: Vec::new(),
            },
        ),))
    }

    fn spawn_test_contour_roi_with_loop(world: &mut World, family: PlaneFamily) -> hecs::Entity {
        let geometry = VoxelGeometry {
            dimensions: [64, 48, 32],
            spacing: [1.0, 1.0, 1.0],
            origin: [0.0, 0.0, 0.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let plane = orthogonal_plane_from_volume_uv(family, [0.4, 0.55, 0.2], geometry).unwrap();
        let entity = world.spawn((Roi::new_contour_with_geometry(
            crate::components::RoiId(101),
            "ContourWithLoop".to_string(),
            crate::convert::RoiGeometry::from_legacy_parts(
                geometry.dimensions,
                geometry.spacing,
                geometry.origin,
                geometry.orientation,
            )
            .unwrap(),
            ContourData {
                active_plane_family: family,
                slices: vec![ContourSlice {
                    plane,
                    loops: vec![ContourLoop {
                        points: vec![
                            ContourPoint {
                                local_mm: [-5.0, -5.0],
                            },
                            ContourPoint {
                                local_mm: [5.0, -5.0],
                            },
                            ContourPoint {
                                local_mm: [5.0, 5.0],
                            },
                            ContourPoint {
                                local_mm: [-5.0, 5.0],
                            },
                        ],
                        is_closed: true,
                    }],
                }],
            },
        ),));
        world.get::<&mut Roi>(entity).unwrap().session_caches.voxel = Some(VoxelCache {
            data: VoxelData {
                geometry,
                raw_data: vec![0; 64 * 48 * 32],
            },
            gpu_resources: None,
        });
        entity
    }

    fn reframe_first_contour_slice(world: &mut World, roi_entity: hecs::Entity) {
        let mut roi = world.get::<&mut Roi>(roi_entity).unwrap();
        let RoiAuthoritativeData::Contour(contour) = &mut roi.authoritative_data else {
            panic!("expected contour authority");
        };
        let slice = &mut contour.slices[0];
        let old_plane = slice.plane;
        let old_u = glam::Vec3::from_array(old_plane.u_axis_mm);
        let old_v = glam::Vec3::from_array(old_plane.v_axis_mm);
        let old_origin = glam::Vec3::from_array(old_plane.origin_mm);
        let mut new_plane = old_plane;
        new_plane.origin_mm = (old_origin + old_u * 12.0 + old_v * 7.0).to_array();
        new_plane.u_axis_mm = old_v.to_array();
        new_plane.v_axis_mm = (-old_u).to_array();
        for contour_loop in &mut slice.loops {
            for point in &mut contour_loop.points {
                let world = plane_local_mm_to_world_mm(point.local_mm, old_plane);
                point.local_mm = crate::convert::world_mm_to_plane_local_mm(world, new_plane);
            }
        }
        slice.plane = new_plane;
    }

    #[test]
    fn test_contour_edit_rejects_viewport_family_mismatch() {
        let mut world = World::new();
        let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
        let contour = ContourData {
            active_plane_family: PlaneFamily::Coronal,
            slices: Vec::new(),
        };

        let result = resolve_active_contour_edit_viewport(&world, &entities, &contour);
        assert_eq!(
            result,
            Err(ContourEditMappingError::PlaneFamilyMismatch {
                contour_family: PlaneFamily::Coronal,
                viewport_family: PlaneFamily::Axial,
            })
        );
    }

    #[test]
    fn test_contour_edit_rejects_three_d_viewport() {
        let mut world = World::new();
        let entities =
            spawn_test_entities(&mut world, ViewMode::ThreeD, [0.0, 0.0, 0.0, 1.0], None);
        let contour = ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: Vec::new(),
        };

        let result = resolve_active_contour_edit_viewport(&world, &entities, &contour);
        assert_eq!(
            result,
            Err(ContourEditMappingError::UnsupportedViewportMode)
        );
    }

    #[test]
    fn test_contour_edit_rejects_invalid_main_volume_geometry() {
        let mut world = World::new();
        let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
        for (_, volume) in world
            .query_mut::<&mut crate::components::VolumeData>()
            .with::<&MainVolumeTag>()
        {
            volume.orientation = [0.0; 4];
        }
        let contour = ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: Vec::new(),
        };

        assert_eq!(
            resolve_active_contour_edit_viewport(&world, &entities, &contour),
            Err(ContourEditMappingError::InvalidMainVolumeGeometry)
        );
    }

    #[test]
    fn test_contour_edit_oblique_path_uses_plane_definition_roundtrip() {
        let mut world = World::new();
        let oblique_rotation =
            glam::Quat::from_euler(glam::EulerRot::XYZ, 0.3, -0.2, 0.15).to_array();
        let entities = spawn_test_entities(&mut world, ViewMode::Oblique, oblique_rotation, None);
        let contour = ContourData {
            active_plane_family: PlaneFamily::Oblique,
            slices: Vec::new(),
        };

        let viewport =
            resolve_active_contour_edit_viewport(&world, &entities, &contour).expect("viewport");
        assert_eq!(viewport.plane.family, PlaneFamily::Oblique);

        let click_a = [0.32, 0.67];
        let click_b = [0.61, 0.28];
        let local_a = viewport_uv_to_contour_plane_local_mm(click_a, viewport).expect("local mm a");
        let local_b = viewport_uv_to_contour_plane_local_mm(click_b, viewport).expect("local mm b");
        let viewport_a =
            contour_plane_local_mm_to_viewport_uv(local_a, viewport).expect("viewport uv a");
        let viewport_b =
            contour_plane_local_mm_to_viewport_uv(local_b, viewport).expect("viewport uv b");

        assert!(local_a[0].is_finite() && local_a[1].is_finite());
        assert!(local_b[0].is_finite() && local_b[1].is_finite());
        assert_ne!(local_a, local_b);
        assert!(viewport_a[0].is_finite() && viewport_a[1].is_finite());
        assert!(viewport_b[0].is_finite() && viewport_b[1].is_finite());
    }

    #[test]
    fn test_contour_draw_click_appends_draft_without_authoritative_mutation() {
        let mut world = World::new();
        let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
        let roi_entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial);
        {
            let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
            editor.active_roi = Some(roi_entity);
            editor.active_tool = EditorTool::ContourDraw;
        }

        let result = handle_contour_draw_click(&mut world, &entities, [0.4, 0.5]);
        assert_eq!(result, Ok(ContourDrawClickOutcome::PointAdded));

        let editor = world.get::<&EditorState>(entities.editor).unwrap();
        let draft = editor.contour_draft.as_ref().expect("draft");
        assert_eq!(draft.points.len(), 1);
        let roi = world.get::<&Roi>(roi_entity).unwrap();
        assert!(roi.contour_data().unwrap().slices.is_empty());
    }

    #[test]
    fn test_contour_draw_loop_closure_rejects_fewer_than_three_points() {
        let mut world = World::new();
        let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
        let roi_entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial);
        {
            let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
            editor.active_roi = Some(roi_entity);
            editor.active_tool = EditorTool::ContourDraw;
        }

        assert_eq!(
            handle_contour_draw_click(&mut world, &entities, [0.45, 0.45]),
            Ok(ContourDrawClickOutcome::PointAdded)
        );
        assert_eq!(
            handle_contour_draw_click(&mut world, &entities, [0.55, 0.45]),
            Ok(ContourDrawClickOutcome::PointAdded)
        );
        let close_result = handle_contour_draw_click(&mut world, &entities, [0.45, 0.45]);
        assert_eq!(
            close_result,
            Err(ContourDrawClickError::LoopNeedsThreePoints)
        );
    }

    #[test]
    fn test_contour_draw_loop_commit_adds_slice_loop_and_queues_voxel_rebuild() {
        let mut world = World::new();
        let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
        let roi_entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial);
        {
            let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
            editor.active_roi = Some(roi_entity);
            editor.active_tool = EditorTool::ContourDraw;
        }

        assert_eq!(
            handle_contour_draw_click(&mut world, &entities, [0.40, 0.40]),
            Ok(ContourDrawClickOutcome::PointAdded)
        );
        assert_eq!(
            handle_contour_draw_click(&mut world, &entities, [0.58, 0.42]),
            Ok(ContourDrawClickOutcome::PointAdded)
        );
        assert_eq!(
            handle_contour_draw_click(&mut world, &entities, [0.52, 0.62]),
            Ok(ContourDrawClickOutcome::PointAdded)
        );
        let close_result = handle_contour_draw_click(&mut world, &entities, [0.40, 0.40]);
        assert_eq!(close_result, Ok(ContourDrawClickOutcome::LoopCommitted));

        let roi = world.get::<&Roi>(roi_entity).unwrap();
        let contour = roi.contour_data().unwrap();
        assert_eq!(contour.slices.len(), 1);
        assert_eq!(contour.slices[0].loops.len(), 1);
        assert!(contour.slices[0].loops[0].is_closed);
        assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildVoxelCache));
        assert!(roi.job_state.pending.iter().any(|request| {
            request.kind == RoiJobKind::RebuildVoxelCache
                && request.dirty_region
                    == crate::components::RoiDirtyRegion::ContourSlice(
                        crate::components::ContourSliceKey::from_plane(contour.slices[0].plane),
                    )
        }));
        assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert!(world
            .get::<&EditorState>(entities.editor)
            .unwrap()
            .contour_draft
            .is_none());
    }

    #[test]
    fn test_add_loop_path_reentering_existing_contour_commits_union_and_fills_voxels() {
        let mut world = World::new();
        let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
        let roi_entity = spawn_test_contour_roi_with_loop(&mut world, PlaneFamily::Axial);
        let initial_contour = world
            .get::<&Roi>(roi_entity)
            .unwrap()
            .contour_data()
            .unwrap()
            .clone();
        let initial_voxel =
            crate::convert::rasterize_contours_to_voxel_data(&initial_contour, test_geometry())
                .unwrap();
        {
            let mut roi = world.get::<&mut Roi>(roi_entity).unwrap();
            roi.session_caches.voxel = Some(VoxelCache {
                data: initial_voxel,
                gpu_resources: None,
            });
            roi.dirty_state.voxel_cache_dirty = false;
            roi.dirty_state.generations.voxel = roi.dirty_state.generations.authoritative;
        }
        {
            let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
            editor.active_roi = Some(roi_entity);
            editor.active_tool = EditorTool::ContourDraw;
        }
        let viewport = resolve_active_contour_edit_viewport(&world, &entities, &initial_contour)
            .expect("edit viewport");
        let click_uvs = [[4.0, -2.0], [10.0, -5.0], [10.0, 5.0], [4.0, 2.0]]
            .map(|local_mm| contour_plane_local_mm_to_viewport_uv(local_mm, viewport).unwrap());

        for click_uv in &click_uvs[..3] {
            assert_eq!(
                handle_contour_draw_click(&mut world, &entities, *click_uv),
                Ok(ContourDrawClickOutcome::PointAdded)
            );
        }
        assert_eq!(
            handle_contour_draw_click(&mut world, &entities, click_uvs[3]),
            Ok(ContourDrawClickOutcome::LoopCommitted)
        );

        let merged_contour = world
            .get::<&Roi>(roi_entity)
            .unwrap()
            .contour_data()
            .unwrap()
            .clone();
        assert_eq!(merged_contour.slices[0].loops.len(), 1);
        assert!(contour_slice_contains_point(
            &merged_contour.slices[0],
            [0.0, 0.0]
        ));
        assert!(contour_slice_contains_point(
            &merged_contour.slices[0],
            [8.0, 0.0]
        ));

        roi_runtime::process_contour_voxel_rebuild_jobs(&mut world);

        let plane = merged_contour.slices[0].plane;
        let added_world = plane_local_mm_to_world_mm([8.0, 0.0], plane);
        let added_index = crate::convert::world_mm_to_voxel_index(added_world, test_geometry())
            .map(|value| value.round() as u32);
        let dimensions = test_geometry().dimensions;
        let linear = ((added_index[2] * dimensions[1] + added_index[1]) * dimensions[0]
            + added_index[0]) as usize;
        let roi = world.get::<&Roi>(roi_entity).unwrap();
        assert_eq!(roi.voxel_cache().unwrap().data.raw_data[linear], 1);
        assert!(roi.is_cache_current(RoiCacheKind::Voxel));
    }

    #[test]
    fn test_add_loop_reprojects_display_points_into_existing_coplanar_slice_frame() {
        let mut world = World::new();
        let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
        let roi_entity = spawn_test_contour_roi_with_loop(&mut world, PlaneFamily::Axial);
        reframe_first_contour_slice(&mut world, roi_entity);
        let contour = world
            .get::<&Roi>(roi_entity)
            .unwrap()
            .contour_data()
            .unwrap()
            .clone();
        let viewport = resolve_active_contour_edit_viewport(&world, &entities, &contour).unwrap();
        {
            let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
            editor.active_roi = Some(roi_entity);
            editor.active_tool = EditorTool::ContourDraw;
        }
        let click_uvs = [[4.0, -2.0], [10.0, -5.0], [10.0, 5.0], [4.0, 2.0]]
            .map(|local_mm| contour_plane_local_mm_to_viewport_uv(local_mm, viewport).unwrap());

        for click_uv in &click_uvs[..3] {
            assert_eq!(
                handle_contour_draw_click(&mut world, &entities, *click_uv),
                Ok(ContourDrawClickOutcome::PointAdded)
            );
        }
        assert_eq!(
            handle_contour_draw_click(&mut world, &entities, click_uvs[3]),
            Ok(ContourDrawClickOutcome::LoopCommitted)
        );

        let roi = world.get::<&Roi>(roi_entity).unwrap();
        let merged_slice = &roi.contour_data().unwrap().slices[0];
        let added_world = plane_local_mm_to_world_mm([8.0, 0.0], viewport.plane);
        let added_in_slice =
            crate::convert::world_mm_to_plane_local_mm(added_world, merged_slice.plane);
        assert!(contour_slice_contains_point(merged_slice, added_in_slice));
        assert!(roi.job_state.pending.iter().any(|request| {
            request.kind == RoiJobKind::RebuildVoxelCache
                && request.dirty_region
                    == crate::components::RoiDirtyRegion::ContourSlice(
                        crate::components::ContourSliceKey::from_plane(merged_slice.plane),
                    )
        }));
    }

    #[test]
    fn test_clear_contour_draft_when_tool_not_draw() {
        let mut world = World::new();
        let editor = world.spawn((EditorState {
            active_roi: Some(hecs::Entity::DANGLING),
            active_tool: EditorTool::Navigation,
            contour_draft: Some(ContourDraft {
                roi_entity: hecs::Entity::DANGLING,
                plane: PlaneDefinition {
                    family: PlaneFamily::Axial,
                    origin_mm: [0.0, 0.0, 0.0],
                    u_axis_mm: [1.0, 0.0, 0.0],
                    v_axis_mm: [0.0, 1.0, 0.0],
                    normal_mm: [0.0, 0.0, 1.0],
                },
                points: vec![],
            }),
            contour_selection: None,
            roi_edit_preview: None,
            ..EditorState::default()
        },));

        clear_contour_draft_if_inactive(&mut world, editor);
        assert!(world
            .get::<&EditorState>(editor)
            .unwrap()
            .contour_draft
            .is_none());
    }

    #[test]
    fn test_nearest_point_hit_selects_closest_point() {
        let candidates = vec![
            (0usize, 0usize, 0usize, [0.4, 0.4]),
            (0usize, 0usize, 1usize, [0.42, 0.4]),
            (0usize, 1usize, 0usize, [0.8, 0.8]),
        ];
        let result = nearest_point_hit(&candidates, [0.421, 0.401], [800.0, 600.0], 10.0);
        assert_eq!(result, Some((0, 0, 1)));
    }

    #[test]
    fn test_nearest_point_hit_respects_threshold() {
        let candidates = vec![(0usize, 0usize, 0usize, [0.4, 0.4])];
        let result = nearest_point_hit(&candidates, [0.6, 0.6], [800.0, 600.0], 5.0);
        assert_eq!(result, None);
    }

    #[test]
    fn test_contour_selection_rejects_non_contour_active_roi() {
        let mut world = World::new();
        let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
        let voxel_roi = world.spawn((Roi::new_voxel_with_cache(
            crate::components::RoiId(1),
            "Voxel".to_string(),
            VoxelGeometry {
                dimensions: [8, 8, 8],
                spacing: [1.0, 1.0, 1.0],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            vec![0; 512],
            None,
        ),));
        {
            let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
            editor.active_roi = Some(voxel_roi);
            editor.active_tool = EditorTool::ContourSelect;
        }

        let result = handle_contour_select_click(&mut world, &entities, [0.5, 0.5]);
        assert_eq!(result, Err(ContourSelectClickError::ActiveRoiNotContour));
    }

    #[test]
    fn test_contour_selection_rejects_mismatched_plane_family() {
        let mut world = World::new();
        let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
        let roi_entity = spawn_test_contour_roi(&mut world, PlaneFamily::Coronal);
        {
            let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
            editor.active_roi = Some(roi_entity);
            editor.active_tool = EditorTool::ContourSelect;
        }

        let result = handle_contour_select_click(&mut world, &entities, [0.5, 0.5]);
        assert_eq!(
            result,
            Err(ContourSelectClickError::Mapping(
                ContourEditMappingError::PlaneFamilyMismatch {
                    contour_family: PlaneFamily::Coronal,
                    viewport_family: PlaneFamily::Axial,
                }
            ))
        );
    }

    #[test]
    fn test_move_selected_point_updates_only_selected_point() {
        let mut world = World::new();
        let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
        let roi_entity = spawn_test_contour_roi_with_loop(&mut world, PlaneFamily::Axial);
        let before_points = world
            .get::<&Roi>(roi_entity)
            .unwrap()
            .contour_data()
            .unwrap()
            .slices[0]
            .loops[0]
            .points
            .clone();
        {
            let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
            editor.active_roi = Some(roi_entity);
            editor.active_tool = EditorTool::ContourSelect;
            editor.contour_selection = Some(ContourSelection {
                roi_entity,
                slice_index: 0,
                loop_index: 0,
                point_index: Some(1),
            });
        }

        move_selected_point(&mut world, &entities, [0.6, 0.55]).unwrap();

        let roi = world.get::<&Roi>(roi_entity).unwrap();
        let after_points = &roi.contour_data().unwrap().slices[0].loops[0].points;
        assert_ne!(after_points[1].local_mm, before_points[1].local_mm);
        assert_eq!(after_points[0].local_mm, before_points[0].local_mm);
        assert_eq!(after_points[2].local_mm, before_points[2].local_mm);
        assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildVoxelCache));
    }

    #[test]
    fn test_move_selected_point_reprojects_display_point_into_slice_frame() {
        let mut world = World::new();
        let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
        let roi_entity = spawn_test_contour_roi_with_loop(&mut world, PlaneFamily::Axial);
        reframe_first_contour_slice(&mut world, roi_entity);
        let contour = world
            .get::<&Roi>(roi_entity)
            .unwrap()
            .contour_data()
            .unwrap()
            .clone();
        let viewport = resolve_active_contour_edit_viewport(&world, &entities, &contour).unwrap();
        let target_local = [2.0, 3.0];
        let target_uv = contour_plane_local_mm_to_viewport_uv(target_local, viewport).unwrap();
        {
            let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
            editor.active_roi = Some(roi_entity);
            editor.active_tool = EditorTool::ContourSelect;
            editor.contour_selection = Some(ContourSelection {
                roi_entity,
                slice_index: 0,
                loop_index: 0,
                point_index: Some(1),
            });
        }

        move_selected_point(&mut world, &entities, target_uv).unwrap();

        let roi = world.get::<&Roi>(roi_entity).unwrap();
        let slice = &roi.contour_data().unwrap().slices[0];
        let actual_world =
            plane_local_mm_to_world_mm(slice.loops[0].points[1].local_mm, slice.plane);
        let expected_world = plane_local_mm_to_world_mm(target_local, viewport.plane);
        for axis in 0..3 {
            assert!((actual_world[axis] - expected_world[axis]).abs() < 1e-4);
        }
        assert!(roi.job_state.pending.iter().any(|request| {
            request.kind == RoiJobKind::RebuildVoxelCache
                && request.dirty_region
                    == crate::components::RoiDirtyRegion::ContourSlice(
                        crate::components::ContourSliceKey::from_plane(slice.plane),
                    )
        }));
    }

    #[test]
    fn test_move_selected_point_preview_defers_authoritative_commit_until_finalize() {
        let mut world = World::new();
        let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
        let roi_entity = spawn_test_contour_roi_with_loop(&mut world, PlaneFamily::Axial);
        let before_points = world
            .get::<&Roi>(roi_entity)
            .unwrap()
            .contour_data()
            .unwrap()
            .slices[0]
            .loops[0]
            .points
            .clone();
        {
            let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
            editor.active_roi = Some(roi_entity);
            editor.active_tool = EditorTool::ContourSelect;
            editor.contour_selection = Some(ContourSelection {
                roi_entity,
                slice_index: 0,
                loop_index: 0,
                point_index: Some(1),
            });
        }

        move_selected_point_preview(&mut world, &entities, [0.9, 0.55]).unwrap();

        {
            let roi = world.get::<&Roi>(roi_entity).unwrap();
            let authoritative_points = &roi.contour_data().unwrap().slices[0].loops[0].points;
            assert_eq!(authoritative_points, &before_points);
            assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildVoxelCache));
            assert!(roi.preview_state.active);
            assert_eq!(roi.preview_state.revision, 1);
        }
        {
            let editor = world.get::<&EditorState>(entities.editor).unwrap();
            assert!(editor.contour_move_preview().is_some());
            assert!(editor.roi_undo_stack.is_empty());
        }

        roi_runtime::process_contour_voxel_rebuild_jobs(&mut world);
        {
            let roi = world.get::<&Roi>(roi_entity).unwrap();
            assert!(roi.session_caches.preview_voxel.is_some());
            assert!(roi.session_caches.preview_mesh.is_none());
            assert_eq!(roi.running_job_kind(), Some(RoiJobKind::RebuildVoxelCache));
        }
        move_selected_point_preview(&mut world, &entities, [0.6, 0.55]).unwrap();
        for _ in 0..8 {
            roi_runtime::process_contour_voxel_rebuild_jobs(&mut world);
            if world.get::<&Roi>(roi_entity).is_ok_and(|roi| {
                roi.session_caches
                    .preview_mesh
                    .as_ref()
                    .is_some_and(|cache| cache.preview_revision == 2)
            }) {
                break;
            }
        }
        let geometry = roi_runtime::main_volume_voxel_geometry(&world).unwrap();
        let cross_plane =
            orthogonal_plane_from_volume_uv(PlaneFamily::Coronal, [0.5, 0.5, 0.5], geometry)
                .unwrap();
        let cross_key = crate::components::ContourViewKey::from_plane(cross_plane);
        let cross_status =
            roi_runtime::ensure_contour_view_cache(&mut world, roi_entity, &cross_key);
        assert_eq!(
            cross_status.state,
            roi_runtime::RepresentationRequestState::Preview
        );
        {
            let roi = world.get::<&Roi>(roi_entity).unwrap();
            assert!(roi.session_caches.preview_voxel.is_some());
            assert!(roi
                .session_caches
                .preview_mesh
                .as_ref()
                .is_some_and(|cache| cache.chunks.is_some()));
            let preview_voxel = &roi.session_caches.preview_voxel.as_ref().unwrap().data;
            let clean_chunks = crate::convert::extract_chunked_mesh_from_voxel_data(
                preview_voxel,
                crate::convert::DEFAULT_MESH_CHUNK_SIZE,
            )
            .unwrap();
            let preview_chunks = roi
                .session_caches
                .preview_mesh
                .as_ref()
                .unwrap()
                .chunks
                .as_ref()
                .unwrap();
            assert_eq!(
                preview_chunks
                    .chunks
                    .iter()
                    .map(|chunk| chunk.key)
                    .collect::<Vec<_>>(),
                clean_chunks
                    .chunks
                    .iter()
                    .map(|chunk| chunk.key)
                    .collect::<Vec<_>>()
            );
            for (preview, clean) in preview_chunks.chunks.iter().zip(&clean_chunks.chunks) {
                assert_eq!(preview.key, clean.key);
                assert_eq!(
                    preview.data.faces.len(),
                    clean.data.faces.len(),
                    "{:?}",
                    preview.key
                );
                assert_eq!(
                    preview.data.vertices.len(),
                    clean.data.vertices.len(),
                    "{:?}",
                    preview.key
                );
            }
            assert_eq!(
                roi.session_caches.preview_mesh.as_ref().unwrap().data,
                clean_chunks.merged_mesh()
            );
            assert!(matches!(
                roi.contour_view_cache(&cross_key).map(|cache| &cache.state),
                Some(crate::components::CacheViewState::Preview { revision: 2 })
            ));
            assert_eq!(roi.running_job_kind(), None);
            assert_eq!(roi.queued_job_kind(), None);
        }

        finalize_selected_point_move(&mut world, &entities).unwrap();

        let roi = world.get::<&Roi>(roi_entity).unwrap();
        let after_points = &roi.contour_data().unwrap().slices[0].loops[0].points;
        assert_ne!(after_points[1].local_mm, before_points[1].local_mm);
        assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildVoxelCache));
        assert!(!roi.preview_state.active);
        {
            let editor = world.get::<&EditorState>(entities.editor).unwrap();
            assert!(editor.contour_move_preview().is_none());
            assert_eq!(editor.roi_undo_stack.len(), 1);
        }
        drop(roi);

        roi_runtime::process_contour_voxel_rebuild_jobs(&mut world);

        let roi = world.get::<&Roi>(roi_entity).unwrap();
        assert!(roi.is_cache_current(RoiCacheKind::Voxel));
        assert_eq!(roi.running_job_kind(), None);
        assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildMeshCache));
    }

    #[test]
    fn test_insert_point_adds_at_expected_loop_position() {
        let mut world = World::new();
        let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
        let roi_entity = spawn_test_contour_roi_with_loop(&mut world, PlaneFamily::Axial);
        {
            let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
            editor.active_roi = Some(roi_entity);
            editor.active_tool = EditorTool::ContourSelect;
            editor.contour_selection = Some(ContourSelection {
                roi_entity,
                slice_index: 0,
                loop_index: 0,
                point_index: Some(1),
            });
        }

        insert_point_into_selected_loop(&mut world, &entities, [0.55, 0.45]).unwrap();

        let roi = world.get::<&Roi>(roi_entity).unwrap();
        let points = &roi.contour_data().unwrap().slices[0].loops[0].points;
        assert_eq!(points.len(), 5);
        assert_eq!(
            world
                .get::<&EditorState>(entities.editor)
                .unwrap()
                .contour_selection,
            Some(ContourSelection {
                roi_entity,
                slice_index: 0,
                loop_index: 0,
                point_index: Some(2),
            })
        );
        assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildVoxelCache));
    }

    #[test]
    fn test_insert_point_reprojects_display_point_into_slice_frame() {
        let mut world = World::new();
        let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
        let roi_entity = spawn_test_contour_roi_with_loop(&mut world, PlaneFamily::Axial);
        reframe_first_contour_slice(&mut world, roi_entity);
        let contour = world
            .get::<&Roi>(roi_entity)
            .unwrap()
            .contour_data()
            .unwrap()
            .clone();
        let viewport = resolve_active_contour_edit_viewport(&world, &entities, &contour).unwrap();
        let target_local = [2.0, 3.0];
        let target_uv = contour_plane_local_mm_to_viewport_uv(target_local, viewport).unwrap();
        {
            let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
            editor.active_roi = Some(roi_entity);
            editor.active_tool = EditorTool::ContourSelect;
            editor.contour_selection = Some(ContourSelection {
                roi_entity,
                slice_index: 0,
                loop_index: 0,
                point_index: Some(1),
            });
        }

        insert_point_into_selected_loop(&mut world, &entities, target_uv).unwrap();

        let roi = world.get::<&Roi>(roi_entity).unwrap();
        let slice = &roi.contour_data().unwrap().slices[0];
        let actual_world =
            plane_local_mm_to_world_mm(slice.loops[0].points[2].local_mm, slice.plane);
        let expected_world = plane_local_mm_to_world_mm(target_local, viewport.plane);
        for axis in 0..3 {
            assert!((actual_world[axis] - expected_world[axis]).abs() < 1e-4);
        }
    }

    #[test]
    fn test_delete_selected_point_removes_expected_point() {
        let mut world = World::new();
        let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
        let roi_entity = spawn_test_contour_roi_with_loop(&mut world, PlaneFamily::Axial);
        {
            let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
            editor.active_roi = Some(roi_entity);
            editor.active_tool = EditorTool::ContourSelect;
            editor.contour_selection = Some(ContourSelection {
                roi_entity,
                slice_index: 0,
                loop_index: 0,
                point_index: Some(1),
            });
        }

        delete_selected_contour_element(&mut world, &entities).unwrap();

        let roi = world.get::<&Roi>(roi_entity).unwrap();
        let points = &roi.contour_data().unwrap().slices[0].loops[0].points;
        assert_eq!(points.len(), 3);
        assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildVoxelCache));
    }

    #[test]
    fn test_delete_below_valid_size_removes_loop_and_clears_selection() {
        let mut world = World::new();
        let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
        let geometry = VoxelGeometry {
            dimensions: [64, 48, 32],
            spacing: [1.0, 1.0, 1.0],
            origin: [0.0, 0.0, 0.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let plane = orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.4, 0.55, 0.2], geometry)
            .unwrap();
        let roi_entity = world.spawn((Roi::new_contour(
            crate::components::RoiId(102),
            "TinyLoop".to_string(),
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
                                local_mm: [2.0, 0.0],
                            },
                            ContourPoint {
                                local_mm: [1.0, 2.0],
                            },
                        ],
                        is_closed: true,
                    }],
                }],
            },
        ),));
        {
            let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
            editor.active_roi = Some(roi_entity);
            editor.active_tool = EditorTool::ContourSelect;
            editor.contour_selection = Some(ContourSelection {
                roi_entity,
                slice_index: 0,
                loop_index: 0,
                point_index: Some(1),
            });
        }

        delete_selected_contour_element(&mut world, &entities).unwrap();

        let roi = world.get::<&Roi>(roi_entity).unwrap();
        assert!(roi.contour_data().unwrap().slices.is_empty());
        assert!(world
            .get::<&EditorState>(entities.editor)
            .unwrap()
            .contour_selection
            .is_none());
        assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildVoxelCache));
    }
}
