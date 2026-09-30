use super::*;

pub(super) struct MeshVoxelRebuildWork {
    source_generation: u64,
    rebuild: IncrementalMeshVoxelization,
    started_at: Instant,
    voxelization_duration: Duration,
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

pub(super) fn process_mesh_voxel_rebuild_jobs_with_context(
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

pub(super) fn process_mesh_voxel_rebuild_for_entity(
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

pub(super) fn resume_mesh_voxel_rebuild_work(
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
        discard_job(
            world,
            roi_entity,
            RoiJobKind::RebuildVoxelCache,
            work.started_at.elapsed(),
        );
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
        discard_job(
            world,
            roi_entity,
            RoiJobKind::RebuildVoxelCache,
            started_at.elapsed(),
        );
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
    report_roi_status(world, roi_entity, "Mesh voxel cache rebuilt.".to_string());
    true
}

pub(super) fn fail_mesh_voxel_rebuild(
    world: &mut World,
    roi_entity: hecs::Entity,
    reason: &'static str,
) {
    fail_job_marking_dirty(
        world,
        roi_entity,
        RoiJobKind::RebuildVoxelCache,
        RoiCacheKind::Voxel,
    );
    report_roi_status(
        world,
        roi_entity,
        format!("Mesh voxel rebuild failed: {reason}."),
    );
}
