use super::*;
use crate::convert::{
    changed_mesh_chunks, smooth_mesh_field_from_signed_distance, ChunkedMeshData,
    ContourFieldBuild, ContourFieldState,
};

/// Where the chunks of a mesh rebuild are sampled from.
enum MeshSource {
    Voxels(Box<VoxelData>),
    /// The signed distance field of a contour ROI (`convert::contour_field`), installed on the
    /// ROI together with the finished mesh so the next edit reuses its per-slice work.
    ContourField(Box<ContourFieldState>),
}

/// What a mesh build starts from exists: a contour ROI's loops, else a current voxel form.
pub(super) fn mesh_source_is_ready(roi: &Roi) -> bool {
    matches!(roi.body, RoiBody::Contour(_))
        || (roi.voxel_cache().is_some() && roi.is_cache_current(RoiCacheKind::Voxel))
}

/// A contour ROI's field while it is built (before its chunks are meshed), with the chunks of the
/// mesh it replaces.
struct FieldStage {
    build: Box<ContourFieldBuild>,
    base_chunks: Option<ChunkedMeshData>,
}

struct MeshingStage {
    source: MeshSource,
    rebuild: Box<IncrementalChunkedMeshRebuild>,
}

/// A mesh rebuild spread over frames: for a contour ROI first `field`, then `meshing`.
pub(super) struct VoxelMeshRebuildWork {
    source_generation: u64,
    field: Option<FieldStage>,
    meshing: Option<MeshingStage>,
    started_at: Instant,
}

pub(crate) fn process_voxel_mesh_rebuild_jobs(world: &mut World) {
    // A committed mesh is what the user waits for after an edit or a switch; a drag preview has
    // its own, smaller budget.
    const FRAME_JOB_BUDGET: Duration = Duration::from_millis(8);
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
            // The mesh job must be the next one in line: starting the queue would otherwise
            // start a voxel job queued ahead of it, which nothing here would ever run.
            && roi.job_state.queued_kind() == Some(RoiJobKind::RebuildMeshCache)
            && mesh_source_is_ready(roi))
        .then_some(entity)
    });
    let Some(entity) = entity else {
        warm_contour_fields(world, frame_started_at, FRAME_JOB_BUDGET);
        return;
    };
    let (source_generation, voxel_data, base_chunks) = {
        let roi = world.get::<&Roi>(entity).expect("queued ROI must exist");
        (
            roi.dirty_state.authoritative.shape,
            // A contour ROI meshes from its own loops and needs no voxels.
            roi.voxel_cache().map(|cache| cache.data.clone()),
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
    let (field, meshing) = if is_contour_roi(world, entity) {
        match begin_contour_field_build(world, entity) {
            Some(build) => (
                Some(FieldStage {
                    build: Box::new(build),
                    base_chunks,
                }),
                None,
            ),
            None => {
                install_empty_contour_mesh(world, entity, source_generation, started_at);
                return;
            }
        }
    } else {
        let Some(voxel_data) = voxel_data else {
            fail_job(world, entity, RoiJobKind::RebuildMeshCache);
            return;
        };
        let result = match (dirty_region, base_chunks) {
            (RoiDirtyRegion::VoxelAabb { min, max }, Some(chunks)) => {
                IncrementalChunkedMeshRebuild::begin_for_voxel_aabb(chunks, &voxel_data, min, max)
            }
            _ => IncrementalChunkedMeshRebuild::begin_full(&voxel_data, DEFAULT_MESH_CHUNK_SIZE),
        };
        match result {
            Ok(rebuild) => (
                None,
                Some(MeshingStage {
                    source: MeshSource::Voxels(Box::new(voxel_data)),
                    rebuild: Box::new(rebuild),
                }),
            ),
            Err(error) => {
                fail_voxel_mesh_rebuild(world, entity, error);
                return;
            }
        }
    };
    if world
        .insert_one(
            entity,
            VoxelMeshRebuildWork {
                source_generation,
                field,
                meshing,
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
            && (matches!(roi.body, RoiBody::Contour(_))
                || roi.is_cache_current(RoiCacheKind::Voxel))
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

    if let Some(mut stage) = work.field.take() {
        let deadline = frame_started_at + frame_budget;
        if !stage.build.step(Some(deadline)) {
            work.field = Some(stage);
            suspend_work(world, entity, RoiJobKind::RebuildMeshCache, work);
            return;
        }
        match plan_contour_meshing(stage) {
            Ok(meshing) => work.meshing = Some(meshing),
            Err(error) => {
                fail_voxel_mesh_rebuild(world, entity, error);
                return;
            }
        }
    }
    let Some(mut meshing) = work.meshing.take() else {
        fail_job(world, entity, RoiJobKind::RebuildMeshCache);
        return;
    };
    let mut completed = meshing.rebuild.is_complete();
    while !completed && frame_started_at.elapsed() < frame_budget {
        let stepped = match &meshing.source {
            MeshSource::Voxels(voxel_data) => meshing.rebuild.step(voxel_data),
            MeshSource::ContourField(_) => meshing.rebuild.step_field(),
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
        work.meshing = Some(meshing);
        suspend_work(world, entity, RoiJobKind::RebuildMeshCache, work);
        return;
    }

    let duration = work.started_at.elapsed();
    let Some(chunked_mesh) = meshing.rebuild.into_result() else {
        return;
    };
    let mesh_data = chunked_mesh.merged_mesh();
    let field_state = match meshing.source {
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

/// A contour field built ahead of the first edit, with the revision it is built for.
struct FieldWarmup {
    generation: u64,
    build: Box<ContourFieldBuild>,
}

/// Marks the revision a warm-up was tried for, so one that cannot finish is not retried forever.
struct FieldWarmupTried(u64);

/// Builds the field of an idle contour ROI that has none (a mesh just cut into contours keeps its
/// exact surface and has no field), a little each frame, so the first edit updates it instead of
/// building all of it. The surface shown is not touched; only the per-slice work is kept.
fn warm_contour_fields(world: &mut World, frame_started_at: Instant, frame_budget: Duration) {
    // A field is only useful to a contour ROI; drop it when the ROI is something else.
    let stale: Vec<hecs::Entity> = world
        .query::<(&Roi, &ContourFieldState)>()
        .iter()
        .filter(|(_, (roi, _))| !matches!(roi.body, RoiBody::Contour(_)))
        .map(|(entity, _)| entity)
        .collect();
    for entity in stale {
        let _ = world.remove_one::<ContourFieldState>(entity);
    }

    let running = world
        .query::<&FieldWarmup>()
        .iter()
        .map(|(entity, _)| entity)
        .next();
    let entity = match running {
        Some(entity) => entity,
        None => {
            let candidate = world.query::<&Roi>().iter().find_map(|(entity, roi)| {
                let generation = roi.dirty_state.authoritative.shape;
                (roi.contour_data()
                    .is_some_and(|contour| contour.has_loops())
                    && roi.running_job_kind().is_none()
                    && roi.job_state.queued_kind().is_none()
                    && !roi.preview_state.active
                    && roi.is_cache_current(RoiCacheKind::Mesh)
                    && roi.mesh_cache().is_some_and(|cache| cache.chunks.is_none())
                    && world.get::<&ContourFieldState>(entity).is_err()
                    && world
                        .get::<&FieldWarmupTried>(entity)
                        .map_or(true, |tried| tried.0 != generation))
                .then_some((entity, generation))
            });
            let Some((entity, generation)) = candidate else {
                return;
            };
            let _ = world.insert_one(entity, FieldWarmupTried(generation));
            let Some(build) = begin_contour_field_build(world, entity) else {
                return;
            };
            let warmup = FieldWarmup {
                generation,
                build: Box::new(build),
            };
            if world.insert_one(entity, warmup).is_err() {
                return;
            }
            entity
        }
    };
    let Ok(mut warmup) = world.remove_one::<FieldWarmup>(entity) else {
        return;
    };
    let still_idle = world.get::<&Roi>(entity).is_ok_and(|roi| {
        roi.dirty_state.authoritative.shape == warmup.generation
            && roi.running_job_kind().is_none()
            && roi.job_state.queued_kind().is_none()
    });
    if !still_idle {
        return;
    }
    if warmup.build.step(Some(frame_started_at + frame_budget)) {
        let (state, _) = warmup.build.finish();
        let _ = world.insert_one(entity, state);
    } else {
        let _ = world.insert_one(entity, warmup);
    }
}

fn is_contour_roi(world: &World, entity: hecs::Entity) -> bool {
    world
        .get::<&Roi>(entity)
        .is_ok_and(|roi| matches!(roi.body, RoiBody::Contour(_)))
}

/// Plans the field of a contour ROI from the state of the previous revision, so only changed
/// slices are recomputed. `None` when the contours have no loops (the mesh is empty) or the
/// field cannot be planned.
fn begin_contour_field_build(world: &World, entity: hecs::Entity) -> Option<ContourFieldBuild> {
    let previous = world.get::<&ContourFieldState>(entity).ok();
    let roi = world.get::<&Roi>(entity).ok()?;
    let contour = roi.contour_data()?;
    let keep = previous.as_ref().map(|state| state.field.geometry);
    match ContourFieldBuild::begin(previous.as_deref(), contour, roi.reference_geometry(), keep) {
        Ok(build) => build,
        Err(error) => {
            log::warn!("Contour field failed for ROI {entity:?}: {error:?}");
            None
        }
    }
}

/// The finished field's chunks to mesh: those the changed samples can reach when the previous
/// mesh is on the same grid, else all.
fn plan_contour_meshing(stage: FieldStage) -> Result<MeshingStage, VoxelMeshExtractionError> {
    let (state, previous) = stage.build.finish();
    let geometry = state.field.geometry;
    let smooth = smooth_mesh_field_from_signed_distance(geometry, &state.field.values);
    let rebuild = match (stage.base_chunks, previous.as_ref()) {
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
            )?
        }
        _ => IncrementalChunkedMeshRebuild::begin_full_from_field(
            geometry,
            smooth,
            DEFAULT_MESH_CHUNK_SIZE,
        )?,
    };
    Ok(MeshingStage {
        source: MeshSource::ContourField(Box::new(state)),
        rebuild: Box::new(rebuild),
    })
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
