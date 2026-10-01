use crate::app::components::{
    ContourData, ContourMovePreview, ContourSliceKey, EditorState, MeshEditPreview, Roi, RoiBody,
    RoiDirtyRegion, RoiJobKind, RoiJobPriority, RoiJobRequest,
};
use crate::app::roi::authority::{ContourMutationError, MeshMutationError};
use crate::app::roi::history::{replace_contour_data_with_history, replace_mesh_data_with_history};
use crate::convert::PlaneDefinition;
use hecs::World;

impl Roi {
    pub fn contour_move_preview(&self) -> Option<&ContourMovePreview> {
        match &self.body {
            RoiBody::Contour(body) => body.preview.as_ref(),
            _ => None,
        }
    }

    pub fn mesh_edit_preview(&self) -> Option<&MeshEditPreview> {
        match &self.body {
            RoiBody::Mesh(body) => body.preview.as_ref(),
            _ => None,
        }
    }

    /// Whether an edit is in flight on this ROI.
    pub fn has_edit_preview(&self) -> bool {
        self.contour_move_preview().is_some() || self.mesh_edit_preview().is_some()
    }

    pub fn take_contour_move_preview(&mut self) -> Option<ContourMovePreview> {
        match &mut self.body {
            RoiBody::Contour(body) => body.preview.take(),
            _ => None,
        }
    }

    pub fn take_mesh_edit_preview(&mut self) -> Option<MeshEditPreview> {
        match &mut self.body {
            RoiBody::Mesh(body) => body.preview.take(),
            _ => None,
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
        match &mut self.body {
            RoiBody::Contour(body) => body.preview = None,
            RoiBody::Mesh(body) => body.preview = None,
            RoiBody::Voxel(_) => {}
        }
        self.session_caches.preview_voxel = None;
        self.session_caches.preview_mesh = None;
    }
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
    match &mut roi.body {
        RoiBody::Contour(body) => body.preview = Some(ContourMovePreview { contour_data }),
        _ => return Err(ContourMutationError::NotContourRoi),
    }
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
    editor: &EditorState,
) -> Result<(), ContourMutationError> {
    let roi_entity = editor
        .active_roi
        .ok_or(ContourMutationError::MissingPreview)?;
    let preview = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| ContourMutationError::MissingRoi)?
        .take_contour_move_preview()
        .ok_or(ContourMutationError::MissingPreview)?;
    let result = replace_contour_data_with_history(world, roi_entity, preview.contour_data);
    end_roi_preview(world, roi_entity);
    result
}

pub fn begin_mesh_edit_preview(
    world: &mut World,
    roi_entity: hecs::Entity,
    mesh_data: crate::app::roi::MeshData,
) -> Result<u64, MeshMutationError> {
    set_mesh_edit_preview(world, roi_entity, mesh_data, None)
}

/// Sets the mesh preview together with the deform topology shared by a whole drag.
pub fn set_mesh_edit_preview(
    world: &mut World,
    roi_entity: hecs::Entity,
    mesh_data: crate::app::roi::MeshData,
    deform_base: Option<std::sync::Arc<crate::convert::MeshDeformBase>>,
) -> Result<u64, MeshMutationError> {
    let mut roi = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| MeshMutationError::MissingRoi)?;
    match &mut roi.body {
        RoiBody::Mesh(body) => {
            body.preview = Some(MeshEditPreview {
                mesh_data,
                deform_base,
            })
        }
        _ => return Err(MeshMutationError::NotMeshRoi),
    }
    Ok(roi.begin_preview())
}

/// Commits the active ROI's mesh deformation as one undoable edit, after validating that the
/// deformed mesh is still a closed manifold.
pub fn commit_mesh_edit_preview(
    world: &mut World,
    editor: &EditorState,
) -> Result<(), MeshMutationError> {
    let roi_entity = editor.active_roi.ok_or(MeshMutationError::MissingPreview)?;
    // Validate before taking the preview: an invalid deformation is reported and the preview
    // stays, so the user's drag is not thrown away and can still be cancelled or adjusted.
    {
        let roi = world
            .get::<&Roi>(roi_entity)
            .map_err(|_| MeshMutationError::MissingRoi)?;
        let preview = roi
            .mesh_edit_preview()
            .ok_or(MeshMutationError::MissingPreview)?;
        crate::convert::validate_mesh_for_voxelization(&preview.mesh_data)
            .map_err(MeshMutationError::InvalidMesh)?;
    }
    let preview = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| MeshMutationError::MissingRoi)?
        .take_mesh_edit_preview()
        .ok_or(MeshMutationError::MissingPreview)?;
    let result =
        replace_mesh_data_with_history(world, roi_entity, preview.mesh_data).and_then(|()| {
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
    editor: &EditorState,
) -> Result<(), MeshMutationError> {
    let roi_entity = editor.active_roi.ok_or(MeshMutationError::MissingPreview)?;
    world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| MeshMutationError::MissingRoi)?
        .take_mesh_edit_preview()
        .ok_or(MeshMutationError::MissingPreview)?;
    end_roi_preview(world, roi_entity);
    Ok(())
}

/// Discards whatever edit the active ROI is previewing. Returns whether there was one.
pub fn cancel_roi_edit_preview(world: &mut World, editor: &EditorState) -> bool {
    let Some(roi_entity) = editor.active_roi else {
        return false;
    };
    let had_preview = world
        .get::<&Roi>(roi_entity)
        .is_ok_and(|roi| roi.has_edit_preview());
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
    use crate::model::OrthogonalFamily;

    fn contour_data() -> ContourData {
        ContourData {
            active_plane_family: OrthogonalFamily::Axial,
            slices: Vec::new(),
        }
    }

    #[test]
    fn test_a_contour_roi_holds_only_a_contour_preview() {
        let (mut roi, _) = Roi::new_contour(RoiId(1), "test".to_string(), contour_data());
        let RoiBody::Contour(body) = &mut roi.body else {
            unreachable!()
        };
        body.preview = Some(ContourMovePreview {
            contour_data: contour_data(),
        });
        assert!(roi.contour_move_preview().is_some());
        assert!(roi.mesh_edit_preview().is_none());
    }

    #[test]
    fn test_previews_belong_to_their_own_roi() {
        let mut world = World::new();
        let first = world.spawn(Roi::new_contour(RoiId(1), "a".into(), contour_data()));
        let second = world.spawn(Roi::new_contour(RoiId(2), "b".into(), contour_data()));
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
        let previous = world.spawn(Roi::new_contour(RoiId(1), "a".into(), contour_data()));
        let next = world.spawn(Roi::new_contour(RoiId(2), "b".into(), contour_data()));
        let mut editor = EditorState {
            active_roi: Some(previous),
            ..EditorState::default()
        };
        let plane = crate::convert::PlaneDefinition {
            family: PlaneFamily::Axial,
            origin_mm: [0.0; 3],
            u_axis_mm: [1.0, 0.0, 0.0],
            v_axis_mm: [0.0, 1.0, 0.0],
            normal_mm: [0.0, 0.0, 1.0],
        };
        begin_contour_move_preview(&mut world, previous, contour_data(), plane).unwrap();
        begin_contour_move_preview(&mut world, next, contour_data(), plane).unwrap();

        crate::systems::clear_contour_selection_for_roi_change(&mut world, &mut editor, Some(next));

        let previous_roi = world.get::<&Roi>(previous).unwrap();
        assert!(!previous_roi.has_edit_preview());
        assert!(!previous_roi.preview_state.active);
        drop(previous_roi);
        assert!(
            world.get::<&Roi>(next).unwrap().has_edit_preview(),
            "the newly active ROI keeps its own preview"
        );
    }

    #[test]
    fn test_cancel_roi_edit_preview_clears_the_active_rois_preview_and_session() {
        let mut world = World::new();
        let roi_entity = world.spawn(Roi::new_contour(
            RoiId(1),
            "test".to_string(),
            contour_data(),
        ));
        world.get::<&mut Roi>(roi_entity).unwrap().begin_preview();
        let mut roi = world.get::<&mut Roi>(roi_entity).unwrap();
        let RoiBody::Contour(body) = &mut roi.body else {
            unreachable!()
        };
        body.preview = Some(ContourMovePreview {
            contour_data: contour_data(),
        });
        drop(roi);
        let editor = EditorState {
            active_roi: Some(roi_entity),
            ..EditorState::default()
        };

        assert!(cancel_roi_edit_preview(&mut world, &editor));

        assert!(!world.get::<&Roi>(roi_entity).unwrap().has_edit_preview());
        assert!(!world.get::<&Roi>(roi_entity).unwrap().preview_state.active);
    }

    #[test]
    fn test_invalid_mesh_preview_is_kept_and_leaves_authority_and_history_alone() {
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
        let roi_entity = world.spawn(Roi::new_mesh(
            RoiId(1),
            "test".to_string(),
            original.clone(),
        ));
        let editor = EditorState {
            active_roi: Some(roi_entity),
            ..EditorState::default()
        };
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
            commit_mesh_edit_preview(&mut world, &editor),
            Err(MeshMutationError::InvalidMesh(_))
        ));
        // The rejected drag is kept so the user can adjust or cancel it; nothing was committed.
        let roi = world.get::<&Roi>(roi_entity).unwrap();
        assert!(roi.mesh_edit_preview().is_some());
        assert!(roi.preview_state.active);
        assert_eq!(roi.mesh_data(), Some(&original));
        drop(roi);
        assert!(!crate::app::roi::can_undo_roi_edit(&world, &editor));

        cancel_mesh_edit_preview(&mut world, &editor).unwrap();
        let roi = world.get::<&Roi>(roi_entity).unwrap();
        assert!(roi.mesh_edit_preview().is_none());
        assert!(!roi.preview_state.active);
        drop(roi);
        crate::app::roi::request_mesh_voxel_cache_rebuild(&mut world, roi_entity).unwrap();
    }
}
