use crate::app::components::{
    ContourBody, ContourData, EditorState, MeshBody, MeshData, Roi, RoiBody, RoiEditSnapshot,
    RoiHistory, RoiJobState, VoxelBody,
};
use crate::app::roi::authority::{
    replace_contour_data, replace_mesh_data, ContourMutationError, MeshMutationError,
};
use crate::app::roi::model::is_roi_locked;
use hecs::World;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoiEditHistoryError {
    MissingRoi,
    NoActiveRoi,
    NoUndo,
    NoRedo,
    Locked,
}

/// Replaces the ROI's contour data and records the previous data as an undo step.
pub fn replace_contour_data_with_history(
    world: &mut World,
    roi_entity: hecs::Entity,
    contour_data: ContourData,
) -> Result<(), ContourMutationError> {
    let before = world
        .get::<&Roi>(roi_entity)
        .map_err(|_| ContourMutationError::MissingRoi)?
        .contour_data()
        .cloned()
        .ok_or(ContourMutationError::NotContourRoi)?;
    if before == contour_data {
        return Ok(());
    }
    replace_contour_data(world, roi_entity, contour_data)?;
    record_history_entry(world, roi_entity, RoiEditSnapshot::Contour(before))
        .map_err(|_| ContourMutationError::MissingRoi)
}

/// Replaces the ROI's mesh data and records the previous data as an undo step.
pub fn replace_mesh_data_with_history(
    world: &mut World,
    roi_entity: hecs::Entity,
    mesh_data: MeshData,
) -> Result<(), MeshMutationError> {
    let before = world
        .get::<&Roi>(roi_entity)
        .map_err(|_| MeshMutationError::MissingRoi)?
        .mesh_data()
        .cloned()
        .ok_or(MeshMutationError::NotMeshRoi)?;
    if before == mesh_data {
        return Ok(());
    }
    replace_mesh_data(world, roi_entity, mesh_data)?;
    record_history_entry(world, roi_entity, RoiEditSnapshot::Mesh(before))
        .map_err(|_| MeshMutationError::MissingRoi)
}

/// Whether the active ROI has an edit to undo.
pub fn can_undo_roi_edit(world: &World, editor: &EditorState) -> bool {
    active_roi_history(world, editor).is_some_and(|history| !history.undo.is_empty())
}

/// Whether the active ROI has an undone edit to redo.
pub fn can_redo_roi_edit(world: &World, editor: &EditorState) -> bool {
    active_roi_history(world, editor).is_some_and(|history| !history.redo.is_empty())
}

fn active_roi_history(world: &World, editor: &EditorState) -> Option<RoiHistory> {
    let roi_entity = editor.active_roi?;
    world
        .get::<&Roi>(roi_entity)
        .ok()
        .map(|roi| roi.history.clone())
}

/// Undoes the active ROI's latest edit. Returns the ROI, which stays active.
pub fn undo_roi_edit(
    world: &mut World,
    editor: &mut EditorState,
) -> Result<hecs::Entity, RoiEditHistoryError> {
    apply_roi_edit_history(world, editor, true)
}

/// Redoes the active ROI's latest undone edit. Returns the ROI, which stays active.
pub fn redo_roi_edit(
    world: &mut World,
    editor: &mut EditorState,
) -> Result<hecs::Entity, RoiEditHistoryError> {
    apply_roi_edit_history(world, editor, false)
}

fn apply_roi_edit_history(
    world: &mut World,
    editor: &mut EditorState,
    undo: bool,
) -> Result<hecs::Entity, RoiEditHistoryError> {
    let roi_entity = editor.active_roi.ok_or(RoiEditHistoryError::NoActiveRoi)?;
    let snapshot = {
        let roi = world
            .get::<&Roi>(roi_entity)
            .map_err(|_| RoiEditHistoryError::MissingRoi)?;
        let stack = if undo {
            &roi.history.undo
        } else {
            &roi.history.redo
        };
        stack.last().cloned().ok_or(if undo {
            RoiEditHistoryError::NoUndo
        } else {
            RoiEditHistoryError::NoRedo
        })?
    };
    let current = capture_roi_edit_snapshot(world, roi_entity)?;
    restore_roi_edit_snapshot(world, roi_entity, snapshot)?;

    if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
        roi.history.step(undo, current);
    }
    editor.contour_draft = None;
    editor.contour_selection = None;
    editor.mesh_selection = None;
    Ok(roi_entity)
}

fn capture_roi_edit_snapshot(
    world: &World,
    roi_entity: hecs::Entity,
) -> Result<RoiEditSnapshot, RoiEditHistoryError> {
    let roi = world
        .get::<&Roi>(roi_entity)
        .map_err(|_| RoiEditHistoryError::MissingRoi)?;
    match &roi.body {
        RoiBody::Contour(ContourBody { data: contour, .. }) => {
            Ok(RoiEditSnapshot::Contour(contour.clone()))
        }
        RoiBody::Mesh(MeshBody { data: mesh, .. }) => Ok(RoiEditSnapshot::Mesh(mesh.clone())),
        RoiBody::Voxel(VoxelBody { data: voxel }) => {
            Ok(RoiEditSnapshot::Voxel(Box::new(voxel.clone())))
        }
    }
}

fn restore_roi_edit_snapshot(
    world: &mut World,
    roi_entity: hecs::Entity,
    snapshot: RoiEditSnapshot,
) -> Result<(), RoiEditHistoryError> {
    if is_roi_locked(world, roi_entity) {
        return Err(RoiEditHistoryError::Locked);
    }
    let mut roi = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| RoiEditHistoryError::MissingRoi)?;
    roi.job_state = RoiJobState::default();
    roi.end_preview();
    // A snapshot of a different authority is an authority change being reversed: the body is
    // replaced, and every derived cache is rebuilt from the restored one.
    match snapshot {
        RoiEditSnapshot::Contour(contour) => {
            roi.body = RoiBody::Contour(ContourBody::new(contour));
            roi.mark_contour_authoritative_changed();
            roi.mark_all_contour_view_caches_stale();
        }
        RoiEditSnapshot::Mesh(mesh) => {
            roi.body = RoiBody::Mesh(MeshBody::new(mesh));
            roi.mark_mesh_authoritative_changed();
            roi.mark_all_contour_view_caches_stale();
        }
        RoiEditSnapshot::Voxel(voxel) => {
            roi.body = RoiBody::Voxel(VoxelBody { data: *voxel });
            roi.mark_voxel_authoritative_changed();
            roi.mark_all_contour_view_caches_stale();
        }
    }
    Ok(())
}

/// Records an authority change (the ROI's previous body) as one undo step.
pub(crate) fn record_authority_change(
    world: &mut World,
    roi_entity: hecs::Entity,
    previous: RoiEditSnapshot,
) {
    let _ = record_history_entry(world, roi_entity, previous);
}

fn record_history_entry(
    world: &mut World,
    roi_entity: hecs::Entity,
    snapshot: RoiEditSnapshot,
) -> Result<(), RoiEditHistoryError> {
    let mut roi = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| RoiEditHistoryError::MissingRoi)?;
    roi.history.record(snapshot);
    Ok(())
}
