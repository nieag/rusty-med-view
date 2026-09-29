use crate::app::components::*;
pub use crate::app::roi::authority::{ContourMutationError, MeshMutationError};
pub use crate::app::roi::history::RoiEditHistoryError;
use crate::app::roi::label_import::{
    check_label_import_budget, label_color, label_roi_name, present_label_ids, split_labelmap,
    LabelMask,
};
pub use crate::app::roi::requests::{
    ContourRepresentationStatus, RepresentationRequestState, RepresentationRequestStatus,
    RoiCacheStatus,
};
#[cfg(test)]
use crate::convert::PlaneDefinition;
use crate::convert::{
    contour_geometry_voxel_aabb, contour_slices_voxel_aabb, extract_contour_slice_from_voxel_data,
    intersect_mesh_with_plane, rasterize_contour_preview_slices_to_voxel_data,
    rasterize_contours_to_voxel_data, IncrementalChunkedMeshRebuild, IncrementalMeshVoxelization,
    PlaneFamily, VoxelContourExtractionError, VoxelMeshExtractionError, DEFAULT_MESH_CHUNK_SIZE,
};
#[cfg(test)]
use crate::convert::{extract_contours_from_voxel_data, extract_mesh_from_voxel_data};
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

struct MeshVoxelRebuildWork {
    source_generation: u64,
    rebuild: IncrementalMeshVoxelization,
    started_at: Instant,
    voxelization_duration: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoxelRoiStats {
    pub occupied_voxels: u64,
    pub volume_mm3: f32,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoxelContourCreationError {
    MissingRoi,
    NotVoxelRoi,
    ExtractionFailed(VoxelContourExtractionError),
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoxelMeshCreationError {
    MissingRoi,
    NotVoxelRoi,
    EmptyMeshFromNonEmptySource,
    ExtractionFailed(VoxelMeshExtractionError),
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContourMeshCreationError {
    MissingRoi,
    NotContourRoi,
    MissingCurrentVoxelCache,
    ExtractionFailed(VoxelMeshExtractionError),
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayVoxelSourceError {
    MissingRoi,
    NotVoxelRoi,
}

pub const MAX_SIMULTANEOUS_ROI_OVERLAYS: usize =
    crate::render::roi_views::DEFAULT_MAX_VOXEL_OVERLAYS;

#[cfg(test)]
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

/// GPU resources available while advancing ROI-derived work for one frame.
pub struct RoiWorkGpuContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub bind_groups: BindGroupResources<'a>,
}

/// Whether the ROI runtime needs another frame to finish queued work.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RoiWorkStatus {
    pub pending: bool,
}

/// Advances all demanded ROI work in the only supported frame order.
///
/// Conversion processors remain concrete functions during the migration, but callers no longer
/// choose their order. The returned pending flag must drive another redraw independently of GUI
/// repaint requests.
pub fn advance_roi_work(world: &mut World, gpu: Option<&RoiWorkGpuContext<'_>>) -> RoiWorkStatus {
    let active_roi = world
        .query::<&EditorState>()
        .iter()
        .next()
        .and_then(|(_, editor)| editor.active_roi);

    if let Some(gpu) = gpu {
        process_contour_voxel_rebuild_jobs_with_gpu(
            gpu.device,
            gpu.queue,
            world,
            &gpu.bind_groups,
            active_roi,
        );
        process_mesh_voxel_rebuild_jobs_with_gpu(
            gpu.device,
            gpu.queue,
            world,
            &gpu.bind_groups,
            active_roi,
        );
    } else {
        process_contour_voxel_rebuild_jobs(world);
        process_mesh_voxel_rebuild_jobs(world);
    }
    let installed_voxel_bodies =
        process_voxel_body_install_jobs(world, gpu.map(|gpu| (gpu.device, gpu.queue)));
    if let (true, Some(gpu)) = (installed_voxel_bodies, gpu) {
        recreate_scene_bind_groups(gpu.device, world, &gpu.bind_groups, active_roi);
    }
    for (_, outcome) in crate::app::roi::complete_pending_switches(world) {
        set_runtime_status_message(
            world,
            match outcome {
                Ok(report) => report.message(),
                Err(error) => error.message(),
            },
        );
    }

    // Demand is resolved after voxel-producing work so a newly current voxel cache can schedule
    // its mesh in this same frame.
    sync_roi_contour_view_caches_for_viewports(world);
    sync_active_roi_mesh_cache_for_viewports(world);
    process_voxel_mesh_rebuild_jobs(world);
    record_completed_work_cycles(world);

    RoiWorkStatus {
        pending: world
            .query::<&Roi>()
            .iter()
            .any(|(_, roi)| roi.running_job_kind().is_some() || !roi.job_state.pending.is_empty()),
    }
}

fn record_completed_work_cycles(world: &mut World) {
    for (_, roi) in world.query_mut::<&mut Roi>() {
        if roi.job_state.running_request.is_none() && roi.job_state.pending.is_empty() {
            if let Some(started_at) = roi.job_state.work_cycle_started_at.take() {
                roi.job_metrics.last_work_convergence_ms =
                    started_at.elapsed().as_secs_f32() * 1000.0;
            }
        }
    }
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
    volume.geometry
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

fn build_contour_view_data_for_plane(
    voxel_data: &VoxelData,
    view_key: &ContourViewKey,
) -> Result<ContourData, VoxelContourExtractionError> {
    extract_contour_slice_from_voxel_data(voxel_data, view_key.plane)
}

pub(crate) fn ensure_contour_view_cache(
    world: &mut World,
    roi_entity: hecs::Entity,
    view_key: &ContourViewKey,
) -> RepresentationRequestStatus {
    let mesh_preview = crate::app::roi::preview::mesh_edit_preview_for_roi(world, roi_entity);
    let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) else {
        return RepresentationRequestStatus::blocked("roi_missing");
    };
    if let Some(contour) = roi.contour_data() {
        if contour.active_plane_family == view_key.family {
            return RepresentationRequestStatus::current();
        }
    }
    let mesh_preview_revision = roi.preview_state.revision;
    if matches!(&roi.body, RoiBody::Mesh(_)) {
        let mesh = mesh_preview
            .as_ref()
            .or_else(|| roi.mesh_data())
            .expect("mesh-authoritative ROI must expose mesh data");
        let state = if mesh_preview.is_some() {
            CacheViewState::Preview {
                revision: mesh_preview_revision,
            }
        } else {
            CacheViewState::Current
        };
        let built_from = roi.dirty_state.authoritative;
        if roi
            .contour_view_cache(view_key)
            .is_some_and(|cache| cache.built_from == built_from && cache.state == state)
        {
            return if mesh_preview.is_some() {
                RepresentationRequestStatus::preview("mesh_plane_intersection_preview")
            } else {
                RepresentationRequestStatus::current()
            };
        }
        return match intersect_mesh_with_plane(mesh, view_key.plane) {
            Ok(data) => {
                let install =
                    roi.install_contour_view_result(view_key.clone(), data, built_from, state);
                if install.is_err() {
                    RepresentationRequestStatus::stale("mesh_plane_intersection_superseded")
                } else if mesh_preview.is_some() {
                    RepresentationRequestStatus::preview("mesh_plane_intersection_preview")
                } else {
                    RepresentationRequestStatus::current()
                }
            }
            Err(_) => RepresentationRequestStatus::blocked("mesh_plane_intersection_failed"),
        };
    }
    let contour_preview_voxel = roi.session_caches.preview_voxel.as_ref().and_then(|cache| {
        (roi.preview_state.active
            && cache.source_generation == roi.dirty_state.authoritative.shape
            && cache.preview_revision == roi.preview_state.revision)
            .then(|| cache.data.clone())
    });
    if let Some(preview_voxel) = contour_preview_voxel {
        return match build_contour_view_data_for_plane(&preview_voxel, view_key) {
            Ok(data) => {
                let built_from = roi.dirty_state.authoritative;
                let revision = roi.preview_state.revision;
                let _ = roi.install_contour_view_result(
                    view_key.clone(),
                    data,
                    built_from,
                    CacheViewState::Preview { revision },
                );
                RepresentationRequestStatus::preview("contour_edit_cross_plane_preview")
            }
            Err(_) => {
                RepresentationRequestStatus::unsupported("contour_edit_preview_plane_unsupported")
            }
        };
    }
    if roi.running_job_kind() == Some(RoiJobKind::RebuildVoxelCache) {
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

    let built_from = roi.dirty_state.authoritative;
    if let Some(cache) = roi.contour_view_cache(view_key) {
        if cache.built_from == built_from && cache.state == CacheViewState::Current {
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
            match roi.install_current_contour_view_result(view_key.clone(), data, built_from) {
                Ok(()) => RepresentationRequestStatus::current(),
                Err(_) => RepresentationRequestStatus::stale("contour_view_result_superseded"),
            }
        }
        Err(
            VoxelContourExtractionError::UnsupportedPlaneFamily { .. }
            | VoxelContourExtractionError::UnsupportedPlaneGeometry,
        ) => {
            let built_from = roi.dirty_state.authoritative;
            let _ = roi.install_contour_view_result(
                view_key.clone(),
                ContourData {
                    active_plane_family: view_key.family,
                    slices: Vec::new(),
                },
                built_from,
                CacheViewState::Unsupported {
                    reason: "derived_contour_view_geometry_unsupported".to_string(),
                },
            );
            RepresentationRequestStatus::unsupported("derived_contour_view_geometry_unsupported")
        }
    }
}

pub(crate) fn sync_roi_contour_view_caches_for_viewports(world: &mut World) {
    let active_roi = world
        .query::<&EditorState>()
        .iter()
        .next()
        .and_then(|(_, editor)| editor.active_roi);
    let mut roi_entities = active_roi.into_iter().collect::<Vec<_>>();
    roi_entities.extend(world.query::<&Roi>().iter().filter_map(|(entity, roi)| {
        (Some(entity) != active_roi
            && roi.metadata.is_visible
            && matches!(roi.body, RoiBody::Mesh(_)))
        .then_some(entity)
    }));
    let main_geometry = main_volume_geometry(world);

    let cursor_uv = world
        .query::<&Transform>()
        .iter()
        .next()
        .map(|(_, transform)| transform.position)
        .unwrap_or([0.5, 0.5, 0.5]);

    for roi_entity in roi_entities {
        let geometry = main_geometry.or_else(|| {
            world
                .get::<&Roi>(roi_entity)
                .ok()
                .and_then(|roi| roi.voxel_cache().map(|cache| cache.data.geometry))
        });
        let Some(geometry) = geometry else {
            continue;
        };
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
            let _ = ensure_contour_view_cache(world, roi_entity, &view_key);
        }
    }
}

pub(crate) fn sync_active_roi_mesh_cache_for_viewports(world: &mut World) {
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
    if matches!(&roi.body, RoiBody::Mesh(_))
        || roi.is_cache_current(RoiCacheKind::Mesh)
        || roi.has_queued_job(RoiJobKind::RebuildMeshCache)
        || roi.running_job_kind() == Some(RoiJobKind::RebuildMeshCache)
    {
        return;
    }
    if roi.voxel_cache().is_some() && roi.is_cache_current(RoiCacheKind::Voxel) {
        roi.mark_cache_dirty(RoiCacheKind::Mesh);
        roi.enqueue_rebuild(RoiJobKind::RebuildMeshCache);
    }
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
    // The loader already validated the geometry (finite, non-singular affine), so the ROI
    // constructor cannot fail on it.
    let geometry = loaded_label.geometry;
    let geometry_matches_main = if let Some(main_geometry) = main_volume_geometry(world) {
        let differs = main_geometry.identity() != geometry.identity();
        if differs {
            log::warn!(
                "Loaded label geometry differs from main volume geometry; preserving label-owned geometry. label dims={:?} spacing={:?} origin={:?}, main dims={:?} spacing={:?} origin={:?}",
                geometry.dimensions,
                geometry.spacing(),
                geometry.origin(),
                main_geometry.dimensions,
                main_geometry.spacing(),
                main_geometry.origin(),
            );
        }
        !differs
    } else {
        true
    };

    Ok(VoxelRoiImportSpec {
        geometry,
        start_visible: visible_voxel_overlay_count(world) < MAX_SIMULTANEOUS_ROI_OVERLAYS,
        geometry_matches_main,
    })
}

/// Imports a labelmap as one voxel ROI per non-zero label, so structures such as a liver and its
/// tumor stay separate through every later conversion. A map with no labels becomes one empty ROI.
///
/// The first `MAX_SIMULTANEOUS_ROI_OVERLAYS` ROIs start visible; the rest are created hidden.
pub fn create_voxel_rois_from_label(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &mut World,
    loaded_label: &LoadedLabel,
) -> Result<Vec<hecs::Entity>, String> {
    let import_spec = prepare_voxel_roi_import(world, loaded_label)?;
    let masks = label_masks_for_import(&loaded_label.data)?;

    let placeholder_bg = world
        .query::<&GpuVolumeResources>()
        .with::<&MainVolumeTag>()
        .iter()
        .next()
        .map(|(_, res)| res.bind_group.clone())
        .ok_or_else(|| {
            "Cannot create a label ROI without an initialized main volume resource".to_string()
        })?;
    let dimensions = import_spec.geometry.dimensions();

    spawn_label_rois(
        world,
        import_spec.geometry,
        &loaded_label.filename,
        masks,
        |mask_bytes| {
            let (texture, view, sampler) = crate::io::volume::create_r8_texture_from_label_bytes(
                device,
                queue,
                dimensions,
                mask_bytes,
                "NIfTI Labelmap",
            )?;
            Ok(Some(GpuVolumeResources {
                texture,
                view,
                sampler,
                bind_group: placeholder_bg.clone(),
            }))
        },
    )
}

/// One mask per non-zero label, or a single all-zero mask for a map without labels. Fails when
/// the split would exceed the import memory budget.
fn label_masks_for_import(data: &[u8]) -> Result<Vec<LabelMask>, String> {
    let labels = present_label_ids(data);
    check_label_import_budget(data.len(), labels.len())?;
    if labels.is_empty() {
        return Ok(vec![LabelMask {
            label: 0,
            data: data.to_vec(),
        }]);
    }
    Ok(split_labelmap(data, &labels))
}

/// Spawns one ROI entity per mask. `gpu_for_mask` builds the GPU mirror for a mask (`None` when
/// there is no GPU), which keeps the world-building logic testable without a device.
fn spawn_label_rois(
    world: &mut World,
    geometry: VoxelGeometry,
    filename: &str,
    masks: Vec<LabelMask>,
    mut gpu_for_mask: impl FnMut(&[u8]) -> Result<Option<GpuVolumeResources>, String>,
) -> Result<Vec<hecs::Entity>, String> {
    let already_visible = visible_voxel_overlay_count(world);
    let label_count = masks.len();
    let mut entities = Vec::with_capacity(label_count);
    for (index, mask) in masks.into_iter().enumerate() {
        let gpu_resources = gpu_for_mask(&mask.data)?;
        let next_roi_id = world.query::<&Roi>().iter().count() as u64 + 1;
        let mut roi = Roi::new_voxel_with_cache(
            RoiId(next_roi_id),
            label_roi_name(filename, mask.label, label_count),
            geometry,
            mask.data,
            gpu_resources,
        );
        roi.metadata.is_visible = already_visible + index < MAX_SIMULTANEOUS_ROI_OVERLAYS;
        if mask.label != 0 {
            roi.metadata.color = label_color(mask.label);
        }
        entities.push(world.spawn((roi, LayerSettings { opacity: 0.5 }, RoiTag)));
    }
    Ok(entities)
}

pub fn create_empty_contour_roi(
    world: &mut World,
    editor_entity: hecs::Entity,
    active_plane_family: PlaneFamily,
) -> Result<hecs::Entity, String> {
    if world.get::<&EditorState>(editor_entity).is_err() {
        return Err("Missing editor state; contour ROI was not created.".to_string());
    }

    let reference_voxel_geometry = main_volume_geometry(world)
        .ok_or_else(|| "Missing main volume geometry; contour ROI was not created.".to_string())?;
    let reference_geometry = reference_voxel_geometry;
    let voxel_count = reference_voxel_geometry
        .dimensions
        .into_iter()
        .try_fold(1usize, |count, dimension| {
            count.checked_mul(dimension as usize)
        })
        .ok_or_else(|| "Contour ROI reference grid is too large.".to_string())?;

    let next_roi_id = world.query::<&Roi>().iter().count() as u64 + 1;
    let roi_name = format!("Contour ROI {}", next_roi_id);
    let entity = world.spawn((
        Roi::new_contour_with_geometry(
            RoiId(next_roi_id),
            roi_name,
            reference_geometry,
            ContourData {
                active_plane_family,
                slices: Vec::new(),
            },
        ),
        LayerSettings { opacity: 0.5 },
        RoiTag,
    ));

    if let Ok(mut roi) = world.get::<&mut Roi>(entity) {
        // The empty cache is valid for the initial empty contour authority. Its generation lets
        // the first changed-slice commit use the incremental slab rasterizer.
        let generation = roi.dirty_state.authoritative.shape;
        roi.install_voxel_cache_result(
            VoxelCache {
                data: VoxelData {
                    geometry: reference_voxel_geometry,
                    raw_data: vec![0; voxel_count],
                },
                gpu_resources: None,
            },
            generation,
        )
        .expect("new contour ROI cache must match its reference geometry");
    }

    let mut editor = world
        .get::<&mut EditorState>(editor_entity)
        .map_err(|_| "Missing editor state; contour ROI was not created.".to_string())?;
    editor.active_roi = Some(entity);
    Ok(entity)
}

#[cfg(test)]
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
    crate::app::roi::authority::replace_mesh_data(world, roi_entity, mesh)
}

#[cfg(test)]
pub fn begin_mesh_translation_preview(
    world: &mut World,
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
    crate::app::roi::preview::begin_mesh_edit_preview(world, roi_entity, mesh)
}

#[cfg(test)]
pub fn create_contour_roi_from_voxel_roi(
    world: &mut World,
    source_roi: hecs::Entity,
    family: PlaneFamily,
) -> Result<hecs::Entity, VoxelContourCreationError> {
    let source_voxel = {
        let roi = world
            .get::<&Roi>(source_roi)
            .map_err(|_| VoxelContourCreationError::MissingRoi)?;
        match &roi.body {
            RoiBody::Voxel(VoxelBody { data: voxel }) => voxel.clone(),
            RoiBody::Contour(_) | RoiBody::Mesh(_) => {
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
    let reference_geometry = source_voxel.geometry;

    let entity = world.spawn((
        Roi::new_contour_with_geometry(RoiId(next_roi_id), new_name, reference_geometry, extracted),
        LayerSettings { opacity: 0.5 },
        RoiTag,
    ));
    if let Ok(mut roi) = world.get::<&mut Roi>(entity) {
        // Preserve source voxel geometry as the initial contour reference frame.
        // This keeps extracted contour projection/edit mapping aligned before any
        // contour->voxel rebuild retargets caches to main-volume geometry.
        let generation = roi.dirty_state.authoritative.shape;
        let _ = roi.install_voxel_cache_result(
            VoxelCache {
                data: source_voxel,
                gpu_resources: None,
            },
            generation,
        );
    }
    Ok(entity)
}

#[cfg(test)]
pub fn create_mesh_roi_from_voxel_roi(
    world: &mut World,
    source_roi: hecs::Entity,
) -> Result<hecs::Entity, VoxelMeshCreationError> {
    let source_voxel =
        voxel_data_for_display_surface_extraction(world, source_roi).map_err(|err| match err {
            DisplayVoxelSourceError::MissingRoi => VoxelMeshCreationError::MissingRoi,
            DisplayVoxelSourceError::NotVoxelRoi => VoxelMeshCreationError::NotVoxelRoi,
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
        Roi::new_mesh_with_geometry(
            RoiId(next_roi_id),
            format!("{source_name} (Mesh)"),
            source_voxel.geometry,
            extracted,
        ),
        LayerSettings { opacity: 0.5 },
        RoiTag,
    ));
    if let Ok(mut roi) = world.get::<&mut Roi>(entity) {
        // Preserve source voxel geometry as extraction/provenance context.
        // Rendering projects mesh world-mm vertices through main display volume geometry.
        roi.store_stale_voxel_cache(VoxelCache {
            data: source_voxel,
            gpu_resources: None,
        });
    }
    Ok(entity)
}

#[cfg(test)]
pub fn voxel_data_for_display_surface_extraction(
    world: &World,
    source_roi: hecs::Entity,
) -> Result<VoxelData, DisplayVoxelSourceError> {
    let source_voxel = {
        let roi = world
            .get::<&Roi>(source_roi)
            .map_err(|_| DisplayVoxelSourceError::MissingRoi)?;
        match &roi.body {
            RoiBody::Voxel(VoxelBody { data: voxel }) => voxel.clone(),
            RoiBody::Contour(_) | RoiBody::Mesh(_) => {
                return Err(DisplayVoxelSourceError::NotVoxelRoi);
            }
        }
    };

    Ok(source_voxel)
}

#[cfg(test)]
pub fn create_mesh_roi_from_contour_roi(
    world: &mut World,
    source_roi: hecs::Entity,
) -> Result<hecs::Entity, ContourMeshCreationError> {
    let source_voxel = {
        let roi = world
            .get::<&Roi>(source_roi)
            .map_err(|_| ContourMeshCreationError::MissingRoi)?;
        if !matches!(&roi.body, RoiBody::Contour(_)) {
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
        Roi::new_mesh_with_geometry(
            RoiId(next_roi_id),
            format!("{source_name} (Mesh)"),
            source_voxel.geometry,
            extracted,
        ),
        LayerSettings { opacity: 0.5 },
        RoiTag,
    ));
    if let Ok(mut roi) = world.get::<&mut Roi>(entity) {
        // Preserve contour-derived voxel geometry as extraction/provenance context.
        // Rendering projects mesh world-mm vertices through main display volume geometry.
        roi.store_stale_voxel_cache(VoxelCache {
            data: source_voxel,
            gpu_resources: None,
        });
    }
    Ok(entity)
}

#[cfg(test)]
pub fn request_cache_rebuild(
    world: &mut World,
    roi_entity: hecs::Entity,
    kind: RoiCacheKind,
) -> Option<RoiJobKind> {
    let mut roi = world.get::<&mut Roi>(roi_entity).ok()?;
    roi.mark_cache_dirty(kind);
    let job_kind = kind.rebuild_job();
    roi.enqueue_rebuild(job_kind);
    Some(job_kind)
}

pub(crate) fn begin_next_job(world: &mut World, roi_entity: hecs::Entity) -> Option<RoiJobKind> {
    let mut roi = world.get::<&mut Roi>(roi_entity).ok()?;
    roi.start_queued_job()
}

#[cfg(test)]
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

pub(crate) fn process_contour_voxel_rebuild_jobs(world: &mut World) {
    let _ = process_contour_voxel_rebuild_jobs_with_hook(world, |_world, _entity| {}, None, None);
}

pub(crate) fn process_contour_voxel_rebuild_jobs_with_gpu(
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

pub(crate) fn process_mesh_voxel_rebuild_jobs(world: &mut World) {
    let _ = process_mesh_voxel_rebuild_jobs_with_context(world, None, None);
}

pub(crate) fn process_mesh_voxel_rebuild_jobs_with_gpu(
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

pub(crate) fn process_voxel_mesh_rebuild_jobs(world: &mut World) {
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
        (!matches!(roi.body, RoiBody::Mesh(_))
            && roi.running_job_kind().is_none()
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
            roi.dirty_state.authoritative.shape,
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
        roi.dirty_state.authoritative.shape == work.source_generation
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
    if roi.dirty_state.authoritative.shape != work.source_generation {
        roi.job_metrics.discarded_count = roi.job_metrics.discarded_count.saturating_add(1);
        roi.finish_job(RoiJobKind::RebuildMeshCache);
        return;
    }
    let cache_install_started_at = Instant::now();
    let installed = roi.install_mesh_cache_result(
        MeshCache {
            data: mesh_data,
            chunks: Some(chunked_mesh),
        },
        work.source_generation,
    );
    roi.job_metrics.last_cpu_cache_install_ms =
        cache_install_started_at.elapsed().as_secs_f32() * 1000.0;
    if installed.is_err() {
        roi.job_metrics.discarded_count = roi.job_metrics.discarded_count.saturating_add(1);
        roi.finish_job(RoiJobKind::RebuildMeshCache);
        return;
    }
    roi.job_metrics.completed_count = roi.job_metrics.completed_count.saturating_add(1);
    roi.job_metrics.last_completed_kind = Some(RoiJobKind::RebuildMeshCache);
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
    const FRAME_JOB_BUDGET: Duration = Duration::from_millis(4);
    let frame_started_at = Instant::now();
    let running_entity = {
        let mut query = world.query::<&MeshVoxelRebuildWork>();
        query.iter().map(|(entity, _)| entity).next()
    };
    if let Some(entity) = running_entity {
        return resume_mesh_voxel_rebuild_work(
            world,
            entity,
            upload_context,
            frame_started_at,
            FRAME_JOB_BUDGET,
        );
    }
    let mut entities = world
        .query::<&Roi>()
        .iter()
        .filter_map(|(entity, roi)| {
            (matches!(roi.body, RoiBody::Mesh(_))
                && roi.running_job_kind().is_none()
                && roi.has_queued_job(RoiJobKind::RebuildVoxelCache))
            .then_some(entity)
        })
        .collect::<Vec<_>>();
    if let Some(preferred) = preferred_roi {
        if let Some(index) = entities.iter().position(|entity| *entity == preferred) {
            entities.swap(0, index);
        }
    }
    entities.into_iter().take(1).any(|entity| {
        process_mesh_voxel_rebuild_for_entity(
            world,
            entity,
            upload_context,
            frame_started_at,
            FRAME_JOB_BUDGET,
        )
    })
}

fn process_mesh_voxel_rebuild_for_entity(
    world: &mut World,
    roi_entity: hecs::Entity,
    upload_context: Option<(&wgpu::Device, &wgpu::Queue)>,
    frame_started_at: Instant,
    frame_budget: Duration,
) -> bool {
    let (source_generation, mesh, target_geometry, prevalidated) = {
        let Ok(roi) = world.get::<&Roi>(roi_entity) else {
            return false;
        };
        let RoiBody::Mesh(MeshBody { data: mesh, .. }) = &roi.body else {
            return false;
        };
        let target_geometry = roi.voxel_cache().map(|cache| cache.data.geometry);
        (
            roi.dirty_state.authoritative.shape,
            mesh.clone(),
            target_geometry,
            roi.validated_mesh_generation == Some(roi.dirty_state.authoritative.shape),
        )
    };
    if begin_next_job(world, roi_entity) != Some(RoiJobKind::RebuildVoxelCache) {
        return false;
    }
    let started_at = Instant::now();
    let Some(target_geometry) = target_geometry else {
        fail_mesh_voxel_rebuild(world, roi_entity, "roi_reference_grid_missing");
        return false;
    };
    let voxelization_started_at = Instant::now();
    let rebuild = match if prevalidated {
        IncrementalMeshVoxelization::begin_prevalidated(&mesh, target_geometry)
    } else {
        IncrementalMeshVoxelization::begin(&mesh, target_geometry)
    } {
        Ok(work) => work,
        Err(error) => {
            log::warn!("Mesh voxel rebuild failed for ROI {roi_entity:?}: {error:?}");
            fail_mesh_voxel_rebuild(world, roi_entity, "mesh_voxelization_failed");
            return false;
        }
    };
    if world
        .insert_one(
            roi_entity,
            MeshVoxelRebuildWork {
                source_generation,
                rebuild,
                started_at,
                voxelization_duration: voxelization_started_at.elapsed(),
            },
        )
        .is_err()
    {
        fail_mesh_voxel_rebuild(world, roi_entity, "mesh_voxel_work_insert_failed");
        return false;
    }
    resume_mesh_voxel_rebuild_work(
        world,
        roi_entity,
        upload_context,
        frame_started_at,
        frame_budget,
    )
}

fn resume_mesh_voxel_rebuild_work(
    world: &mut World,
    roi_entity: hecs::Entity,
    upload_context: Option<(&wgpu::Device, &wgpu::Queue)>,
    frame_started_at: Instant,
    frame_budget: Duration,
) -> bool {
    let Ok(mut work) = world.remove_one::<MeshVoxelRebuildWork>(roi_entity) else {
        return false;
    };
    let is_current = world.get::<&Roi>(roi_entity).is_ok_and(|roi| {
        roi.dirty_state.authoritative.shape == work.source_generation
            && matches!(roi.body, RoiBody::Mesh(_))
            && roi.job_state.running_request.is_some_and(|request| {
                request.kind == RoiJobKind::RebuildVoxelCache
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
    let scan_started_at = Instant::now();
    let mut completed = false;
    while !completed && frame_started_at.elapsed() < frame_budget {
        completed = work.rebuild.step(512);
    }
    work.voxelization_duration += scan_started_at.elapsed();
    if !completed {
        if world.insert_one(roi_entity, work).is_err() {
            fail_mesh_voxel_rebuild(world, roi_entity, "mesh_voxel_work_insert_failed");
        }
        return false;
    }
    let voxelization_ms = work.voxelization_duration.as_secs_f32() * 1000.0;
    let voxel_data = work
        .rebuild
        .into_result()
        .expect("completed scan must yield voxels");
    let source_generation = work.source_generation;
    let started_at = work.started_at;

    if world
        .get::<&Roi>(roi_entity)
        .is_ok_and(|roi| roi.dirty_state.authoritative.shape != source_generation)
    {
        record_job_discarded(world, roi_entity, started_at.elapsed());
        if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
            roi.finish_job(RoiJobKind::RebuildVoxelCache);
        }
        return false;
    }

    let (gpu_resources, gpu_upload_ms) = if let Some((device, queue)) = upload_context {
        let Some(placeholder_bg) = main_volume_bind_group(world) else {
            fail_mesh_voxel_rebuild(world, roi_entity, "main_volume_bind_group_missing");
            return false;
        };
        let upload_started_at = Instant::now();
        let (texture, view, sampler) =
            match crate::io::volume::create_texture_from_voxel_data(device, queue, &voxel_data) {
                Ok(resources) => resources,
                Err(error) => {
                    log::warn!("Mesh voxel GPU upload failed for ROI {roi_entity:?}: {error}");
                    fail_mesh_voxel_rebuild(world, roi_entity, "mesh_voxel_gpu_upload_failed");
                    return false;
                }
            };
        (
            Some(GpuVolumeResources {
                texture,
                view,
                sampler,
                bind_group: placeholder_bg,
            }),
            Some(upload_started_at.elapsed().as_secs_f32() * 1000.0),
        )
    } else {
        (None, None)
    };

    let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) else {
        return false;
    };
    roi.job_metrics.last_mesh_voxelization_ms = voxelization_ms;
    if let Some(upload_ms) = gpu_upload_ms {
        roi.job_metrics.last_gpu_upload_ms = upload_ms;
    }
    let cache_install_started_at = Instant::now();
    let installed = roi.install_voxel_cache_result(
        VoxelCache {
            data: voxel_data,
            gpu_resources,
        },
        source_generation,
    );
    roi.job_metrics.last_cpu_cache_install_ms =
        cache_install_started_at.elapsed().as_secs_f32() * 1000.0;
    if installed.is_err() {
        roi.job_metrics.discarded_count = roi.job_metrics.discarded_count.saturating_add(1);
        roi.finish_job(RoiJobKind::RebuildVoxelCache);
        return false;
    }
    // Mesh-authority contours depend on the mesh, not this optional voxel cache.
    // Keep their current plane intersections intact across the resample.
    roi.job_metrics.completed_count = roi.job_metrics.completed_count.saturating_add(1);
    roi.job_metrics.last_completed_kind = Some(RoiJobKind::RebuildVoxelCache);
    roi.job_metrics.last_duration_ms = started_at.elapsed().as_secs_f32() * 1000.0;
    drop(roi);
    set_runtime_status_message(world, "Mesh voxel cache rebuilt.".to_string());
    true
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
        if !matches!(roi.body, RoiBody::Contour(_)) {
            continue;
        }
        if roi.running_job_kind().is_none() && roi.has_queued_job(RoiJobKind::RebuildVoxelCache) {
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
            && roi.dirty_state.authoritative.shape == work.source_generation
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
        || roi.dirty_state.authoritative.shape != source_generation
    {
        roi.job_metrics.discarded_count = roi.job_metrics.discarded_count.saturating_add(1);
        roi.finish_job(RoiJobKind::RebuildVoxelCache);
        return false;
    }
    if roi
        .install_preview_mesh_result(PreviewMeshCache {
            data: mesh_data,
            chunks: Some(chunked_mesh),
            dirty_voxel_aabb: preview_aabb,
            source_generation,
            preview_revision,
        })
        .is_err()
    {
        roi.job_metrics.discarded_count = roi.job_metrics.discarded_count.saturating_add(1);
        roi.finish_job(RoiJobKind::RebuildVoxelCache);
        return false;
    }
    roi.finish_job(RoiJobKind::RebuildVoxelCache);
    roi.job_metrics.completed_count = roi.job_metrics.completed_count.saturating_add(1);
    roi.job_metrics.last_completed_kind = Some(RoiJobKind::RebuildVoxelCache);
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
        let RoiBody::Contour(_) = &roi.body else {
            return false;
        };
        roi.dirty_state.authoritative.shape
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
        let preview = world.get::<&Roi>(roi_entity).ok().and_then(|roi| {
            roi.contour_move_preview()
                .map(|preview| preview.contour_data.clone())
        });
        let is_current_preview = world.get::<&Roi>(roi_entity).is_ok_and(|roi| {
            roi.preview_state.active
                && roi.preview_state.revision == revision
                && roi.dirty_state.authoritative.shape == authoritative_generation
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
        let RoiBody::Contour(ContourBody {
            data: contour_data, ..
        }) = &roi.body
        else {
            return false;
        };
        contour_data.clone()
    };
    // The slice-local rasterizer patches a retained cache, so it is only valid while that cache is
    // at most one generation behind. Otherwise every authoritative slice must be rasterized.
    let base_slice_voxel = dirty_slice_key.and_then(|_| {
        world.get::<&Roi>(roi_entity).ok().and_then(|roi| {
            let cache_generation = roi.dirty_state.voxel.built_from.shape;
            let authoritative_generation = roi.dirty_state.authoritative.shape;
            if cache_generation == authoritative_generation
                || cache_generation.saturating_add(1) == authoritative_generation
            {
                roi.voxel_cache().map(|cache| cache.data.clone())
            } else {
                None
            }
        })
    });
    if let (Some(slice_key), Some(_)) = (dirty_slice_key, base_slice_voxel.as_ref()) {
        contour_data
            .slices
            .retain(|slice| ContourSliceKey::from_plane(slice.plane) == slice_key);
    }

    let target_geometry = world
        .get::<&Roi>(roi_entity)
        .ok()
        .and_then(|roi| roi.voxel_cache().map(|cache| cache.data.geometry));
    let Some(target_geometry) = target_geometry else {
        log::warn!(
            "Skipping contour voxel rebuild for ROI {:?}: missing ROI reference grid",
            roi_entity
        );
        set_runtime_status_message(
            world,
            "Contour voxel rebuild failed: ROI reference geometry is unavailable.".to_string(),
        );
        fail_contour_voxel_rebuild(world, roi_entity);
        return false;
    };

    let raster_started_at = Instant::now();
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
    let slice_local_rebuild_lost_base = dirty_slice_key.is_some() && base_slice_voxel.is_none();
    let committed_mesh_dirty_region =
        if preview_revision.is_none() && !slice_local_rebuild_lost_base {
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
            || roi.dirty_state.authoritative.shape != authoritative_generation
        {
            roi.job_metrics.discarded_count = roi.job_metrics.discarded_count.saturating_add(1);
            roi.finish_job(RoiJobKind::RebuildVoxelCache);
            return false;
        }
        if roi
            .install_preview_voxel_result(PreviewVoxelCache {
                data: voxel_data.clone(),
                source_generation: authoritative_generation,
                preview_revision: revision,
            })
            .is_err()
        {
            roi.job_metrics.discarded_count = roi.job_metrics.discarded_count.saturating_add(1);
            roi.finish_job(RoiJobKind::RebuildVoxelCache);
            return false;
        }
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
        Ok(roi) => roi.dirty_state.authoritative.shape != authoritative_generation,
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

    let (gpu_resources, gpu_upload_ms) = if let Some((device, queue)) = upload_context {
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

        let upload_started_at = Instant::now();
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

        (
            Some(GpuVolumeResources {
                texture,
                view,
                sampler,
                bind_group: placeholder_bg,
            }),
            Some(upload_started_at.elapsed().as_secs_f32() * 1000.0),
        )
    } else {
        (None, None)
    };

    let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) else {
        return false;
    };
    roi.job_metrics.last_contour_raster_ms = raster_started_at.elapsed().as_secs_f32() * 1000.0;
    if let Some(upload_ms) = gpu_upload_ms {
        roi.job_metrics.last_gpu_upload_ms = upload_ms;
    }

    let cache_install_started_at = Instant::now();
    let installed = roi.install_voxel_cache_result(
        VoxelCache {
            data: voxel_data,
            gpu_resources,
        },
        authoritative_generation,
    );
    roi.job_metrics.last_cpu_cache_install_ms =
        cache_install_started_at.elapsed().as_secs_f32() * 1000.0;
    if installed.is_err() {
        roi.job_metrics.discarded_count = roi.job_metrics.discarded_count.saturating_add(1);
        roi.finish_job(RoiJobKind::RebuildVoxelCache);
        return false;
    }
    roi.mark_cache_dirty(RoiCacheKind::Mesh);
    roi.enqueue_job(RoiJobRequest {
        kind: RoiJobKind::RebuildMeshCache,
        source_generation: authoritative_generation,
        preview_revision: None,
        priority: RoiJobPriority::VisibleCommitted,
        dirty_region: committed_mesh_dirty_region,
    });
    roi.job_metrics.completed_count = roi.job_metrics.completed_count.saturating_add(1);
    roi.job_metrics.last_completed_kind = Some(RoiJobKind::RebuildVoxelCache);
    roi.job_metrics.last_duration_ms = started_at.elapsed().as_secs_f32() * 1000.0;
    drop(roi);
    set_runtime_status_message(world, "Contour voxel cache rebuilt.".to_string());
    true
}

/// A voxel ROI's voxel cache is its own body. It is rebuilt only when an undo restored a voxel
/// body: the cache is replaced by the body and uploaded to the GPU.
fn process_voxel_body_install_jobs(
    world: &mut World,
    upload_context: Option<(&wgpu::Device, &wgpu::Queue)>,
) -> bool {
    let entities = world
        .query::<&Roi>()
        .iter()
        .filter(|(_, roi)| {
            matches!(roi.body, RoiBody::Voxel(_))
                && roi.running_job_kind().is_none()
                && roi.has_queued_job(RoiJobKind::RebuildVoxelCache)
        })
        .map(|(entity, _)| entity)
        .collect::<Vec<_>>();
    let mut installed_any = false;
    for entity in entities {
        if begin_next_job(world, entity) != Some(RoiJobKind::RebuildVoxelCache) {
            continue;
        }
        let Some((data, generation)) = world.get::<&Roi>(entity).ok().and_then(|roi| {
            let RoiBody::Voxel(body) = &roi.body else {
                return None;
            };
            Some((body.data.clone(), roi.dirty_state.authoritative.shape))
        }) else {
            continue;
        };
        let gpu_resources = match upload_context {
            Some((device, queue)) => {
                let Some(bind_group) = main_volume_bind_group(world) else {
                    fail_contour_voxel_rebuild(world, entity);
                    continue;
                };
                match crate::io::volume::create_texture_from_voxel_data(device, queue, &data) {
                    Ok((texture, view, sampler)) => Some(GpuVolumeResources {
                        texture,
                        view,
                        sampler,
                        bind_group,
                    }),
                    Err(error) => {
                        set_runtime_status_message(
                            world,
                            format!("Voxel upload failed ({error})."),
                        );
                        fail_contour_voxel_rebuild(world, entity);
                        continue;
                    }
                }
            }
            None => None,
        };
        let Ok(mut roi) = world.get::<&mut Roi>(entity) else {
            continue;
        };
        if roi
            .install_voxel_cache_result(
                VoxelCache {
                    data,
                    gpu_resources,
                },
                generation,
            )
            .is_ok()
        {
            installed_any = true;
        } else {
            roi.finish_job(RoiJobKind::RebuildVoxelCache);
        }
    }
    installed_any
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

pub fn roi_voxel_stats(world: &World, roi_entity: hecs::Entity) -> Option<VoxelRoiStats> {
    let roi = world.get::<&Roi>(roi_entity).ok()?;
    let voxel_data = match &roi.body {
        RoiBody::Contour(_) | RoiBody::Mesh(_) => {
            if !roi.is_cache_current(RoiCacheKind::Voxel) {
                return None;
            }
            &roi.voxel_cache()?.data
        }
        RoiBody::Voxel(VoxelBody { data: voxel }) => voxel,
    };

    let occupied_voxels = voxel_data
        .raw_data
        .iter()
        .filter(|value| **value != 0)
        .count() as u64;

    let volume_scale_mm3 = voxel_data.geometry.spacing()[0]
        * voxel_data.geometry.spacing()[1]
        * voxel_data.geometry.spacing()[2];

    Some(VoxelRoiStats {
        occupied_voxels,
        volume_mm3: occupied_voxels as f32 * volume_scale_mm3,
    })
}

#[cfg(test)]
mod tests;
