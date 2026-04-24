use crate::app::components::*;
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

pub const MAX_SIMULTANEOUS_ROI_OVERLAYS: usize = 2;

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
    let mut overlay_views = Vec::new();

    {
        let query = world.query::<&GpuVolumeResources>();
        let mut with_tag = query.with::<&MainVolumeTag>();
        main_view = with_tag.iter().next().map(|(_, res)| res.view.clone());
    }

    {
        if let Some(active) = active_roi {
            if let Ok(roi) = world.get::<&Roi>(active) {
                if let Some(res) = roi.renderable_voxel_cache() {
                    overlay_views.push(res.view.clone());
                }
            }
        }

        let mut query = world.query::<&Roi>();
        for (entity, roi) in query.iter() {
            if Some(entity) == active_roi {
                continue;
            }
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
        orientation: volume.orientation,
    })
}

pub fn visible_roi_count(world: &World) -> usize {
    world
        .query::<&Roi>()
        .iter()
        .filter(|(_, roi)| roi.metadata.is_visible)
        .count()
}

pub fn can_enable_roi_visibility(world: &World, roi_entity: hecs::Entity) -> bool {
    if let Ok(roi) = world.get::<&Roi>(roi_entity) {
        if roi.metadata.is_visible {
            return true;
        }
    }

    visible_roi_count(world) < MAX_SIMULTANEOUS_ROI_OVERLAYS
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
}

pub fn prepare_voxel_roi_import(
    world: &World,
    loaded_label: &LoadedLabel,
) -> Result<VoxelRoiImportSpec, String> {
    if let Some(main_geometry) = main_volume_voxel_geometry(world) {
        if main_geometry.dimensions != loaded_label.dimensions
            || !approx_eq_slice(main_geometry.spacing, loaded_label.spacing, 1e-5)
            || !approx_eq_slice(main_geometry.orientation, loaded_label.orientation, 1e-5)
        {
            log::warn!(
                "Loaded label geometry differs from main volume geometry; label dims={:?} spacing={:?} orientation={:?}, main dims={:?} spacing={:?} orientation={:?}",
                loaded_label.dimensions,
                loaded_label.spacing,
                loaded_label.orientation,
                main_geometry.dimensions,
                main_geometry.spacing,
                main_geometry.orientation
            );
        }
    }

    Ok(VoxelRoiImportSpec {
        geometry: VoxelGeometry {
            dimensions: loaded_label.dimensions,
            spacing: loaded_label.spacing,
            orientation: loaded_label.orientation,
        },
        start_visible: visible_roi_count(world) < MAX_SIMULTANEOUS_ROI_OVERLAYS,
    })
}

pub fn create_voxel_roi_from_label(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &mut World,
    loaded_label: &LoadedLabel,
) -> Result<hecs::Entity, String> {
    let import_spec = prepare_voxel_roi_import(world, loaded_label)?;
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
        RoiAuthoritativeData::Contour | RoiAuthoritativeData::Mesh => return None,
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
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            vec![1; 64],
            None,
        ),))
    }

    fn spawn_main_volume(world: &mut World, spacing: [f32; 3]) {
        world.spawn((
            VolumeData {
                dimensions: [4, 4, 4],
                spacing,
                intensities: vec![],
                intensity_range: [0.0, 1.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            MainVolumeTag,
        ));
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
        spawn_main_volume(&mut world, [0.5, 0.5, 2.0]);
        let entity = world.spawn((Roi::new_voxel_with_cache(
            RoiId(2),
            "Mask".to_string(),
            VoxelGeometry {
                dimensions: [2, 2, 2],
                spacing: [0.5, 0.5, 2.0],
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
        spawn_main_volume(&mut world, [0.25, 0.5, 2.0]);

        let geometry = main_volume_voxel_geometry(&world).unwrap();

        assert_eq!(geometry.dimensions, [4, 4, 4]);
        assert_eq!(geometry.spacing, [0.25, 0.5, 2.0]);
        assert_eq!(geometry.orientation, [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn test_visible_roi_count_and_visibility_gate_respect_overlay_cap() {
        let mut world = World::new();
        let first = spawn_test_roi(&mut world);
        let second = spawn_test_roi(&mut world);
        let third = spawn_test_roi(&mut world);

        {
            let mut roi = world.get::<&mut Roi>(first).unwrap();
            roi.metadata.is_visible = true;
        }
        {
            let mut roi = world.get::<&mut Roi>(second).unwrap();
            roi.metadata.is_visible = true;
        }
        {
            let mut roi = world.get::<&mut Roi>(third).unwrap();
            roi.metadata.is_visible = false;
        }

        assert_eq!(visible_roi_count(&world), 2);
        assert!(can_enable_roi_visibility(&world, first));
        assert!(can_enable_roi_visibility(&world, second));
        assert!(!can_enable_roi_visibility(&world, third));
    }

    #[test]
    fn test_prepare_voxel_roi_import_uses_label_geometry_without_main_volume() {
        let world = World::new();
        let loaded_label = LoadedLabel {
            dimensions: [2, 2, 2],
            spacing: [1.25, 1.5, 2.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
            data: vec![0; 8],
            filename: "Label".to_string(),
        };

        let import_spec = prepare_voxel_roi_import(&world, &loaded_label).unwrap();
        assert_eq!(import_spec.geometry.dimensions, [2, 2, 2]);
        assert_eq!(import_spec.geometry.spacing, [1.25, 1.5, 2.0]);
        assert_eq!(import_spec.geometry.orientation, [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn test_prepare_voxel_roi_import_preserves_label_geometry_even_when_main_volume_differs() {
        let mut world = World::new();
        spawn_main_volume(&mut world, [0.5, 0.5, 2.0]);
        let loaded_label = LoadedLabel {
            dimensions: [3, 4, 5],
            spacing: [0.75, 0.8, 1.25],
            orientation: [0.0, 0.0, 1.0, 0.0],
            data: vec![0; 60],
            filename: "Label".to_string(),
        };

        let import_spec = prepare_voxel_roi_import(&world, &loaded_label).unwrap();

        assert_eq!(import_spec.geometry.dimensions, [3, 4, 5]);
        assert_eq!(import_spec.geometry.spacing, [0.75, 0.8, 1.25]);
        assert_eq!(import_spec.geometry.orientation, [0.0, 0.0, 1.0, 0.0]);
    }
}
