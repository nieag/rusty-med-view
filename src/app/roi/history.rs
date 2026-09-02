use crate::app::components::{
    ContourData, ContourSliceKey, EditorState, MeshData, Roi, RoiAuthoritativeData, RoiDirtyRegion,
    RoiEditHistoryEntry, RoiEditSnapshot, RoiJobKind, RoiJobPriority, RoiJobRequest, RoiJobState,
};
use crate::app::roi::authority::{
    replace_contour_data, replace_contour_data_for_slice, replace_mesh_data, ContourMutationError,
    MeshMutationError,
};
use crate::convert::PlaneDefinition;
use hecs::World;

const MAX_ROI_EDIT_HISTORY: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoiEditHistoryError {
    MissingEditorState,
    MissingRoi,
    NoUndo,
    NoRedo,
    Locked,
    RepresentationChanged,
}

pub fn replace_contour_data_with_history(
    world: &mut World,
    editor_entity: hecs::Entity,
    roi_entity: hecs::Entity,
    contour_data: ContourData,
) -> Result<(), ContourMutationError> {
    replace_contour_data_with_history_impl(world, editor_entity, roi_entity, contour_data, None)
}

pub fn replace_contour_data_for_slice_with_history(
    world: &mut World,
    editor_entity: hecs::Entity,
    roi_entity: hecs::Entity,
    contour_data: ContourData,
    dirty_plane: PlaneDefinition,
) -> Result<(), ContourMutationError> {
    replace_contour_data_with_history_impl(
        world,
        editor_entity,
        roi_entity,
        contour_data,
        Some(dirty_plane),
    )
}

fn replace_contour_data_with_history_impl(
    world: &mut World,
    editor_entity: hecs::Entity,
    roi_entity: hecs::Entity,
    contour_data: ContourData,
    dirty_plane: Option<PlaneDefinition>,
) -> Result<(), ContourMutationError> {
    if world.get::<&EditorState>(editor_entity).is_err() {
        return Err(ContourMutationError::MissingEditorState);
    }
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
    push_roi_undo_entry(
        world,
        editor_entity,
        RoiEditHistoryEntry {
            roi_entity,
            snapshot: RoiEditSnapshot::Contour(before),
            dirty_region,
        },
    )
    .map_err(|_| ContourMutationError::MissingEditorState)
}

pub fn replace_mesh_data_with_history(
    world: &mut World,
    editor_entity: hecs::Entity,
    roi_entity: hecs::Entity,
    mesh_data: MeshData,
) -> Result<(), MeshMutationError> {
    if world.get::<&EditorState>(editor_entity).is_err() {
        return Err(MeshMutationError::MissingEditorState);
    }
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
    push_roi_undo_entry(
        world,
        editor_entity,
        RoiEditHistoryEntry {
            roi_entity,
            snapshot: RoiEditSnapshot::Mesh(before),
            dirty_region: RoiDirtyRegion::Full,
        },
    )
    .map_err(|_| MeshMutationError::MissingEditorState)
}

pub fn can_undo_roi_edit(world: &World, editor_entity: hecs::Entity) -> bool {
    world
        .get::<&EditorState>(editor_entity)
        .is_ok_and(|editor| !editor.roi_undo_stack.is_empty())
}

pub fn can_redo_roi_edit(world: &World, editor_entity: hecs::Entity) -> bool {
    world
        .get::<&EditorState>(editor_entity)
        .is_ok_and(|editor| !editor.roi_redo_stack.is_empty())
}

pub fn clear_roi_edit_history_for_roi(
    world: &mut World,
    editor_entity: hecs::Entity,
    roi_entity: hecs::Entity,
) {
    if let Ok(mut editor) = world.get::<&mut EditorState>(editor_entity) {
        editor
            .roi_undo_stack
            .retain(|entry| entry.roi_entity != roi_entity);
        editor
            .roi_redo_stack
            .retain(|entry| entry.roi_entity != roi_entity);
    }
}

pub fn undo_roi_edit(
    world: &mut World,
    editor_entity: hecs::Entity,
) -> Result<hecs::Entity, RoiEditHistoryError> {
    apply_roi_edit_history(world, editor_entity, true)
}

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
    let entry = {
        let editor = world
            .get::<&EditorState>(editor_entity)
            .map_err(|_| RoiEditHistoryError::MissingEditorState)?;
        let stack = if undo {
            &editor.roi_undo_stack
        } else {
            &editor.roi_redo_stack
        };
        stack.last().cloned().ok_or(if undo {
            RoiEditHistoryError::NoUndo
        } else {
            RoiEditHistoryError::NoRedo
        })?
    };
    let current = capture_roi_edit_snapshot(world, entry.roi_entity)?;
    restore_roi_edit_snapshot(
        world,
        entry.roi_entity,
        entry.snapshot.clone(),
        entry.dirty_region,
    )?;

    let mut editor = world
        .get::<&mut EditorState>(editor_entity)
        .map_err(|_| RoiEditHistoryError::MissingEditorState)?;
    let source_stack = if undo {
        &mut editor.roi_undo_stack
    } else {
        &mut editor.roi_redo_stack
    };
    source_stack.pop();
    let destination_stack = if undo {
        &mut editor.roi_redo_stack
    } else {
        &mut editor.roi_undo_stack
    };
    destination_stack.push(RoiEditHistoryEntry {
        roi_entity: entry.roi_entity,
        snapshot: current,
        dirty_region: entry.dirty_region,
    });
    trim_history_stack(destination_stack);
    editor.active_roi = Some(entry.roi_entity);
    editor.contour_draft = None;
    editor.contour_selection = None;
    editor.take_roi_edit_preview();
    editor.mesh_selection = None;
    Ok(entry.roi_entity)
}

fn capture_roi_edit_snapshot(
    world: &World,
    roi_entity: hecs::Entity,
) -> Result<RoiEditSnapshot, RoiEditHistoryError> {
    let roi = world
        .get::<&Roi>(roi_entity)
        .map_err(|_| RoiEditHistoryError::MissingRoi)?;
    match &roi.authoritative_data {
        RoiAuthoritativeData::Contour(contour) => Ok(RoiEditSnapshot::Contour(contour.clone())),
        RoiAuthoritativeData::Mesh(mesh) => Ok(RoiEditSnapshot::Mesh(mesh.clone())),
        RoiAuthoritativeData::Voxel(_) => Err(RoiEditHistoryError::RepresentationChanged),
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
    match (&mut roi.authoritative_data, snapshot) {
        (RoiAuthoritativeData::Contour(existing), RoiEditSnapshot::Contour(contour)) => {
            *existing = contour;
            roi.mark_contour_authoritative_changed();
            roi.mark_all_contour_view_caches_stale();
            let source_generation = roi.dirty_state.generations.authoritative;
            roi.enqueue_job(RoiJobRequest {
                kind: RoiJobKind::RebuildVoxelCache,
                source_generation,
                preview_revision: None,
                priority: RoiJobPriority::VisibleCommitted,
                dirty_region: match dirty_region {
                    RoiDirtyRegion::ContourSlice(key) => RoiDirtyRegion::ContourSlice(key),
                    _ => RoiDirtyRegion::Full,
                },
            });
        }
        (RoiAuthoritativeData::Mesh(existing), RoiEditSnapshot::Mesh(mesh)) => {
            *existing = mesh;
            roi.mark_mesh_authoritative_changed();
            roi.mark_all_contour_view_caches_stale();
        }
        _ => return Err(RoiEditHistoryError::RepresentationChanged),
    }
    Ok(())
}

fn push_roi_undo_entry(
    world: &mut World,
    editor_entity: hecs::Entity,
    entry: RoiEditHistoryEntry,
) -> Result<(), RoiEditHistoryError> {
    let mut editor = world
        .get::<&mut EditorState>(editor_entity)
        .map_err(|_| RoiEditHistoryError::MissingEditorState)?;
    editor.roi_undo_stack.push(entry);
    trim_history_stack(&mut editor.roi_undo_stack);
    editor.roi_redo_stack.clear();
    Ok(())
}

fn trim_history_stack(stack: &mut Vec<RoiEditHistoryEntry>) {
    if stack.len() > MAX_ROI_EDIT_HISTORY {
        stack.remove(0);
    }
}
