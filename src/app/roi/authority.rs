use crate::app::components::{
    ContourBody, ContourData, MeshBody, MeshData, Roi, RoiBody, RoiCacheKind, RoiJobKind,
    RoiJobState,
};
use crate::convert::MeshVoxelizationError;
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
    roi.job_state = RoiJobState::default();
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

/// Asks for the voxel form of a contour ROI (the fill overlay, an export). It is derived from the
/// loops and not built by an edit; the revision it is built for is the current one.
pub fn request_contour_voxel_form(
    world: &mut World,
    roi_entity: hecs::Entity,
) -> Result<(), ContourMutationError> {
    let mut roi = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| ContourMutationError::MissingRoi)?;
    if !matches!(roi.body, RoiBody::Contour(_)) {
        return Err(ContourMutationError::NotContourRoi);
    }
    roi.mark_cache_dirty(RoiCacheKind::Voxel);
    roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
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
