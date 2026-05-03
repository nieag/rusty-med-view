use crate::app::components::*;
#[cfg(test)]
use crate::convert::PlaneDefinition;
use crate::convert::{
    extract_contours_from_voxel_data, rasterize_contours_to_voxel_data, PlaneFamily,
    VoxelContourExtractionError,
};
use hecs::World;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoiCacheStatus {
    pub authoritative_generation: u64,
    pub cache_generation: u64,
    pub is_dirty: bool,
    pub is_current: bool,
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
    NotContourRoi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoxelContourCreationError {
    MissingRoi,
    NotVoxelRoi,
    ExtractionFailed(VoxelContourExtractionError),
}

pub const MAX_SIMULTANEOUS_ROI_OVERLAYS: usize = 2;

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
    let overlay1_view = overlay_views.first().unwrap_or(resources.dummy_view);
    let overlay2_view = overlay_views.get(1).unwrap_or(resources.dummy_view);

    let new_bind_group = crate::render::pipeline::create_scene_bind_group(
        device,
        resources.layout,
        &crate::render::pipeline::SceneTextureViews {
            volume_view: main_view_ref,
            volume_sampler: resources.dummy_sampler,
            uniform_buffer: resources.uniform_buffer,
            overlay1_view,
            overlay1_lut: resources.default_lut_view,
            overlay2_view,
            overlay2_lut: resources.default_lut_view,
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

pub fn main_volume_voxel_geometry(world: &World) -> Option<VoxelGeometry> {
    let mut query = world.query::<&VolumeData>().with::<&MainVolumeTag>();
    let (_, volume) = query.iter().next()?;
    Some(VoxelGeometry {
        dimensions: volume.dimensions,
        spacing: volume.spacing,
        origin: volume.origin,
        orientation: volume.orientation,
    })
}

pub fn renderable_voxel_overlay_rois(
    world: &World,
    active_roi: Option<hecs::Entity>,
) -> Vec<RenderableVoxelOverlay> {
    let mut overlays = Vec::new();

    if let Some(active) = active_roi {
        if let (Ok(roi), Ok(settings)) = (
            world.get::<&Roi>(active),
            world.get::<&LayerSettings>(active),
        ) {
            if roi.renderable_voxel_cache().is_some() {
                overlays.push(RenderableVoxelOverlay {
                    entity: active,
                    opacity: settings.opacity,
                });
            }
        }
    }

    let mut query = world.query::<(&Roi, &LayerSettings)>();
    for (entity, (roi, settings)) in query.iter() {
        if Some(entity) == active_roi {
            continue;
        }
        if roi.renderable_voxel_cache().is_none() {
            continue;
        }
        overlays.push(RenderableVoxelOverlay {
            entity,
            opacity: settings.opacity,
        });
        if overlays.len() >= MAX_SIMULTANEOUS_ROI_OVERLAYS {
            break;
        }
    }

    overlays.truncate(MAX_SIMULTANEOUS_ROI_OVERLAYS);
    overlays
}

pub fn visible_voxel_overlay_count(world: &World) -> usize {
    renderable_voxel_overlay_rois(world, None).len()
}

pub fn can_enable_roi_visibility(world: &World, roi_entity: hecs::Entity) -> bool {
    if let Ok(roi) = world.get::<&Roi>(roi_entity) {
        if roi.metadata.is_visible {
            return true;
        }
        if !matches!(roi.authoritative_data, RoiAuthoritativeData::Voxel(_)) {
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
        RoiAuthoritativeData::Voxel(_) | RoiAuthoritativeData::Mesh => {
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
        RoiAuthoritativeData::Voxel(_) | RoiAuthoritativeData::Mesh => {
            return Err(ContourMutationError::NotContourRoi);
        }
    }

    roi.mark_contour_authoritative_changed();
    roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
    Ok(())
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
            RoiAuthoritativeData::Contour(_) | RoiAuthoritativeData::Mesh => {
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
    let new_name = format!("{source_name} (Contour)");

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
    }
    Ok(entity)
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
        roi.job_state.queued = Some(RoiJobKind::RebuildVoxelCache);
    }
}

pub fn process_contour_voxel_rebuild_jobs(world: &mut World) {
    let _ = process_contour_voxel_rebuild_jobs_with_hook(world, |_world, _entity| {}, None);
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
    );
    if rebuilt_any {
        recreate_scene_bind_groups(device, world, resources, active_roi);
    }
}

fn process_contour_voxel_rebuild_jobs_with_hook(
    world: &mut World,
    mut before_commit: impl FnMut(&mut World, hecs::Entity),
    upload_context: Option<(&wgpu::Device, &wgpu::Queue)>,
) -> bool {
    let mut rebuilt_any = false;
    let mut rebuild_entities = Vec::new();
    for (entity, roi) in world.query::<&Roi>().iter() {
        if !matches!(roi.authoritative_data, RoiAuthoritativeData::Contour(_)) {
            continue;
        }
        if roi.job_state.running.is_none()
            && roi.job_state.queued == Some(RoiJobKind::RebuildVoxelCache)
        {
            rebuild_entities.push(entity);
        }
    }

    for entity in rebuild_entities {
        rebuilt_any |= process_contour_voxel_rebuild_for_entity(
            world,
            entity,
            &mut before_commit,
            upload_context,
        );
    }
    rebuilt_any
}

fn process_contour_voxel_rebuild_for_entity(
    world: &mut World,
    roi_entity: hecs::Entity,
    before_commit: &mut impl FnMut(&mut World, hecs::Entity),
    upload_context: Option<(&wgpu::Device, &wgpu::Queue)>,
) -> bool {
    let (authoritative_generation, contour_data) = {
        let Ok(roi) = world.get::<&Roi>(roi_entity) else {
            return false;
        };
        let RoiAuthoritativeData::Contour(contour_data) = &roi.authoritative_data else {
            return false;
        };
        (
            roi.dirty_state.generations.authoritative,
            contour_data.clone(),
        )
    };

    if begin_next_job(world, roi_entity) != Some(RoiJobKind::RebuildVoxelCache) {
        return false;
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

    let voxel_data = match rasterize_contours_to_voxel_data(&contour_data, target_geometry) {
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
    let voxel_data = match (&roi.primary_representation, &roi.authoritative_data) {
        (PrimaryRepresentation::Contour, RoiAuthoritativeData::Contour(_)) => {
            if !roi.is_cache_current(RoiCacheKind::Voxel) {
                return None;
            }
            &roi.voxel_cache()?.data
        }
        (_, RoiAuthoritativeData::Contour(_)) => return None,
        (_, RoiAuthoritativeData::Voxel(voxel)) => voxel,
        (_, RoiAuthoritativeData::Mesh) => return None,
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
    use crate::convert::orthogonal_plane_from_volume_uv;

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
        assert_eq!(roi.primary_representation, PrimaryRepresentation::Contour);
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
        assert_eq!(roi.primary_representation, PrimaryRepresentation::Voxel);
    }

    #[test]
    fn test_create_contour_roi_from_voxel_roi_returns_contour_primary_roi() {
        let mut world = World::new();
        let source = spawn_test_roi(&mut world);

        let created = create_contour_roi_from_voxel_roi(&mut world, source, PlaneFamily::Axial)
            .expect("extraction should create contour roi");

        let roi = world.get::<&Roi>(created).unwrap();
        assert_eq!(roi.primary_representation, PrimaryRepresentation::Contour);
        assert!(matches!(
            roi.authoritative_data,
            RoiAuthoritativeData::Contour(_)
        ));
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

        let roi = world.get::<&Roi>(extracted).unwrap();
        assert_eq!(roi.primary_representation, PrimaryRepresentation::Contour);
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
        assert_eq!(roi.job_state.queued, None);
        let voxel_cache = roi.voxel_cache().expect("voxel cache should exist");
        assert_eq!(voxel_cache.data.geometry.dimensions, [4, 4, 4]);
        assert!(voxel_cache.data.raw_data.iter().any(|v| *v != 0));
        assert!(voxel_cache.gpu_resources.is_none());
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
        assert_eq!(roi.job_state.queued, None);
        assert!(roi.is_cache_current(RoiCacheKind::Voxel));
        assert_eq!(
            roi.cache_generation(RoiCacheKind::Voxel),
            roi.dirty_state.generations.authoritative
        );
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
}
