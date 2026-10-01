use crate::app::roi::{self, EditTarget, Readiness, SwitchError};
#[cfg(test)]
use crate::app::roi_runtime;
use crate::components::Session;
#[cfg(test)]
use crate::components::VoxelData;
use crate::components::{
    ContourData, ContourDraft, ContourLoop, ContourPoint, ContourSelection, ContourSlice,
    EditorState, EditorTool, MainVolumeTag, Roi, ViewMode, Viewport, VoxelGeometry,
};
use crate::convert::{
    contour_slice_contains_point, nearest_matching_slice, oblique_plane_from_view_rotation,
    orthogonal_plane_from_volume_uv, plane_local_mm_to_world_mm, planes_are_same_slice,
    reproject_plane_local_mm, union_contour_slice_with_loop, viewport_uv_to_plane_local_mm,
    volume_uv_to_viewport_uv, world_mm_to_volume_uv, PlaneDefinition, PlaneFamily, ViewportMapping,
};
use hecs::World;

const LOOP_CLOSE_RADIUS_PX: f32 = 10.0;
const SELECTION_RADIUS_PX: f32 = 10.0;

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
    session: &Session,
    contour_data: &ContourData,
) -> Result<ContourEditViewport, ContourEditMappingError> {
    let viewport = resolve_active_edit_view(world, session)?;
    if viewport.plane.family != contour_data.active_plane_family {
        return Err(ContourEditMappingError::PlaneFamilyMismatch {
            contour_family: contour_data.active_plane_family.into(),
            viewport_family: viewport.plane.family,
        });
    }
    Ok(viewport)
}

/// The active 2D viewport and the plane it shows, whatever family the ROI's contours are in.
fn resolve_active_edit_view(
    world: &World,
    session: &Session,
) -> Result<ContourEditViewport, ContourEditMappingError> {
    let viewport_entity = session
        .input
        .active_viewport
        .ok_or(ContourEditMappingError::MissingActiveViewport)?;

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
        crate::app::roi_runtime::main_volume_geometry(world).ok_or(if has_main_volume {
            ContourEditMappingError::InvalidMainVolumeGeometry
        } else {
            ContourEditMappingError::MissingMainVolume
        })?;

    let cursor = &session.cursor;

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

#[cfg(test)]
pub fn contour_plane_local_mm_to_viewport_uv(
    local_mm: [f32; 2],
    viewport: ContourEditViewport,
) -> Option<[f32; 2]> {
    crate::convert::plane_local_mm_to_viewport_uv(
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
    /// The ROI is being prepared for contour editing; the click is ignored.
    SwitchPending,
    Switch(SwitchError),
    ProjectionFailed,
    LoopNeedsThreePoints,
    CommitFailed,
}

impl From<ContourEditMappingError> for ContourDrawClickError {
    fn from(value: ContourEditMappingError) -> Self {
        ContourDrawClickError::Mapping(value)
    }
}

/// Why an ROI could not be made contour-editable in the active view.
enum ContourPrepareError {
    Mapping(ContourEditMappingError),
    Pending,
    Switch(SwitchError),
}

impl From<ContourPrepareError> for ContourDrawClickError {
    fn from(value: ContourPrepareError) -> Self {
        match value {
            ContourPrepareError::Mapping(error) => Self::Mapping(error),
            ContourPrepareError::Pending => Self::SwitchPending,
            ContourPrepareError::Switch(error) => Self::Switch(error),
        }
    }
}

impl From<ContourPrepareError> for ContourSelectClickError {
    fn from(value: ContourPrepareError) -> Self {
        match value {
            ContourPrepareError::Mapping(error) => Self::Mapping(error),
            ContourPrepareError::Pending => Self::SwitchPending,
            ContourPrepareError::Switch(error) => Self::Switch(error),
        }
    }
}

/// Makes the ROI's contours editable in the family of the active viewport, converting the ROI
/// if needed (see `app::roi::switch`). Editing works in whichever 2D view the user is in.
fn prepare_contour_edit(
    world: &mut World,
    session: &mut Session,
    roi_entity: hecs::Entity,
) -> Result<(), ContourPrepareError> {
    let view_family = resolve_active_edit_view(world, session)
        .map_err(ContourPrepareError::Mapping)?
        .plane
        .family;
    // Oblique views are derived per-slice views; only the orthogonal families are editable.
    let family = view_family
        .orthogonal()
        .ok_or(ContourPrepareError::Switch(SwitchError::UnsupportedTarget))?;
    match roi::ensure_editable(world, roi_entity, EditTarget::Contour(family))
        .map_err(ContourPrepareError::Switch)?
    {
        Readiness::Ready => Ok(()),
        Readiness::Pending => Err(ContourPrepareError::Pending),
        Readiness::Switched(report) => {
            {
                let editor = &mut session.editor;
                editor.contour_draft = None;
                editor.contour_selection = None;
            }
            crate::app::handlers::set_status_message(session, report.message());
            Ok(())
        }
    }
}

fn contour_data_for_active_roi(world: &World, roi_entity: hecs::Entity) -> Option<ContourData> {
    world
        .get::<&Roi>(roi_entity)
        .ok()
        .and_then(|roi| roi.contour_data().cloned())
}

pub fn clear_contour_draft_if_inactive(editor: &mut EditorState) {
    if editor.active_tool != EditorTool::ContourDraw {
        editor.contour_draft = None;
    }
}

/// Ends the contour drag preview of `roi_entity`, if it has one.
fn end_contour_move_preview_of(world: &mut World, roi_entity: Option<hecs::Entity>) {
    let Some(roi_entity) = roi_entity else {
        return;
    };
    if world
        .get::<&Roi>(roi_entity)
        .is_ok_and(|roi| roi.contour_move_preview().is_some())
    {
        roi::end_roi_preview(world, roi_entity);
    }
}

/// Clears the draft of the previously active ROI and ends its in-flight edit when the active ROI
/// is about to change to `new_active_roi`.
pub fn clear_contour_draft_for_roi_change(
    world: &mut World,
    editor: &mut EditorState,
    new_active_roi: Option<hecs::Entity>,
) {
    let mut previous_active_roi = None;
    if editor.active_roi != new_active_roi {
        editor.contour_draft = None;
        editor.mesh_selection = None;
        previous_active_roi = editor.active_roi;
    }
    if let Some(roi_entity) = previous_active_roi {
        if world
            .get::<&Roi>(roi_entity)
            .is_ok_and(|roi| roi.has_edit_preview())
        {
            roi::end_roi_preview(world, roi_entity);
        }
    }
}

pub fn clear_contour_selection_for_roi_change(
    world: &mut World,
    editor: &mut EditorState,
    new_active_roi: Option<hecs::Entity>,
) {
    let mut previous_active_roi = None;
    if editor.active_roi != new_active_roi {
        editor.contour_selection = None;
        editor.mesh_selection = None;
        previous_active_roi = editor.active_roi;
    }
    end_contour_move_preview_of(world, previous_active_roi);
}

pub fn clear_contour_selection_if_inactive(world: &mut World, editor: &mut EditorState) {
    let mut active_roi = None;
    if editor.active_tool != EditorTool::ContourSelect {
        editor.contour_selection = None;
        active_roi = editor.active_roi;
    }
    end_contour_move_preview_of(world, active_roi);
}

pub fn handle_contour_draw_click(
    world: &mut World,
    session: &mut Session,
    viewport_uv: [f32; 2],
) -> Result<ContourDrawClickOutcome, ContourDrawClickError> {
    let (active_tool, active_roi) = {
        let editor = &session.editor;
        (editor.active_tool, editor.active_roi)
    };
    if active_tool != EditorTool::ContourDraw {
        return Err(ContourDrawClickError::ToolNotActive);
    }
    let roi_entity = active_roi.ok_or(ContourDrawClickError::MissingActiveRoi)?;
    prepare_contour_edit(world, session, roi_entity)?;

    let contour_data = contour_data_for_active_roi(world, roi_entity)
        .ok_or(ContourDrawClickError::ActiveRoiNotContour)?;
    let viewport = resolve_active_contour_edit_viewport(world, session, &contour_data)?;
    let point_local_mm = viewport_uv_to_contour_plane_local_mm(viewport_uv, viewport)
        .ok_or(ContourDrawClickError::ProjectionFailed)?;
    let existing_slice = nearest_matching_slice(
        contour_data
            .slices
            .iter()
            .enumerate()
            .map(|(index, slice)| (index, slice.plane)),
        viewport.plane,
        viewport.geometry,
    )
    .map(|index| contour_data.slices[index].clone());

    let viewport_rect = world
        .get::<&Viewport>(viewport.viewport_entity)
        .map(|vp| vp.rect)
        .map_err(|_| ContourDrawClickError::ProjectionFailed)?;

    let loop_points_to_commit = {
        let editor = &mut session.editor;
        let reset_draft = editor
            .contour_draft
            .as_ref()
            .map(|draft| {
                draft.roi_entity != roi_entity
                    || !planes_are_same_slice(draft.plane, viewport.plane, viewport.geometry)
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
        let existing_index = nearest_matching_slice(
            next_contour_data
                .slices
                .iter()
                .enumerate()
                .map(|(index, slice)| (index, slice.plane)),
            viewport.plane,
            viewport.geometry,
        );
        if let Some(existing_slice) =
            existing_index.map(|index| &mut next_contour_data.slices[index])
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
            roi_entity,
            next_contour_data,
            committed_plane,
        )
        .map_err(|_| ContourDrawClickError::CommitFailed)?;
        {
            let editor = &mut session.editor;
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
    /// The ROI is being prepared for contour editing; the click is ignored.
    SwitchPending,
    Switch(SwitchError),
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
    session: &mut Session,
    viewport_uv: [f32; 2],
) -> Result<Option<ContourSelection>, ContourSelectClickError> {
    let (active_tool, active_roi) = {
        let editor = &session.editor;
        (editor.active_tool, editor.active_roi)
    };
    if active_tool != EditorTool::ContourSelect {
        return Err(ContourSelectClickError::ToolNotActive);
    }
    let roi_entity = active_roi.ok_or(ContourSelectClickError::MissingActiveRoi)?;
    prepare_contour_edit(world, session, roi_entity)?;

    let contour_data = contour_data_for_active_roi(world, roi_entity)
        .ok_or(ContourSelectClickError::ActiveRoiNotContour)?;
    let viewport = resolve_active_contour_edit_viewport(world, session, &contour_data)?;
    let viewport_rect = world
        .get::<&Viewport>(viewport.viewport_entity)
        .map(|vp| vp.rect)
        .map_err(|_| ContourSelectClickError::Mapping(ContourEditMappingError::MissingViewport))?;
    let viewport_px = [viewport_rect[2], viewport_rect[3]];

    let mut candidate_points = Vec::new();
    let mut candidate_loops = Vec::new();
    let selectable_slice = nearest_matching_slice(
        contour_data
            .slices
            .iter()
            .enumerate()
            .map(|(index, slice)| (index, slice.plane)),
        viewport.plane,
        viewport.geometry,
    );
    for (slice_idx, slice) in contour_data.slices.iter().enumerate() {
        if Some(slice_idx) != selectable_slice {
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

    {
        let editor = &mut session.editor;
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
    session: &Session,
) -> Result<(ContourSelection, ContourData, ContourEditViewport), ContourEditOperationError> {
    let editor = &session.editor;
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
    let viewport = resolve_active_contour_edit_viewport(world, session, &contour_data)?;
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

#[cfg(test)]
pub fn move_selected_point(
    world: &mut World,
    session: &Session,
    viewport_uv: [f32; 2],
) -> Result<(), ContourEditOperationError> {
    let (selection, mut contour_data, viewport) = selected_context(world, session)?;
    let dirty_plane =
        update_selected_point_in_data(&selection, &mut contour_data, viewport, viewport_uv)?;
    roi::replace_contour_data_for_slice_with_history(
        world,
        selection.roi_entity,
        contour_data,
        dirty_plane,
    )
    .map_err(|_| ContourEditOperationError::ReplaceFailed)
}

pub fn move_selected_point_preview(
    world: &mut World,
    session: &Session,
    viewport_uv: [f32; 2],
) -> Result<(), ContourEditOperationError> {
    let (selection, mut contour_data, viewport) = selected_context(world, session)?;
    let dirty_plane =
        update_selected_point_in_data(&selection, &mut contour_data, viewport, viewport_uv)?;
    roi::begin_contour_move_preview(world, selection.roi_entity, contour_data, dirty_plane)
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
    session: &Session,
) -> Result<(), ContourEditOperationError> {
    let dirty_plane = {
        let editor = &session.editor;
        let selection = editor
            .contour_selection
            .as_ref()
            .ok_or(ContourEditOperationError::MissingSelection)?;
        let roi = world
            .get::<&Roi>(selection.roi_entity)
            .map_err(|_| ContourEditOperationError::InvalidSelection)?;
        let preview = roi
            .contour_move_preview()
            .ok_or(ContourEditOperationError::MissingSelection)?;
        preview
            .contour_data
            .slices
            .get(selection.slice_index)
            .map(|slice| slice.plane)
            .ok_or(ContourEditOperationError::InvalidSelection)?
    };
    roi::commit_contour_move_preview(world, &session.editor, dirty_plane)
        .map_err(|_| ContourEditOperationError::ReplaceFailed)
}

pub fn insert_point_into_selected_loop(
    world: &mut World,
    session: &mut Session,
    viewport_uv: [f32; 2],
) -> Result<(), ContourEditOperationError> {
    let (selection, mut contour_data, viewport) = selected_context(world, session)?;
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
        selection.roi_entity,
        contour_data,
        dirty_plane,
    )
    .map_err(|_| ContourEditOperationError::ReplaceFailed)?;
    {
        let editor = &mut session.editor;
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
    session: &mut Session,
) -> Result<(), ContourEditOperationError> {
    let (selection, mut contour_data, _) = selected_context(world, session)?;
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

    roi::replace_contour_data_with_history(world, selection.roi_entity, contour_data)
        .map_err(|_| ContourEditOperationError::ReplaceFailed)?;

    {
        let editor = &mut session.editor;
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
mod tests;
