use crate::app::components::*;
#[cfg(test)]
use crate::convert::PlaneDefinition;
use crate::convert::PlaneFamily;
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

pub fn voxel_roi_stats(world: &World, roi_entity: hecs::Entity) -> Option<VoxelRoiStats> {
    let roi = world.get::<&Roi>(roi_entity).ok()?;
    let voxel_data = match &roi.authoritative_data {
        RoiAuthoritativeData::Voxel(voxel) => voxel,
        RoiAuthoritativeData::Contour(_) | RoiAuthoritativeData::Mesh => return None,
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
}
