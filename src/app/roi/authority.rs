use crate::app::components::{
    CacheGeneration, CacheViewState, ContourData, ContourSliceKey, ContourViewKey, MeshCache,
    MeshData, Roi, RoiAuthoritativeData, RoiCacheKind, RoiDirtyRegion, RoiDirtyState, RoiJobKind,
    RoiJobPriority, RoiJobRequest, RoiJobState, VoxelCache,
};
use crate::convert::{
    extract_contours_from_voxel_data, PlaneDefinition, PlaneFamily, VoxelContourExtractionError,
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
    NotContourRoi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshMutationError {
    MissingRoi,
    MissingEditorState,
    MissingPreview,
    NotMeshRoi,
    InvalidDelta,
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
    let source_generation = roi.dirty_state.generations.authoritative;
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
        || view_cache.source_generation != roi.dirty_state.generations.authoritative
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
    let new_generation = roi.dirty_state.generations.authoritative;
    for slice in previous_authoritative
        .slices
        .iter()
        .filter(|slice| slice.plane.family == active_family)
    {
        roi.upsert_contour_view_cache(
            ContourViewKey::from_plane(slice.plane),
            previous_authoritative.clone(),
            new_generation,
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
    if let Some(mesh) = authoritative_mesh {
        roi.session_caches.mesh = Some(MeshCache {
            data: mesh,
            chunks: None,
        });
    }
    let new_generation = roi.dirty_state.generations.authoritative.saturating_add(1);

    roi.authoritative_data = RoiAuthoritativeData::Contour(extracted);
    roi.session_caches.contour = None;
    if let Some(voxel_cache) = roi.session_caches.voxel.as_mut() {
        voxel_cache.data = source_voxel;
    } else {
        roi.session_caches.voxel = Some(VoxelCache {
            data: source_voxel,
            gpu_resources: None,
        });
    }
    roi.job_state = RoiJobState::default();
    roi.end_preview();
    roi.dirty_state = RoiDirtyState {
        authoritative_dirty: true,
        voxel_cache_dirty: false,
        contour_cache_dirty: false,
        mesh_cache_dirty: !mesh_cache_was_current,
        generations: CacheGeneration {
            authoritative: new_generation,
            voxel: new_generation,
            contour: new_generation,
            mesh: if mesh_cache_was_current {
                new_generation
            } else {
                0
            },
        },
    };
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
    let new_generation = roi.dirty_state.generations.authoritative.saturating_add(1);

    roi.authoritative_data = RoiAuthoritativeData::Voxel(source_voxel.clone());
    if let Some(voxel_cache) = roi.session_caches.voxel.as_mut() {
        voxel_cache.data = source_voxel;
    } else {
        unreachable!("current voxel cache was checked before promotion");
    }
    roi.session_caches.contour = None;
    roi.session_caches.mesh = retained_mesh;
    roi.job_state = RoiJobState::default();
    roi.end_preview();
    roi.dirty_state = RoiDirtyState {
        authoritative_dirty: true,
        voxel_cache_dirty: false,
        contour_cache_dirty: true,
        mesh_cache_dirty: !mesh_cache_is_current,
        generations: CacheGeneration {
            authoritative: new_generation,
            voxel: new_generation,
            contour: 0,
            mesh: if mesh_cache_is_current {
                new_generation
            } else {
                0
            },
        },
    };
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
    let new_generation = roi.dirty_state.generations.authoritative.saturating_add(1);

    roi.authoritative_data = RoiAuthoritativeData::Mesh(mesh);
    roi.session_caches.mesh = None;
    roi.session_caches.contour = None;
    roi.job_state = RoiJobState::default();
    roi.end_preview();
    roi.dirty_state = RoiDirtyState {
        authoritative_dirty: true,
        voxel_cache_dirty: !voxel_cache_was_current,
        contour_cache_dirty: true,
        mesh_cache_dirty: false,
        generations: CacheGeneration {
            authoritative: new_generation,
            voxel: if voxel_cache_was_current {
                new_generation
            } else {
                0
            },
            contour: 0,
            mesh: new_generation,
        },
    };
    if !voxel_cache_was_current {
        roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
    }
    Ok(())
}
