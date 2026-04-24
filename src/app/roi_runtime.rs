use crate::app::components::*;
use hecs::World;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoiCacheStatus {
    pub authoritative_generation: u64,
    pub cache_generation: u64,
    pub is_dirty: bool,
    pub is_current: bool,
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
}
