use crate::app::components::*;
pub use crate::app::roi::requests::{
    ContourRepresentationStatus, RepresentationRequestState, RepresentationRequestStatus,
    RoiCacheStatus,
};
#[cfg(test)]
use crate::convert::PlaneDefinition;
use crate::convert::{
    contour_geometry_voxel_aabb, contour_slices_voxel_aabb, extract_contour_slice_from_voxel_data,
    extract_contours_from_voxel_data, extract_mesh_from_voxel_data, intersect_mesh_with_plane,
    rasterize_contour_preview_slices_to_voxel_data, rasterize_contours_to_voxel_data,
    voxelize_mesh_to_voxel_data, IncrementalChunkedMeshRebuild, MeshVoxelizationError, PlaneFamily,
    VoxelContourExtractionError, VoxelMeshExtractionError, DEFAULT_MESH_CHUNK_SIZE,
};
use crate::render::roi_views::{RenderRepresentationRequest, RoiRenderViews};
use hecs::World;
use web_time::{Duration, Instant};

struct ContourPreviewMeshWork {
    source_generation: u64,
    preview_revision: u64,
    preview_aabb: Option<([u32; 3], [u32; 3])>,
    voxel_data: VoxelData,
    rebuild: IncrementalChunkedMeshRebuild,
    started_at: Instant,
}

struct VoxelMeshRebuildWork {
    source_generation: u64,
    voxel_data: VoxelData,
    rebuild: IncrementalChunkedMeshRebuild,
    started_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoxelRoiStats {
    pub occupied_voxels: u64,
    pub volume_mm3: f32,
}

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
pub enum RoiEditHistoryError {
    MissingEditorState,
    MissingRoi,
    NoUndo,
    NoRedo,
    Locked,
    RepresentationChanged,
}

const MAX_ROI_EDIT_HISTORY: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoxelContourCreationError {
    MissingRoi,
    NotVoxelRoi,
    ExtractionFailed(VoxelContourExtractionError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoxelMeshCreationError {
    MissingRoi,
    NotVoxelRoi,
    MissingMainVolume,
    EmptyMeshFromNonEmptySource,
    ExtractionFailed(VoxelMeshExtractionError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContourMeshCreationError {
    MissingRoi,
    NotContourRoi,
    MissingCurrentVoxelCache,
    ExtractionFailed(VoxelMeshExtractionError),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayVoxelSourceError {
    MissingRoi,
    NotVoxelRoi,
    MissingMainVolume,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshDerivedRebuildError {
    MissingRoi,
    NotMeshRoi,
    MissingTargetGeometry,
    VoxelizationFailed(MeshVoxelizationError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContourPromotionError {
    MissingRoi,
    NotContourRoi,
    ViewCacheMissing,
    ViewCacheNotCurrent,
}

pub const MAX_SIMULTANEOUS_ROI_OVERLAYS: usize =
    crate::render::roi_views::DEFAULT_MAX_VOXEL_OVERLAYS;

fn plane_family_label(family: PlaneFamily) -> &'static str {
    match family {
        PlaneFamily::Axial => "Axial",
        PlaneFamily::Coronal => "Coronal",
        PlaneFamily::Sagittal => "Sagittal",
        PlaneFamily::Oblique => "Oblique",
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderableVoxelOverlay {
    pub entity: hecs::Entity,
    pub opacity: f32,
}

/// GPU handles needed to rebuild scene bind groups after ROI or volume updates.
pub struct BindGroupResources<'a> {
    pub layout: &'a wgpu::BindGroupLayout,
    pub uniform_buffer: &'a wgpu::Buffer,
    pub dummy_view: &'a wgpu::TextureView,
    pub dummy_sampler: &'a wgpu::Sampler,
    pub default_lut_view: &'a wgpu::TextureView,
    pub overlay_buffer: &'a wgpu::Buffer,
}

/// Recreate the scene bind group with current volume and renderable ROI textures.
///
/// This is a runtime concern rather than a load-handler concern because it
/// consumes current ROI cache state and updates render-facing derived resources.
pub fn recreate_scene_bind_groups(
    device: &wgpu::Device,
    world: &mut World,
    resources: &BindGroupResources<'_>,
    active_roi: Option<hecs::Entity>,
) {
    let main_view: Option<wgpu::TextureView>;
    let overlay_entities = renderable_voxel_overlay_rois(world, active_roi);
    let mut overlay_views = Vec::new();

    {
        let query = world.query::<&GpuVolumeResources>();
        let mut with_tag = query.with::<&MainVolumeTag>();
        main_view = with_tag.iter().next().map(|(_, res)| res.view.clone());
    }

    for overlay in &overlay_entities {
        if let Ok(roi) = world.get::<&Roi>(overlay.entity) {
            if let Some(res) = roi.renderable_voxel_cache() {
                overlay_views.push(res.view.clone());
            }
        }
    }

    let main_view_ref = main_view.as_ref().unwrap_or(resources.dummy_view);
    let overlay_views =
        std::array::from_fn(|slot| overlay_views.get(slot).unwrap_or(resources.dummy_view));

    let new_bind_group = crate::render::pipeline::create_scene_bind_group(
        device,
        resources.layout,
        &crate::render::pipeline::SceneTextureViews {
            volume_view: main_view_ref,
            volume_sampler: resources.dummy_sampler,
            uniform_buffer: resources.uniform_buffer,
            overlay_views,
            overlay_lut: resources.default_lut_view,
            overlay_buffer: resources.overlay_buffer,
        },
    );

    for (_, res) in world.query_mut::<&mut GpuVolumeResources>() {
        res.bind_group = new_bind_group.clone();
    }
    for (_, roi) in world.query_mut::<&mut Roi>() {
        roi.update_voxel_bind_group(new_bind_group.clone());
    }
}

pub fn main_volume_geometry(world: &World) -> Option<VoxelGeometry> {
    let mut query = world.query::<&VolumeData>().with::<&MainVolumeTag>();
    let (_, volume) = query.iter().next()?;
    Some(VoxelGeometry {
        dimensions: volume.dimensions,
        spacing: volume.spacing,
        origin: volume.origin,
        orientation: volume.orientation,
    })
}

pub fn main_volume_voxel_geometry(world: &World) -> Option<VoxelGeometry> {
    main_volume_geometry(world)
}

pub fn renderable_voxel_overlay_rois(
    world: &World,
    active_roi: Option<hecs::Entity>,
) -> Vec<RenderableVoxelOverlay> {
    let views = RoiRenderViews::for_world(
        world,
        RenderRepresentationRequest {
            active_roi,
            max_voxel_overlays: MAX_SIMULTANEOUS_ROI_OVERLAYS,
            contour_active_only: true,
        },
    );
    views
        .voxel_overlays
        .into_iter()
        .map(|overlay| RenderableVoxelOverlay {
            entity: overlay.entity,
            opacity: overlay.opacity,
        })
        .collect()
}

pub fn visible_voxel_overlay_count(world: &World) -> usize {
    renderable_voxel_overlay_rois(world, None).len()
}

pub fn can_enable_roi_visibility(world: &World, roi_entity: hecs::Entity) -> bool {
    if let Ok(roi) = world.get::<&Roi>(roi_entity) {
        if roi.metadata.is_visible {
            return true;
        }
        if roi.renderable_voxel_cache().is_none() {
            return true;
        }
    }

    visible_voxel_overlay_count(world) < MAX_SIMULTANEOUS_ROI_OVERLAYS
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

pub fn request_voxel_overlay_state(
    world: &World,
    roi_entity: hecs::Entity,
) -> RepresentationRequestStatus {
    crate::app::roi::requests::request_voxel_overlay_state(world, roi_entity)
}

pub fn request_mesh_cache_state(
    world: &World,
    roi_entity: hecs::Entity,
) -> RepresentationRequestStatus {
    crate::app::roi::requests::request_mesh_cache_state(world, roi_entity)
}

pub fn request_contour_view_state(
    world: &World,
    roi_entity: hecs::Entity,
    view_key: &ContourViewKey,
) -> ContourRepresentationStatus {
    crate::app::roi::requests::request_contour_view_state(world, roi_entity, view_key)
}

fn build_contour_view_data_for_plane(
    voxel_data: &VoxelData,
    view_key: &ContourViewKey,
) -> Result<ContourData, VoxelContourExtractionError> {
    extract_contour_slice_from_voxel_data(voxel_data, view_key.plane)
}

pub fn ensure_contour_view_cache(
    world: &mut World,
    roi_entity: hecs::Entity,
    view_key: &ContourViewKey,
) -> RepresentationRequestStatus {
    let mesh_preview = world
        .query::<&EditorState>()
        .iter()
        .find_map(|(_, editor)| {
            editor
                .mesh_edit_preview
                .as_ref()
                .filter(|preview| preview.roi_entity == roi_entity)
                .map(|preview| preview.mesh_data.clone())
        });
    let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) else {
        return RepresentationRequestStatus::blocked("roi_missing");
    };
    if let Some(contour) = roi.contour_data() {
        if contour.active_plane_family == view_key.family {
            return RepresentationRequestStatus::current();
        }
    }
    let mesh_preview_revision = roi.preview_state.revision;
    let mesh_needs_direct_preview =
        matches!(&roi.authoritative_data, RoiAuthoritativeData::Mesh(_))
            && (mesh_preview.is_some()
                || roi.is_cache_dirty(RoiCacheKind::Voxel)
                || !roi.is_cache_current(RoiCacheKind::Voxel));
    if mesh_needs_direct_preview {
        let mesh = mesh_preview
            .as_ref()
            .or_else(|| roi.mesh_data())
            .expect("mesh-authoritative ROI must expose mesh data");
        return match intersect_mesh_with_plane(mesh, view_key.plane) {
            Ok(data) => {
                let generation = roi.dirty_state.generations.authoritative;
                roi.upsert_contour_view_cache(
                    view_key.clone(),
                    data,
                    generation,
                    CacheViewState::Preview {
                        revision: mesh_preview_revision,
                    },
                );
                RepresentationRequestStatus::preview("mesh_plane_intersection_preview")
            }
            Err(_) => RepresentationRequestStatus::blocked("mesh_plane_intersection_failed"),
        };
    }
    let contour_preview_voxel = roi.session_caches.preview_voxel.as_ref().and_then(|cache| {
        (roi.preview_state.active
            && cache.source_generation == roi.dirty_state.generations.authoritative
            && cache.preview_revision == roi.preview_state.revision)
            .then(|| cache.data.clone())
    });
    if let Some(preview_voxel) = contour_preview_voxel {
        return match build_contour_view_data_for_plane(&preview_voxel, view_key) {
            Ok(data) => {
                let generation = roi.dirty_state.generations.authoritative;
                let revision = roi.preview_state.revision;
                roi.upsert_contour_view_cache(
                    view_key.clone(),
                    data,
                    generation,
                    CacheViewState::Preview { revision },
                );
                RepresentationRequestStatus::preview("contour_edit_cross_plane_preview")
            }
            Err(_) => {
                RepresentationRequestStatus::unsupported("contour_edit_preview_plane_unsupported")
            }
        };
    }
    if roi.job_state.running == Some(RoiJobKind::RebuildVoxelCache) {
        if let Some(cache) = roi.contour_view_cache_mut(view_key) {
            if !matches!(
                cache.state,
                CacheViewState::Blocked { .. } | CacheViewState::Unsupported { .. }
            ) {
                cache.state = CacheViewState::Rebuilding;
            }
        }
        return RepresentationRequestStatus::rebuilding("voxel_cache_rebuilding_for_contour_view");
    }

    if roi.has_queued_job(RoiJobKind::RebuildVoxelCache) {
        if let Some(cache) = roi.contour_view_cache_mut(view_key) {
            if !matches!(
                cache.state,
                CacheViewState::Blocked { .. } | CacheViewState::Unsupported { .. }
            ) {
                cache.state = CacheViewState::Queued;
            }
        }
        return RepresentationRequestStatus::queued("voxel_cache_queued_for_contour_view");
    }

    if roi.voxel_cache().is_none() {
        return RepresentationRequestStatus::blocked("voxel_cache_missing_for_contour_view");
    }
    if roi.is_cache_dirty(RoiCacheKind::Voxel) || !roi.is_cache_current(RoiCacheKind::Voxel) {
        if let Some(cache) = roi.contour_view_cache_mut(view_key) {
            if !matches!(
                cache.state,
                CacheViewState::Blocked { .. } | CacheViewState::Unsupported { .. }
            ) {
                cache.state = CacheViewState::Stale;
            }
        }
        return RepresentationRequestStatus::stale("voxel_cache_stale_for_contour_view");
    }

    let generation = roi.dirty_state.generations.authoritative;
    if let Some(cache) = roi.contour_view_cache(view_key) {
        if cache.source_generation == generation && cache.state == CacheViewState::Current {
            return RepresentationRequestStatus::current();
        }
    }
    let voxel_data = roi
        .voxel_cache()
        .expect("voxel cache presence already checked")
        .data
        .clone();

    match build_contour_view_data_for_plane(&voxel_data, view_key) {
        Ok(data) => {
            roi.upsert_contour_view_cache(
                view_key.clone(),
                data,
                generation,
                CacheViewState::Current,
            );
            roi.dirty_state.contour_cache_dirty = false;
            roi.dirty_state.generations.contour = generation;
            RepresentationRequestStatus::current()
        }
        Err(
            VoxelContourExtractionError::UnsupportedPlaneFamily { .. }
            | VoxelContourExtractionError::UnsupportedPlaneGeometry,
        ) => {
            let generation = roi.dirty_state.generations.authoritative;
            roi.upsert_contour_view_cache(
                view_key.clone(),
                ContourData {
                    active_plane_family: view_key.family,
                    slices: Vec::new(),
                },
                generation,
                CacheViewState::Unsupported {
                    reason: "derived_contour_view_geometry_unsupported".to_string(),
                },
            );
            RepresentationRequestStatus::unsupported("derived_contour_view_geometry_unsupported")
        }
    }
}

pub fn request_viewport_voxel_overlay_state(
    world: &World,
    viewport_mode: ViewMode,
    roi_entity: hecs::Entity,
) -> RepresentationRequestStatus {
    crate::app::roi::requests::request_viewport_voxel_overlay_state(
        world,
        viewport_mode,
        roi_entity,
    )
}

pub fn request_viewport_mesh_state(
    world: &World,
    viewport_mode: ViewMode,
    roi_entity: hecs::Entity,
) -> RepresentationRequestStatus {
    crate::app::roi::requests::request_viewport_mesh_state(world, viewport_mode, roi_entity)
}

pub fn sync_active_roi_contour_view_caches_for_viewports(world: &mut World) {
    let active_roi = world
        .query::<&EditorState>()
        .iter()
        .next()
        .and_then(|(_, editor)| editor.active_roi);
    let Some(active_roi) = active_roi else {
        return;
    };

    let geometry = main_volume_voxel_geometry(world).or_else(|| {
        world
            .get::<&Roi>(active_roi)
            .ok()
            .and_then(|roi| roi.voxel_cache().map(|cache| cache.data.geometry))
    });
    let Some(geometry) = geometry else {
        return;
    };

    let cursor_uv = world
        .query::<&Transform>()
        .iter()
        .next()
        .map(|(_, transform)| transform.position)
        .unwrap_or([0.5, 0.5, 0.5]);

    let viewport_requests: Vec<_> = world
        .query::<(&Viewport, &ViewportState)>()
        .iter()
        .filter_map(|(_, (viewport, viewport_state))| {
            crate::render::roi_views::displayed_plane_for_viewport(
                viewport.mode,
                cursor_uv,
                viewport_state.user_rotation,
                geometry,
            )
            .map(ContourViewKey::from_plane)
        })
        .collect();

    for view_key in viewport_requests {
        let _ = ensure_contour_view_cache(world, active_roi, &view_key);
    }
}

pub fn sync_active_roi_mesh_cache_for_viewports(world: &mut World) {
    if !world
        .query::<&Viewport>()
        .iter()
        .any(|(_, viewport)| viewport.mode == ViewMode::ThreeD)
    {
        return;
    }
    let active_roi = world
        .query::<&EditorState>()
        .iter()
        .next()
        .and_then(|(_, editor)| editor.active_roi);
    let Some(active_roi) = active_roi else {
        return;
    };
    let Ok(mut roi) = world.get::<&mut Roi>(active_roi) else {
        return;
    };
    if matches!(&roi.authoritative_data, RoiAuthoritativeData::Mesh(_))
        || roi.has_queued_job(RoiJobKind::RebuildMeshCache)
        || roi.job_state.running == Some(RoiJobKind::RebuildMeshCache)
    {
        return;
    }
    if roi.voxel_cache().is_some() && roi.is_cache_current(RoiCacheKind::Voxel) {
        roi.mark_cache_dirty(RoiCacheKind::Mesh);
        roi.enqueue_rebuild(RoiJobKind::RebuildMeshCache);
    }
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
    if view_cache.state != CacheViewState::Current {
        return Err(ContourPromotionError::ViewCacheNotCurrent);
    }
    if roi.is_cache_dirty(RoiCacheKind::Voxel) || !roi.is_cache_current(RoiCacheKind::Voxel) {
        return Err(ContourPromotionError::ViewCacheNotCurrent);
    }
    if view_cache.source_generation != roi.dirty_state.generations.authoritative {
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
    dirty_plane: crate::convert::PlaneDefinition,
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
    dirty_plane: crate::convert::PlaneDefinition,
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
    dirty_plane: Option<crate::convert::PlaneDefinition>,
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
    editor.contour_move_preview = None;
    editor.mesh_edit_preview = None;
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
            roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
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

fn approx_eq_slice<const N: usize>(lhs: [f32; N], rhs: [f32; N], epsilon: f32) -> bool {
    lhs.into_iter()
        .zip(rhs)
        .all(|(left, right)| (left - right).abs() <= epsilon)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoxelRoiImportSpec {
    pub geometry: VoxelGeometry,
    pub start_visible: bool,
    pub geometry_matches_main: bool,
}

pub fn prepare_voxel_roi_import(
    world: &World,
    loaded_label: &LoadedLabel,
) -> Result<VoxelRoiImportSpec, String> {
    let geometry_matches_main = if let Some(main_geometry) = main_volume_voxel_geometry(world) {
        let differs = main_geometry.dimensions != loaded_label.dimensions
            || !approx_eq_slice(main_geometry.spacing, loaded_label.spacing, 1e-5)
            || !approx_eq_slice(main_geometry.origin, loaded_label.origin, 1e-5)
            || !approx_eq_slice(main_geometry.orientation, loaded_label.orientation, 1e-5);
        if differs {
            log::warn!(
                "Loaded label geometry differs from main volume geometry; preserving label-owned geometry. label dims={:?} spacing={:?} origin={:?} orientation={:?}, main dims={:?} spacing={:?} origin={:?} orientation={:?}",
                loaded_label.dimensions,
                loaded_label.spacing,
                loaded_label.origin,
                loaded_label.orientation,
                main_geometry.dimensions,
                main_geometry.spacing,
                main_geometry.origin,
                main_geometry.orientation
            );
        }
        !differs
    } else {
        true
    };

    Ok(VoxelRoiImportSpec {
        geometry: VoxelGeometry {
            dimensions: loaded_label.dimensions,
            spacing: loaded_label.spacing,
            origin: loaded_label.origin,
            orientation: loaded_label.orientation,
        },
        start_visible: visible_voxel_overlay_count(world) < MAX_SIMULTANEOUS_ROI_OVERLAYS,
        geometry_matches_main,
    })
}

pub fn create_voxel_roi_from_label(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &mut World,
    loaded_label: &LoadedLabel,
) -> Result<hecs::Entity, String> {
    let import_spec = prepare_voxel_roi_import(world, loaded_label)?;
    create_voxel_roi_from_label_with_spec(device, queue, world, loaded_label, import_spec)
}

pub fn create_voxel_roi_from_label_with_spec(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &mut World,
    loaded_label: &LoadedLabel,
    import_spec: VoxelRoiImportSpec,
) -> Result<hecs::Entity, String> {
    let (new_texture, new_view, new_sampler) =
        crate::io::volume::create_texture_from_labelmap(device, queue, loaded_label);

    let placeholder_bg = world
        .query::<&GpuVolumeResources>()
        .with::<&MainVolumeTag>()
        .iter()
        .next()
        .map(|(_, res)| res.bind_group.clone())
        .ok_or_else(|| {
            "Cannot create a label ROI without an initialized main volume resource".to_string()
        })?;

    let next_roi_id = world.query::<&Roi>().iter().count() as u64 + 1;
    let entity = world.spawn((
        Roi::new_voxel(
            RoiId(next_roi_id),
            loaded_label.filename.clone(),
            import_spec.geometry,
            loaded_label.data.clone(),
            GpuVolumeResources {
                texture: new_texture,
                view: new_view,
                sampler: new_sampler,
                bind_group: placeholder_bg,
            },
        ),
        LayerSettings { opacity: 0.5 },
        RoiTag,
    ));

    if let Ok(mut roi) = world.get::<&mut Roi>(entity) {
        roi.metadata.is_visible = import_spec.start_visible;
    }

    Ok(entity)
}

pub fn create_empty_contour_roi(
    world: &mut World,
    editor_entity: hecs::Entity,
    active_plane_family: PlaneFamily,
) -> Result<hecs::Entity, String> {
    if world.get::<&EditorState>(editor_entity).is_err() {
        return Err("Missing editor state; contour ROI was not created.".to_string());
    }

    let next_roi_id = world.query::<&Roi>().iter().count() as u64 + 1;
    let roi_name = format!("Contour ROI {}", next_roi_id);
    let entity = world.spawn((
        Roi::new_contour(
            RoiId(next_roi_id),
            roi_name,
            ContourData {
                active_plane_family,
                slices: Vec::new(),
            },
        ),
        LayerSettings { opacity: 0.5 },
        RoiTag,
    ));

    let mut editor = world
        .get::<&mut EditorState>(editor_entity)
        .map_err(|_| "Missing editor state; contour ROI was not created.".to_string())?;
    editor.active_roi = Some(entity);
    Ok(entity)
}

pub fn create_empty_mesh_roi(
    world: &mut World,
    editor_entity: hecs::Entity,
) -> Result<hecs::Entity, String> {
    if world.get::<&EditorState>(editor_entity).is_err() {
        return Err("Missing editor state; mesh ROI was not created.".to_string());
    }

    let next_roi_id = world.query::<&Roi>().iter().count() as u64 + 1;
    let roi_name = format!("Mesh ROI {}", next_roi_id);
    let entity = world.spawn((
        Roi::new_mesh(
            RoiId(next_roi_id),
            roi_name,
            MeshData {
                vertices: Vec::new(),
                faces: Vec::new(),
            },
        ),
        LayerSettings { opacity: 0.5 },
        RoiTag,
    ));

    let mut editor = world
        .get::<&mut EditorState>(editor_entity)
        .map_err(|_| "Missing editor state; mesh ROI was not created.".to_string())?;
    editor.active_roi = Some(entity);
    Ok(entity)
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

pub fn translate_mesh_data(
    world: &mut World,
    roi_entity: hecs::Entity,
    delta_world_mm: [f32; 3],
) -> Result<(), MeshMutationError> {
    if delta_world_mm.iter().any(|value| !value.is_finite()) {
        return Err(MeshMutationError::InvalidDelta);
    }
    let mut mesh = world
        .get::<&Roi>(roi_entity)
        .map_err(|_| MeshMutationError::MissingRoi)?
        .mesh_data()
        .cloned()
        .ok_or(MeshMutationError::NotMeshRoi)?;
    for vertex in &mut mesh.vertices {
        for (coordinate, delta) in vertex.world_mm.iter_mut().zip(delta_world_mm) {
            *coordinate += delta;
        }
    }
    replace_mesh_data(world, roi_entity, mesh)
}

pub fn begin_mesh_translation_preview(
    world: &mut World,
    editor_entity: hecs::Entity,
    roi_entity: hecs::Entity,
    delta_world_mm: [f32; 3],
) -> Result<u64, MeshMutationError> {
    if delta_world_mm.iter().any(|value| !value.is_finite()) {
        return Err(MeshMutationError::InvalidDelta);
    }
    let mut mesh = world
        .get::<&Roi>(roi_entity)
        .map_err(|_| MeshMutationError::MissingRoi)?
        .mesh_data()
        .cloned()
        .ok_or(MeshMutationError::NotMeshRoi)?;
    for vertex in &mut mesh.vertices {
        for (coordinate, delta) in vertex.world_mm.iter_mut().zip(delta_world_mm) {
            *coordinate += delta;
        }
    }
    let mut editor = world
        .get::<&mut EditorState>(editor_entity)
        .map_err(|_| MeshMutationError::MissingEditorState)?;
    editor.mesh_edit_preview = Some(MeshEditPreview {
        roi_entity,
        mesh_data: mesh,
    });
    drop(editor);
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
        .mesh_edit_preview
        .take()
        .ok_or(MeshMutationError::MissingPreview)?;
    let result =
        replace_mesh_data_with_history(world, editor_entity, preview.roi_entity, preview.mesh_data);
    if let Ok(mut roi) = world.get::<&mut Roi>(preview.roi_entity) {
        roi.end_preview();
    }
    result
}

pub fn cancel_mesh_edit_preview(
    world: &mut World,
    editor_entity: hecs::Entity,
) -> Result<(), MeshMutationError> {
    let preview = world
        .get::<&mut EditorState>(editor_entity)
        .map_err(|_| MeshMutationError::MissingEditorState)?
        .mesh_edit_preview
        .take()
        .ok_or(MeshMutationError::MissingPreview)?;
    if let Ok(mut roi) = world.get::<&mut Roi>(preview.roi_entity) {
        roi.end_preview();
    }
    Ok(())
}

pub fn request_rebuild_voxel_cache_from_mesh(
    world: &mut World,
    roi_entity: hecs::Entity,
) -> Result<(), MeshDerivedRebuildError> {
    let roi = world
        .get::<&Roi>(roi_entity)
        .map_err(|_| MeshDerivedRebuildError::MissingRoi)?;
    if !matches!(&roi.authoritative_data, RoiAuthoritativeData::Mesh(_)) {
        return Err(MeshDerivedRebuildError::NotMeshRoi);
    }
    drop(roi);
    if main_volume_voxel_geometry(world).is_none() {
        return Err(MeshDerivedRebuildError::MissingTargetGeometry);
    }
    let mut roi = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| MeshDerivedRebuildError::MissingRoi)?;
    roi.mark_cache_dirty(RoiCacheKind::Voxel);
    roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
    Ok(())
}

pub fn request_rebuild_contour_cache_from_mesh(
    world: &mut World,
    roi_entity: hecs::Entity,
) -> Result<(), MeshDerivedRebuildError> {
    let mut roi = world
        .get::<&mut Roi>(roi_entity)
        .map_err(|_| MeshDerivedRebuildError::MissingRoi)?;
    if !matches!(&roi.authoritative_data, RoiAuthoritativeData::Mesh(_)) {
        return Err(MeshDerivedRebuildError::NotMeshRoi);
    }
    if roi.voxel_cache().is_none() || !roi.is_cache_current(RoiCacheKind::Voxel) {
        return Err(MeshDerivedRebuildError::MissingTargetGeometry);
    }
    roi.mark_cache_dirty(RoiCacheKind::Contour);
    roi.mark_all_contour_view_caches_stale();
    Ok(())
}

pub fn create_contour_roi_from_voxel_roi(
    world: &mut World,
    source_roi: hecs::Entity,
    family: PlaneFamily,
) -> Result<hecs::Entity, VoxelContourCreationError> {
    let source_voxel = {
        let roi = world
            .get::<&Roi>(source_roi)
            .map_err(|_| VoxelContourCreationError::MissingRoi)?;
        match &roi.authoritative_data {
            RoiAuthoritativeData::Voxel(voxel) => voxel.clone(),
            RoiAuthoritativeData::Contour(_) | RoiAuthoritativeData::Mesh(_) => {
                return Err(VoxelContourCreationError::NotVoxelRoi);
            }
        }
    };

    let extracted = extract_contours_from_voxel_data(&source_voxel, family)
        .map_err(VoxelContourCreationError::ExtractionFailed)?;

    let next_roi_id = world.query::<&Roi>().iter().count() as u64 + 1;
    let source_name = world
        .get::<&Roi>(source_roi)
        .ok()
        .map(|roi| roi.metadata.name.clone())
        .unwrap_or_else(|| "Voxel ROI".to_string());
    let new_name = format!("{source_name} ({} Contour)", plane_family_label(family));

    let entity = world.spawn((
        Roi::new_contour(RoiId(next_roi_id), new_name, extracted),
        LayerSettings { opacity: 0.5 },
        RoiTag,
    ));
    if let Ok(mut roi) = world.get::<&mut Roi>(entity) {
        // Preserve source voxel geometry as the initial contour reference frame.
        // This keeps extracted contour projection/edit mapping aligned before any
        // contour->voxel rebuild retargets caches to main-volume geometry.
        roi.session_caches.voxel = Some(VoxelCache {
            data: source_voxel,
            gpu_resources: None,
        });
        roi.dirty_state.voxel_cache_dirty = false;
        roi.dirty_state.generations.voxel = roi.dirty_state.generations.authoritative;
    }
    Ok(entity)
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

pub fn create_mesh_roi_from_voxel_roi(
    world: &mut World,
    source_roi: hecs::Entity,
) -> Result<hecs::Entity, VoxelMeshCreationError> {
    let source_voxel =
        voxel_data_for_display_surface_extraction(world, source_roi).map_err(|err| match err {
            DisplayVoxelSourceError::MissingRoi => VoxelMeshCreationError::MissingRoi,
            DisplayVoxelSourceError::NotVoxelRoi => VoxelMeshCreationError::NotVoxelRoi,
            DisplayVoxelSourceError::MissingMainVolume => VoxelMeshCreationError::MissingMainVolume,
        })?;

    let source_has_occupancy = source_voxel.raw_data.iter().any(|value| *value != 0);
    let extracted = extract_mesh_from_voxel_data(&source_voxel)
        .map_err(VoxelMeshCreationError::ExtractionFailed)?;
    if source_has_occupancy && (extracted.vertices.is_empty() || extracted.faces.is_empty()) {
        return Err(VoxelMeshCreationError::EmptyMeshFromNonEmptySource);
    }

    let next_roi_id = world.query::<&Roi>().iter().count() as u64 + 1;
    let source_name = world
        .get::<&Roi>(source_roi)
        .ok()
        .map(|roi| roi.metadata.name.clone())
        .unwrap_or_else(|| "Voxel ROI".to_string());
    let entity = world.spawn((
        Roi::new_mesh(
            RoiId(next_roi_id),
            format!("{source_name} (Mesh)"),
            extracted,
        ),
        LayerSettings { opacity: 0.5 },
        RoiTag,
    ));
    if let Ok(mut roi) = world.get::<&mut Roi>(entity) {
        // Preserve source voxel geometry as extraction/provenance context.
        // Rendering projects mesh world-mm vertices through main display volume geometry.
        roi.session_caches.voxel = Some(VoxelCache {
            data: source_voxel,
            gpu_resources: None,
        });
    }
    Ok(entity)
}

pub fn voxel_data_for_display_surface_extraction(
    world: &World,
    source_roi: hecs::Entity,
) -> Result<VoxelData, DisplayVoxelSourceError> {
    let source_voxel = {
        let roi = world
            .get::<&Roi>(source_roi)
            .map_err(|_| DisplayVoxelSourceError::MissingRoi)?;
        match &roi.authoritative_data {
            RoiAuthoritativeData::Voxel(voxel) => voxel.clone(),
            RoiAuthoritativeData::Contour(_) | RoiAuthoritativeData::Mesh(_) => {
                return Err(DisplayVoxelSourceError::NotVoxelRoi);
            }
        }
    };

    let _ = main_volume_geometry(world).ok_or(DisplayVoxelSourceError::MissingMainVolume)?;
    Ok(source_voxel)
}

pub fn create_mesh_roi_from_contour_roi(
    world: &mut World,
    source_roi: hecs::Entity,
) -> Result<hecs::Entity, ContourMeshCreationError> {
    let source_voxel = {
        let roi = world
            .get::<&Roi>(source_roi)
            .map_err(|_| ContourMeshCreationError::MissingRoi)?;
        if !matches!(&roi.authoritative_data, RoiAuthoritativeData::Contour(_)) {
            return Err(ContourMeshCreationError::NotContourRoi);
        }
        let Some(cache) = roi.voxel_cache() else {
            return Err(ContourMeshCreationError::MissingCurrentVoxelCache);
        };
        if !roi.is_cache_current(RoiCacheKind::Voxel) {
            return Err(ContourMeshCreationError::MissingCurrentVoxelCache);
        }
        cache.data.clone()
    };

    let extracted = extract_mesh_from_voxel_data(&source_voxel)
        .map_err(ContourMeshCreationError::ExtractionFailed)?;

    let next_roi_id = world.query::<&Roi>().iter().count() as u64 + 1;
    let source_name = world
        .get::<&Roi>(source_roi)
        .ok()
        .map(|roi| roi.metadata.name.clone())
        .unwrap_or_else(|| "Contour ROI".to_string());
    let entity = world.spawn((
        Roi::new_mesh(
            RoiId(next_roi_id),
            format!("{source_name} (Mesh)"),
            extracted,
        ),
        LayerSettings { opacity: 0.5 },
        RoiTag,
    ));
    if let Ok(mut roi) = world.get::<&mut Roi>(entity) {
        // Preserve contour-derived voxel geometry as extraction/provenance context.
        // Rendering projects mesh world-mm vertices through main display volume geometry.
        roi.session_caches.voxel = Some(VoxelCache {
            data: source_voxel,
            gpu_resources: None,
        });
    }
    Ok(entity)
}

pub fn cache_status(
    world: &World,
    roi_entity: hecs::Entity,
    kind: RoiCacheKind,
) -> Option<RoiCacheStatus> {
    crate::app::roi::requests::cache_status(world, roi_entity, kind)
}

pub fn request_cache_rebuild(
    world: &mut World,
    roi_entity: hecs::Entity,
    kind: RoiCacheKind,
) -> Option<RoiJobKind> {
    let mut roi = world.get::<&mut Roi>(roi_entity).ok()?;
    roi.mark_cache_dirty(kind);
    let job_kind = cache_kind_to_job_kind(kind);
    roi.enqueue_rebuild(job_kind);
    Some(job_kind)
}

pub fn begin_next_job(world: &mut World, roi_entity: hecs::Entity) -> Option<RoiJobKind> {
    let mut roi = world.get::<&mut Roi>(roi_entity).ok()?;
    roi.start_queued_job()
}

pub fn complete_cache_rebuild(
    world: &mut World,
    roi_entity: hecs::Entity,
    kind: RoiCacheKind,
) -> bool {
    let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) else {
        return false;
    };
    roi.finish_cache_rebuild(kind);
    true
}

pub fn invalidate_contour_voxel_caches_for_main_volume_change(world: &mut World) {
    for (_, roi) in world.query_mut::<&mut Roi>() {
        if !matches!(roi.authoritative_data, RoiAuthoritativeData::Contour(_)) {
            continue;
        }

        // Derived contour voxel caches target the current main-volume grid, so they
        // must be invalidated whenever that reference grid changes.
        roi.session_caches.voxel = None;
        roi.finish_job(RoiJobKind::RebuildVoxelCache);
        roi.mark_cache_dirty(RoiCacheKind::Voxel);
        roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
    }
}

pub fn process_contour_voxel_rebuild_jobs(world: &mut World) {
    let _ = process_contour_voxel_rebuild_jobs_with_hook(world, |_world, _entity| {}, None, None);
}

pub fn process_contour_voxel_rebuild_jobs_with_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &mut World,
    resources: &BindGroupResources<'_>,
    active_roi: Option<hecs::Entity>,
) {
    let rebuilt_any = process_contour_voxel_rebuild_jobs_with_hook(
        world,
        |_world, _entity| {},
        Some((device, queue)),
        active_roi,
    );
    if rebuilt_any {
        recreate_scene_bind_groups(device, world, resources, active_roi);
    }
}

pub fn process_mesh_voxel_rebuild_jobs(world: &mut World) {
    let _ = process_mesh_voxel_rebuild_jobs_with_context(world, None, None);
}

pub fn process_mesh_voxel_rebuild_jobs_with_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &mut World,
    resources: &BindGroupResources<'_>,
    active_roi: Option<hecs::Entity>,
) {
    let rebuilt_any =
        process_mesh_voxel_rebuild_jobs_with_context(world, Some((device, queue)), active_roi);
    if rebuilt_any {
        recreate_scene_bind_groups(device, world, resources, active_roi);
    }
}

pub fn process_voxel_mesh_rebuild_jobs(world: &mut World) {
    const FRAME_JOB_BUDGET: Duration = Duration::from_millis(4);
    let frame_started_at = Instant::now();

    let running_entity = {
        let mut query = world.query::<&VoxelMeshRebuildWork>();
        query.iter().map(|(entity, _)| entity).next()
    };
    if let Some(entity) = running_entity {
        resume_voxel_mesh_rebuild_work(world, entity, frame_started_at, FRAME_JOB_BUDGET);
        return;
    }

    let entity = world.query::<&Roi>().iter().find_map(|(entity, roi)| {
        (!matches!(roi.authoritative_data, RoiAuthoritativeData::Mesh(_))
            && roi.job_state.running.is_none()
            && roi.has_queued_job(RoiJobKind::RebuildMeshCache)
            && roi.voxel_cache().is_some()
            && roi.is_cache_current(RoiCacheKind::Voxel))
        .then_some(entity)
    });
    let Some(entity) = entity else {
        return;
    };
    let (source_generation, voxel_data, base_chunks) = {
        let roi = world.get::<&Roi>(entity).expect("queued ROI must exist");
        (
            roi.dirty_state.generations.authoritative,
            roi.voxel_cache()
                .expect("current voxel cache must exist")
                .data
                .clone(),
            roi.mesh_cache().and_then(|cache| cache.chunks.clone()),
        )
    };
    if begin_next_job(world, entity) != Some(RoiJobKind::RebuildMeshCache) {
        return;
    }
    let started_at = Instant::now();
    let dirty_region = world
        .get::<&Roi>(entity)
        .ok()
        .and_then(|roi| roi.job_state.running_request)
        .map(|request| request.dirty_region)
        .unwrap_or(RoiDirtyRegion::Full);
    let rebuild_result = match (dirty_region, base_chunks) {
        (RoiDirtyRegion::VoxelAabb { min, max }, Some(chunks)) => {
            IncrementalChunkedMeshRebuild::begin_for_voxel_aabb(chunks, &voxel_data, min, max)
        }
        _ => IncrementalChunkedMeshRebuild::begin_full(&voxel_data, DEFAULT_MESH_CHUNK_SIZE),
    };
    let rebuild = match rebuild_result {
        Ok(rebuild) => rebuild,
        Err(error) => {
            fail_voxel_mesh_rebuild(world, entity, error);
            return;
        }
    };
    if world
        .insert_one(
            entity,
            VoxelMeshRebuildWork {
                source_generation,
                voxel_data,
                rebuild,
                started_at,
            },
        )
        .is_err()
    {
        if let Ok(mut roi) = world.get::<&mut Roi>(entity) {
            roi.finish_job(RoiJobKind::RebuildMeshCache);
            roi.job_metrics.failed_count = roi.job_metrics.failed_count.saturating_add(1);
        }
        return;
    }
    resume_voxel_mesh_rebuild_work(world, entity, frame_started_at, FRAME_JOB_BUDGET);
}

fn resume_voxel_mesh_rebuild_work(
    world: &mut World,
    entity: hecs::Entity,
    frame_started_at: Instant,
    frame_budget: Duration,
) {
    let Ok(mut work) = world.remove_one::<VoxelMeshRebuildWork>(entity) else {
        return;
    };
    let is_current = world.get::<&Roi>(entity).is_ok_and(|roi| {
        roi.dirty_state.generations.authoritative == work.source_generation
            && roi.is_cache_current(RoiCacheKind::Voxel)
            && roi.job_state.running_request.is_some_and(|request| {
                request.kind == RoiJobKind::RebuildMeshCache
                    && request.source_generation == work.source_generation
            })
    });
    if !is_current {
        record_job_discarded(world, entity, work.started_at.elapsed());
        if let Ok(mut roi) = world.get::<&mut Roi>(entity) {
            roi.finish_job(RoiJobKind::RebuildMeshCache);
        }
        return;
    }

    let mut completed = work.rebuild.is_complete();
    while !completed && frame_started_at.elapsed() < frame_budget {
        completed = match work.rebuild.step(&work.voxel_data) {
            Ok(done) => done,
            Err(error) => {
                fail_voxel_mesh_rebuild(world, entity, error);
                return;
            }
        };
    }
    if !completed {
        if world.insert_one(entity, work).is_err() {
            if let Ok(mut roi) = world.get::<&mut Roi>(entity) {
                roi.finish_job(RoiJobKind::RebuildMeshCache);
                roi.job_metrics.failed_count = roi.job_metrics.failed_count.saturating_add(1);
            }
        }
        return;
    }

    let duration = work.started_at.elapsed();
    let Some(chunked_mesh) = work.rebuild.into_result() else {
        return;
    };
    let mesh_data = chunked_mesh.merged_mesh();
    let Ok(mut roi) = world.get::<&mut Roi>(entity) else {
        return;
    };
    if roi.dirty_state.generations.authoritative != work.source_generation {
        roi.job_metrics.discarded_count = roi.job_metrics.discarded_count.saturating_add(1);
        roi.finish_job(RoiJobKind::RebuildMeshCache);
        return;
    }
    roi.session_caches.mesh = Some(MeshCache {
        data: mesh_data,
        chunks: Some(chunked_mesh),
    });
    roi.finish_cache_rebuild(RoiCacheKind::Mesh);
    roi.job_metrics.completed_count = roi.job_metrics.completed_count.saturating_add(1);
    roi.job_metrics.last_duration_ms = duration.as_secs_f32() * 1000.0;
}

fn fail_voxel_mesh_rebuild(
    world: &mut World,
    entity: hecs::Entity,
    error: VoxelMeshExtractionError,
) {
    log::warn!("Voxel mesh rebuild failed for ROI {entity:?}: {error:?}");
    if let Ok(mut roi) = world.get::<&mut Roi>(entity) {
        roi.finish_job(RoiJobKind::RebuildMeshCache);
        roi.mark_cache_dirty(RoiCacheKind::Mesh);
        roi.job_metrics.failed_count = roi.job_metrics.failed_count.saturating_add(1);
    }
}

fn process_mesh_voxel_rebuild_jobs_with_context(
    world: &mut World,
    upload_context: Option<(&wgpu::Device, &wgpu::Queue)>,
    preferred_roi: Option<hecs::Entity>,
) -> bool {
    let mut entities = world
        .query::<&Roi>()
        .iter()
        .filter_map(|(entity, roi)| {
            (matches!(roi.authoritative_data, RoiAuthoritativeData::Mesh(_))
                && roi.job_state.running.is_none()
                && roi.has_queued_job(RoiJobKind::RebuildVoxelCache))
            .then_some(entity)
        })
        .collect::<Vec<_>>();
    if let Some(preferred) = preferred_roi {
        if let Some(index) = entities.iter().position(|entity| *entity == preferred) {
            entities.swap(0, index);
        }
    }
    entities
        .into_iter()
        .take(1)
        .any(|entity| process_mesh_voxel_rebuild_for_entity(world, entity, upload_context))
}

fn process_mesh_voxel_rebuild_for_entity(
    world: &mut World,
    roi_entity: hecs::Entity,
    upload_context: Option<(&wgpu::Device, &wgpu::Queue)>,
) -> bool {
    let (source_generation, mesh) = {
        let Ok(roi) = world.get::<&Roi>(roi_entity) else {
            return false;
        };
        let RoiAuthoritativeData::Mesh(mesh) = &roi.authoritative_data else {
            return false;
        };
        (roi.dirty_state.generations.authoritative, mesh.clone())
    };
    if begin_next_job(world, roi_entity) != Some(RoiJobKind::RebuildVoxelCache) {
        return false;
    }
    let started_at = Instant::now();
    let Some(target_geometry) = main_volume_voxel_geometry(world) else {
        fail_mesh_voxel_rebuild(world, roi_entity, "main_volume_geometry_missing");
        return false;
    };
    let voxel_data = match voxelize_mesh_to_voxel_data(&mesh, target_geometry) {
        Ok(data) => data,
        Err(error) => {
            log::warn!("Mesh voxel rebuild failed for ROI {roi_entity:?}: {error:?}");
            fail_mesh_voxel_rebuild(world, roi_entity, "mesh_voxelization_failed");
            return false;
        }
    };

    if world
        .get::<&Roi>(roi_entity)
        .is_ok_and(|roi| roi.dirty_state.generations.authoritative != source_generation)
    {
        record_job_discarded(world, roi_entity, started_at.elapsed());
        requeue_mesh_voxel_rebuild(world, roi_entity);
        return false;
    }

    let gpu_resources = if let Some((device, queue)) = upload_context {
        let Some(placeholder_bg) = main_volume_bind_group(world) else {
            fail_mesh_voxel_rebuild(world, roi_entity, "main_volume_bind_group_missing");
            return false;
        };
        let (texture, view, sampler) =
            match crate::io::volume::create_texture_from_voxel_data(device, queue, &voxel_data) {
                Ok(resources) => resources,
                Err(error) => {
                    log::warn!("Mesh voxel GPU upload failed for ROI {roi_entity:?}: {error}");
                    fail_mesh_voxel_rebuild(world, roi_entity, "mesh_voxel_gpu_upload_failed");
                    return false;
                }
            };
        Some(GpuVolumeResources {
            texture,
            view,
            sampler,
            bind_group: placeholder_bg,
        })
    } else {
        None
    };

    let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) else {
        return false;
    };
    roi.session_caches.voxel = Some(VoxelCache {
        data: voxel_data,
        gpu_resources,
    });
    roi.finish_cache_rebuild(RoiCacheKind::Voxel);
    roi.mark_cache_dirty(RoiCacheKind::Contour);
    roi.mark_all_contour_view_caches_stale();
    roi.job_metrics.completed_count = roi.job_metrics.completed_count.saturating_add(1);
    roi.job_metrics.last_duration_ms = started_at.elapsed().as_secs_f32() * 1000.0;
    drop(roi);
    set_runtime_status_message(world, "Mesh voxel cache rebuilt.".to_string());
    true
}

fn requeue_mesh_voxel_rebuild(world: &mut World, roi_entity: hecs::Entity) {
    if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
        roi.finish_job(RoiJobKind::RebuildVoxelCache);
        roi.mark_cache_dirty(RoiCacheKind::Voxel);
        roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
    }
}

fn fail_mesh_voxel_rebuild(world: &mut World, roi_entity: hecs::Entity, reason: &'static str) {
    if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
        roi.finish_job(RoiJobKind::RebuildVoxelCache);
        roi.mark_cache_dirty(RoiCacheKind::Voxel);
        roi.job_metrics.failed_count = roi.job_metrics.failed_count.saturating_add(1);
    }
    set_runtime_status_message(world, format!("Mesh voxel rebuild failed: {reason}."));
}

fn process_contour_voxel_rebuild_jobs_with_hook(
    world: &mut World,
    mut before_commit: impl FnMut(&mut World, hecs::Entity),
    upload_context: Option<(&wgpu::Device, &wgpu::Queue)>,
    preferred_roi: Option<hecs::Entity>,
) -> bool {
    const MAX_JOBS_PER_FRAME: usize = 1;
    const FRAME_JOB_BUDGET: Duration = Duration::from_millis(4);
    let frame_started_at = Instant::now();
    let mut rebuilt_any = false;

    let mut running_preview_entities = world
        .query::<&ContourPreviewMeshWork>()
        .iter()
        .map(|(entity, _)| entity)
        .collect::<Vec<_>>();
    prioritize_entity(&mut running_preview_entities, preferred_roi);
    if let Some(entity) = running_preview_entities.first().copied() {
        rebuilt_any |=
            resume_contour_preview_mesh_work(world, entity, frame_started_at, FRAME_JOB_BUDGET);
    }

    if frame_started_at.elapsed() >= FRAME_JOB_BUDGET {
        return rebuilt_any;
    }

    let mut rebuild_entities = Vec::new();
    for (entity, roi) in world.query::<&Roi>().iter() {
        if !matches!(roi.authoritative_data, RoiAuthoritativeData::Contour(_)) {
            continue;
        }
        if roi.job_state.running.is_none() && roi.has_queued_job(RoiJobKind::RebuildVoxelCache) {
            rebuild_entities.push(entity);
        }
    }

    prioritize_entity(&mut rebuild_entities, preferred_roi);

    for entity in rebuild_entities.into_iter().take(MAX_JOBS_PER_FRAME) {
        if frame_started_at.elapsed() >= FRAME_JOB_BUDGET {
            break;
        }
        rebuilt_any |= process_contour_voxel_rebuild_for_entity(
            world,
            entity,
            &mut before_commit,
            upload_context,
        );
    }
    rebuilt_any
}

fn prioritize_entity(entities: &mut [hecs::Entity], preferred: Option<hecs::Entity>) {
    let Some(preferred) = preferred else {
        return;
    };
    if let Some(index) = entities.iter().position(|entity| *entity == preferred) {
        entities.swap(0, index);
    }
}

fn resume_contour_preview_mesh_work(
    world: &mut World,
    roi_entity: hecs::Entity,
    frame_started_at: Instant,
    frame_budget: Duration,
) -> bool {
    let Ok(mut work) = world.remove_one::<ContourPreviewMeshWork>(roi_entity) else {
        return false;
    };
    let is_current = world.get::<&Roi>(roi_entity).is_ok_and(|roi| {
        roi.preview_state.active
            && roi.preview_state.revision == work.preview_revision
            && roi.dirty_state.generations.authoritative == work.source_generation
            && roi.job_state.running_request.is_some_and(|request| {
                request.preview_revision == Some(work.preview_revision)
                    && request.source_generation == work.source_generation
            })
    });
    if !is_current {
        record_job_discarded(world, roi_entity, work.started_at.elapsed());
        if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
            roi.finish_job(RoiJobKind::RebuildVoxelCache);
        }
        return false;
    }

    let mut completed = work.rebuild.is_complete();
    while !completed {
        match work.rebuild.step(&work.voxel_data) {
            Ok(done) => completed = done,
            Err(error) => {
                log::warn!("Contour preview mesh extraction failed: {error:?}");
                if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
                    roi.finish_job(RoiJobKind::RebuildVoxelCache);
                    roi.job_metrics.failed_count = roi.job_metrics.failed_count.saturating_add(1);
                }
                return false;
            }
        }
        if !completed && frame_started_at.elapsed() >= frame_budget {
            break;
        }
    }

    if !completed {
        if world.insert_one(roi_entity, work).is_err() {
            if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
                roi.finish_job(RoiJobKind::RebuildVoxelCache);
                roi.job_metrics.failed_count = roi.job_metrics.failed_count.saturating_add(1);
            }
        }
        return false;
    }

    let duration = work.started_at.elapsed();
    let source_generation = work.source_generation;
    let preview_revision = work.preview_revision;
    let preview_aabb = work.preview_aabb;
    let Some(chunked_mesh) = work.rebuild.into_result() else {
        return false;
    };
    let mesh_data = chunked_mesh.merged_mesh();
    let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) else {
        return false;
    };
    if !roi.preview_state.active
        || roi.preview_state.revision != preview_revision
        || roi.dirty_state.generations.authoritative != source_generation
    {
        roi.job_metrics.discarded_count = roi.job_metrics.discarded_count.saturating_add(1);
        roi.finish_job(RoiJobKind::RebuildVoxelCache);
        return false;
    }
    roi.session_caches.preview_mesh = Some(PreviewMeshCache {
        data: mesh_data,
        chunks: Some(chunked_mesh),
        dirty_voxel_aabb: preview_aabb,
        source_generation,
        preview_revision,
    });
    roi.finish_job(RoiJobKind::RebuildVoxelCache);
    roi.job_metrics.completed_count = roi.job_metrics.completed_count.saturating_add(1);
    roi.job_metrics.last_duration_ms = duration.as_secs_f32() * 1000.0;
    true
}

fn process_contour_voxel_rebuild_for_entity(
    world: &mut World,
    roi_entity: hecs::Entity,
    before_commit: &mut impl FnMut(&mut World, hecs::Entity),
    upload_context: Option<(&wgpu::Device, &wgpu::Queue)>,
) -> bool {
    let authoritative_generation = {
        let Ok(roi) = world.get::<&Roi>(roi_entity) else {
            return false;
        };
        let RoiAuthoritativeData::Contour(_) = &roi.authoritative_data else {
            return false;
        };
        roi.dirty_state.generations.authoritative
    };

    if begin_next_job(world, roi_entity) != Some(RoiJobKind::RebuildVoxelCache) {
        return false;
    }
    let started_at = Instant::now();
    let running_request = world
        .get::<&Roi>(roi_entity)
        .ok()
        .and_then(|roi| roi.job_state.running_request);
    let preview_revision = running_request.and_then(|request| request.preview_revision);
    let dirty_slice_key = running_request
        .and_then(|request| match request.dirty_region {
            RoiDirtyRegion::ContourSlice(key) => Some(key),
            _ => None,
        })
        .filter(|_| {
            world.get::<&Roi>(roi_entity).is_ok_and(|roi| {
                roi.contour_data()
                    .is_some_and(|contour| contour.active_plane_family != PlaneFamily::Oblique)
            })
        });
    let previous_dirty_contour = dirty_slice_key.and_then(|key| {
        world.get::<&Roi>(roi_entity).ok().and_then(|roi| {
            let mut contour = roi.contour_data()?.clone();
            contour
                .slices
                .retain(|slice| ContourSliceKey::from_plane(slice.plane) == key);
            Some(contour)
        })
    });
    let mut contour_data = if let Some(revision) = preview_revision {
        let preview = world
            .query::<&EditorState>()
            .iter()
            .find_map(|(_, editor)| {
                editor
                    .contour_move_preview
                    .as_ref()
                    .filter(|preview| preview.roi_entity == roi_entity)
                    .map(|preview| preview.contour_data.clone())
            });
        let is_current_preview = world.get::<&Roi>(roi_entity).is_ok_and(|roi| {
            roi.preview_state.active
                && roi.preview_state.revision == revision
                && roi.dirty_state.generations.authoritative == authoritative_generation
        });
        if !is_current_preview || preview.is_none() {
            record_job_discarded(world, roi_entity, started_at.elapsed());
            if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
                roi.finish_job(RoiJobKind::RebuildVoxelCache);
            }
            return false;
        }
        preview.expect("preview presence checked")
    } else {
        let Ok(roi) = world.get::<&Roi>(roi_entity) else {
            return false;
        };
        let RoiAuthoritativeData::Contour(contour_data) = &roi.authoritative_data else {
            return false;
        };
        contour_data.clone()
    };
    if let Some(slice_key) = dirty_slice_key {
        contour_data
            .slices
            .retain(|slice| ContourSliceKey::from_plane(slice.plane) == slice_key);
    }

    let Some(target_geometry) = main_volume_voxel_geometry(world) else {
        log::warn!(
            "Skipping contour voxel rebuild for ROI {:?}: missing main volume geometry",
            roi_entity
        );
        set_runtime_status_message(
            world,
            "Contour voxel rebuild failed: main volume geometry is unavailable.".to_string(),
        );
        fail_contour_voxel_rebuild(world, roi_entity);
        return false;
    };

    let base_slice_voxel = dirty_slice_key.and_then(|_| {
        world.get::<&Roi>(roi_entity).ok().and_then(|roi| {
            let cache_generation = roi.dirty_state.generations.voxel;
            let authoritative_generation = roi.dirty_state.generations.authoritative;
            if cache_generation == authoritative_generation
                || cache_generation.saturating_add(1) == authoritative_generation
            {
                roi.voxel_cache().map(|cache| cache.data.clone())
            } else {
                None
            }
        })
    });
    let raster_result = if let Some(base) = base_slice_voxel.as_ref() {
        rasterize_contour_preview_slices_to_voxel_data(&contour_data, base)
    } else {
        rasterize_contours_to_voxel_data(&contour_data, target_geometry)
    };
    let voxel_data = match raster_result {
        Ok(voxel_data) => voxel_data,
        Err(err) => {
            log::warn!(
                "Skipping contour voxel rebuild for ROI {:?}: rasterization failed: {:?}",
                roi_entity,
                err
            );
            set_runtime_status_message(
                world,
                format!("Contour voxel rebuild failed: rasterization error ({err:?})."),
            );
            fail_contour_voxel_rebuild(world, roi_entity);
            return false;
        }
    };
    let committed_mesh_dirty_region = if preview_revision.is_none() {
        contour_slices_voxel_aabb(&contour_data, voxel_data.geometry)
            .map(|(min, max)| RoiDirtyRegion::VoxelAabb { min, max })
            .unwrap_or(RoiDirtyRegion::Full)
    } else {
        RoiDirtyRegion::Full
    };

    if let Some(revision) = preview_revision {
        let (base_chunks, prior_preview_aabb) = world
            .get::<&Roi>(roi_entity)
            .ok()
            .map(|roi| {
                let previous = roi
                    .session_caches
                    .preview_mesh
                    .as_ref()
                    .filter(|cache| cache.source_generation == authoritative_generation)
                    .map(|cache| (cache.chunks.clone(), cache.dirty_voxel_aabb));
                match previous {
                    Some((Some(chunks), aabb)) => (Some(chunks), aabb),
                    _ => (
                        roi.mesh_cache().and_then(|cache| cache.chunks.clone()),
                        None,
                    ),
                }
            })
            .unwrap_or((None, None));
        let raster_geometry = voxel_data.geometry;
        let preview_aabb = contour_geometry_voxel_aabb(&contour_data, raster_geometry);
        let previous_aabb = previous_dirty_contour
            .as_ref()
            .and_then(|contour| contour_geometry_voxel_aabb(contour, raster_geometry));
        let dirty_aabb = merge_voxel_aabbs(
            merge_voxel_aabbs(preview_aabb, previous_aabb),
            prior_preview_aabb,
        );
        let rebuild_result = if let (Some(chunks), Some((min, max))) = (base_chunks, dirty_aabb) {
            IncrementalChunkedMeshRebuild::begin_for_voxel_aabb(chunks, &voxel_data, min, max)
        } else {
            IncrementalChunkedMeshRebuild::begin_full(&voxel_data, DEFAULT_MESH_CHUNK_SIZE)
        };
        let rebuild = match rebuild_result {
            Ok(rebuild) => rebuild,
            Err(error) => {
                log::warn!("Contour preview mesh extraction failed: {error:?}");
                if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
                    roi.finish_job(RoiJobKind::RebuildVoxelCache);
                    roi.job_metrics.failed_count = roi.job_metrics.failed_count.saturating_add(1);
                }
                return false;
            }
        };
        let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) else {
            return false;
        };
        if !roi.preview_state.active
            || roi.preview_state.revision != revision
            || roi.dirty_state.generations.authoritative != authoritative_generation
        {
            roi.job_metrics.discarded_count = roi.job_metrics.discarded_count.saturating_add(1);
            roi.finish_job(RoiJobKind::RebuildVoxelCache);
            return false;
        }
        roi.session_caches.preview_voxel = Some(PreviewVoxelCache {
            data: voxel_data.clone(),
            source_generation: authoritative_generation,
            preview_revision: revision,
        });
        roi.mark_all_contour_view_caches_stale();
        drop(roi);
        let work = ContourPreviewMeshWork {
            source_generation: authoritative_generation,
            preview_revision: revision,
            preview_aabb,
            voxel_data,
            rebuild,
            started_at,
        };
        if world.insert_one(roi_entity, work).is_err() {
            if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
                roi.finish_job(RoiJobKind::RebuildVoxelCache);
                roi.job_metrics.failed_count = roi.job_metrics.failed_count.saturating_add(1);
            }
            return false;
        }
        return true;
    }

    before_commit(world, roi_entity);

    let is_stale = match world.get::<&Roi>(roi_entity) {
        Ok(roi) => roi.dirty_state.generations.authoritative != authoritative_generation,
        Err(_) => return false,
    };
    if is_stale {
        log::info!(
            "Discarding stale contour voxel rebuild for ROI {:?}: generation changed from {}",
            roi_entity,
            authoritative_generation,
        );
        record_job_discarded(world, roi_entity, started_at.elapsed());
        requeue_contour_voxel_rebuild(world, roi_entity);
        return false;
    }

    let gpu_resources = if let Some((device, queue)) = upload_context {
        let Some(placeholder_bg) = main_volume_bind_group(world) else {
            log::warn!(
                "Skipping contour voxel rebuild GPU upload for ROI {:?}: missing main volume bind group",
                roi_entity
            );
            requeue_contour_voxel_rebuild(world, roi_entity);
            set_runtime_status_message(
                world,
                "Contour voxel rebuild failed: main volume bind-group is unavailable.".to_string(),
            );
            fail_contour_voxel_rebuild(world, roi_entity);
            return false;
        };

        let (texture, view, sampler) =
            match crate::io::volume::create_texture_from_voxel_data(device, queue, &voxel_data) {
                Ok(gpu_tuple) => gpu_tuple,
                Err(err) => {
                    log::warn!(
                        "Skipping contour voxel rebuild for ROI {:?}: GPU upload failed: {}",
                        roi_entity,
                        err
                    );
                    set_runtime_status_message(
                        world,
                        format!("Contour voxel rebuild failed: GPU upload error ({err})."),
                    );
                    fail_contour_voxel_rebuild(world, roi_entity);
                    return false;
                }
            };

        Some(GpuVolumeResources {
            texture,
            view,
            sampler,
            bind_group: placeholder_bg,
        })
    } else {
        None
    };

    let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) else {
        return false;
    };

    roi.session_caches.voxel = Some(VoxelCache {
        data: voxel_data,
        gpu_resources,
    });
    roi.finish_cache_rebuild(RoiCacheKind::Voxel);
    roi.mark_cache_dirty(RoiCacheKind::Mesh);
    roi.enqueue_job(RoiJobRequest {
        kind: RoiJobKind::RebuildMeshCache,
        source_generation: authoritative_generation,
        preview_revision: None,
        priority: RoiJobPriority::VisibleCommitted,
        dirty_region: committed_mesh_dirty_region,
    });
    roi.job_metrics.completed_count = roi.job_metrics.completed_count.saturating_add(1);
    roi.job_metrics.last_duration_ms = started_at.elapsed().as_secs_f32() * 1000.0;
    drop(roi);
    set_runtime_status_message(world, "Contour voxel cache rebuilt.".to_string());
    true
}

fn requeue_contour_voxel_rebuild(world: &mut World, roi_entity: hecs::Entity) {
    if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
        roi.finish_job(RoiJobKind::RebuildVoxelCache);
        roi.mark_cache_dirty(RoiCacheKind::Voxel);
        roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
    }
}

fn fail_contour_voxel_rebuild(world: &mut World, roi_entity: hecs::Entity) {
    if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
        roi.finish_job(RoiJobKind::RebuildVoxelCache);
        roi.mark_cache_dirty(RoiCacheKind::Voxel);
        roi.job_metrics.failed_count = roi.job_metrics.failed_count.saturating_add(1);
    }
}

fn merge_voxel_aabbs(
    left: Option<([u32; 3], [u32; 3])>,
    right: Option<([u32; 3], [u32; 3])>,
) -> Option<([u32; 3], [u32; 3])> {
    match (left, right) {
        (Some((left_min, left_max)), Some((right_min, right_max))) => Some((
            std::array::from_fn(|axis| left_min[axis].min(right_min[axis])),
            std::array::from_fn(|axis| left_max[axis].max(right_max[axis])),
        )),
        (left, right) => left.or(right),
    }
}

fn record_job_discarded(world: &mut World, roi_entity: hecs::Entity, duration: Duration) {
    if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
        roi.job_metrics.discarded_count = roi.job_metrics.discarded_count.saturating_add(1);
        roi.job_metrics.last_duration_ms = duration.as_secs_f32() * 1000.0;
    }
}

fn main_volume_bind_group(world: &World) -> Option<wgpu::BindGroup> {
    let query = world.query::<&GpuVolumeResources>();
    let mut with_tag = query.with::<&MainVolumeTag>();
    with_tag
        .iter()
        .next()
        .map(|(_, res)| res.bind_group.clone())
}

fn set_runtime_status_message(world: &mut World, message: String) {
    if let Some((_, gui_state)) = world.query_mut::<&mut GuiState>().into_iter().next() {
        gui_state.status_message = Some(message);
    }
}

pub fn voxel_roi_stats(world: &World, roi_entity: hecs::Entity) -> Option<VoxelRoiStats> {
    roi_voxel_stats(world, roi_entity)
}

pub fn roi_voxel_stats(world: &World, roi_entity: hecs::Entity) -> Option<VoxelRoiStats> {
    let roi = world.get::<&Roi>(roi_entity).ok()?;
    let voxel_data = match &roi.authoritative_data {
        RoiAuthoritativeData::Contour(_) | RoiAuthoritativeData::Mesh(_) => {
            if !roi.is_cache_current(RoiCacheKind::Voxel) {
                return None;
            }
            &roi.voxel_cache()?.data
        }
        RoiAuthoritativeData::Voxel(voxel) => voxel,
    };

    let occupied_voxels = voxel_data
        .raw_data
        .iter()
        .filter(|value| **value != 0)
        .count() as u64;

    let volume_scale_mm3 = voxel_data.geometry.spacing[0]
        * voxel_data.geometry.spacing[1]
        * voxel_data.geometry.spacing[2];

    Some(VoxelRoiStats {
        occupied_voxels,
        volume_mm3: occupied_voxels as f32 * volume_scale_mm3,
    })
}

fn cache_kind_to_job_kind(kind: RoiCacheKind) -> RoiJobKind {
    match kind {
        RoiCacheKind::Voxel => RoiJobKind::RebuildVoxelCache,
        RoiCacheKind::Contour => RoiJobKind::RebuildContourCache,
        RoiCacheKind::Mesh => RoiJobKind::RebuildMeshCache,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::{orthogonal_plane_from_volume_uv, world_mm_to_voxel_index};

    fn spawn_test_roi(world: &mut World) -> hecs::Entity {
        world.spawn((Roi::new_voxel_with_cache(
            RoiId(1),
            "Test".to_string(),
            VoxelGeometry {
                dimensions: [4, 4, 4],
                spacing: [1.0, 1.0, 1.0],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            vec![1; 64],
            None,
        ),))
    }

    fn spawn_sparse_voxel_roi(world: &mut World) -> hecs::Entity {
        let mut raw = vec![0_u8; 64];
        raw[(2 * 4 + 1) * 4 + 1] = 1;
        world.spawn((Roi::new_voxel_with_cache(
            RoiId(2),
            "Sparse".to_string(),
            VoxelGeometry {
                dimensions: [4, 4, 4],
                spacing: [1.0, 1.0, 1.0],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            raw,
            None,
        ),))
    }

    fn spawn_main_volume(world: &mut World, spacing: [f32; 3], origin: [f32; 3]) {
        world.spawn((
            VolumeData {
                dimensions: [4, 4, 4],
                spacing,
                origin,
                intensities: vec![],
                intensity_range: [0.0, 1.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            MainVolumeTag,
        ));
    }

    fn test_plane_definition(family: PlaneFamily) -> PlaneDefinition {
        PlaneDefinition {
            family,
            origin_mm: [0.0, 0.0, 0.0],
            u_axis_mm: [1.0, 0.0, 0.0],
            v_axis_mm: [0.0, 1.0, 0.0],
            normal_mm: [0.0, 0.0, 1.0],
        }
    }

    fn spawn_test_contour_roi(
        world: &mut World,
        family: PlaneFamily,
        with_loops: bool,
    ) -> hecs::Entity {
        let slices = if with_loops {
            vec![ContourSlice {
                plane: test_plane_definition(family),
                loops: vec![ContourLoop {
                    points: vec![
                        ContourPoint {
                            local_mm: [0.0, 0.0],
                        },
                        ContourPoint {
                            local_mm: [1.0, 0.0],
                        },
                        ContourPoint {
                            local_mm: [0.0, 1.0],
                        },
                    ],
                    is_closed: true,
                }],
            }]
        } else {
            Vec::new()
        };

        world.spawn((Roi::new_contour(
            RoiId(100),
            "Contour".to_string(),
            ContourData {
                active_plane_family: family,
                slices,
            },
        ),))
    }

    fn seed_current_voxel_cache_for_contour_roi(world: &mut World, entity: hecs::Entity) {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        roi.session_caches.voxel = Some(VoxelCache {
            data: VoxelData {
                geometry: VoxelGeometry {
                    dimensions: [4, 4, 4],
                    spacing: [1.0, 1.0, 1.0],
                    origin: [0.0, 0.0, 0.0],
                    orientation: [0.0, 0.0, 0.0, 1.0],
                },
                raw_data: vec![0; 64],
            },
            gpu_resources: None,
        });
        roi.dirty_state.voxel_cache_dirty = false;
        roi.dirty_state.generations.voxel = roi.dirty_state.generations.authoritative;
    }

    fn square_contour_data_for_main_volume(world: &World, half_extent: f32) -> ContourData {
        let geometry = main_volume_voxel_geometry(world).expect("main volume geometry must exist");
        let plane = orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 0.5], geometry)
            .expect("axial plane should resolve");
        ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: vec![ContourSlice {
                plane,
                loops: vec![ContourLoop {
                    points: vec![
                        ContourPoint {
                            local_mm: [-half_extent, -half_extent],
                        },
                        ContourPoint {
                            local_mm: [half_extent, -half_extent],
                        },
                        ContourPoint {
                            local_mm: [half_extent, half_extent],
                        },
                        ContourPoint {
                            local_mm: [-half_extent, half_extent],
                        },
                    ],
                    is_closed: true,
                }],
            }],
        }
    }

    fn simple_mesh_data() -> MeshData {
        MeshData {
            vertices: vec![
                MeshVertex {
                    world_mm: [0.0, 0.0, 0.0],
                },
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
        }
    }

    fn closed_tetra_mesh_data() -> MeshData {
        MeshData {
            vertices: vec![
                MeshVertex {
                    world_mm: [0.0, 0.0, 0.0],
                },
                MeshVertex {
                    world_mm: [2.0, 0.0, 0.0],
                },
                MeshVertex {
                    world_mm: [0.0, 2.0, 0.0],
                },
                MeshVertex {
                    world_mm: [0.0, 0.0, 2.0],
                },
            ],
            faces: vec![
                MeshFace {
                    vertex_indices: [0, 2, 1],
                },
                MeshFace {
                    vertex_indices: [0, 1, 3],
                },
                MeshFace {
                    vertex_indices: [0, 3, 2],
                },
                MeshFace {
                    vertex_indices: [1, 2, 3],
                },
            ],
        }
    }

    #[test]
    fn test_request_cache_rebuild_marks_cache_dirty_and_queues_job() {
        let mut world = World::new();
        let entity = spawn_test_roi(&mut world);

        let job = request_cache_rebuild(&mut world, entity, RoiCacheKind::Contour);
        let status = cache_status(&world, entity, RoiCacheKind::Contour).unwrap();
        let roi = world.get::<&Roi>(entity).unwrap();

        assert_eq!(job, Some(RoiJobKind::RebuildContourCache));
        assert!(status.is_dirty);
        assert!(!status.is_current);
        assert_eq!(roi.job_state.queued, Some(RoiJobKind::RebuildContourCache));
    }

    #[test]
    fn test_begin_and_complete_job_update_runtime_status() {
        let mut world = World::new();
        let entity = spawn_test_roi(&mut world);

        request_cache_rebuild(&mut world, entity, RoiCacheKind::Voxel);
        assert_eq!(
            begin_next_job(&mut world, entity),
            Some(RoiJobKind::RebuildVoxelCache)
        );

        let status_before = cache_status(&world, entity, RoiCacheKind::Voxel).unwrap();
        assert!(status_before.is_dirty);

        assert!(complete_cache_rebuild(
            &mut world,
            entity,
            RoiCacheKind::Voxel
        ));

        let status_after = cache_status(&world, entity, RoiCacheKind::Voxel).unwrap();
        let roi = world.get::<&Roi>(entity).unwrap();
        assert!(!status_after.is_dirty);
        assert!(status_after.is_current);
        assert_eq!(roi.job_state.running, None);
    }

    #[test]
    fn test_voxel_roi_stats_use_nonzero_voxels_and_volume_spacing() {
        let mut world = World::new();
        spawn_main_volume(&mut world, [0.5, 0.5, 2.0], [0.0, 0.0, 0.0]);
        let entity = world.spawn((Roi::new_voxel_with_cache(
            RoiId(2),
            "Mask".to_string(),
            VoxelGeometry {
                dimensions: [2, 2, 2],
                spacing: [0.5, 0.5, 2.0],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            vec![0, 1, 2, 0, 0, 3, 4, 0],
            None,
        ),));

        let stats = voxel_roi_stats(&world, entity).unwrap();

        assert_eq!(stats.occupied_voxels, 4);
        assert!((stats.volume_mm3 - 2.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_roi_voxel_stats_contour_primary_returns_none_before_rebuild() {
        let mut world = World::new();
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, true);
        let stats = roi_voxel_stats(&world, entity);
        assert!(stats.is_none());
    }

    #[test]
    fn test_roi_voxel_stats_contour_primary_returns_derived_stats_after_rebuild() {
        let mut world = World::new();
        spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
        let replacement = square_contour_data_for_main_volume(&world, 1.4);
        replace_contour_data(&mut world, entity, replacement).unwrap();
        process_contour_voxel_rebuild_jobs(&mut world);

        let expected_occupied = {
            let roi = world.get::<&Roi>(entity).unwrap();
            roi.voxel_cache()
                .expect("expected derived voxel cache")
                .data
                .raw_data
                .iter()
                .filter(|v| **v != 0)
                .count() as u64
        };
        let stats = roi_voxel_stats(&world, entity).expect("expected derived stats");
        assert!(expected_occupied > 0);
        assert_eq!(stats.occupied_voxels, expected_occupied);
        assert!((stats.volume_mm3 - expected_occupied as f32).abs() < f32::EPSILON);
    }

    #[test]
    fn test_main_volume_voxel_geometry_reads_main_volume_fields() {
        let mut world = World::new();
        spawn_main_volume(&mut world, [0.25, 0.5, 2.0], [3.0, -1.5, 2.25]);

        let geometry = main_volume_voxel_geometry(&world).unwrap();

        assert_eq!(geometry.dimensions, [4, 4, 4]);
        assert_eq!(geometry.spacing, [0.25, 0.5, 2.0]);
        assert_eq!(geometry.origin, [3.0, -1.5, 2.25]);
        assert_eq!(geometry.orientation, [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn test_main_volume_geometry_returns_none_when_missing() {
        let world = World::new();
        assert!(main_volume_geometry(&world).is_none());
    }

    #[test]
    fn test_visible_voxel_overlay_count_ignores_non_renderable_rois() {
        let mut world = World::new();
        let first = spawn_test_roi(&mut world);
        let second = spawn_test_roi(&mut world);
        let contour = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);

        {
            let mut roi = world.get::<&mut Roi>(first).unwrap();
            roi.metadata.is_visible = true;
        }
        {
            let mut roi = world.get::<&mut Roi>(second).unwrap();
            roi.metadata.is_visible = true;
        }
        {
            let mut roi = world.get::<&mut Roi>(contour).unwrap();
            roi.metadata.is_visible = true;
        }

        assert_eq!(visible_voxel_overlay_count(&world), 0);
        assert!(renderable_voxel_overlay_rois(&world, Some(contour)).is_empty());
        assert!(can_enable_roi_visibility(&world, contour));
    }

    #[test]
    fn test_voxel_overlay_request_blocks_when_cpu_cache_is_current_but_gpu_missing() {
        let mut world = World::new();
        let entity = spawn_sparse_voxel_roi(&mut world);

        let status = request_voxel_overlay_state(&world, entity);

        assert_eq!(status.state, RepresentationRequestState::Blocked);
        assert_eq!(status.reason.as_deref(), Some("voxel_gpu_missing"));
    }

    #[test]
    fn test_prepare_voxel_roi_import_uses_label_geometry_without_main_volume() {
        let world = World::new();
        let loaded_label = LoadedLabel {
            dimensions: [2, 2, 2],
            spacing: [1.25, 1.5, 2.0],
            origin: [5.0, 6.0, 7.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
            data: vec![0; 8],
            filename: "Label".to_string(),
        };

        let import_spec = prepare_voxel_roi_import(&world, &loaded_label).unwrap();
        assert_eq!(import_spec.geometry.dimensions, [2, 2, 2]);
        assert_eq!(import_spec.geometry.spacing, [1.25, 1.5, 2.0]);
        assert_eq!(import_spec.geometry.origin, [5.0, 6.0, 7.0]);
        assert_eq!(import_spec.geometry.orientation, [0.0, 0.0, 0.0, 1.0]);
        assert!(import_spec.geometry_matches_main);
        assert!(import_spec.start_visible);
    }

    #[test]
    fn test_prepare_voxel_roi_import_preserves_label_geometry_even_when_main_volume_differs() {
        let mut world = World::new();
        spawn_main_volume(&mut world, [0.5, 0.5, 2.0], [10.0, 10.0, 10.0]);
        let loaded_label = LoadedLabel {
            dimensions: [3, 4, 5],
            spacing: [0.75, 0.8, 1.25],
            origin: [-2.0, 4.5, 6.0],
            orientation: [0.0, 0.0, 1.0, 0.0],
            data: vec![0; 60],
            filename: "Label".to_string(),
        };

        let import_spec = prepare_voxel_roi_import(&world, &loaded_label).unwrap();

        assert_eq!(import_spec.geometry.dimensions, [3, 4, 5]);
        assert_eq!(import_spec.geometry.spacing, [0.75, 0.8, 1.25]);
        assert_eq!(import_spec.geometry.origin, [-2.0, 4.5, 6.0]);
        assert_eq!(import_spec.geometry.orientation, [0.0, 0.0, 1.0, 0.0]);
        assert!(!import_spec.geometry_matches_main);
        assert!(import_spec.start_visible);
    }

    #[test]
    fn test_create_empty_contour_roi_creates_contour_primary_with_requested_plane_family() {
        let mut world = World::new();
        let editor = world.spawn((EditorState::default(),));

        let entity = create_empty_contour_roi(&mut world, editor, PlaneFamily::Coronal).unwrap();

        let roi = world.get::<&Roi>(entity).unwrap();
        assert_eq!(roi.primary_representation(), PrimaryRepresentation::Contour);
        let contour_data = roi.contour_data().expect("expected contour roi");
        assert_eq!(contour_data.active_plane_family, PlaneFamily::Coronal);
        assert!(contour_data.slices.is_empty());
    }

    #[test]
    fn test_create_empty_contour_roi_sets_active_roi_to_new_entity() {
        let mut world = World::new();
        let editor = world.spawn((EditorState::default(),));

        let entity = create_empty_contour_roi(&mut world, editor, PlaneFamily::Axial).unwrap();

        let editor_state = world.get::<&EditorState>(editor).unwrap();
        assert_eq!(editor_state.active_roi, Some(entity));
    }

    #[test]
    fn test_create_empty_mesh_roi_creates_mesh_primary_and_sets_active_roi() {
        let mut world = World::new();
        let editor = world.spawn((EditorState::default(),));

        let entity = create_empty_mesh_roi(&mut world, editor).unwrap();

        let roi = world.get::<&Roi>(entity).unwrap();
        assert_eq!(roi.primary_representation(), PrimaryRepresentation::Mesh);
        let mesh_data = roi.mesh_data().expect("expected mesh roi");
        assert!(mesh_data.vertices.is_empty());
        assert!(mesh_data.faces.is_empty());
        let editor_state = world.get::<&EditorState>(editor).unwrap();
        assert_eq!(editor_state.active_roi, Some(entity));
    }

    #[test]
    fn test_promote_voxel_to_contour_authority_preserves_roi_and_rebases_voxel_cache() {
        let mut world = World::new();
        let entity = spawn_sparse_voxel_roi(&mut world);
        let (roi_id, name, source_voxel) = {
            let roi = world.get::<&Roi>(entity).unwrap();
            (
                roi.metadata.roi_id,
                roi.metadata.name.clone(),
                roi.voxel_cache().unwrap().data.clone(),
            )
        };
        let roi_count_before = world.query::<&Roi>().iter().count();

        promote_voxel_roi_to_contour_authority(&mut world, entity, PlaneFamily::Axial).unwrap();

        assert_eq!(world.query::<&Roi>().iter().count(), roi_count_before);
        let roi = world.get::<&Roi>(entity).unwrap();
        assert_eq!(roi.metadata.roi_id, roi_id);
        assert_eq!(roi.metadata.name, name);
        assert_eq!(roi.primary_representation(), PrimaryRepresentation::Contour);
        assert_eq!(
            roi.contour_data().unwrap().active_plane_family,
            PlaneFamily::Axial
        );
        assert!(roi.contour_data().unwrap().has_loops());
        assert_eq!(roi.dirty_state.generations.authoritative, 2);
        assert_eq!(roi.voxel_cache().unwrap().data, source_voxel);
        assert!(roi.is_cache_current(RoiCacheKind::Voxel));
        assert_eq!(roi.job_state.queued, Some(RoiJobKind::RebuildMeshCache));
    }

    #[test]
    fn test_promote_voxel_to_contour_authority_rejects_locked_roi() {
        let mut world = World::new();
        let entity = spawn_sparse_voxel_roi(&mut world);
        world.get::<&mut Roi>(entity).unwrap().metadata.is_locked = true;

        let result = promote_voxel_roi_to_contour_authority(&mut world, entity, PlaneFamily::Axial);

        assert_eq!(result, Err(VoxelContourPromotionError::Locked));
        assert_eq!(
            world.get::<&Roi>(entity).unwrap().primary_representation(),
            PrimaryRepresentation::Voxel
        );
    }

    #[test]
    fn test_promote_current_mesh_cache_to_authority_preserves_roi_and_voxel_cache() {
        let mut world = World::new();
        let entity = spawn_sparse_voxel_roi(&mut world);
        let mesh = closed_tetra_mesh_data();
        {
            let mut roi = world.get::<&mut Roi>(entity).unwrap();
            roi.session_caches.mesh = Some(MeshCache {
                data: mesh.clone(),
                chunks: None,
            });
            roi.dirty_state.mesh_cache_dirty = false;
            roi.dirty_state.generations.mesh = roi.dirty_state.generations.authoritative;
        }
        let (roi_id, name) = {
            let roi = world.get::<&Roi>(entity).unwrap();
            (roi.metadata.roi_id, roi.metadata.name.clone())
        };
        let roi_count_before = world.query::<&Roi>().iter().count();

        promote_current_mesh_cache_to_authority(&mut world, entity).unwrap();

        assert_eq!(world.query::<&Roi>().iter().count(), roi_count_before);
        let roi = world.get::<&Roi>(entity).unwrap();
        assert_eq!(roi.metadata.roi_id, roi_id);
        assert_eq!(roi.metadata.name, name);
        assert_eq!(roi.primary_representation(), PrimaryRepresentation::Mesh);
        assert_eq!(roi.mesh_data(), Some(&mesh));
        assert_eq!(roi.dirty_state.generations.authoritative, 2);
        assert!(roi.is_cache_current(RoiCacheKind::Voxel));
        assert!(!roi.is_cache_dirty(RoiCacheKind::Mesh));
        assert!(roi.session_caches.mesh.is_none());
    }

    #[test]
    fn test_promote_current_mesh_cache_to_authority_rejects_stale_cache() {
        let mut world = World::new();
        let entity = spawn_sparse_voxel_roi(&mut world);
        {
            let mut roi = world.get::<&mut Roi>(entity).unwrap();
            roi.session_caches.mesh = Some(MeshCache {
                data: closed_tetra_mesh_data(),
                chunks: None,
            });
            roi.dirty_state.mesh_cache_dirty = true;
        }

        let result = promote_current_mesh_cache_to_authority(&mut world, entity);

        assert_eq!(
            result,
            Err(MeshAuthorityPromotionError::MeshCacheNotCurrent)
        );
        assert_eq!(
            world.get::<&Roi>(entity).unwrap().primary_representation(),
            PrimaryRepresentation::Voxel
        );
    }

    #[test]
    fn test_authority_roundtrip_voxel_contour_voxel_preserves_entity() {
        let mut world = World::new();
        let entity = spawn_sparse_voxel_roi(&mut world);
        let roi_id = world.get::<&Roi>(entity).unwrap().metadata.roi_id;

        promote_roi_to_contour_authority(&mut world, entity, PlaneFamily::Axial).unwrap();
        promote_current_voxel_cache_to_authority(&mut world, entity).unwrap();

        let roi = world.get::<&Roi>(entity).unwrap();
        assert_eq!(roi.metadata.roi_id, roi_id);
        assert_eq!(roi.primary_representation(), PrimaryRepresentation::Voxel);
        assert!(matches!(
            roi.authoritative_data,
            RoiAuthoritativeData::Voxel(_)
        ));
        assert!(roi.is_cache_current(RoiCacheKind::Voxel));
        assert_eq!(roi.dirty_state.generations.authoritative, 3);
    }

    #[test]
    fn test_authority_roundtrip_mesh_voxel_mesh_reuses_current_mesh() {
        let mut world = World::new();
        let entity = spawn_sparse_voxel_roi(&mut world);
        let mesh = closed_tetra_mesh_data();
        {
            let mut roi = world.get::<&mut Roi>(entity).unwrap();
            roi.session_caches.mesh = Some(MeshCache {
                data: mesh.clone(),
                chunks: None,
            });
            roi.dirty_state.mesh_cache_dirty = false;
            roi.dirty_state.generations.mesh = roi.dirty_state.generations.authoritative;
        }

        promote_current_mesh_cache_to_authority(&mut world, entity).unwrap();
        promote_current_voxel_cache_to_authority(&mut world, entity).unwrap();
        assert!(world
            .get::<&Roi>(entity)
            .unwrap()
            .is_cache_current(RoiCacheKind::Mesh));
        promote_current_mesh_cache_to_authority(&mut world, entity).unwrap();

        let roi = world.get::<&Roi>(entity).unwrap();
        assert_eq!(roi.primary_representation(), PrimaryRepresentation::Mesh);
        assert_eq!(roi.mesh_data(), Some(&mesh));
        assert_eq!(roi.dirty_state.generations.authoritative, 4);
    }

    #[test]
    fn test_replace_mesh_data_updates_authoritative_state_and_queues_voxel_rebuild() {
        let mut world = World::new();
        let entity = world.spawn((Roi::new_mesh(
            RoiId(300),
            "Mesh".to_string(),
            MeshData {
                vertices: Vec::new(),
                faces: Vec::new(),
            },
        ),));
        {
            let mut roi = world.get::<&mut Roi>(entity).unwrap();
            roi.dirty_state.authoritative_dirty = false;
            roi.dirty_state.voxel_cache_dirty = false;
            roi.dirty_state.contour_cache_dirty = false;
            roi.dirty_state.mesh_cache_dirty = false;
            roi.dirty_state.generations.voxel = roi.dirty_state.generations.authoritative;
            roi.dirty_state.generations.contour = roi.dirty_state.generations.authoritative;
            roi.dirty_state.generations.mesh = roi.dirty_state.generations.authoritative;
        }

        let replacement = simple_mesh_data();
        let result = replace_mesh_data(&mut world, entity, replacement.clone());
        assert_eq!(result, Ok(()));

        let roi = world.get::<&Roi>(entity).unwrap();
        assert_eq!(roi.mesh_data(), Some(&replacement));
        assert!(roi.dirty_state.authoritative_dirty);
        assert_eq!(roi.dirty_state.generations.authoritative, 2);
        assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert!(roi.is_cache_dirty(RoiCacheKind::Contour));
        assert!(!roi.is_cache_dirty(RoiCacheKind::Mesh));
        assert_eq!(roi.job_state.queued, Some(RoiJobKind::RebuildVoxelCache));
    }

    #[test]
    fn test_replace_mesh_data_rejects_non_mesh_roi() {
        let mut world = World::new();
        let entity = spawn_test_roi(&mut world);
        let result = replace_mesh_data(&mut world, entity, simple_mesh_data());
        assert_eq!(result, Err(MeshMutationError::NotMeshRoi));
    }

    #[test]
    fn test_translate_mesh_data_changes_authority_and_queues_voxel_rebuild() {
        let mut world = World::new();
        let original = closed_tetra_mesh_data();
        let entity = world.spawn((Roi::new_mesh(
            RoiId(302),
            "Translated".to_string(),
            original.clone(),
        ),));

        translate_mesh_data(&mut world, entity, [1.0, -2.0, 0.5]).unwrap();

        let roi = world.get::<&Roi>(entity).unwrap();
        let translated = roi.mesh_data().unwrap();
        assert_eq!(translated.vertices[0].world_mm, [1.0, -2.0, 0.5]);
        assert_eq!(translated.faces, original.faces);
        assert_eq!(roi.job_state.queued, Some(RoiJobKind::RebuildVoxelCache));
        assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert!(roi.is_cache_dirty(RoiCacheKind::Contour));
    }

    #[test]
    fn test_mesh_edit_preview_updates_direct_contour_view_before_commit() {
        let mut world = World::new();
        spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        let editor = world.spawn((EditorState::default(),));
        let original = closed_tetra_mesh_data();
        let entity = world.spawn((Roi::new_mesh(
            RoiId(303),
            "Preview".to_string(),
            original.clone(),
        ),));
        let geometry = main_volume_voxel_geometry(&world).unwrap();
        let plane = orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 0.25], geometry)
            .unwrap();
        let key = ContourViewKey::from_plane(plane);

        let revision =
            begin_mesh_translation_preview(&mut world, editor, entity, [0.25, 0.0, 0.0]).unwrap();
        let status = ensure_contour_view_cache(&mut world, entity, &key);

        assert_eq!(revision, 1);
        assert_eq!(status.state, RepresentationRequestState::Preview);
        {
            let editor_state = world.get::<&EditorState>(editor).unwrap();
            assert!(editor_state.roi_undo_stack.is_empty());
            assert!(editor_state.roi_redo_stack.is_empty());
        }
        {
            let roi = world.get::<&Roi>(entity).unwrap();
            assert_eq!(roi.mesh_data(), Some(&original));
            assert!(roi.preview_state.active);
            assert!(matches!(
                roi.contour_view_cache(&key).map(|cache| &cache.state),
                Some(CacheViewState::Preview { revision: 1 })
            ));
        }

        commit_mesh_edit_preview(&mut world, editor).unwrap();
        let roi = world.get::<&Roi>(entity).unwrap();
        assert_eq!(roi.mesh_data().unwrap().vertices[0].world_mm[0], 0.25);
        assert!(!roi.preview_state.active);
        assert_eq!(roi.job_state.queued, Some(RoiJobKind::RebuildVoxelCache));
        drop(roi);

        {
            let editor_state = world.get::<&EditorState>(editor).unwrap();
            assert_eq!(editor_state.roi_undo_stack.len(), 1);
            assert!(editor_state.roi_redo_stack.is_empty());
        }

        assert_eq!(undo_roi_edit(&mut world, editor), Ok(entity));
        {
            let roi = world.get::<&Roi>(entity).unwrap();
            assert_eq!(roi.mesh_data(), Some(&original));
            assert_eq!(roi.job_state.queued, Some(RoiJobKind::RebuildVoxelCache));
            assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
            assert!(roi.is_cache_dirty(RoiCacheKind::Contour));
        }
        {
            let editor_state = world.get::<&EditorState>(editor).unwrap();
            assert!(editor_state.roi_undo_stack.is_empty());
            assert_eq!(editor_state.roi_redo_stack.len(), 1);
        }

        assert_eq!(redo_roi_edit(&mut world, editor), Ok(entity));
        let roi = world.get::<&Roi>(entity).unwrap();
        assert_eq!(roi.mesh_data().unwrap().vertices[0].world_mm[0], 0.25);
        drop(roi);
        let editor_state = world.get::<&EditorState>(editor).unwrap();
        assert_eq!(editor_state.roi_undo_stack.len(), 1);
        assert!(editor_state.roi_redo_stack.is_empty());
    }

    #[test]
    fn test_cancel_mesh_edit_preview_preserves_authority() {
        let mut world = World::new();
        let editor = world.spawn((EditorState::default(),));
        let original = closed_tetra_mesh_data();
        let entity = world.spawn((Roi::new_mesh(
            RoiId(304),
            "Cancel".to_string(),
            original.clone(),
        ),));

        begin_mesh_translation_preview(&mut world, editor, entity, [1.0, 0.0, 0.0]).unwrap();
        cancel_mesh_edit_preview(&mut world, editor).unwrap();

        let roi = world.get::<&Roi>(entity).unwrap();
        assert_eq!(roi.mesh_data(), Some(&original));
        assert!(!roi.preview_state.active);
        assert_eq!(roi.job_state.queued, None);
    }

    #[test]
    fn test_mesh_rebuild_contract_builds_voxel_then_enables_contour_refresh() {
        let mut world = World::new();
        spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        let entity = world.spawn((Roi::new_mesh(
            RoiId(301),
            "Mesh Contract".to_string(),
            closed_tetra_mesh_data(),
        ),));

        let voxel_result = request_rebuild_voxel_cache_from_mesh(&mut world, entity);
        assert_eq!(voxel_result, Ok(()));
        {
            let roi = world.get::<&Roi>(entity).unwrap();
            assert_eq!(roi.job_state.queued, Some(RoiJobKind::RebuildVoxelCache));
        }
        process_mesh_voxel_rebuild_jobs(&mut world);
        let voxel_status = cache_status(&world, entity, RoiCacheKind::Voxel).unwrap();
        assert!(voxel_status.is_current);
        assert!(world
            .get::<&Roi>(entity)
            .unwrap()
            .voxel_cache()
            .is_some_and(|cache| cache.data.raw_data.iter().any(|value| *value != 0)));
        assert!(roi_voxel_stats(&world, entity).is_some_and(|stats| stats.occupied_voxels > 0));

        let contour_result = request_rebuild_contour_cache_from_mesh(&mut world, entity);
        assert_eq!(contour_result, Ok(()));
        assert!(
            cache_status(&world, entity, RoiCacheKind::Contour)
                .unwrap()
                .is_dirty
        );
    }

    #[test]
    fn test_mesh_rebuild_contract_requests_reject_non_mesh_and_missing_roi() {
        let mut world = World::new();
        let voxel_entity = spawn_test_roi(&mut world);

        assert_eq!(
            request_rebuild_voxel_cache_from_mesh(&mut world, voxel_entity),
            Err(MeshDerivedRebuildError::NotMeshRoi)
        );
        assert_eq!(
            request_rebuild_contour_cache_from_mesh(&mut world, voxel_entity),
            Err(MeshDerivedRebuildError::NotMeshRoi)
        );
        assert_eq!(
            request_rebuild_voxel_cache_from_mesh(&mut world, hecs::Entity::DANGLING),
            Err(MeshDerivedRebuildError::MissingRoi)
        );
        assert_eq!(
            request_rebuild_contour_cache_from_mesh(&mut world, hecs::Entity::DANGLING),
            Err(MeshDerivedRebuildError::MissingRoi)
        );
    }

    #[test]
    fn test_create_contour_roi_from_voxel_roi_keeps_source_unchanged() {
        let mut world = World::new();
        let source = spawn_test_roi(&mut world);
        let source_before = {
            let roi = world.get::<&Roi>(source).unwrap();
            let RoiAuthoritativeData::Voxel(voxel) = &roi.authoritative_data else {
                panic!("expected voxel roi");
            };
            voxel.clone()
        };

        let _new = create_contour_roi_from_voxel_roi(&mut world, source, PlaneFamily::Axial)
            .expect("extraction should create contour roi");

        let roi = world.get::<&Roi>(source).unwrap();
        let RoiAuthoritativeData::Voxel(voxel_after) = &roi.authoritative_data else {
            panic!("source should remain voxel authoritative");
        };
        assert_eq!(*voxel_after, source_before);
        assert_eq!(roi.primary_representation(), PrimaryRepresentation::Voxel);
    }

    #[test]
    fn test_create_contour_roi_from_voxel_roi_returns_contour_primary_roi() {
        let mut world = World::new();
        let source = spawn_test_roi(&mut world);

        let created = create_contour_roi_from_voxel_roi(&mut world, source, PlaneFamily::Axial)
            .expect("extraction should create contour roi");

        let roi = world.get::<&Roi>(created).unwrap();
        assert_eq!(roi.primary_representation(), PrimaryRepresentation::Contour);
        assert!(matches!(
            roi.authoritative_data,
            RoiAuthoritativeData::Contour(_)
        ));
        assert_eq!(roi.metadata.name, "Test (Axial Contour)");
    }

    #[test]
    fn test_create_contour_roi_from_voxel_roi_populates_extractable_contours() {
        let mut world = World::new();
        let source = spawn_sparse_voxel_roi(&mut world);

        let created = create_contour_roi_from_voxel_roi(&mut world, source, PlaneFamily::Axial)
            .expect("extraction should create contour roi");

        let roi = world.get::<&Roi>(created).unwrap();
        let contour_data = roi.contour_data().expect("expected contour data");
        assert_eq!(contour_data.active_plane_family, PlaneFamily::Axial);
        assert!(contour_data.has_loops());
        let seeded_geometry = roi
            .voxel_cache()
            .expect("expected source geometry cache on extracted contour roi")
            .data
            .geometry;
        let source_geometry = world
            .get::<&Roi>(source)
            .ok()
            .and_then(|source_roi| source_roi.voxel_cache().map(|cache| cache.data.geometry))
            .expect("expected source geometry");
        assert_eq!(seeded_geometry, source_geometry);
    }

    #[test]
    fn test_extracted_contour_roi_seeds_current_cpu_voxel_cache() {
        let mut world = World::new();
        let source = spawn_sparse_voxel_roi(&mut world);

        let created = create_contour_roi_from_voxel_roi(&mut world, source, PlaneFamily::Axial)
            .expect("extraction should create contour roi");

        let roi = world.get::<&Roi>(created).unwrap();
        assert!(roi.is_cache_current(RoiCacheKind::Voxel));
        assert!(roi.voxel_gpu_cache().is_none());
        assert_eq!(roi.job_state.queued, None);
    }

    #[test]
    fn test_mesh_authoritative_roi_reports_current_mesh_representation() {
        let mut world = World::new();
        let entity = world.spawn((Roi::new_mesh(
            RoiId(1),
            "Mesh".to_string(),
            simple_mesh_data(),
        ),));

        let status = request_mesh_cache_state(&world, entity);

        assert_eq!(status.state, RepresentationRequestState::Current);
        assert_eq!(status.reason, None);
    }

    #[test]
    fn test_create_contour_roi_from_voxel_roi_rejects_non_voxel_source() {
        let mut world = World::new();
        let source = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
        let result = create_contour_roi_from_voxel_roi(&mut world, source, PlaneFamily::Axial);
        assert_eq!(result, Err(VoxelContourCreationError::NotVoxelRoi));
    }

    #[test]
    fn test_create_contour_roi_from_voxel_roi_rejects_missing_source() {
        let mut world = World::new();
        let result = create_contour_roi_from_voxel_roi(
            &mut world,
            hecs::Entity::DANGLING,
            PlaneFamily::Axial,
        );
        assert_eq!(result, Err(VoxelContourCreationError::MissingRoi));
    }

    #[test]
    fn test_create_contour_roi_from_voxel_roi_surfaces_extraction_errors() {
        let mut world = World::new();
        let source = spawn_test_roi(&mut world);
        let result = create_contour_roi_from_voxel_roi(&mut world, source, PlaneFamily::Oblique);
        assert_eq!(
            result,
            Err(VoxelContourCreationError::ExtractionFailed(
                VoxelContourExtractionError::UnsupportedPlaneFamily {
                    family: PlaneFamily::Oblique
                }
            ))
        );
    }

    #[test]
    fn test_create_mesh_roi_from_voxel_roi_keeps_source_unchanged() {
        let mut world = World::new();
        spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        let source = spawn_sparse_voxel_roi(&mut world);
        let before = {
            let roi = world.get::<&Roi>(source).unwrap();
            match &roi.authoritative_data {
                RoiAuthoritativeData::Voxel(voxel) => voxel.clone(),
                RoiAuthoritativeData::Contour(_) | RoiAuthoritativeData::Mesh(_) => {
                    panic!("source must remain voxel-primary")
                }
            }
        };

        let _mesh = create_mesh_roi_from_voxel_roi(&mut world, source)
            .expect("mesh ROI creation should succeed for voxel source");

        let after = {
            let roi = world.get::<&Roi>(source).unwrap();
            match &roi.authoritative_data {
                RoiAuthoritativeData::Voxel(voxel) => voxel.clone(),
                RoiAuthoritativeData::Contour(_) | RoiAuthoritativeData::Mesh(_) => {
                    panic!("source must remain voxel-primary")
                }
            }
        };
        assert_eq!(before, after);
    }

    #[test]
    fn test_create_mesh_roi_from_voxel_roi_returns_mesh_primary_roi() {
        let mut world = World::new();
        spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        let source = spawn_sparse_voxel_roi(&mut world);

        let created = create_mesh_roi_from_voxel_roi(&mut world, source)
            .expect("mesh ROI creation should succeed for voxel source");
        let roi = world
            .get::<&Roi>(created)
            .expect("created ROI should exist");

        assert_eq!(roi.primary_representation(), PrimaryRepresentation::Mesh);
        assert!(roi.mesh_data().is_some());
        assert!(roi.voxel_cache().is_some());
    }

    #[test]
    fn test_create_mesh_roi_from_voxel_roi_rejects_non_voxel_source() {
        let mut world = World::new();
        spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        let source = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);

        let result = create_mesh_roi_from_voxel_roi(&mut world, source);
        assert_eq!(result, Err(VoxelMeshCreationError::NotVoxelRoi));
    }

    #[test]
    fn test_create_mesh_roi_from_voxel_roi_rejects_missing_main_volume() {
        let mut world = World::new();
        let source = spawn_sparse_voxel_roi(&mut world);
        let result = create_mesh_roi_from_voxel_roi(&mut world, source);
        assert_eq!(result, Err(VoxelMeshCreationError::MissingMainVolume));
    }

    #[test]
    fn test_voxel_data_for_display_surface_extraction_returns_authoritative_when_geometry_matches()
    {
        let mut world = World::new();
        spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        let source = spawn_sparse_voxel_roi(&mut world);
        let source_voxel = world
            .get::<&Roi>(source)
            .ok()
            .and_then(|roi| roi.voxel_cache().map(|cache| cache.data.clone()))
            .expect("source voxel");

        let display_voxel =
            voxel_data_for_display_surface_extraction(&world, source).expect("display source");
        assert_eq!(display_voxel, source_voxel);
    }

    #[test]
    fn test_voxel_data_for_display_surface_extraction_accepts_mismatched_geometry() {
        let mut world = World::new();
        spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        let source = world.spawn((Roi::new_voxel_with_cache(
            RoiId(42),
            "Mismatched".to_string(),
            VoxelGeometry {
                dimensions: [4, 4, 4],
                spacing: [2.0, 2.0, 2.0],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            {
                let mut raw = vec![0_u8; 64];
                raw[21] = 1;
                raw
            },
            None,
        ),));

        let result = voxel_data_for_display_surface_extraction(&world, source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_create_mesh_roi_from_voxel_roi_accepts_mismatched_geometry() {
        let mut world = World::new();
        spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        let source = world.spawn((Roi::new_voxel_with_cache(
            RoiId(43),
            "Mismatched".to_string(),
            VoxelGeometry {
                dimensions: [4, 4, 4],
                spacing: [2.0, 2.0, 2.0],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            {
                let mut raw = vec![0_u8; 64];
                raw[21] = 1;
                raw
            },
            None,
        ),));

        let result = create_mesh_roi_from_voxel_roi(&mut world, source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_create_mesh_roi_from_voxel_roi_spawns_mesh_when_geometry_is_mismatched() {
        let mut world = World::new();
        spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        let source = world.spawn((Roi::new_voxel_with_cache(
            RoiId(44),
            "Mismatched".to_string(),
            VoxelGeometry {
                dimensions: [4, 4, 4],
                spacing: [2.0, 2.0, 2.0],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            {
                let mut raw = vec![0_u8; 64];
                raw[21] = 1;
                raw
            },
            None,
        ),));

        let roi_count_before = world.query::<&Roi>().iter().count();
        let result = create_mesh_roi_from_voxel_roi(&mut world, source);
        let roi_count_after = world.query::<&Roi>().iter().count();

        assert!(result.is_ok());
        assert!(roi_count_after > roi_count_before);
    }

    #[test]
    fn test_create_mesh_roi_from_contour_roi_succeeds_with_current_voxel_cache() {
        let mut world = World::new();
        let source = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
        let source_voxel = VoxelData {
            geometry: VoxelGeometry {
                dimensions: [4, 4, 4],
                spacing: [1.0, 1.0, 1.0],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            raw_data: {
                let mut raw = vec![0_u8; 64];
                raw[(2 * 4 + 1) * 4 + 1] = 1;
                raw
            },
        };
        {
            let mut roi = world.get::<&mut Roi>(source).unwrap();
            roi.session_caches.voxel = Some(VoxelCache {
                data: source_voxel,
                gpu_resources: None,
            });
            roi.dirty_state.voxel_cache_dirty = false;
            roi.dirty_state.generations.voxel = roi.dirty_state.generations.authoritative;
        }

        let created = create_mesh_roi_from_contour_roi(&mut world, source)
            .expect("contour source should succeed when current voxel cache exists");
        let roi = world.get::<&Roi>(created).unwrap();
        assert_eq!(roi.primary_representation(), PrimaryRepresentation::Mesh);
        assert!(roi.mesh_data().is_some());
        assert!(roi.voxel_cache().is_some());
    }

    #[test]
    fn test_create_mesh_roi_from_contour_roi_requires_current_voxel_cache() {
        let mut world = World::new();
        let source = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);

        let result = create_mesh_roi_from_contour_roi(&mut world, source);
        assert_eq!(
            result,
            Err(ContourMeshCreationError::MissingCurrentVoxelCache)
        );
    }

    #[test]
    fn test_extracted_contour_roi_supports_replace_contour_data_edit_path() {
        let mut world = World::new();
        let source = spawn_sparse_voxel_roi(&mut world);
        let extracted = create_contour_roi_from_voxel_roi(&mut world, source, PlaneFamily::Axial)
            .expect("expected extracted contour roi");

        let replacement = ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: vec![ContourSlice {
                plane: test_plane_definition(PlaneFamily::Axial),
                loops: vec![ContourLoop {
                    points: vec![
                        ContourPoint {
                            local_mm: [0.0, 0.0],
                        },
                        ContourPoint {
                            local_mm: [2.0, 0.0],
                        },
                        ContourPoint {
                            local_mm: [1.0, 2.0],
                        },
                    ],
                    is_closed: true,
                }],
            }],
        };

        let result = replace_contour_data(&mut world, extracted, replacement.clone());
        assert_eq!(result, Ok(()));
        let roi = world.get::<&Roi>(extracted).unwrap();
        assert_eq!(roi.contour_data(), Some(&replacement));
    }

    #[test]
    fn test_extracted_contour_roi_edit_queues_rebuild_voxel_cache_job() {
        let mut world = World::new();
        let source = spawn_sparse_voxel_roi(&mut world);
        let extracted = create_contour_roi_from_voxel_roi(&mut world, source, PlaneFamily::Axial)
            .expect("expected extracted contour roi");

        let replacement = ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: vec![ContourSlice {
                plane: test_plane_definition(PlaneFamily::Axial),
                loops: vec![ContourLoop {
                    points: vec![
                        ContourPoint {
                            local_mm: [0.0, 0.0],
                        },
                        ContourPoint {
                            local_mm: [1.5, 0.0],
                        },
                        ContourPoint {
                            local_mm: [0.0, 1.5],
                        },
                    ],
                    is_closed: true,
                }],
            }],
        };

        replace_contour_data(&mut world, extracted, replacement).expect("replace should succeed");
        let roi = world.get::<&Roi>(extracted).unwrap();
        assert_eq!(roi.job_state.queued, Some(RoiJobKind::RebuildVoxelCache));
        assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
    }

    #[test]
    fn test_extracted_contour_roi_remains_valid_after_edit_and_rebuild_cycle() {
        let mut world = World::new();
        spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        let source = spawn_sparse_voxel_roi(&mut world);
        let extracted = create_contour_roi_from_voxel_roi(&mut world, source, PlaneFamily::Axial)
            .expect("expected extracted contour roi");

        let replacement = square_contour_data_for_main_volume(&world, 1.2);
        replace_contour_data(&mut world, extracted, replacement.clone())
            .expect("replace should succeed");
        process_contour_voxel_rebuild_jobs(&mut world);
        process_voxel_mesh_rebuild_jobs(&mut world);

        let roi = world.get::<&Roi>(extracted).unwrap();
        assert_eq!(roi.primary_representation(), PrimaryRepresentation::Contour);
        assert_eq!(roi.contour_data(), Some(&replacement));
        assert!(roi.is_cache_current(RoiCacheKind::Voxel));
        assert!(roi.voxel_cache().is_some());
        assert!(roi
            .voxel_cache()
            .expect("expected derived voxel cache")
            .data
            .raw_data
            .iter()
            .any(|v| *v != 0));
        assert_eq!(roi.job_state.running, None);
        assert_eq!(roi.job_state.queued, None);
        assert!(roi.is_cache_current(RoiCacheKind::Mesh));
        assert!(roi.mesh_cache().is_some_and(|cache| cache.chunks.is_some()));
    }

    #[test]
    fn test_set_active_contour_plane_family_updates_empty_contour_and_marks_derived_dirty() {
        let mut world = World::new();
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
        {
            let mut roi = world.get::<&mut Roi>(entity).unwrap();
            roi.dirty_state.authoritative_dirty = false;
            roi.dirty_state.voxel_cache_dirty = false;
            roi.dirty_state.contour_cache_dirty = false;
            roi.dirty_state.mesh_cache_dirty = false;
            roi.dirty_state.generations.voxel = roi.dirty_state.generations.authoritative;
            roi.dirty_state.generations.contour = roi.dirty_state.generations.authoritative;
            roi.dirty_state.generations.mesh = roi.dirty_state.generations.authoritative;
        }

        let result = set_active_contour_plane_family(&mut world, entity, PlaneFamily::Coronal);
        assert_eq!(result, Ok(()));

        let roi = world.get::<&Roi>(entity).unwrap();
        let contour = roi.contour_data().unwrap();
        assert_eq!(contour.active_plane_family, PlaneFamily::Coronal);
        assert!(roi.dirty_state.authoritative_dirty);
        assert_eq!(roi.dirty_state.generations.authoritative, 2);
        assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert!(!roi.is_cache_dirty(RoiCacheKind::Contour));
        assert!(roi.is_cache_dirty(RoiCacheKind::Mesh));
    }

    #[test]
    fn test_set_active_contour_plane_family_is_noop_when_unchanged() {
        let mut world = World::new();
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Sagittal, false);
        {
            let mut roi = world.get::<&mut Roi>(entity).unwrap();
            roi.dirty_state.authoritative_dirty = false;
            roi.dirty_state.voxel_cache_dirty = false;
            roi.dirty_state.contour_cache_dirty = false;
            roi.dirty_state.mesh_cache_dirty = false;
            roi.dirty_state.generations.voxel = roi.dirty_state.generations.authoritative;
            roi.dirty_state.generations.contour = roi.dirty_state.generations.authoritative;
            roi.dirty_state.generations.mesh = roi.dirty_state.generations.authoritative;
        }

        let result = set_active_contour_plane_family(&mut world, entity, PlaneFamily::Sagittal);
        assert_eq!(result, Ok(()));

        let roi = world.get::<&Roi>(entity).unwrap();
        let contour = roi.contour_data().unwrap();
        assert_eq!(contour.active_plane_family, PlaneFamily::Sagittal);
        assert!(!roi.dirty_state.authoritative_dirty);
        assert_eq!(roi.dirty_state.generations.authoritative, 1);
        assert!(!roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert!(!roi.is_cache_dirty(RoiCacheKind::Contour));
        assert!(!roi.is_cache_dirty(RoiCacheKind::Mesh));
    }

    #[test]
    fn test_set_active_contour_plane_family_rejects_non_empty_contour() {
        let mut world = World::new();
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, true);

        let result = set_active_contour_plane_family(&mut world, entity, PlaneFamily::Coronal);
        assert_eq!(
            result,
            Err(ContourPlaneFamilySwitchError::RequiresConversion)
        );

        let roi = world.get::<&Roi>(entity).unwrap();
        let contour = roi.contour_data().unwrap();
        assert_eq!(contour.active_plane_family, PlaneFamily::Axial);
    }

    #[test]
    fn test_set_active_contour_plane_family_rejects_voxel_roi() {
        let mut world = World::new();
        let entity = spawn_test_roi(&mut world);

        let result = set_active_contour_plane_family(&mut world, entity, PlaneFamily::Oblique);
        assert_eq!(result, Err(ContourPlaneFamilySwitchError::NotContourRoi));
    }

    #[test]
    fn test_replace_contour_data_updates_authoritative_state_and_queues_voxel_rebuild() {
        let mut world = World::new();
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
        {
            let mut roi = world.get::<&mut Roi>(entity).unwrap();
            roi.dirty_state.authoritative_dirty = false;
            roi.dirty_state.voxel_cache_dirty = false;
            roi.dirty_state.contour_cache_dirty = false;
            roi.dirty_state.mesh_cache_dirty = false;
            roi.dirty_state.generations.voxel = roi.dirty_state.generations.authoritative;
            roi.dirty_state.generations.contour = roi.dirty_state.generations.authoritative;
            roi.dirty_state.generations.mesh = roi.dirty_state.generations.authoritative;
        }

        let replacement = ContourData {
            active_plane_family: PlaneFamily::Coronal,
            slices: vec![ContourSlice {
                plane: test_plane_definition(PlaneFamily::Coronal),
                loops: vec![ContourLoop {
                    points: vec![
                        ContourPoint {
                            local_mm: [0.0, 0.0],
                        },
                        ContourPoint {
                            local_mm: [2.0, 0.0],
                        },
                        ContourPoint {
                            local_mm: [0.0, 2.0],
                        },
                    ],
                    is_closed: true,
                }],
            }],
        };

        let result = replace_contour_data(&mut world, entity, replacement.clone());
        assert_eq!(result, Ok(()));

        {
            let roi = world.get::<&Roi>(entity).unwrap();
            assert_eq!(roi.contour_data(), Some(&replacement));
            assert!(roi.dirty_state.authoritative_dirty);
            assert_eq!(roi.dirty_state.generations.authoritative, 2);
            assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
            assert!(!roi.is_cache_dirty(RoiCacheKind::Contour));
            assert!(roi.is_cache_dirty(RoiCacheKind::Mesh));
            assert_eq!(roi.job_state.queued, Some(RoiJobKind::RebuildVoxelCache));
        }

        assert_eq!(
            begin_next_job(&mut world, entity),
            Some(RoiJobKind::RebuildVoxelCache)
        );

        let status_after_begin = cache_status(&world, entity, RoiCacheKind::Voxel).unwrap();
        assert!(status_after_begin.is_dirty);
        assert!(!status_after_begin.is_current);
    }

    #[test]
    fn test_contour_edit_history_undo_redo_restores_authority_and_requeues_rebuild() {
        let mut world = World::new();
        let editor = world.spawn((EditorState::default(),));
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
        let original = world
            .get::<&Roi>(entity)
            .unwrap()
            .contour_data()
            .unwrap()
            .clone();
        let replacement = ContourData {
            active_plane_family: PlaneFamily::Coronal,
            slices: vec![ContourSlice {
                plane: test_plane_definition(PlaneFamily::Coronal),
                loops: vec![ContourLoop {
                    points: vec![
                        ContourPoint {
                            local_mm: [0.0, 0.0],
                        },
                        ContourPoint {
                            local_mm: [2.0, 0.0],
                        },
                        ContourPoint {
                            local_mm: [0.0, 2.0],
                        },
                    ],
                    is_closed: true,
                }],
            }],
        };

        replace_contour_data_with_history(&mut world, editor, entity, replacement.clone()).unwrap();
        let generation_after_commit = world
            .get::<&Roi>(entity)
            .unwrap()
            .dirty_state
            .generations
            .authoritative;
        {
            let editor_state = world.get::<&EditorState>(editor).unwrap();
            assert_eq!(editor_state.roi_undo_stack.len(), 1);
            assert!(editor_state.roi_redo_stack.is_empty());
        }

        assert_eq!(undo_roi_edit(&mut world, editor), Ok(entity));
        {
            let roi = world.get::<&Roi>(entity).unwrap();
            assert_eq!(roi.contour_data(), Some(&original));
            assert_eq!(
                roi.dirty_state.generations.authoritative,
                generation_after_commit + 1
            );
            assert_eq!(roi.job_state.queued, Some(RoiJobKind::RebuildVoxelCache));
            assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
            assert!(roi.is_cache_dirty(RoiCacheKind::Mesh));
        }
        {
            let editor_state = world.get::<&EditorState>(editor).unwrap();
            assert!(editor_state.roi_undo_stack.is_empty());
            assert_eq!(editor_state.roi_redo_stack.len(), 1);
        }

        assert_eq!(redo_roi_edit(&mut world, editor), Ok(entity));
        {
            let roi = world.get::<&Roi>(entity).unwrap();
            assert_eq!(roi.contour_data(), Some(&replacement));
            assert_eq!(
                roi.dirty_state.generations.authoritative,
                generation_after_commit + 2
            );
            assert_eq!(roi.job_state.queued, Some(RoiJobKind::RebuildVoxelCache));
        }
        let editor_state = world.get::<&EditorState>(editor).unwrap();
        assert_eq!(editor_state.roi_undo_stack.len(), 1);
        assert!(editor_state.roi_redo_stack.is_empty());
    }

    #[test]
    fn test_slice_local_contour_history_preserves_dirty_plane_for_undo_and_redo() {
        let mut world = World::new();
        let editor = world.spawn((EditorState::default(),));
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, true);
        let original = world
            .get::<&Roi>(entity)
            .unwrap()
            .contour_data()
            .unwrap()
            .clone();
        let plane = original.slices[0].plane;
        let expected_dirty_region =
            RoiDirtyRegion::ContourSlice(ContourSliceKey::from_plane(plane));
        let mut replacement = original.clone();
        replacement.slices[0].loops[0].points[1].local_mm[0] += 0.25;

        replace_contour_data_for_slice_with_history(&mut world, editor, entity, replacement, plane)
            .unwrap();
        assert_eq!(undo_roi_edit(&mut world, editor), Ok(entity));
        assert_eq!(
            world.get::<&Roi>(entity).unwrap().job_state.pending[0].dirty_region,
            expected_dirty_region
        );

        assert_eq!(redo_roi_edit(&mut world, editor), Ok(entity));
        assert_eq!(
            world.get::<&Roi>(entity).unwrap().job_state.pending[0].dirty_region,
            expected_dirty_region
        );
    }

    #[test]
    fn test_new_contour_slice_history_uses_full_rebuild_for_safe_undo() {
        let mut world = World::new();
        let editor = world.spawn((EditorState::default(),));
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
        let plane = test_plane_definition(PlaneFamily::Axial);
        let replacement = ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: vec![ContourSlice {
                plane,
                loops: Vec::new(),
            }],
        };

        replace_contour_data_for_slice_with_history(&mut world, editor, entity, replacement, plane)
            .unwrap();
        assert_eq!(undo_roi_edit(&mut world, editor), Ok(entity));
        assert_eq!(
            world.get::<&Roi>(entity).unwrap().job_state.pending[0].dirty_region,
            RoiDirtyRegion::Full
        );
    }

    #[test]
    fn test_noop_contour_history_commit_does_not_advance_generation_or_record_history() {
        let mut world = World::new();
        let editor = world.spawn((EditorState::default(),));
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, true);
        let contour = world
            .get::<&Roi>(entity)
            .unwrap()
            .contour_data()
            .unwrap()
            .clone();
        let generation_before = world
            .get::<&Roi>(entity)
            .unwrap()
            .dirty_state
            .generations
            .authoritative;

        replace_contour_data_with_history(&mut world, editor, entity, contour).unwrap();

        let roi = world.get::<&Roi>(entity).unwrap();
        assert_eq!(roi.dirty_state.generations.authoritative, generation_before);
        drop(roi);
        let editor_state = world.get::<&EditorState>(editor).unwrap();
        assert!(editor_state.roi_undo_stack.is_empty());
        assert!(editor_state.roi_redo_stack.is_empty());
    }

    #[test]
    fn test_new_contour_commit_after_undo_clears_redo_history() {
        let mut world = World::new();
        let editor = world.spawn((EditorState::default(),));
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
        let first = ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: vec![ContourSlice {
                plane: test_plane_definition(PlaneFamily::Axial),
                loops: Vec::new(),
            }],
        };
        let second = ContourData {
            active_plane_family: PlaneFamily::Coronal,
            slices: vec![ContourSlice {
                plane: test_plane_definition(PlaneFamily::Coronal),
                loops: Vec::new(),
            }],
        };

        replace_contour_data_with_history(&mut world, editor, entity, first).unwrap();
        undo_roi_edit(&mut world, editor).unwrap();
        replace_contour_data_with_history(&mut world, editor, entity, second.clone()).unwrap();

        let roi = world.get::<&Roi>(entity).unwrap();
        assert_eq!(roi.contour_data(), Some(&second));
        drop(roi);
        let editor_state = world.get::<&EditorState>(editor).unwrap();
        assert_eq!(editor_state.roi_undo_stack.len(), 1);
        assert!(editor_state.roi_redo_stack.is_empty());
        drop(editor_state);
        assert_eq!(
            redo_roi_edit(&mut world, editor),
            Err(RoiEditHistoryError::NoRedo)
        );
    }

    #[test]
    fn test_replace_contour_data_rejects_voxel_roi() {
        let mut world = World::new();
        let entity = spawn_test_roi(&mut world);

        let result = replace_contour_data(
            &mut world,
            entity,
            ContourData {
                active_plane_family: PlaneFamily::Axial,
                slices: Vec::new(),
            },
        );
        assert_eq!(result, Err(ContourMutationError::NotContourRoi));
    }

    #[test]
    fn test_replace_contour_data_rejects_missing_roi() {
        let mut world = World::new();
        let missing = hecs::Entity::DANGLING;

        let result = replace_contour_data(
            &mut world,
            missing,
            ContourData {
                active_plane_family: PlaneFamily::Axial,
                slices: Vec::new(),
            },
        );
        assert_eq!(result, Err(ContourMutationError::MissingRoi));
    }

    #[test]
    fn test_process_contour_voxel_rebuild_jobs_builds_current_voxel_cache_for_contour_roi() {
        let mut world = World::new();
        spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
        let replacement = square_contour_data_for_main_volume(&world, 1.4);
        replace_contour_data(&mut world, entity, replacement.clone()).unwrap();

        process_contour_voxel_rebuild_jobs(&mut world);

        let roi = world.get::<&Roi>(entity).unwrap();
        assert_eq!(roi.contour_data(), Some(&replacement));
        assert!(roi.is_cache_current(RoiCacheKind::Voxel));
        assert_eq!(roi.job_state.running, None);
        assert_eq!(roi.job_state.queued, Some(RoiJobKind::RebuildMeshCache));
        let voxel_cache = roi.voxel_cache().expect("voxel cache should exist");
        assert_eq!(voxel_cache.data.geometry.dimensions, [4, 4, 4]);
        assert!(voxel_cache.data.raw_data.iter().any(|v| *v != 0));
        assert!(voxel_cache.gpu_resources.is_none());
    }

    #[test]
    fn test_slice_commit_updates_retained_voxel_cache_for_coronal_and_sagittal_contours() {
        for family in [PlaneFamily::Coronal, PlaneFamily::Sagittal] {
            let mut world = World::new();
            spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
            let entity = spawn_test_contour_roi(&mut world, family, false);
            seed_current_voxel_cache_for_contour_roi(&mut world, entity);
            {
                let mut roi = world.get::<&mut Roi>(entity).unwrap();
                roi.voxel_cache_mut().unwrap().data.raw_data[0] = 1;
            }

            let geometry = main_volume_voxel_geometry(&world).unwrap();
            let plane = orthogonal_plane_from_volume_uv(family, [0.5, 0.5, 0.5], geometry)
                .expect("orthogonal edit plane");
            let replacement = ContourData {
                active_plane_family: family,
                slices: vec![ContourSlice {
                    plane,
                    loops: vec![ContourLoop {
                        points: vec![
                            ContourPoint {
                                local_mm: [-1.0, -1.0],
                            },
                            ContourPoint {
                                local_mm: [1.0, -1.0],
                            },
                            ContourPoint {
                                local_mm: [1.0, 1.0],
                            },
                            ContourPoint {
                                local_mm: [-1.0, 1.0],
                            },
                        ],
                        is_closed: true,
                    }],
                }],
            };

            replace_contour_data_for_slice(&mut world, entity, replacement, plane).unwrap();
            process_contour_voxel_rebuild_jobs(&mut world);

            let roi = world.get::<&Roi>(entity).unwrap();
            assert!(roi.is_cache_current(RoiCacheKind::Voxel), "{family:?}");
            let voxel = &roi.voxel_cache().unwrap().data;
            assert_eq!(
                voxel.raw_data[0], 1,
                "unrelated slab changed for {family:?}"
            );
            let depth_axis = match family {
                PlaneFamily::Coronal => 1,
                PlaneFamily::Sagittal => 0,
                _ => unreachable!(),
            };
            let depth =
                world_mm_to_voxel_index(plane.origin_mm, geometry)[depth_axis].round() as u32;
            let mut occupied_on_edited_slab = 0;
            for z in 0..4 {
                for y in 0..4 {
                    for x in 0..4 {
                        let index = [x, y, z];
                        let linear = ((z * 4 + y) * 4 + x) as usize;
                        if index[depth_axis] == depth && voxel.raw_data[linear] != 0 {
                            occupied_on_edited_slab += 1;
                        }
                    }
                }
            }
            assert!(occupied_on_edited_slab > 0, "{family:?}");

            let mesh_request = roi
                .job_state
                .pending
                .iter()
                .find(|request| request.kind == RoiJobKind::RebuildMeshCache)
                .expect("mesh rebuild queued");
            let RoiDirtyRegion::VoxelAabb { min, max } = mesh_request.dirty_region else {
                panic!("expected slab-local mesh rebuild for {family:?}");
            };
            assert_eq!(min[depth_axis], depth, "{family:?}");
            assert_eq!(max[depth_axis], depth + 1, "{family:?}");
        }
    }

    #[test]
    fn test_process_contour_voxel_rebuild_jobs_missing_main_volume_requeues_without_mutation() {
        let mut world = World::new();
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, true);
        let contour_before = world
            .get::<&Roi>(entity)
            .unwrap()
            .contour_data()
            .unwrap()
            .clone();
        {
            let mut roi = world.get::<&mut Roi>(entity).unwrap();
            roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
        }

        process_contour_voxel_rebuild_jobs(&mut world);

        let roi = world.get::<&Roi>(entity).unwrap();
        assert_eq!(roi.contour_data(), Some(&contour_before));
        assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert!(!roi.is_cache_current(RoiCacheKind::Voxel));
        assert_eq!(roi.job_state.running, None);
        assert_eq!(roi.job_state.queued, None);
    }

    #[test]
    fn test_process_contour_voxel_rebuild_jobs_discards_stale_generation_results() {
        let mut world = World::new();
        spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
        let replacement = square_contour_data_for_main_volume(&world, 1.2);
        replace_contour_data(&mut world, entity, replacement).unwrap();

        process_contour_voxel_rebuild_jobs_with_hook(
            &mut world,
            |world, hook_entity| {
                if hook_entity != entity {
                    return;
                }
                let mut roi = world.get::<&mut Roi>(hook_entity).unwrap();
                if let RoiAuthoritativeData::Contour(contour) = &mut roi.authoritative_data {
                    contour.slices.clear();
                }
                roi.mark_contour_authoritative_changed();
            },
            None,
            None,
        );

        let roi = world.get::<&Roi>(entity).unwrap();
        assert!(!roi.is_cache_current(RoiCacheKind::Voxel));
        assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert_eq!(roi.job_state.running, None);
        assert_eq!(roi.job_state.queued, Some(RoiJobKind::RebuildVoxelCache));
    }

    #[test]
    fn test_process_contour_voxel_rebuild_jobs_clears_running_state_on_success() {
        let mut world = World::new();
        spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
        let replacement = square_contour_data_for_main_volume(&world, 1.0);
        replace_contour_data(&mut world, entity, replacement).unwrap();

        process_contour_voxel_rebuild_jobs(&mut world);

        let roi = world.get::<&Roi>(entity).unwrap();
        assert_eq!(roi.job_state.running, None);
        assert_eq!(roi.job_state.queued, Some(RoiJobKind::RebuildMeshCache));
        assert!(roi.is_cache_current(RoiCacheKind::Voxel));
        assert_eq!(
            roi.cache_generation(RoiCacheKind::Voxel),
            roi.dirty_state.generations.authoritative
        );
    }

    #[test]
    fn test_oblique_dirty_slice_rebuild_preserves_other_authoritative_planes() {
        let mut world = World::new();
        spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        let make_slice = |z: f32| ContourSlice {
            plane: PlaneDefinition {
                family: PlaneFamily::Oblique,
                origin_mm: [0.0, 0.0, z],
                u_axis_mm: [1.0, 0.0, 0.0],
                v_axis_mm: [0.0, 1.0, 0.0],
                normal_mm: [0.0, 0.0, 1.0],
            },
            loops: vec![ContourLoop {
                points: vec![
                    ContourPoint {
                        local_mm: [0.5, 0.5],
                    },
                    ContourPoint {
                        local_mm: [2.5, 0.5],
                    },
                    ContourPoint {
                        local_mm: [2.5, 2.5],
                    },
                    ContourPoint {
                        local_mm: [0.5, 2.5],
                    },
                ],
                is_closed: true,
            }],
        };
        let first_slice = make_slice(1.0);
        let first_key = ContourSliceKey::from_plane(first_slice.plane);
        let entity = world.spawn((Roi::new_contour(
            RoiId(101),
            "Oblique".to_string(),
            ContourData {
                active_plane_family: PlaneFamily::Oblique,
                slices: vec![first_slice, make_slice(2.0)],
            },
        ),));
        {
            let mut roi = world.get::<&mut Roi>(entity).unwrap();
            let source_generation = roi.dirty_state.generations.authoritative;
            roi.enqueue_job(RoiJobRequest {
                kind: RoiJobKind::RebuildVoxelCache,
                source_generation,
                preview_revision: None,
                priority: RoiJobPriority::VisibleCommitted,
                dirty_region: RoiDirtyRegion::ContourSlice(first_key),
            });
        }

        process_contour_voxel_rebuild_jobs(&mut world);

        let roi = world.get::<&Roi>(entity).unwrap();
        let raw = &roi.voxel_cache().unwrap().data.raw_data;
        let center_index = |z: usize| z * 16 + 5;
        assert_ne!(raw[center_index(1)], 0);
        assert_ne!(raw[center_index(2)], 0);
    }

    #[test]
    fn test_process_contour_voxel_rebuild_jobs_hard_raster_failure_does_not_requeue() {
        let mut world = World::new();
        spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);

        let invalid_contour = ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: vec![ContourSlice {
                plane: PlaneDefinition {
                    family: PlaneFamily::Axial,
                    origin_mm: [0.0, 0.0, 0.0],
                    u_axis_mm: [1.0, 0.0, 0.0],
                    v_axis_mm: [0.0, 1.0, 0.0],
                    normal_mm: [0.0, 0.0, 0.0],
                },
                loops: vec![ContourLoop {
                    points: vec![
                        ContourPoint {
                            local_mm: [0.0, 0.0],
                        },
                        ContourPoint {
                            local_mm: [1.0, 0.0],
                        },
                        ContourPoint {
                            local_mm: [0.0, 1.0],
                        },
                    ],
                    is_closed: true,
                }],
            }],
        };
        replace_contour_data(&mut world, entity, invalid_contour).unwrap();

        process_contour_voxel_rebuild_jobs(&mut world);

        let roi = world.get::<&Roi>(entity).unwrap();
        assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert_eq!(roi.job_state.running, None);
        assert_eq!(roi.job_state.queued, None);
    }

    #[test]
    fn test_invalidate_contour_voxel_caches_for_main_volume_change_dirties_and_queues() {
        let mut world = World::new();
        spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        let contour_entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
        let voxel_entity = spawn_test_roi(&mut world);
        let replacement = square_contour_data_for_main_volume(&world, 1.2);
        replace_contour_data(&mut world, contour_entity, replacement).unwrap();
        process_contour_voxel_rebuild_jobs(&mut world);

        {
            let contour_roi = world.get::<&Roi>(contour_entity).unwrap();
            assert!(contour_roi.is_cache_current(RoiCacheKind::Voxel));
            assert!(contour_roi.voxel_cache().is_some());
        }

        invalidate_contour_voxel_caches_for_main_volume_change(&mut world);

        let contour_roi = world.get::<&Roi>(contour_entity).unwrap();
        assert!(contour_roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert!(!contour_roi.is_cache_current(RoiCacheKind::Voxel));
        assert!(contour_roi.voxel_cache().is_none());
        assert_eq!(
            contour_roi.job_state.queued,
            Some(RoiJobKind::RebuildVoxelCache)
        );

        let voxel_roi = world.get::<&Roi>(voxel_entity).unwrap();
        assert_eq!(voxel_roi.job_state.queued, None);
    }

    #[test]
    fn test_request_contour_view_state_reports_active_family_editable() {
        let mut world = World::new();
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
        let key = ContourViewKey::from_plane(test_plane_definition(PlaneFamily::Axial));

        let status = request_contour_view_state(&world, entity, &key);
        assert_eq!(status.request.state, RepresentationRequestState::Current);
        assert!(status.editable);
        assert!(!status.promotable);
    }

    #[test]
    fn test_request_contour_view_state_reports_derived_promotable_when_current() {
        let mut world = World::new();
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
        seed_current_voxel_cache_for_contour_roi(&mut world, entity);
        let key = ContourViewKey::from_plane(test_plane_definition(PlaneFamily::Coronal));
        {
            let mut roi = world.get::<&mut Roi>(entity).unwrap();
            let gen = roi.dirty_state.generations.authoritative;
            let mut promoted_data = roi.contour_data().unwrap().clone();
            promoted_data.active_plane_family = PlaneFamily::Coronal;
            roi.upsert_contour_view_cache(key.clone(), promoted_data, gen, CacheViewState::Current);
        }

        let status = request_contour_view_state(&world, entity, &key);
        assert_eq!(status.request.state, RepresentationRequestState::Current);
        assert!(!status.editable);
        assert!(status.promotable);
    }

    #[test]
    fn test_voxel_primary_roi_can_build_and_request_orthogonal_contour_view_cache() {
        let mut world = World::new();
        let entity = spawn_sparse_voxel_roi(&mut world);
        {
            let mut roi = world.get::<&mut Roi>(entity).unwrap();
            roi.dirty_state.voxel_cache_dirty = false;
            roi.dirty_state.generations.voxel = roi.dirty_state.generations.authoritative;
        }
        let geometry = {
            let roi = world.get::<&Roi>(entity).unwrap();
            roi.voxel_cache().unwrap().data.geometry
        };
        let plane =
            orthogonal_plane_from_volume_uv(PlaneFamily::Coronal, [0.5, 2.0 / 3.0, 0.5], geometry)
                .unwrap();
        let key = ContourViewKey::from_plane(plane);

        let build_status = ensure_contour_view_cache(&mut world, entity, &key);
        assert_eq!(build_status.state, RepresentationRequestState::Current);

        let status = request_contour_view_state(&world, entity, &key);
        assert_eq!(status.request.state, RepresentationRequestState::Current);
        assert!(!status.editable);
        assert!(!status.promotable);

        let roi = world.get::<&Roi>(entity).unwrap();
        let cache = roi
            .contour_view_cache(&key)
            .expect("expected derived contour view cache for voxel roi");
        assert_eq!(cache.state, CacheViewState::Current);
        assert_eq!(cache.data.active_plane_family, PlaneFamily::Coronal);
    }

    #[test]
    fn test_ensure_contour_view_cache_builds_orthogonal_view_from_current_voxel_cache() {
        let mut world = World::new();
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
        {
            let mut roi = world.get::<&mut Roi>(entity).unwrap();
            let mut raw_data = vec![0_u8; 64];
            raw_data[(2 * 4 + 1) * 4 + 1] = 1;
            roi.session_caches.voxel = Some(VoxelCache {
                data: VoxelData {
                    geometry: VoxelGeometry {
                        dimensions: [4, 4, 4],
                        spacing: [1.0, 1.0, 1.0],
                        origin: [0.0, 0.0, 0.0],
                        orientation: [0.0, 0.0, 0.0, 1.0],
                    },
                    raw_data,
                },
                gpu_resources: None,
            });
            roi.dirty_state.voxel_cache_dirty = false;
            roi.dirty_state.generations.voxel = roi.dirty_state.generations.authoritative;
        }
        let plane = orthogonal_plane_from_volume_uv(
            PlaneFamily::Coronal,
            [0.5, 2.0 / 3.0, 0.5],
            VoxelGeometry {
                dimensions: [4, 4, 4],
                spacing: [1.0, 1.0, 1.0],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
        )
        .unwrap();
        let key = ContourViewKey::from_plane(plane);

        let status = ensure_contour_view_cache(&mut world, entity, &key);

        assert_eq!(status.state, RepresentationRequestState::Current);
        let roi = world.get::<&Roi>(entity).unwrap();
        let cache = roi
            .contour_view_cache(&key)
            .expect("expected derived contour view cache");
        assert_eq!(cache.state, CacheViewState::Current);
        assert_eq!(
            cache.source_generation,
            roi.dirty_state.generations.authoritative
        );
        assert_eq!(cache.data.active_plane_family, PlaneFamily::Coronal);
    }

    #[test]
    fn test_ensure_contour_view_cache_reuses_current_matching_generation_cache() {
        let mut world = World::new();
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
        seed_current_voxel_cache_for_contour_roi(&mut world, entity);
        let key = ContourViewKey::from_plane(test_plane_definition(PlaneFamily::Coronal));
        {
            let mut roi = world.get::<&mut Roi>(entity).unwrap();
            let generation = roi.dirty_state.generations.authoritative;
            roi.upsert_contour_view_cache(
                key.clone(),
                ContourData {
                    active_plane_family: PlaneFamily::Coronal,
                    slices: vec![ContourSlice {
                        plane: test_plane_definition(PlaneFamily::Coronal),
                        loops: vec![ContourLoop {
                            points: vec![
                                ContourPoint {
                                    local_mm: [10.0, 10.0],
                                },
                                ContourPoint {
                                    local_mm: [12.0, 10.0],
                                },
                                ContourPoint {
                                    local_mm: [10.0, 12.0],
                                },
                            ],
                            is_closed: true,
                        }],
                    }],
                },
                generation,
                CacheViewState::Current,
            );
        }

        let status = ensure_contour_view_cache(&mut world, entity, &key);

        assert_eq!(status.state, RepresentationRequestState::Current);
        let roi = world.get::<&Roi>(entity).unwrap();
        let cache = roi.contour_view_cache(&key).unwrap();
        assert_eq!(cache.state, CacheViewState::Current);
        assert_eq!(cache.data.slices.len(), 1);
        assert_eq!(cache.data.slices[0].loops.len(), 1);
        assert_eq!(
            cache.data.slices[0].loops[0].points[0].local_mm,
            [10.0, 10.0]
        );
    }

    #[test]
    fn test_ensure_contour_view_cache_matches_extracted_slice_with_tolerance() {
        let mut world = World::new();
        let entity = spawn_sparse_voxel_roi(&mut world);
        {
            let mut roi = world.get::<&mut Roi>(entity).unwrap();
            roi.dirty_state.voxel_cache_dirty = false;
            roi.dirty_state.generations.voxel = roi.dirty_state.generations.authoritative;
        }
        let geometry = {
            let roi = world.get::<&Roi>(entity).unwrap();
            roi.voxel_cache().unwrap().data.geometry
        };
        let mut near_plane =
            orthogonal_plane_from_volume_uv(PlaneFamily::Coronal, [0.5, 1.0 / 3.0, 0.5], geometry)
                .unwrap();
        near_plane.origin_mm[1] += 0.4;
        let key = ContourViewKey::from_plane(near_plane);

        let status = ensure_contour_view_cache(&mut world, entity, &key);

        assert_eq!(status.state, RepresentationRequestState::Current);
        let roi = world.get::<&Roi>(entity).unwrap();
        let cache = roi.contour_view_cache(&key).unwrap();
        assert_eq!(cache.state, CacheViewState::Current);
        assert_eq!(cache.data.slices.len(), 1);
        assert_eq!(cache.data.slices[0].loops.len(), 1);
        assert!(!cache.data.slices[0].loops[0].points.is_empty());
    }

    #[test]
    fn test_request_contour_view_state_reflects_voxel_stale_rebuilding_and_missing_states() {
        let mut world = World::new();
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
        let key = ContourViewKey::from_plane(test_plane_definition(PlaneFamily::Coronal));

        let blocked = request_contour_view_state(&world, entity, &key);
        assert_eq!(blocked.request.state, RepresentationRequestState::Blocked);
        assert_eq!(
            blocked.request.reason.as_deref(),
            Some("voxel_cache_missing_for_contour_view")
        );

        seed_current_voxel_cache_for_contour_roi(&mut world, entity);
        {
            let mut roi = world.get::<&mut Roi>(entity).unwrap();
            let gen = roi.dirty_state.generations.authoritative;
            let mut derived_data = roi.contour_data().unwrap().clone();
            derived_data.active_plane_family = PlaneFamily::Coronal;
            roi.upsert_contour_view_cache(key.clone(), derived_data, gen, CacheViewState::Current);
            roi.dirty_state.voxel_cache_dirty = true;
        }

        let stale = request_contour_view_state(&world, entity, &key);
        assert_eq!(stale.request.state, RepresentationRequestState::Stale);
        assert_eq!(
            stale.request.reason.as_deref(),
            Some("voxel_cache_stale_for_contour_view")
        );

        {
            let mut roi = world.get::<&mut Roi>(entity).unwrap();
            roi.job_state.running = Some(RoiJobKind::RebuildVoxelCache);
        }

        let rebuilding = request_contour_view_state(&world, entity, &key);
        assert_eq!(
            rebuilding.request.state,
            RepresentationRequestState::Rebuilding
        );
        assert_eq!(
            rebuilding.request.reason.as_deref(),
            Some("voxel_cache_rebuilding_for_contour_view")
        );
    }

    #[test]
    fn test_promote_contour_view_to_authoritative_marks_other_caches_stale_and_rebuilds() {
        let mut world = World::new();
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
        seed_current_voxel_cache_for_contour_roi(&mut world, entity);
        let coronal_key = ContourViewKey::from_plane(test_plane_definition(PlaneFamily::Coronal));
        let sagittal_key = ContourViewKey::from_plane(test_plane_definition(PlaneFamily::Sagittal));
        {
            let mut roi = world.get::<&mut Roi>(entity).unwrap();
            let gen = roi.dirty_state.generations.authoritative;
            let mut coronal_data = roi.contour_data().unwrap().clone();
            coronal_data.active_plane_family = PlaneFamily::Coronal;
            roi.upsert_contour_view_cache(
                coronal_key.clone(),
                coronal_data,
                gen,
                CacheViewState::Current,
            );
            let mut sagittal_data = roi.contour_data().unwrap().clone();
            sagittal_data.active_plane_family = PlaneFamily::Sagittal;
            roi.upsert_contour_view_cache(
                sagittal_key.clone(),
                sagittal_data,
                gen,
                CacheViewState::Current,
            );
        }

        let result = promote_contour_view_to_authoritative(&mut world, entity, &coronal_key);
        assert_eq!(result, Ok(()));
        let roi = world.get::<&Roi>(entity).unwrap();
        let contour = roi.contour_data().unwrap();
        assert_eq!(contour.active_plane_family, PlaneFamily::Coronal);
        assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert!(roi.is_cache_dirty(RoiCacheKind::Mesh));
        assert_eq!(roi.job_state.queued, Some(RoiJobKind::RebuildVoxelCache));
        assert_eq!(
            roi.contour_view_cache(&sagittal_key).unwrap().state,
            CacheViewState::Stale
        );
    }

    #[test]
    fn test_promote_contour_view_rejects_stale_or_missing_cache() {
        let mut world = World::new();
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
        let key = ContourViewKey::from_plane(test_plane_definition(PlaneFamily::Coronal));
        let missing_result = promote_contour_view_to_authoritative(&mut world, entity, &key);
        assert_eq!(missing_result, Err(ContourPromotionError::ViewCacheMissing));
        {
            let mut roi = world.get::<&mut Roi>(entity).unwrap();
            let gen = roi.dirty_state.generations.authoritative;
            let mut promoted_data = roi.contour_data().unwrap().clone();
            promoted_data.active_plane_family = PlaneFamily::Coronal;
            roi.upsert_contour_view_cache(key.clone(), promoted_data, gen, CacheViewState::Stale);
        }
        let stale_result = promote_contour_view_to_authoritative(&mut world, entity, &key);
        assert_eq!(
            stale_result,
            Err(ContourPromotionError::ViewCacheNotCurrent)
        );

        {
            let mut roi = world.get::<&mut Roi>(entity).unwrap();
            let mut promoted_data = roi.contour_data().unwrap().clone();
            promoted_data.active_plane_family = PlaneFamily::Coronal;
            roi.upsert_contour_view_cache(key.clone(), promoted_data, 1, CacheViewState::Current);
            roi.dirty_state.generations.authoritative = 2;
        }
        let generation_stale = promote_contour_view_to_authoritative(&mut world, entity, &key);
        assert_eq!(
            generation_stale,
            Err(ContourPromotionError::ViewCacheNotCurrent)
        );
    }

    #[test]
    fn test_request_contour_view_state_marks_generation_mismatch_stale() {
        let mut world = World::new();
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
        let key = ContourViewKey::from_plane(test_plane_definition(PlaneFamily::Coronal));
        {
            let mut roi = world.get::<&mut Roi>(entity).unwrap();
            let mut promoted_data = roi.contour_data().unwrap().clone();
            promoted_data.active_plane_family = PlaneFamily::Coronal;
            roi.upsert_contour_view_cache(key.clone(), promoted_data, 1, CacheViewState::Current);
            roi.dirty_state.generations.authoritative = 2;
        }
        let status = request_contour_view_state(&world, entity, &key);
        assert_eq!(status.request.state, RepresentationRequestState::Stale);
        assert_eq!(
            status.request.reason.as_deref(),
            Some("contour_view_cache_generation_stale")
        );
        assert!(!status.promotable);
    }

    #[test]
    fn test_oblique_contour_view_builds_current_and_can_be_promoted() {
        let mut world = World::new();
        spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
        seed_current_voxel_cache_for_contour_roi(&mut world, entity);
        {
            let mut roi = world.get::<&mut Roi>(entity).unwrap();
            let voxel = &mut roi.session_caches.voxel.as_mut().unwrap().data;
            for z in 1..=2 {
                for y in 1..=2 {
                    for x in 1..=2 {
                        voxel.raw_data[(z * 16 + y * 4 + x) as usize] = 1;
                    }
                }
            }
        }
        let inverse_sqrt_two = std::f32::consts::FRAC_1_SQRT_2;
        let key = ContourViewKey::from_plane(PlaneDefinition {
            family: PlaneFamily::Oblique,
            origin_mm: [1.5, 1.5, 1.5],
            u_axis_mm: [inverse_sqrt_two, inverse_sqrt_two, 0.0],
            v_axis_mm: [0.0, 0.0, 1.0],
            normal_mm: [inverse_sqrt_two, -inverse_sqrt_two, 0.0],
        });

        let build_status = ensure_contour_view_cache(&mut world, entity, &key);
        let status = request_contour_view_state(&world, entity, &key);

        assert_eq!(build_status.state, RepresentationRequestState::Current);
        assert_eq!(status.request.state, RepresentationRequestState::Current);
        assert!(!status.editable);
        assert!(status.promotable);

        let (cached_data, generation) = {
            let roi = world.get::<&Roi>(entity).unwrap();
            (
                roi.contour_view_cache(&key).unwrap().data.clone(),
                roi.dirty_state.generations.authoritative,
            )
        };
        assert!(cached_data.has_loops());
        promote_contour_view_to_authoritative(&mut world, entity, &key).unwrap();

        let roi = world.get::<&Roi>(entity).unwrap();
        assert_eq!(roi.contour_data(), Some(&cached_data));
        assert_eq!(
            roi.contour_data().unwrap().active_plane_family,
            PlaneFamily::Oblique
        );
        assert_eq!(roi.dirty_state.generations.authoritative, generation + 1);
        assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert!(roi.has_queued_job(RoiJobKind::RebuildVoxelCache));
        drop(roi);

        process_contour_voxel_rebuild_jobs(&mut world);
        let roi = world.get::<&Roi>(entity).unwrap();
        assert!(roi.is_cache_current(RoiCacheKind::Voxel));
        assert!(roi
            .voxel_cache()
            .unwrap()
            .data
            .raw_data
            .iter()
            .any(|value| *value != 0));
    }

    #[test]
    fn test_oblique_voxel_overlay_uses_same_cache_state_contract_as_orthogonal_views() {
        let mut world = World::new();
        let entity = spawn_sparse_voxel_roi(&mut world);
        {
            let mut roi = world.get::<&mut Roi>(entity).unwrap();
            roi.dirty_state.voxel_cache_dirty = false;
            roi.dirty_state.generations.voxel = roi.dirty_state.generations.authoritative;
        }

        let axial = request_viewport_voxel_overlay_state(&world, ViewMode::Axial, entity);
        let oblique = request_viewport_voxel_overlay_state(&world, ViewMode::Oblique, entity);

        assert_eq!(oblique, axial);
        assert_ne!(
            oblique.reason.as_deref(),
            Some("overlay_not_supported_in_oblique")
        );
    }

    #[test]
    fn test_replace_contour_data_marks_existing_derived_contour_views_stale() {
        let mut world = World::new();
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
        let key = ContourViewKey::from_plane(test_plane_definition(PlaneFamily::Coronal));
        {
            let mut roi = world.get::<&mut Roi>(entity).unwrap();
            let mut derived_data = roi.contour_data().unwrap().clone();
            derived_data.active_plane_family = PlaneFamily::Coronal;
            let gen = roi.dirty_state.generations.authoritative;
            roi.upsert_contour_view_cache(key.clone(), derived_data, gen, CacheViewState::Current);
        }
        replace_contour_data(
            &mut world,
            entity,
            ContourData {
                active_plane_family: PlaneFamily::Axial,
                slices: Vec::new(),
            },
        )
        .unwrap();
        let roi = world.get::<&Roi>(entity).unwrap();
        assert_eq!(
            roi.contour_view_cache(&key).unwrap().state,
            CacheViewState::Stale
        );
    }

    #[test]
    fn test_promotion_preserves_previous_active_family_as_stale_derived_cache() {
        let mut world = World::new();
        let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, true);
        seed_current_voxel_cache_for_contour_roi(&mut world, entity);
        let coronal_key = ContourViewKey::from_plane(test_plane_definition(PlaneFamily::Coronal));
        let previous_active_key = {
            let roi = world.get::<&Roi>(entity).unwrap();
            let contour = roi.contour_data().unwrap();
            ContourViewKey::from_plane(contour.slices[0].plane)
        };
        {
            let mut roi = world.get::<&mut Roi>(entity).unwrap();
            let gen = roi.dirty_state.generations.authoritative;
            let mut promoted_data = roi.contour_data().unwrap().clone();
            promoted_data.active_plane_family = PlaneFamily::Coronal;
            roi.upsert_contour_view_cache(
                coronal_key.clone(),
                promoted_data,
                gen,
                CacheViewState::Current,
            );
        }

        promote_contour_view_to_authoritative(&mut world, entity, &coronal_key).unwrap();
        let roi = world.get::<&Roi>(entity).unwrap();
        let preserved = roi
            .contour_view_cache(&previous_active_key)
            .expect("expected previous active family stale derived cache");
        assert_eq!(preserved.state, CacheViewState::Stale);
    }
}
