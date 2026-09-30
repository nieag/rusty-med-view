use super::*;

pub(super) struct VoxelMeshRebuildWork {
    source_generation: u64,
    voxel_data: VoxelData,
    rebuild: IncrementalChunkedMeshRebuild,
    started_at: Instant,
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

pub(super) fn resume_voxel_mesh_rebuild_work(
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

pub(super) fn fail_voxel_mesh_rebuild(
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
