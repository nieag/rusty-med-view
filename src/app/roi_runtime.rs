use crate::app::components::*;
pub use crate::app::roi::authority::{ContourMutationError, MeshMutationError};
pub use crate::app::roi::history::RoiEditHistoryError;
use crate::app::roi::label_import::{
    check_label_import_budget, label_color, label_roi_name, present_label_ids, split_labelmap,
    LabelMask,
};
use crate::app::roi::model::is_roi_visible;
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
use crate::model::OrthogonalFamily;
use crate::render::roi_views::{RenderRepresentationRequest, RoiRenderViews};
use hecs::World;
use web_time::{Duration, Instant};

mod contour_voxel;
mod create;
mod jobs;
mod mesh_voxel;
mod views;
mod voxel_mesh;
pub(crate) use self::contour_voxel::*;
pub use self::create::*;
use self::jobs::*;
use self::mesh_voxel::*;
pub use self::views::*;
use self::voxel_mesh::*;

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

/// What the user is looking at, which decides which derived forms are demanded first.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewFocus {
    /// The ROI being edited or inspected, if any.
    pub active_roi: Option<hecs::Entity>,
    /// The cursor in volume UV; it selects the slice every 2D view shows.
    pub cursor_uv: [f32; 3],
}

impl Default for ViewFocus {
    fn default() -> Self {
        Self {
            active_roi: None,
            cursor_uv: [0.5; 3],
        }
    }
}

/// What one `advance_roi_work` call found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoiWorkStatus {
    /// The ROI runtime needs another frame to finish queued work.
    pub pending: bool,
    /// Outcomes worth telling the user (a cache was rebuilt, a rebuild failed, a pending
    /// representation switch completed), oldest first.
    pub messages: Vec<String>,
}

/// Advances all demanded ROI work in the only supported frame order.
///
/// Conversion processors remain concrete functions during the migration, but callers no longer
/// choose their order. The returned pending flag must drive another redraw independently of GUI
/// repaint requests.
pub fn advance_roi_work(
    world: &mut World,
    gpu: Option<&RoiWorkGpuContext<'_>>,
    focus: &ViewFocus,
) -> RoiWorkStatus {
    let active_roi = focus.active_roi;
    let mut messages = Vec::new();

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
        messages.push(match outcome {
            Ok(report) => report.message(),
            Err(error) => error.message(),
        });
    }

    // Demand is resolved after voxel-producing work so a newly current voxel cache can schedule
    // its mesh in this same frame.
    sync_roi_contour_view_caches_for_viewports(world, focus);
    sync_active_roi_mesh_cache_for_viewports(world, focus.active_roi);
    process_voxel_mesh_rebuild_jobs(world);
    record_completed_work_cycles(world);

    let mut pending = false;
    for (_, roi) in world.query_mut::<&mut Roi>() {
        pending |= roi.running_job_kind().is_some() || !roi.job_state.pending.is_empty();
        messages.append(&mut roi.job_state.messages);
    }
    RoiWorkStatus { pending, messages }
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
            if let Some(res) = roi.renderable_voxel_cache(is_roi_visible(world, overlay.entity)) {
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
                        report_roi_status(world, entity, format!("Voxel upload failed ({error})."));
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

fn main_volume_bind_group(world: &World) -> Option<wgpu::BindGroup> {
    let query = world.query::<&GpuVolumeResources>();
    let mut with_tag = query.with::<&MainVolumeTag>();
    with_tag
        .iter()
        .next()
        .map(|(_, res)| res.bind_group.clone())
}

/// Queues a message about `roi_entity`'s work; `advance_roi_work` returns it to the caller.
fn report_roi_status(world: &mut World, roi_entity: hecs::Entity, message: String) {
    if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
        roi.job_state.messages.push(message);
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
