use crate::app::components::{
    ContourBody, ContourData, ContourSliceKey, MeshBody, MeshData, Roi, RoiBody, RoiCacheKind,
    RoiDirtyRegion, RoiJobKind, RoiJobPriority, RoiJobRequest, RoiJobState,
};
use crate::convert::{MeshVoxelizationError, PlaneDefinition};
use hecs::World;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContourMutationError {
    MissingRoi,
    MissingEditorState,
    MissingPreview,
    NotContourRoi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshMutationError {
    MissingRoi,
    MissingEditorState,
    MissingPreview,
    NotMeshRoi,
    InvalidDelta,
    InvalidMesh(MeshVoxelizationError),
}

pub fn replace_contour_data(
    world: &mut World,
    roi_entity: hecs::Entity,
    contour_data: ContourData,
) -> Result<(), ContourMutationError> {
    let mut roi = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| ContourMutationError::MissingRoi)?;

    match &mut roi.body {
        RoiBody::Contour(ContourBody { data: existing, .. }) => *existing = contour_data,
        RoiBody::Voxel(_) | RoiBody::Mesh(_) => {
            return Err(ContourMutationError::NotContourRoi);
        }
    }

    roi.mark_contour_authoritative_changed();
    roi.mark_all_contour_view_caches_stale();
    roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
    Ok(())
}

fn has_slice(data: &ContourData, key: ContourSliceKey) -> bool {
    data.slices
        .iter()
        .any(|slice| ContourSliceKey::from_plane(slice.plane) == key)
}

/// What the voxel cache has to rebuild after a commit that left `after` with edits at `plane`.
///
/// One slice can be re-rasterized on its own when it exists after the edit: the rebuild clears and
/// redraws the slab it covers (a new slice only adds to an empty slab). A slice that was removed
/// leaves nothing to locate its old slab by, so the whole cache is rebuilt.
pub fn dirty_region_for_slice_edit(after: &ContourData, plane: PlaneDefinition) -> RoiDirtyRegion {
    let key = ContourSliceKey::from_plane(plane);
    if has_slice(after, key) {
        RoiDirtyRegion::ContourSlice(key)
    } else {
        RoiDirtyRegion::Full
    }
}

/// The same for an undo or redo step, which swaps two states and may run either way: a
/// slice-local rebuild is only valid when the slice exists in both.
pub fn dirty_region_for_slice_swap(
    a: &ContourData,
    b: &ContourData,
    plane: PlaneDefinition,
) -> RoiDirtyRegion {
    let key = ContourSliceKey::from_plane(plane);
    if has_slice(a, key) && has_slice(b, key) {
        RoiDirtyRegion::ContourSlice(key)
    } else {
        RoiDirtyRegion::Full
    }
}

/// Replaces the contour data after an edit at `dirty_plane`, rebuilding only as much of the voxel
/// cache as the edit can have changed (see [`dirty_region_for_slice_edit`]).
pub fn replace_contour_data_for_slice(
    world: &mut World,
    roi_entity: hecs::Entity,
    contour_data: ContourData,
    dirty_plane: PlaneDefinition,
) -> Result<(), ContourMutationError> {
    let mut roi = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| ContourMutationError::MissingRoi)?;

    let dirty_region = match &mut roi.body {
        RoiBody::Contour(ContourBody { data: existing, .. }) => {
            let region = dirty_region_for_slice_edit(&contour_data, dirty_plane);
            *existing = contour_data;
            region
        }
        RoiBody::Voxel(_) | RoiBody::Mesh(_) => {
            return Err(ContourMutationError::NotContourRoi);
        }
    };

    roi.mark_contour_authoritative_changed();
    roi.mark_all_contour_view_caches_stale();
    roi.job_state = RoiJobState::default();
    let source_generation = roi.dirty_state.authoritative.shape;
    roi.enqueue_job(RoiJobRequest {
        kind: RoiJobKind::RebuildVoxelCache,
        source_generation,
        preview_revision: None,
        priority: RoiJobPriority::VisibleCommitted,
        dirty_region,
    });
    Ok(())
}

pub fn replace_mesh_data(
    world: &mut World,
    roi_entity: hecs::Entity,
    mesh_data: MeshData,
) -> Result<(), MeshMutationError> {
    let mut roi = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| MeshMutationError::MissingRoi)?;

    match &mut roi.body {
        RoiBody::Mesh(MeshBody { data: existing, .. }) => *existing = mesh_data,
        RoiBody::Voxel(_) | RoiBody::Contour(_) => {
            return Err(MeshMutationError::NotMeshRoi);
        }
    }

    roi.mark_mesh_authoritative_changed();
    // Mesh contours and the 3D surface read the authority directly. Rebuilding
    // the optional voxel cache here can take nearly a second for a liver ROI,
    // so only an explicit voxel/export request may schedule it.
    roi.job_state = RoiJobState::default();
    Ok(())
}

pub fn request_mesh_voxel_cache_rebuild(
    world: &mut World,
    roi_entity: hecs::Entity,
) -> Result<(), MeshMutationError> {
    let mut roi = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| MeshMutationError::MissingRoi)?;
    let RoiBody::Mesh(MeshBody { data: mesh, .. }) = &roi.body else {
        return Err(MeshMutationError::NotMeshRoi);
    };
    if roi.validated_mesh_generation != Some(roi.dirty_state.authoritative.shape) {
        crate::convert::validate_mesh_for_voxelization(mesh)
            .map_err(MeshMutationError::InvalidMesh)?;
        roi.validated_mesh_generation = Some(roi.dirty_state.authoritative.shape);
    }
    roi.mark_cache_dirty(RoiCacheKind::Voxel);
    roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
    Ok(())
}
