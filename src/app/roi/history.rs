use crate::app::components::{
    ContourBody, ContourData, ContourSliceKey, EditorState, MeshBody, MeshData, Roi, RoiBody,
    RoiDirtyRegion, RoiEditHistoryEntry, RoiEditSnapshot, RoiHistory, RoiJobKind, RoiJobPriority,
    RoiJobRequest, RoiJobState, VoxelBody,
};
use crate::app::roi::authority::{
    replace_contour_data, replace_contour_data_for_slice, replace_mesh_data, ContourMutationError,
    MeshMutationError,
};
use crate::convert::PlaneDefinition;
use hecs::World;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoiEditHistoryError {
    MissingEditorState,
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
    replace_contour_data_with_history_impl(world, roi_entity, contour_data, None)
}

/// Like [`replace_contour_data_with_history`], with a rebuild scoped to the changed slice.
pub fn replace_contour_data_for_slice_with_history(
    world: &mut World,
    roi_entity: hecs::Entity,
    contour_data: ContourData,
    dirty_plane: PlaneDefinition,
) -> Result<(), ContourMutationError> {
    replace_contour_data_with_history_impl(world, roi_entity, contour_data, Some(dirty_plane))
}

fn replace_contour_data_with_history_impl(
    world: &mut World,
    roi_entity: hecs::Entity,
    contour_data: ContourData,
    dirty_plane: Option<PlaneDefinition>,
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
    let dirty_region = dirty_plane
        .map(ContourSliceKey::from_plane)
        .filter(|key| {
            before
                .slices
                .iter()
                .any(|slice| ContourSliceKey::from_plane(slice.plane) == *key)
                && contour_data
                    .slices
                    .iter()
                    .any(|slice| ContourSliceKey::from_plane(slice.plane) == *key)
        })
        .map(RoiDirtyRegion::ContourSlice)
        .unwrap_or(RoiDirtyRegion::Full);
    match dirty_plane {
        Some(plane) => replace_contour_data_for_slice(world, roi_entity, contour_data, plane)?,
        None => replace_contour_data(world, roi_entity, contour_data)?,
    }
    record_history_entry(
        world,
        roi_entity,
        RoiEditHistoryEntry {
            snapshot: RoiEditSnapshot::Contour(before),
            dirty_region,
        },
    )
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
    record_history_entry(
        world,
        roi_entity,
        RoiEditHistoryEntry {
            snapshot: RoiEditSnapshot::Mesh(before),
            dirty_region: RoiDirtyRegion::Full,
        },
    )
    .map_err(|_| MeshMutationError::MissingRoi)
}

/// Whether the active ROI has an edit to undo.
pub fn can_undo_roi_edit(world: &World, editor_entity: hecs::Entity) -> bool {
    active_roi_history(world, editor_entity).is_some_and(|history| !history.undo.is_empty())
}

/// Whether the active ROI has an undone edit to redo.
pub fn can_redo_roi_edit(world: &World, editor_entity: hecs::Entity) -> bool {
    active_roi_history(world, editor_entity).is_some_and(|history| !history.redo.is_empty())
}

fn active_roi_history(world: &World, editor_entity: hecs::Entity) -> Option<RoiHistory> {
    let roi_entity = world
        .get::<&EditorState>(editor_entity)
        .ok()
        .and_then(|editor| editor.active_roi)?;
    world
        .get::<&Roi>(roi_entity)
        .ok()
        .map(|roi| roi.history.clone())
}

/// Undoes the active ROI's latest edit. Returns the ROI, which stays active.
pub fn undo_roi_edit(
    world: &mut World,
    editor_entity: hecs::Entity,
) -> Result<hecs::Entity, RoiEditHistoryError> {
    apply_roi_edit_history(world, editor_entity, true)
}

/// Redoes the active ROI's latest undone edit. Returns the ROI, which stays active.
pub fn redo_roi_edit(
    world: &mut World,
    editor_entity: hecs::Entity,
) -> Result<hecs::Entity, RoiEditHistoryError> {
    apply_roi_edit_history(world, editor_entity, false)
}

fn apply_roi_edit_history(
    world: &mut World,
    editor_entity: hecs::Entity,
    undo: bool,
) -> Result<hecs::Entity, RoiEditHistoryError> {
    let roi_entity = world
        .get::<&EditorState>(editor_entity)
        .map_err(|_| RoiEditHistoryError::MissingEditorState)?
        .active_roi
        .ok_or(RoiEditHistoryError::NoActiveRoi)?;
    let entry = {
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
    restore_roi_edit_snapshot(
        world,
        roi_entity,
        entry.snapshot.clone(),
        entry.dirty_region,
    )?;

    if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
        roi.history.step(undo, current);
    }
    if let Ok(mut editor) = world.get::<&mut EditorState>(editor_entity) {
        editor.contour_draft = None;
        editor.contour_selection = None;
        editor.mesh_selection = None;
    }
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
    dirty_region: RoiDirtyRegion,
) -> Result<(), RoiEditHistoryError> {
    let mut roi = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| RoiEditHistoryError::MissingRoi)?;
    if roi.metadata.is_locked {
        return Err(RoiEditHistoryError::Locked);
    }
    roi.job_state = RoiJobState::default();
    roi.end_preview();
    // A snapshot of a different authority is an authority change being reversed: the body is
    // replaced, and every derived cache is rebuilt from the restored one.
    match snapshot {
        RoiEditSnapshot::Contour(contour) => {
            let same_authority = matches!(roi.body, RoiBody::Contour(_));
            roi.body = RoiBody::Contour(ContourBody::new(contour));
            roi.mark_contour_authoritative_changed();
            roi.mark_all_contour_view_caches_stale();
            let source_generation = roi.dirty_state.authoritative.shape;
            roi.enqueue_job(RoiJobRequest {
                kind: RoiJobKind::RebuildVoxelCache,
                source_generation,
                preview_revision: None,
                priority: RoiJobPriority::VisibleCommitted,
                dirty_region: match dirty_region {
                    RoiDirtyRegion::ContourSlice(key) if same_authority => {
                        RoiDirtyRegion::ContourSlice(key)
                    }
                    _ => RoiDirtyRegion::Full,
                },
            });
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
    let _ = record_history_entry(
        world,
        roi_entity,
        RoiEditHistoryEntry {
            snapshot: previous,
            dirty_region: RoiDirtyRegion::Full,
        },
    );
}

fn record_history_entry(
    world: &mut World,
    roi_entity: hecs::Entity,
    entry: RoiEditHistoryEntry,
) -> Result<(), RoiEditHistoryError> {
    let mut roi = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| RoiEditHistoryError::MissingRoi)?;
    roi.history.record(entry);
    Ok(())
}
