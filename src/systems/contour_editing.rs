use crate::app::roi_runtime;
use crate::components::{
    AppEntities, ContourData, ContourDraft, ContourLoop, ContourPoint, ContourSelection,
    ContourSlice, EditorState, EditorTool, InputState, MainVolumeTag, Roi, RoiCacheKind,
    RoiJobKind, Transform, ViewMode, Viewport, VoxelGeometry,
};
use crate::convert::{
    oblique_plane_from_view_rotation, orthogonal_plane_from_volume_uv,
    plane_local_mm_to_viewport_uv, plane_local_mm_to_world_mm, viewport_uv_to_plane_local_mm,
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

    let geometry = {
        let mut main_volume_query = world
            .query::<&crate::components::VolumeData>()
            .with::<&MainVolumeTag>();
        let (_, volume) = main_volume_query
            .iter()
            .next()
            .ok_or(ContourEditMappingError::MissingMainVolume)?;
        VoxelGeometry {
            dimensions: volume.dimensions,
            spacing: volume.spacing,
            origin: volume.origin,
            orientation: volume.orientation,
        }
    };

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
    if let Ok(mut editor) = world.get::<&mut EditorState>(editor_entity) {
        if editor.active_roi != new_active_roi {
            editor.contour_draft = None;
        }
    }
}

pub fn clear_contour_selection_for_roi_change(
    world: &mut World,
    editor_entity: hecs::Entity,
    new_active_roi: Option<hecs::Entity>,
) {
    if let Ok(mut editor) = world.get::<&mut EditorState>(editor_entity) {
        if editor.active_roi != new_active_roi {
            editor.contour_selection = None;
        }
    }
}

pub fn clear_contour_selection_if_inactive(world: &mut World, editor_entity: hecs::Entity) {
    if let Ok(mut editor) = world.get::<&mut EditorState>(editor_entity) {
        if editor.active_tool != EditorTool::ContourSelect {
            editor.contour_selection = None;
        }
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

        if near_first && !draft.points.is_empty() {
            if draft.points.len() < 3 {
                return Err(ContourDrawClickError::LoopNeedsThreePoints);
            }
            Some(draft.points.clone())
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
        let loop_to_commit = ContourLoop {
            points: loop_points,
            is_closed: true,
        };
        if let Some(existing_slice) = next_contour_data
            .slices
            .iter_mut()
            .find(|slice| planes_match_for_slice(slice.plane, viewport.plane))
        {
            existing_slice.loops.push(loop_to_commit);
        } else {
            next_contour_data.slices.push(ContourSlice {
                plane: viewport.plane,
                loops: vec![loop_to_commit],
            });
        }

        roi_runtime::replace_contour_data(world, roi_entity, next_contour_data)
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
    move_selected_point_internal(world, entities, viewport_uv, true)
}

pub fn move_selected_point_preview(
    world: &mut World,
    entities: &AppEntities,
    viewport_uv: [f32; 2],
) -> Result<(), ContourEditOperationError> {
    move_selected_point_internal(world, entities, viewport_uv, false)
}

fn move_selected_point_internal(
    world: &mut World,
    entities: &AppEntities,
    viewport_uv: [f32; 2],
    queue_rebuild: bool,
) -> Result<(), ContourEditOperationError> {
    let (selection, _contour_data, viewport) = selected_context(world, entities)?;
    let point_index = selection
        .point_index
        .ok_or(ContourEditOperationError::InvalidSelection)?;
    let local_mm = viewport_uv_to_contour_plane_local_mm(viewport_uv, viewport)
        .ok_or(ContourEditOperationError::ProjectionFailed)?;

    let mut roi = world
        .get::<&mut Roi>(selection.roi_entity)
        .map_err(|_| ContourEditOperationError::ActiveRoiNotContour)?;
    let contour = match &mut roi.authoritative_data {
        crate::components::RoiAuthoritativeData::Contour(contour) => contour,
        _ => return Err(ContourEditOperationError::ActiveRoiNotContour),
    };
    let slice = contour
        .slices
        .get_mut(selection.slice_index)
        .ok_or(ContourEditOperationError::InvalidSelection)?;
    let contour_loop = slice
        .loops
        .get_mut(selection.loop_index)
        .ok_or(ContourEditOperationError::InvalidSelection)?;
    let point = contour_loop
        .points
        .get_mut(point_index)
        .ok_or(ContourEditOperationError::InvalidSelection)?;
    point.local_mm = local_mm;

    roi.mark_contour_authoritative_changed();
    if queue_rebuild {
        roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
    }
    Ok(())
}

pub fn finalize_selected_point_move(
    world: &mut World,
    entities: &AppEntities,
) -> Result<(), ContourEditOperationError> {
    let (selection, _, _) = selected_context(world, entities)?;
    let mut roi = world
        .get::<&mut Roi>(selection.roi_entity)
        .map_err(|_| ContourEditOperationError::ActiveRoiNotContour)?;
    if !roi.is_cache_dirty(RoiCacheKind::Voxel) {
        roi.mark_cache_dirty(RoiCacheKind::Voxel);
    }
    roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
    Ok(())
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
        .unwrap_or_else(|| {
            let start = contour_loop.points[segment_start_index].local_mm;
            let end = contour_loop.points[segment_end_index].local_mm;
            [(start[0] + end[0]) * 0.5, (start[1] + end[1]) * 0.5]
        });

    let insert_index = segment_start_index + 1;
    contour_loop.points.insert(
        insert_index,
        ContourPoint {
            local_mm: inserted_local,
        },
    );

    roi_runtime::replace_contour_data(world, selection.roi_entity, contour_data)
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

    roi_runtime::replace_contour_data(world, selection.roi_entity, contour_data)
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
        ContourData, InputState, RoiCacheKind, RoiJobKind, ViewportState, WindowSettings,
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
        world.spawn((Roi::new_contour(
            crate::components::RoiId(101),
            "ContourWithLoop".to_string(),
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
        ),))
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
        assert_eq!(roi.job_state.queued, Some(RoiJobKind::RebuildVoxelCache));
        assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert!(world
            .get::<&EditorState>(entities.editor)
            .unwrap()
            .contour_draft
            .is_none());
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
        assert_eq!(roi.job_state.queued, Some(RoiJobKind::RebuildVoxelCache));
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
        assert_eq!(roi.job_state.queued, Some(RoiJobKind::RebuildVoxelCache));
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
        assert_eq!(roi.job_state.queued, Some(RoiJobKind::RebuildVoxelCache));
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
        assert_eq!(roi.job_state.queued, Some(RoiJobKind::RebuildVoxelCache));
    }
}
