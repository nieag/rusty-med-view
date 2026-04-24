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

pub fn create_voxel_roi_from_label(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &mut World,
    loaded_label: &LoadedLabel,
) -> hecs::Entity {
    let (new_texture, new_view, new_sampler) =
        crate::io::volume::create_texture_from_labelmap(device, queue, loaded_label);

    let placeholder_bg = world
        .query::<&GpuVolumeResources>()
        .iter()
        .next()
        .map(|(_, res)| res.bind_group.clone())
        .expect("Main volume should exist");

    let next_roi_id = world.query::<&Roi>().iter().count() as u64 + 1;
    world.spawn((
        Roi::new_voxel(
            RoiId(next_roi_id),
            loaded_label.filename.clone(),
            loaded_label.dimensions,
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
    ))
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

    let volume_scale_mm3 = world
        .query::<&VolumeData>()
        .with::<&MainVolumeTag>()
        .iter()
        .next()
        .map(|(_, volume)| volume.spacing[0] * volume.spacing[1] * volume.spacing[2])
        .unwrap_or(1.0);

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
            [4, 4, 4],
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
            [2, 2, 2],
            vec![0, 1, 2, 0, 0, 3, 4, 0],
            None,
        ),));

        let stats = voxel_roi_stats(&world, entity).unwrap();

        assert_eq!(stats.occupied_voxels, 4);
        assert!((stats.volume_mm3 - 2.0).abs() < f32::EPSILON);
    }
}
