use crate::app::components::{
    ContourData, ContourMovePreview, ContourSliceKey, EditorState, MeshEditPreview, Roi,
    RoiDirtyRegion, RoiEditPreview, RoiJobKind, RoiJobPriority, RoiJobRequest,
};
use crate::app::roi::authority::{ContourMutationError, MeshMutationError};
use crate::app::roi::history::{
    replace_contour_data_for_slice_with_history, replace_mesh_data_with_history,
};
use crate::convert::PlaneDefinition;
use hecs::World;

impl RoiEditPreview {
    pub fn roi_entity(&self) -> hecs::Entity {
        match self {
            Self::ContourMove(preview) => preview.roi_entity,
            Self::MeshDeform(preview) => preview.roi_entity,
        }
    }
}

impl EditorState {
    pub fn contour_move_preview(&self) -> Option<&ContourMovePreview> {
        match self.roi_edit_preview.as_ref()? {
            RoiEditPreview::ContourMove(preview) => Some(preview),
            RoiEditPreview::MeshDeform(_) => None,
        }
    }

    pub fn mesh_edit_preview(&self) -> Option<&MeshEditPreview> {
        match self.roi_edit_preview.as_ref()? {
            RoiEditPreview::MeshDeform(preview) => Some(preview),
            RoiEditPreview::ContourMove(_) => None,
        }
    }

    pub(crate) fn set_contour_move_preview(&mut self, preview: ContourMovePreview) {
        self.roi_edit_preview = Some(RoiEditPreview::ContourMove(preview));
    }

    pub(crate) fn set_mesh_edit_preview(&mut self, preview: MeshEditPreview) {
        self.roi_edit_preview = Some(RoiEditPreview::MeshDeform(preview));
    }

    pub fn take_contour_move_preview(&mut self) -> Option<ContourMovePreview> {
        if !matches!(self.roi_edit_preview, Some(RoiEditPreview::ContourMove(_))) {
            return None;
        }
        match self.roi_edit_preview.take()? {
            RoiEditPreview::ContourMove(preview) => Some(preview),
            RoiEditPreview::MeshDeform(_) => unreachable!("preview kind checked before take"),
        }
    }

    pub fn take_mesh_edit_preview(&mut self) -> Option<MeshEditPreview> {
        if !matches!(self.roi_edit_preview, Some(RoiEditPreview::MeshDeform(_))) {
            return None;
        }
        match self.roi_edit_preview.take()? {
            RoiEditPreview::MeshDeform(preview) => Some(preview),
            RoiEditPreview::ContourMove(_) => unreachable!("preview kind checked before take"),
        }
    }

    pub fn take_roi_edit_preview(&mut self) -> Option<RoiEditPreview> {
        self.roi_edit_preview.take()
    }
}

impl Roi {
    pub fn begin_preview(&mut self) -> u64 {
        self.preview_state.active = true;
        self.preview_state.revision = self.preview_state.revision.saturating_add(1);
        self.preview_state.revision
    }

    pub fn end_preview(&mut self) {
        self.preview_state.active = false;
        self.session_caches.preview_voxel = None;
        self.session_caches.preview_mesh = None;
    }
}

pub fn begin_contour_move_preview(
    world: &mut World,
    editor_entity: hecs::Entity,
    roi_entity: hecs::Entity,
    contour_data: ContourData,
    dirty_plane: PlaneDefinition,
) -> Result<u64, ContourMutationError> {
    let roi = world
        .get::<&Roi>(roi_entity)
        .map_err(|_| ContourMutationError::MissingRoi)?;
    if roi.contour_data().is_none() {
        return Err(ContourMutationError::NotContourRoi);
    }
    drop(roi);

    let mut editor = world
        .get::<&mut EditorState>(editor_entity)
        .map_err(|_| ContourMutationError::MissingEditorState)?;
    let previous_roi = editor.roi_edit_preview.as_ref().and_then(|preview| {
        (!matches!(
            preview,
            RoiEditPreview::ContourMove(current) if current.roi_entity == roi_entity
        ))
        .then(|| preview.roi_entity())
    });
    editor.set_contour_move_preview(ContourMovePreview {
        roi_entity,
        contour_data,
    });
    drop(editor);
    if let Some(previous_roi) = previous_roi {
        end_roi_preview(world, previous_roi);
    }

    let mut roi = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| ContourMutationError::MissingRoi)?;
    let revision = roi.begin_preview();
    let source_generation = roi.dirty_state.generations.authoritative;
    roi.enqueue_job(RoiJobRequest {
        kind: RoiJobKind::RebuildVoxelCache,
        source_generation,
        preview_revision: Some(revision),
        priority: RoiJobPriority::InteractivePreview,
        dirty_region: RoiDirtyRegion::ContourSlice(ContourSliceKey::from_plane(dirty_plane)),
    });
    Ok(revision)
}

pub fn commit_contour_move_preview(
    world: &mut World,
    editor_entity: hecs::Entity,
    dirty_plane: PlaneDefinition,
) -> Result<(), ContourMutationError> {
    let preview = world
        .get::<&mut EditorState>(editor_entity)
        .map_err(|_| ContourMutationError::MissingEditorState)?
        .take_contour_move_preview()
        .ok_or(ContourMutationError::MissingPreview)?;
    let result = replace_contour_data_for_slice_with_history(
        world,
        editor_entity,
        preview.roi_entity,
        preview.contour_data,
        dirty_plane,
    );
    end_roi_preview(world, preview.roi_entity);
    result
}

pub fn begin_mesh_edit_preview(
    world: &mut World,
    editor_entity: hecs::Entity,
    roi_entity: hecs::Entity,
    mesh_data: crate::app::roi::MeshData,
) -> Result<u64, MeshMutationError> {
    let roi = world
        .get::<&Roi>(roi_entity)
        .map_err(|_| MeshMutationError::MissingRoi)?;
    if roi.mesh_data().is_none() {
        return Err(MeshMutationError::NotMeshRoi);
    }
    drop(roi);

    let mut editor = world
        .get::<&mut EditorState>(editor_entity)
        .map_err(|_| MeshMutationError::MissingEditorState)?;
    let previous_roi = editor.roi_edit_preview.as_ref().and_then(|preview| {
        (!matches!(
            preview,
            RoiEditPreview::MeshDeform(current) if current.roi_entity == roi_entity
        ))
        .then(|| preview.roi_entity())
    });
    editor.set_mesh_edit_preview(MeshEditPreview {
        roi_entity,
        mesh_data,
    });
    drop(editor);
    if let Some(previous_roi) = previous_roi {
        end_roi_preview(world, previous_roi);
    }

    let mut roi = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| MeshMutationError::MissingRoi)?;
    Ok(roi.begin_preview())
}

pub fn commit_mesh_edit_preview(
    world: &mut World,
    editor_entity: hecs::Entity,
) -> Result<(), MeshMutationError> {
    let preview = world
        .get::<&mut EditorState>(editor_entity)
        .map_err(|_| MeshMutationError::MissingEditorState)?
        .take_mesh_edit_preview()
        .ok_or(MeshMutationError::MissingPreview)?;
    let result =
        replace_mesh_data_with_history(world, editor_entity, preview.roi_entity, preview.mesh_data);
    end_roi_preview(world, preview.roi_entity);
    result
}

pub fn cancel_mesh_edit_preview(
    world: &mut World,
    editor_entity: hecs::Entity,
) -> Result<(), MeshMutationError> {
    let preview = world
        .get::<&mut EditorState>(editor_entity)
        .map_err(|_| MeshMutationError::MissingEditorState)?
        .take_mesh_edit_preview()
        .ok_or(MeshMutationError::MissingPreview)?;
    end_roi_preview(world, preview.roi_entity);
    Ok(())
}

pub fn cancel_roi_edit_preview(world: &mut World, editor_entity: hecs::Entity) -> bool {
    let preview = world
        .get::<&mut EditorState>(editor_entity)
        .ok()
        .and_then(|mut editor| editor.take_roi_edit_preview());
    let Some(preview) = preview else {
        return false;
    };
    end_roi_preview(world, preview.roi_entity());
    true
}

pub fn mesh_edit_preview_for_roi(
    world: &World,
    roi_entity: hecs::Entity,
) -> Option<crate::app::roi::MeshData> {
    world
        .query::<&EditorState>()
        .iter()
        .find_map(|(_, editor)| {
            editor
                .mesh_edit_preview()
                .filter(|preview| preview.roi_entity == roi_entity)
                .map(|preview| preview.mesh_data.clone())
        })
}

pub fn end_roi_preview(world: &mut World, roi_entity: hecs::Entity) {
    if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
        roi.end_preview();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::components::RoiId;
    use crate::convert::PlaneFamily;

    fn contour_data() -> ContourData {
        ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: Vec::new(),
        }
    }

    #[test]
    fn test_setting_mesh_preview_replaces_contour_preview_session() {
        let mut editor = EditorState::default();
        editor.set_contour_move_preview(ContourMovePreview {
            roi_entity: hecs::Entity::DANGLING,
            contour_data: contour_data(),
        });

        editor.set_mesh_edit_preview(MeshEditPreview {
            roi_entity: hecs::Entity::DANGLING,
            mesh_data: crate::app::roi::MeshData {
                vertices: Vec::new(),
                faces: Vec::new(),
            },
        });

        assert!(editor.contour_move_preview().is_none());
        assert!(editor.mesh_edit_preview().is_some());
    }

    #[test]
    fn test_cancel_roi_edit_preview_clears_editor_and_roi_session() {
        let mut world = World::new();
        let roi_entity = world.spawn((Roi::new_contour(
            RoiId(1),
            "test".to_string(),
            contour_data(),
        ),));
        world.get::<&mut Roi>(roi_entity).unwrap().begin_preview();
        let mut editor = EditorState::default();
        editor.set_contour_move_preview(ContourMovePreview {
            roi_entity,
            contour_data: contour_data(),
        });
        let editor_entity = world.spawn((editor,));

        assert!(cancel_roi_edit_preview(&mut world, editor_entity));

        assert!(world
            .get::<&EditorState>(editor_entity)
            .unwrap()
            .roi_edit_preview
            .is_none());
        assert!(!world.get::<&Roi>(roi_entity).unwrap().preview_state.active);
    }
}
