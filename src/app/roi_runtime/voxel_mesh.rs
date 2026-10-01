use super::*;
use crate::convert::{
    changed_mesh_chunks, smooth_mesh_field_from_signed_distance, ChunkedMeshData, ContourFieldState,
};

/// Where the chunks of a mesh rebuild are sampled from.
enum MeshSource {
    Voxels(Box<VoxelData>),
    /// The signed distance field of a contour ROI (`convert::contour_field`), installed on the
    /// ROI together with the finished mesh so the next edit reuses its per-slice work.
    ContourField(Box<ContourFieldState>),
}

pub(super) struct VoxelMeshRebuildWork {
    source_generation: u64,
    source: MeshSource,
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
    let (source, rebuild_result) = if is_contour_roi(world, entity) {
        match begin_contour_field_rebuild(world, entity, base_chunks) {
            ContourFieldStart::Rebuild(state, rebuild) => {
                (MeshSource::ContourField(state), Ok(*rebuild))
            }
            ContourFieldStart::Empty => {
                install_empty_contour_mesh(world, entity, source_generation, started_at);
                return;
            }
            ContourFieldStart::Failed => {
                fail_job(world, entity, RoiJobKind::RebuildMeshCache);
                return;
            }
        }
    } else {
        let result = match (dirty_region, base_chunks) {
            (RoiDirtyRegion::VoxelAabb { min, max }, Some(chunks)) => {
                IncrementalChunkedMeshRebuild::begin_for_voxel_aabb(chunks, &voxel_data, min, max)
            }
            _ => IncrementalChunkedMeshRebuild::begin_full(&voxel_data, DEFAULT_MESH_CHUNK_SIZE),
        };
        (MeshSource::Voxels(Box::new(voxel_data)), result)
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
                source,
                rebuild,
                started_at,
            },
        )
        .is_err()
    {
        fail_job(world, entity, RoiJobKind::RebuildMeshCache);
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
        discard_job(
            world,
            entity,
            RoiJobKind::RebuildMeshCache,
            work.started_at.elapsed(),
        );
        return;
    }

    let mut completed = work.rebuild.is_complete();
    while !completed && frame_started_at.elapsed() < frame_budget {
        let stepped = match &work.source {
            MeshSource::Voxels(voxel_data) => work.rebuild.step(voxel_data),
            MeshSource::ContourField(_) => work.rebuild.step_field(),
        };
        completed = match stepped {
            Ok(done) => done,
            Err(error) => {
                fail_voxel_mesh_rebuild(world, entity, error);
                return;
            }
        };
    }
    if !completed {
        suspend_work(world, entity, RoiJobKind::RebuildMeshCache, work);
        return;
    }

    let duration = work.started_at.elapsed();
    let Some(chunked_mesh) = work.rebuild.into_result() else {
        return;
    };
    let mesh_data = chunked_mesh.merged_mesh();
    let field_state = match work.source {
        MeshSource::ContourField(state) => Some(state),
        MeshSource::Voxels(_) => None,
    };
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
    drop(roi);
    if let Some(state) = field_state {
        let _ = world.insert_one(entity, *state);
    }
}

fn is_contour_roi(world: &World, entity: hecs::Entity) -> bool {
    world
        .get::<&Roi>(entity)
        .is_ok_and(|roi| matches!(roi.body, RoiBody::Contour(_)))
}

enum ContourFieldStart {
    Rebuild(Box<ContourFieldState>, Box<IncrementalChunkedMeshRebuild>),
    /// The contours have no loops: the mesh is empty.
    Empty,
    Failed,
}

/// Builds the field of a contour ROI from the state of the previous revision (only changed slices
/// are recomputed) and plans the chunks to rebuild: those the changed samples can reach when the
/// previous mesh is on the same grid, else all.
fn begin_contour_field_rebuild(
    world: &mut World,
    entity: hecs::Entity,
    base_chunks: Option<ChunkedMeshData>,
) -> ContourFieldStart {
    let previous = world
        .get::<&ContourFieldState>(entity)
        .ok()
        .map(|state| (*state).clone());
    let Ok(roi) = world.get::<&Roi>(entity) else {
        return ContourFieldStart::Failed;
    };
    let Some(contour) = roi.contour_data() else {
        return ContourFieldStart::Failed;
    };
    let keep = previous.as_ref().map(|state| state.field.geometry);
    let state = match ContourFieldState::build(
        previous.as_ref(),
        contour,
        roi.reference_geometry(),
        keep,
    ) {
        Ok(Some(state)) => state,
        Ok(None) => return ContourFieldStart::Empty,
        Err(error) => {
            log::warn!("Contour field failed for ROI {entity:?}: {error:?}");
            return ContourFieldStart::Failed;
        }
    };
    drop(roi);
    let geometry = state.field.geometry;
    let smooth = smooth_mesh_field_from_signed_distance(geometry, &state.field.values);
    let result = match (base_chunks, previous.as_ref()) {
        (Some(chunks), Some(previous))
            if chunks.grid == geometry.identity()
                && previous.field.geometry.identity() == geometry.identity() =>
        {
            let changed = changed_mesh_chunks(
                geometry,
                &previous.field.values,
                &state.field.values,
                chunks.chunk_size,
            );
            IncrementalChunkedMeshRebuild::begin_changed_from_field(
                chunks, geometry, smooth, changed,
            )
        }
        _ => IncrementalChunkedMeshRebuild::begin_full_from_field(
            geometry,
            smooth,
            DEFAULT_MESH_CHUNK_SIZE,
        ),
    };
    match result {
        Ok(rebuild) => ContourFieldStart::Rebuild(Box::new(state), Box::new(rebuild)),
        Err(error) => {
            log::warn!("Contour mesh extraction failed for ROI {entity:?}: {error:?}");
            ContourFieldStart::Failed
        }
    }
}

fn install_empty_contour_mesh(
    world: &mut World,
    entity: hecs::Entity,
    source_generation: u64,
    started_at: Instant,
) {
    let _ = world.remove_one::<ContourFieldState>(entity);
    let Ok(mut roi) = world.get::<&mut Roi>(entity) else {
        return;
    };
    let installed = roi.install_mesh_cache_result(
        MeshCache {
            data: MeshData {
                vertices: Vec::new(),
                faces: Vec::new(),
            },
            chunks: None,
        },
        source_generation,
    );
    if installed.is_err() {
        roi.finish_job(RoiJobKind::RebuildMeshCache);
        return;
    }
    roi.job_metrics.completed_count = roi.job_metrics.completed_count.saturating_add(1);
    roi.job_metrics.last_completed_kind = Some(RoiJobKind::RebuildMeshCache);
    roi.job_metrics.last_duration_ms = started_at.elapsed().as_secs_f32() * 1000.0;
}

pub(super) fn fail_voxel_mesh_rebuild(
    world: &mut World,
    entity: hecs::Entity,
    error: VoxelMeshExtractionError,
) {
    log::warn!("Voxel mesh rebuild failed for ROI {entity:?}: {error:?}");
    fail_job_marking_dirty(
        world,
        entity,
        RoiJobKind::RebuildMeshCache,
        RoiCacheKind::Mesh,
    );
}
