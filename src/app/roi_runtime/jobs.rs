//! Shared endings of the incremental ROI jobs: a job that goes stale is discarded, one that
//! cannot continue fails, and one that used up its frame budget is parked on its entity.

use super::*;

/// Counts a job whose result was thrown away, without ending it.
pub(super) fn record_job_discarded(world: &mut World, roi_entity: hecs::Entity, elapsed: Duration) {
    if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
        roi.job_metrics.discarded_count = roi.job_metrics.discarded_count.saturating_add(1);
        roi.job_metrics.last_duration_ms = elapsed.as_secs_f32() * 1000.0;
    }
}

/// Ends the running job of `kind` because its source changed while it ran.
pub(super) fn discard_job(
    world: &mut World,
    roi_entity: hecs::Entity,
    kind: RoiJobKind,
    elapsed: Duration,
) {
    record_job_discarded(world, roi_entity, elapsed);
    if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
        roi.finish_job(kind);
    }
}

/// Ends the running job of `kind` because it could not complete.
pub(super) fn fail_job(world: &mut World, roi_entity: hecs::Entity, kind: RoiJobKind) {
    if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
        roi.finish_job(kind);
        roi.job_metrics.failed_count = roi.job_metrics.failed_count.saturating_add(1);
    }
}

/// Like `fail_job`, and marks the cache the job was rebuilding dirty so it is scheduled again.
pub(super) fn fail_job_marking_dirty(
    world: &mut World,
    roi_entity: hecs::Entity,
    kind: RoiJobKind,
    cache: RoiCacheKind,
) {
    if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
        roi.mark_cache_dirty(cache);
    }
    fail_job(world, roi_entity, kind);
}

/// Parks unfinished `work` on the ROI until the next frame; fails the job if that is impossible.
pub(super) fn suspend_work<T: hecs::Component>(
    world: &mut World,
    roi_entity: hecs::Entity,
    kind: RoiJobKind,
    work: T,
) {
    if world.insert_one(roi_entity, work).is_err() {
        fail_job(world, roi_entity, kind);
    }
}
