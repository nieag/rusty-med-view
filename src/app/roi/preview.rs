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

impl Roi {
    pub fn contour_move_preview(&self) -> Option<&ContourMovePreview> {
        match self.edit_preview.as_ref()? {
            RoiEditPreview::ContourMove(preview) => Some(preview),
            RoiEditPreview::MeshDeform(_) => None,
        }
    }

    pub fn mesh_edit_preview(&self) -> Option<&MeshEditPreview> {
        match self.edit_preview.as_ref()? {
            RoiEditPreview::MeshDeform(preview) => Some(preview),
            RoiEditPreview::ContourMove(_) => None,
        }
    }

    pub fn take_contour_move_preview(&mut self) -> Option<ContourMovePreview> {
        if !matches!(self.edit_preview, Some(RoiEditPreview::ContourMove(_))) {
            return None;
        }
        match self.edit_preview.take()? {
            RoiEditPreview::ContourMove(preview) => Some(preview),
            RoiEditPreview::MeshDeform(_) => unreachable!("preview kind checked before take"),
        }
    }

    pub fn take_mesh_edit_preview(&mut self) -> Option<MeshEditPreview> {
        if !matches!(self.edit_preview, Some(RoiEditPreview::MeshDeform(_))) {
            return None;
        }
        match self.edit_preview.take()? {
            RoiEditPreview::MeshDeform(preview) => Some(preview),
            RoiEditPreview::ContourMove(_) => unreachable!("preview kind checked before take"),
        }
    }

    pub fn begin_preview(&mut self) -> u64 {
        self.preview_state.active = true;
        self.preview_state.revision = self.preview_state.revision.saturating_add(1);
        self.preview_state.revision
    }

    /// Ends the preview session: the preview caches and the in-flight edit are discarded.
    pub fn end_preview(&mut self) {
        self.preview_state.active = false;
        self.edit_preview = None;
        self.session_caches.preview_voxel = None;
        self.session_caches.preview_mesh = None;
    }
}

fn active_roi(world: &World, editor_entity: hecs::Entity) -> Option<hecs::Entity> {
    world
        .get::<&EditorState>(editor_entity)
        .ok()
        .and_then(|editor| editor.active_roi)
}

pub fn begin_contour_move_preview(
    world: &mut World,
    roi_entity: hecs::Entity,
    contour_data: ContourData,
    dirty_plane: PlaneDefinition,
) -> Result<u64, ContourMutationError> {
    let mut roi = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| ContourMutationError::MissingRoi)?;
    if roi.contour_data().is_none() {
        return Err(ContourMutationError::NotContourRoi);
    }
    roi.edit_preview = Some(RoiEditPreview::ContourMove(ContourMovePreview {
        contour_data,
    }));
    let revision = roi.begin_preview();
    let source_generation = roi.dirty_state.authoritative.shape;
    roi.enqueue_job(RoiJobRequest {
        kind: RoiJobKind::RebuildVoxelCache,
        source_generation,
        preview_revision: Some(revision),
        priority: RoiJobPriority::InteractivePreview,
        dirty_region: RoiDirtyRegion::ContourSlice(ContourSliceKey::from_plane(dirty_plane)),
    });
    Ok(revision)
}

/// Commits the active ROI's contour drag as one undoable edit.
pub fn commit_contour_move_preview(
    world: &mut World,
    editor_entity: hecs::Entity,
    dirty_plane: PlaneDefinition,
) -> Result<(), ContourMutationError> {
    let roi_entity =
        active_roi(world, editor_entity).ok_or(ContourMutationError::MissingPreview)?;
    let preview = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| ContourMutationError::MissingRoi)?
        .take_contour_move_preview()
        .ok_or(ContourMutationError::MissingPreview)?;
    let result = replace_contour_data_for_slice_with_history(
        world,
        roi_entity,
        preview.contour_data,
        dirty_plane,
    );
    end_roi_preview(world, roi_entity);
    result
}

pub fn begin_mesh_edit_preview(
    world: &mut World,
    roi_entity: hecs::Entity,
    mesh_data: crate::app::roi::MeshData,
) -> Result<u64, MeshMutationError> {
    let mut roi = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| MeshMutationError::MissingRoi)?;
    if roi.mesh_data().is_none() {
        return Err(MeshMutationError::NotMeshRoi);
    }
    roi.edit_preview = Some(RoiEditPreview::MeshDeform(MeshEditPreview { mesh_data }));
    Ok(roi.begin_preview())
}

/// Commits the active ROI's mesh deformation as one undoable edit, after validating that the
/// deformed mesh is still a closed manifold.
pub fn commit_mesh_edit_preview(
    world: &mut World,
    editor_entity: hecs::Entity,
) -> Result<(), MeshMutationError> {
    let roi_entity = active_roi(world, editor_entity).ok_or(MeshMutationError::MissingPreview)?;
    let preview = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| MeshMutationError::MissingRoi)?
        .take_mesh_edit_preview()
        .ok_or(MeshMutationError::MissingPreview)?;
    let result = crate::convert::validate_mesh_for_voxelization(&preview.mesh_data)
        .map_err(MeshMutationError::InvalidMesh)
        .and_then(|()| {
            replace_mesh_data_with_history(world, roi_entity, preview.mesh_data)?;
            let mut roi = world
                .get::<&mut Roi>(roi_entity)
                .map_err(|_| MeshMutationError::MissingRoi)?;
            roi.validated_mesh_generation = Some(roi.dirty_state.authoritative.shape);
            Ok(())
        });
    end_roi_preview(world, roi_entity);
    result
}

/// Discards the active ROI's mesh deformation without committing it.
pub fn cancel_mesh_edit_preview(
    world: &mut World,
    editor_entity: hecs::Entity,
) -> Result<(), MeshMutationError> {
    let roi_entity = active_roi(world, editor_entity).ok_or(MeshMutationError::MissingPreview)?;
    world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| MeshMutationError::MissingRoi)?
        .take_mesh_edit_preview()
        .ok_or(MeshMutationError::MissingPreview)?;
    end_roi_preview(world, roi_entity);
    Ok(())
}

/// Discards whatever edit the active ROI is previewing. Returns whether there was one.
pub fn cancel_roi_edit_preview(world: &mut World, editor_entity: hecs::Entity) -> bool {
    let Some(roi_entity) = active_roi(world, editor_entity) else {
        return false;
    };
    let had_preview = world
        .get::<&Roi>(roi_entity)
        .is_ok_and(|roi| roi.edit_preview.is_some());
    if had_preview {
        end_roi_preview(world, roi_entity);
    }
    had_preview
}

pub fn mesh_edit_preview_for_roi(
    world: &World,
    roi_entity: hecs::Entity,
) -> Option<crate::app::roi::MeshData> {
    world
        .get::<&Roi>(roi_entity)
        .ok()?
        .mesh_edit_preview()
        .map(|preview| preview.mesh_data.clone())
}

pub fn end_roi_preview(world: &mut World, roi_entity: hecs::Entity) {
    if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
        roi.end_preview();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::components::{MeshData, MeshFace, MeshVertex, RoiId};
    use crate::convert::PlaneFamily;

    fn contour_data() -> ContourData {
        ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: Vec::new(),
        }
    }

    #[test]
    fn test_a_roi_holds_one_edit_preview_and_a_new_kind_replaces_the_old() {
        let mut roi = Roi::new_contour(RoiId(1), "test".to_string(), contour_data());
        roi.edit_preview = Some(RoiEditPreview::ContourMove(ContourMovePreview {
            contour_data: contour_data(),
        }));
        assert!(roi.contour_move_preview().is_some());

        roi.edit_preview = Some(RoiEditPreview::MeshDeform(MeshEditPreview {
            mesh_data: crate::app::roi::MeshData {
                vertices: Vec::new(),
                faces: Vec::new(),
            },
        }));

        assert!(roi.contour_move_preview().is_none());
        assert!(roi.mesh_edit_preview().is_some());
    }

    #[test]
    fn test_previews_belong_to_their_own_roi() {
        let mut world = World::new();
        let first = world.spawn((Roi::new_contour(RoiId(1), "a".into(), contour_data()),));
        let second = world.spawn((Roi::new_contour(RoiId(2), "b".into(), contour_data()),));
        let plane = crate::convert::PlaneDefinition {
            family: PlaneFamily::Axial,
            origin_mm: [0.0; 3],
            u_axis_mm: [1.0, 0.0, 0.0],
            v_axis_mm: [0.0, 1.0, 0.0],
            normal_mm: [0.0, 0.0, 1.0],
        };

        begin_contour_move_preview(&mut world, first, contour_data(), plane).unwrap();

        assert!(world
            .get::<&Roi>(first)
            .unwrap()
            .contour_move_preview()
            .is_some());
        assert!(world
            .get::<&Roi>(second)
            .unwrap()
            .contour_move_preview()
            .is_none());
        assert!(world.get::<&Roi>(first).unwrap().preview_state.active);
        assert!(!world.get::<&Roi>(second).unwrap().preview_state.active);
    }

    #[test]
    fn test_changing_the_active_roi_ends_the_previous_rois_preview_only() {
        let mut world = World::new();
        let previous = world.spawn((Roi::new_contour(RoiId(1), "a".into(), contour_data()),));
        let next = world.spawn((Roi::new_contour(RoiId(2), "b".into(), contour_data()),));
        let editor_entity = world.spawn((EditorState {
            active_roi: Some(previous),
            ..EditorState::default()
        },));
        let plane = crate::convert::PlaneDefinition {
            family: PlaneFamily::Axial,
            origin_mm: [0.0; 3],
            u_axis_mm: [1.0, 0.0, 0.0],
            v_axis_mm: [0.0, 1.0, 0.0],
            normal_mm: [0.0, 0.0, 1.0],
        };
        begin_contour_move_preview(&mut world, previous, contour_data(), plane).unwrap();
        begin_contour_move_preview(&mut world, next, contour_data(), plane).unwrap();

        crate::systems::clear_contour_selection_for_roi_change(
            &mut world,
            editor_entity,
            Some(next),
        );

        let previous_roi = world.get::<&Roi>(previous).unwrap();
        assert!(previous_roi.edit_preview.is_none());
        assert!(!previous_roi.preview_state.active);
        drop(previous_roi);
        assert!(
            world.get::<&Roi>(next).unwrap().edit_preview.is_some(),
            "the newly active ROI keeps its own preview"
        );
    }

    #[test]
    fn test_cancel_roi_edit_preview_clears_the_active_rois_preview_and_session() {
        let mut world = World::new();
        let roi_entity = world.spawn((Roi::new_contour(
            RoiId(1),
            "test".to_string(),
            contour_data(),
        ),));
        world.get::<&mut Roi>(roi_entity).unwrap().begin_preview();
        world.get::<&mut Roi>(roi_entity).unwrap().edit_preview =
            Some(RoiEditPreview::ContourMove(ContourMovePreview {
                contour_data: contour_data(),
            }));
        let editor_entity = world.spawn((EditorState {
            active_roi: Some(roi_entity),
            ..EditorState::default()
        },));

        assert!(cancel_roi_edit_preview(&mut world, editor_entity));

        assert!(world
            .get::<&Roi>(roi_entity)
            .unwrap()
            .edit_preview
            .is_none());
        assert!(!world.get::<&Roi>(roi_entity).unwrap().preview_state.active);
    }

    #[test]
    fn test_invalid_mesh_preview_preserves_authority_and_history() {
        let mut world = World::new();
        let original = MeshData {
            vertices: [
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
            ]
            .map(|world_mm| MeshVertex { world_mm })
            .to_vec(),
            faces: [[0, 2, 1], [0, 1, 3], [1, 2, 3], [2, 0, 3]]
                .map(|vertex_indices| MeshFace { vertex_indices })
                .to_vec(),
        };
        let roi_entity = world.spawn((Roi::new_mesh(
            RoiId(1),
            "test".to_string(),
            original.clone(),
        ),));
        let editor_entity = world.spawn((EditorState {
            active_roi: Some(roi_entity),
            ..EditorState::default()
        },));
        let open_mesh = MeshData {
            vertices: vec![
                MeshVertex { world_mm: [0.0; 3] },
                MeshVertex {
                    world_mm: [1.0, 0.0, 0.0],
                },
                MeshVertex {
                    world_mm: [0.0, 1.0, 0.0],
                },
            ],
            faces: vec![MeshFace {
                vertex_indices: [0, 1, 2],
            }],
        };
        begin_mesh_edit_preview(&mut world, roi_entity, open_mesh.clone()).unwrap();

        assert!(matches!(
            commit_mesh_edit_preview(&mut world, editor_entity),
            Err(MeshMutationError::InvalidMesh(_))
        ));
        assert!(world
            .get::<&Roi>(roi_entity)
            .unwrap()
            .mesh_edit_preview()
            .is_none());
        let roi = world.get::<&Roi>(roi_entity).unwrap();
        assert!(!roi.preview_state.active);
        assert_eq!(roi.mesh_data(), Some(&original));
        drop(roi);
        assert!(!crate::app::roi::can_undo_roi_edit(&world, editor_entity));
        crate::app::roi::request_mesh_voxel_cache_rebuild(&mut world, roi_entity).unwrap();
    }
}
