use crate::app::components::{
    CacheViewState, ContourData, ContourSliceKey, ContourViewKey, MeshCache, MeshData, Roi,
    RoiAuthoritativeData, RoiCacheKind, RoiDirtyRegion, RoiJobKind, RoiJobPriority, RoiJobRequest,
    RoiJobState,
};
use crate::convert::{
    extract_contours_from_voxel_data, MeshVoxelizationError, PlaneDefinition, PlaneFamily,
    VoxelContourExtractionError,
};
use hecs::World;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContourPlaneFamilySwitchError {
    MissingRoi,
    NotContourRoi,
    RequiresConversion,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContourPromotionError {
    MissingRoi,
    NotContourRoi,
    ViewCacheMissing,
    ViewCacheNotCurrent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoxelContourPromotionError {
    MissingRoi,
    Locked,
    NotVoxelRoi,
    VoxelCacheMissing,
    VoxelCacheNotCurrent,
    ExtractionFailed(VoxelContourExtractionError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoxelAuthorityPromotionError {
    MissingRoi,
    Locked,
    AlreadyVoxelPrimary,
    VoxelCacheMissing,
    VoxelCacheNotCurrent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshAuthorityPromotionError {
    MissingRoi,
    Locked,
    AlreadyMeshPrimary,
    MeshCacheMissing,
    MeshCacheNotCurrent,
    EmptyMesh,
}

pub fn set_active_contour_plane_family(
    world: &mut World,
    roi_entity: hecs::Entity,
    family: PlaneFamily,
) -> Result<(), ContourPlaneFamilySwitchError> {
    let mut roi = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| ContourPlaneFamilySwitchError::MissingRoi)?;

    let contour = match &mut roi.authoritative_data {
        RoiAuthoritativeData::Contour(contour) => contour,
        RoiAuthoritativeData::Voxel(_) | RoiAuthoritativeData::Mesh(_) => {
            return Err(ContourPlaneFamilySwitchError::NotContourRoi);
        }
    };

    if contour.active_plane_family == family {
        return Ok(());
    }
    if contour.has_loops() {
        return Err(ContourPlaneFamilySwitchError::RequiresConversion);
    }

    contour.active_plane_family = family;
    roi.mark_contour_authoritative_changed();
    Ok(())
}

pub fn replace_contour_data(
    world: &mut World,
    roi_entity: hecs::Entity,
    contour_data: ContourData,
) -> Result<(), ContourMutationError> {
    let mut roi = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| ContourMutationError::MissingRoi)?;

    match &mut roi.authoritative_data {
        RoiAuthoritativeData::Contour(existing) => *existing = contour_data,
        RoiAuthoritativeData::Voxel(_) | RoiAuthoritativeData::Mesh(_) => {
            return Err(ContourMutationError::NotContourRoi);
        }
    }

    roi.mark_contour_authoritative_changed();
    roi.mark_all_contour_view_caches_stale();
    roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
    Ok(())
}

pub fn replace_contour_data_for_slice(
    world: &mut World,
    roi_entity: hecs::Entity,
    contour_data: ContourData,
    dirty_plane: PlaneDefinition,
) -> Result<(), ContourMutationError> {
    let mut roi = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| ContourMutationError::MissingRoi)?;

    match &mut roi.authoritative_data {
        RoiAuthoritativeData::Contour(existing) => *existing = contour_data,
        RoiAuthoritativeData::Voxel(_) | RoiAuthoritativeData::Mesh(_) => {
            return Err(ContourMutationError::NotContourRoi);
        }
    }

    roi.mark_contour_authoritative_changed();
    roi.mark_all_contour_view_caches_stale();
    roi.job_state = RoiJobState::default();
    let source_generation = roi.dirty_state.authoritative.shape;
    roi.enqueue_job(RoiJobRequest {
        kind: RoiJobKind::RebuildVoxelCache,
        source_generation,
        preview_revision: None,
        priority: RoiJobPriority::VisibleCommitted,
        dirty_region: RoiDirtyRegion::ContourSlice(ContourSliceKey::from_plane(dirty_plane)),
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

    match &mut roi.authoritative_data {
        RoiAuthoritativeData::Mesh(existing) => *existing = mesh_data,
        RoiAuthoritativeData::Voxel(_) | RoiAuthoritativeData::Contour(_) => {
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
    let RoiAuthoritativeData::Mesh(mesh) = &roi.authoritative_data else {
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

pub fn promote_contour_view_to_authoritative(
    world: &mut World,
    roi_entity: hecs::Entity,
    view_key: &ContourViewKey,
) -> Result<(), ContourPromotionError> {
    let mut roi = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| ContourPromotionError::MissingRoi)?;
    let (active_family, previous_authoritative) = match &roi.authoritative_data {
        RoiAuthoritativeData::Contour(contour) => (contour.active_plane_family, contour.clone()),
        RoiAuthoritativeData::Voxel(_) | RoiAuthoritativeData::Mesh(_) => {
            return Err(ContourPromotionError::NotContourRoi);
        }
    };

    if active_family == view_key.family {
        return Ok(());
    }

    let Some(view_cache) = roi.contour_view_cache(view_key).cloned() else {
        return Err(ContourPromotionError::ViewCacheMissing);
    };
    if view_cache.state != CacheViewState::Current
        || roi.is_cache_dirty(RoiCacheKind::Voxel)
        || !roi.is_cache_current(RoiCacheKind::Voxel)
        || view_cache.built_from != roi.dirty_state.authoritative
    {
        return Err(ContourPromotionError::ViewCacheNotCurrent);
    }

    let new_data = view_cache.data;
    let contour = match &mut roi.authoritative_data {
        RoiAuthoritativeData::Contour(contour) => contour,
        RoiAuthoritativeData::Voxel(_) | RoiAuthoritativeData::Mesh(_) => {
            return Err(ContourPromotionError::NotContourRoi);
        }
    };
    *contour = new_data;
    contour.active_plane_family = view_key.family;
    roi.mark_contour_authoritative_changed();
    let new_revision = roi.dirty_state.authoritative;
    for slice in previous_authoritative
        .slices
        .iter()
        .filter(|slice| slice.plane.family == active_family)
    {
        roi.upsert_contour_view_cache(
            ContourViewKey::from_plane(slice.plane),
            previous_authoritative.clone(),
            new_revision,
            CacheViewState::Stale,
        );
    }
    roi.mark_all_contour_view_caches_stale();
    roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
    roi.mark_cache_dirty(RoiCacheKind::Mesh);
    Ok(())
}

pub fn promote_voxel_roi_to_contour_authority(
    world: &mut World,
    roi_entity: hecs::Entity,
    family: PlaneFamily,
) -> Result<(), VoxelContourPromotionError> {
    let roi = world
        .get::<&Roi>(roi_entity)
        .map_err(|_| VoxelContourPromotionError::MissingRoi)?;
    if !matches!(roi.authoritative_data, RoiAuthoritativeData::Voxel(_)) {
        return Err(VoxelContourPromotionError::NotVoxelRoi);
    }
    drop(roi);
    promote_roi_to_contour_authority(world, roi_entity, family)
}

pub fn promote_roi_to_contour_authority(
    world: &mut World,
    roi_entity: hecs::Entity,
    family: PlaneFamily,
) -> Result<(), VoxelContourPromotionError> {
    let (source_voxel, authoritative_mesh) = {
        let roi = world
            .get::<&Roi>(roi_entity)
            .map_err(|_| VoxelContourPromotionError::MissingRoi)?;
        if roi.metadata.is_locked {
            return Err(VoxelContourPromotionError::Locked);
        }
        match &roi.authoritative_data {
            RoiAuthoritativeData::Voxel(voxel) => (voxel.clone(), None),
            RoiAuthoritativeData::Contour(contour) if contour.active_plane_family == family => {
                return Ok(())
            }
            RoiAuthoritativeData::Contour(_) | RoiAuthoritativeData::Mesh(_) => {
                let voxel_cache = roi
                    .voxel_cache()
                    .ok_or(VoxelContourPromotionError::VoxelCacheMissing)?;
                if !roi.is_cache_current(RoiCacheKind::Voxel) {
                    return Err(VoxelContourPromotionError::VoxelCacheNotCurrent);
                }
                let mesh = match &roi.authoritative_data {
                    RoiAuthoritativeData::Mesh(mesh) => Some(mesh.clone()),
                    RoiAuthoritativeData::Voxel(_) | RoiAuthoritativeData::Contour(_) => None,
                };
                (voxel_cache.data.clone(), mesh)
            }
        }
    };
    let extracted = extract_contours_from_voxel_data(&source_voxel, family)
        .map_err(VoxelContourPromotionError::ExtractionFailed)?;

    let mut roi = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| VoxelContourPromotionError::MissingRoi)?;
    let mesh_cache_was_current = authoritative_mesh.is_some()
        || (roi.mesh_cache().is_some() && roi.is_cache_current(RoiCacheKind::Mesh));
    let replacement_mesh = authoritative_mesh.map(|mesh| MeshCache {
        data: mesh,
        chunks: None,
    });

    roi.authoritative_data = RoiAuthoritativeData::Contour(extracted);
    roi.job_state = RoiJobState::default();
    roi.end_preview();
    roi.rebase_after_contour_promotion(source_voxel, replacement_mesh, mesh_cache_was_current);
    if !mesh_cache_was_current {
        roi.enqueue_rebuild(RoiJobKind::RebuildMeshCache);
    }
    Ok(())
}

pub fn promote_current_voxel_cache_to_authority(
    world: &mut World,
    roi_entity: hecs::Entity,
) -> Result<(), VoxelAuthorityPromotionError> {
    let (source_voxel, retained_mesh) = {
        let roi = world
            .get::<&Roi>(roi_entity)
            .map_err(|_| VoxelAuthorityPromotionError::MissingRoi)?;
        if roi.metadata.is_locked {
            return Err(VoxelAuthorityPromotionError::Locked);
        }
        if matches!(roi.authoritative_data, RoiAuthoritativeData::Voxel(_)) {
            return Err(VoxelAuthorityPromotionError::AlreadyVoxelPrimary);
        }
        let voxel_cache = roi
            .voxel_cache()
            .ok_or(VoxelAuthorityPromotionError::VoxelCacheMissing)?;
        if !roi.is_cache_current(RoiCacheKind::Voxel) {
            return Err(VoxelAuthorityPromotionError::VoxelCacheNotCurrent);
        }
        let retained_mesh = match &roi.authoritative_data {
            RoiAuthoritativeData::Mesh(mesh) => Some(MeshCache {
                data: mesh.clone(),
                chunks: None,
            }),
            RoiAuthoritativeData::Voxel(_) | RoiAuthoritativeData::Contour(_) => {
                (roi.mesh_cache().is_some() && roi.is_cache_current(RoiCacheKind::Mesh))
                    .then(|| roi.mesh_cache().expect("mesh cache checked").clone())
            }
        };
        (voxel_cache.data.clone(), retained_mesh)
    };

    let mut roi = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| VoxelAuthorityPromotionError::MissingRoi)?;
    let mesh_cache_is_current = retained_mesh.is_some();

    roi.authoritative_data = RoiAuthoritativeData::Voxel(source_voxel.clone());
    roi.job_state = RoiJobState::default();
    roi.end_preview();
    roi.rebase_after_voxel_promotion(source_voxel, retained_mesh);
    if !mesh_cache_is_current {
        roi.enqueue_rebuild(RoiJobKind::RebuildMeshCache);
    }
    Ok(())
}

pub fn promote_current_mesh_cache_to_authority(
    world: &mut World,
    roi_entity: hecs::Entity,
) -> Result<(), MeshAuthorityPromotionError> {
    let mesh = {
        let roi = world
            .get::<&Roi>(roi_entity)
            .map_err(|_| MeshAuthorityPromotionError::MissingRoi)?;
        if roi.metadata.is_locked {
            return Err(MeshAuthorityPromotionError::Locked);
        }
        if matches!(roi.authoritative_data, RoiAuthoritativeData::Mesh(_)) {
            return Err(MeshAuthorityPromotionError::AlreadyMeshPrimary);
        }
        let mesh_cache = roi
            .mesh_cache()
            .ok_or(MeshAuthorityPromotionError::MeshCacheMissing)?;
        if !roi.is_cache_current(RoiCacheKind::Mesh) {
            return Err(MeshAuthorityPromotionError::MeshCacheNotCurrent);
        }
        if mesh_cache.data.vertices.is_empty() || mesh_cache.data.faces.is_empty() {
            return Err(MeshAuthorityPromotionError::EmptyMesh);
        }
        mesh_cache.data.clone()
    };

    let mut roi = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| MeshAuthorityPromotionError::MissingRoi)?;
    let voxel_cache_was_current =
        roi.voxel_cache().is_some() && roi.is_cache_current(RoiCacheKind::Voxel);

    roi.authoritative_data = RoiAuthoritativeData::Mesh(mesh);
    roi.job_state = RoiJobState::default();
    roi.end_preview();
    roi.rebase_after_mesh_promotion(voxel_cache_was_current);
    Ok(())
}
