use crate::app::components::{
    CacheViewState, ContourViewKey, Roi, RoiAuthoritativeData, RoiCacheKind, RoiJobKind, ViewMode,
};
use hecs::World;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoiCacheStatus {
    pub authoritative_generation: u64,
    pub cache_generation: u64,
    pub is_dirty: bool,
    pub is_current: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepresentationRequestState {
    Current,
    Preview,
    Stale,
    Queued,
    Rebuilding,
    Blocked,
    Unsupported,
}

impl RepresentationRequestState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Preview => "preview",
            Self::Stale => "stale",
            Self::Queued => "queued",
            Self::Rebuilding => "rebuilding",
            Self::Blocked => "blocked",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepresentationRequestStatus {
    pub state: RepresentationRequestState,
    pub reason: Option<String>,
}

impl RepresentationRequestStatus {
    pub(crate) fn current() -> Self {
        Self {
            state: RepresentationRequestState::Current,
            reason: None,
        }
    }

    pub(crate) fn stale(reason: impl Into<String>) -> Self {
        Self {
            state: RepresentationRequestState::Stale,
            reason: Some(reason.into()),
        }
    }

    pub(crate) fn preview(reason: impl Into<String>) -> Self {
        Self {
            state: RepresentationRequestState::Preview,
            reason: Some(reason.into()),
        }
    }

    pub(crate) fn queued(reason: impl Into<String>) -> Self {
        Self {
            state: RepresentationRequestState::Queued,
            reason: Some(reason.into()),
        }
    }

    pub(crate) fn rebuilding(reason: impl Into<String>) -> Self {
        Self {
            state: RepresentationRequestState::Rebuilding,
            reason: Some(reason.into()),
        }
    }

    pub(crate) fn blocked(reason: impl Into<String>) -> Self {
        Self {
            state: RepresentationRequestState::Blocked,
            reason: Some(reason.into()),
        }
    }

    pub(crate) fn unsupported(reason: impl Into<String>) -> Self {
        Self {
            state: RepresentationRequestState::Unsupported,
            reason: Some(reason.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContourRepresentationStatus {
    pub request: RepresentationRequestStatus,
    pub editable: bool,
    pub promotable: bool,
}

pub fn cache_status(
    world: &World,
    roi_entity: hecs::Entity,
    kind: RoiCacheKind,
) -> Option<RoiCacheStatus> {
    let roi = world.get::<&Roi>(roi_entity).ok()?;
    Some(RoiCacheStatus {
        authoritative_generation: roi.dirty_state.generations.authoritative,
        cache_generation: roi.cache_generation(kind),
        is_dirty: roi.is_cache_dirty(kind),
        is_current: roi.is_cache_current(kind),
    })
}

pub fn request_voxel_overlay_state(
    world: &World,
    roi_entity: hecs::Entity,
) -> RepresentationRequestStatus {
    let Ok(roi) = world.get::<&Roi>(roi_entity) else {
        return RepresentationRequestStatus::blocked("roi_missing");
    };
    if roi.voxel_cache().is_none() {
        return RepresentationRequestStatus::blocked("voxel_cache_missing");
    }
    if roi.running_job_kind() == Some(RoiJobKind::RebuildVoxelCache) {
        return RepresentationRequestStatus::rebuilding("voxel_cache_rebuilding");
    }
    if roi.has_queued_job(RoiJobKind::RebuildVoxelCache) {
        return RepresentationRequestStatus::queued("voxel_cache_rebuild_queued");
    }
    if roi.is_cache_dirty(RoiCacheKind::Voxel) || !roi.is_cache_current(RoiCacheKind::Voxel) {
        return RepresentationRequestStatus::stale("voxel_cache_stale");
    }
    if roi.voxel_gpu_cache().is_none() {
        return RepresentationRequestStatus::blocked("voxel_gpu_missing");
    }
    RepresentationRequestStatus::current()
}

pub fn request_mesh_cache_state(
    world: &World,
    roi_entity: hecs::Entity,
) -> RepresentationRequestStatus {
    let Ok(roi) = world.get::<&Roi>(roi_entity) else {
        return RepresentationRequestStatus::blocked("roi_missing");
    };
    if matches!(roi.authoritative_data, RoiAuthoritativeData::Mesh(_)) {
        return RepresentationRequestStatus::current();
    }
    if roi.mesh_cache().is_none() {
        if roi.running_job_kind() == Some(RoiJobKind::RebuildMeshCache) {
            return RepresentationRequestStatus::rebuilding("mesh_cache_rebuilding");
        }
        if roi.has_queued_job(RoiJobKind::RebuildMeshCache) {
            return RepresentationRequestStatus::queued("mesh_cache_rebuild_queued");
        }
        return RepresentationRequestStatus::blocked("mesh_cache_missing");
    }
    if roi.running_job_kind() == Some(RoiJobKind::RebuildMeshCache) {
        return RepresentationRequestStatus::rebuilding("mesh_cache_rebuilding");
    }
    if roi.has_queued_job(RoiJobKind::RebuildMeshCache) {
        return RepresentationRequestStatus::queued("mesh_cache_rebuild_queued");
    }
    if roi.is_cache_dirty(RoiCacheKind::Mesh) || !roi.is_cache_current(RoiCacheKind::Mesh) {
        return RepresentationRequestStatus::stale("mesh_cache_stale");
    }
    RepresentationRequestStatus::current()
}

pub fn request_contour_view_state(
    world: &World,
    roi_entity: hecs::Entity,
    view_key: &ContourViewKey,
) -> ContourRepresentationStatus {
    let Ok(roi) = world.get::<&Roi>(roi_entity) else {
        return ContourRepresentationStatus {
            request: RepresentationRequestStatus::blocked("roi_missing"),
            editable: false,
            promotable: false,
        };
    };
    let contour = roi.contour_data();
    let contour_primary = contour.is_some();
    let mesh_primary = matches!(&roi.authoritative_data, RoiAuthoritativeData::Mesh(_));
    if let Some(contour) = contour {
        if contour.active_plane_family == view_key.family {
            return ContourRepresentationStatus {
                request: RepresentationRequestStatus::current(),
                editable: true,
                promotable: false,
            };
        }
    }

    let Some(view_cache) = roi.contour_view_cache(view_key) else {
        if roi.running_job_kind() == Some(RoiJobKind::RebuildVoxelCache) {
            return ContourRepresentationStatus {
                request: RepresentationRequestStatus::rebuilding(
                    "voxel_cache_rebuilding_for_contour_view",
                ),
                editable: false,
                promotable: false,
            };
        }
        if roi.has_queued_job(RoiJobKind::RebuildVoxelCache) {
            return ContourRepresentationStatus {
                request: RepresentationRequestStatus::queued("voxel_cache_queued_for_contour_view"),
                editable: false,
                promotable: false,
            };
        }
        if roi.voxel_cache().is_none() {
            return ContourRepresentationStatus {
                request: RepresentationRequestStatus::blocked(
                    "voxel_cache_missing_for_contour_view",
                ),
                editable: false,
                promotable: false,
            };
        }
        if roi.is_cache_dirty(RoiCacheKind::Voxel) || !roi.is_cache_current(RoiCacheKind::Voxel) {
            return ContourRepresentationStatus {
                request: RepresentationRequestStatus::stale("voxel_cache_stale_for_contour_view"),
                editable: false,
                promotable: false,
            };
        }
        return ContourRepresentationStatus {
            request: RepresentationRequestStatus::rebuilding("contour_view_cache_missing"),
            editable: false,
            promotable: false,
        };
    };

    if view_cache.source_generation != roi.dirty_state.generations.authoritative {
        return ContourRepresentationStatus {
            request: RepresentationRequestStatus::stale("contour_view_cache_generation_stale"),
            editable: false,
            promotable: false,
        };
    }

    let request = match &view_cache.state {
        CacheViewState::Current => {
            if mesh_primary {
                RepresentationRequestStatus::current()
            } else if roi.running_job_kind() == Some(RoiJobKind::RebuildVoxelCache) {
                RepresentationRequestStatus::rebuilding("voxel_cache_rebuilding_for_contour_view")
            } else if roi.has_queued_job(RoiJobKind::RebuildVoxelCache) {
                RepresentationRequestStatus::queued("voxel_cache_queued_for_contour_view")
            } else if roi.is_cache_dirty(RoiCacheKind::Voxel)
                || !roi.is_cache_current(RoiCacheKind::Voxel)
            {
                RepresentationRequestStatus::stale("voxel_cache_stale_for_contour_view")
            } else {
                RepresentationRequestStatus::current()
            }
        }
        CacheViewState::Preview { .. } => {
            RepresentationRequestStatus::preview("contour_view_cache_preview")
        }
        CacheViewState::Stale => RepresentationRequestStatus::stale("contour_view_cache_stale"),
        CacheViewState::Queued => RepresentationRequestStatus::queued("contour_view_cache_queued"),
        CacheViewState::Rebuilding => {
            RepresentationRequestStatus::rebuilding("contour_view_cache_rebuilding")
        }
        CacheViewState::Blocked { reason } => RepresentationRequestStatus::blocked(reason.clone()),
        CacheViewState::Unsupported { reason } => {
            RepresentationRequestStatus::unsupported(reason.clone())
        }
    };
    let promotable = contour_primary && request.state == RepresentationRequestState::Current;
    ContourRepresentationStatus {
        request,
        editable: false,
        promotable,
    }
}

pub fn request_viewport_voxel_overlay_state(
    world: &World,
    viewport_mode: ViewMode,
    roi_entity: hecs::Entity,
) -> RepresentationRequestStatus {
    match viewport_mode {
        ViewMode::ThreeD => {
            RepresentationRequestStatus::unsupported("overlay_not_applicable_in_three_d_view")
        }
        ViewMode::Axial | ViewMode::Coronal | ViewMode::Sagittal | ViewMode::Oblique => {
            request_voxel_overlay_state(world, roi_entity)
        }
    }
}

pub fn request_viewport_mesh_state(
    world: &World,
    viewport_mode: ViewMode,
    roi_entity: hecs::Entity,
) -> RepresentationRequestStatus {
    match viewport_mode {
        ViewMode::ThreeD => request_mesh_cache_state(world, roi_entity),
        ViewMode::Axial | ViewMode::Coronal | ViewMode::Sagittal | ViewMode::Oblique => {
            RepresentationRequestStatus::unsupported("mesh_not_applicable_in_non_three_d_view")
        }
    }
}
