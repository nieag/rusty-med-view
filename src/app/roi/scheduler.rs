use crate::app::components::{
    Roi, RoiDirtyRegion, RoiJobKind, RoiJobPriority, RoiJobRequest, RoiJobState,
};
use web_time::Instant;

impl RoiJobState {
    pub fn running_kind(&self) -> Option<RoiJobKind> {
        self.running_request.map(|request| request.kind)
    }

    pub fn queued_kind(&self) -> Option<RoiJobKind> {
        self.pending.first().map(|request| request.kind)
    }
}

impl Roi {
    pub fn enqueue_rebuild(&mut self, kind: RoiJobKind) {
        self.enqueue_job(RoiJobRequest {
            kind,
            source_generation: self.dirty_state.authoritative.shape,
            preview_revision: None,
            priority: RoiJobPriority::VisibleCommitted,
            dirty_region: RoiDirtyRegion::Full,
        });
    }

    pub fn enqueue_job(&mut self, request: RoiJobRequest) {
        if self.job_state.running_request == Some(request) {
            return;
        }

        if let Some(index) = self
            .job_state
            .pending
            .iter()
            .position(|queued| queued.kind == request.kind)
        {
            let queued = self.job_state.pending[index];
            if queued.source_generation > request.source_generation
                || (queued.source_generation == request.source_generation
                    && queued.preview_revision > request.preview_revision)
            {
                return;
            }
            self.job_state.pending[index] = RoiJobRequest {
                dirty_region: merge_dirty_regions(queued.dirty_region, request.dirty_region),
                priority: higher_priority(queued.priority, request.priority),
                ..request
            };
        } else {
            self.job_state.pending.push(request);
            if self.job_state.oldest_pending_since.is_none() {
                self.job_state.oldest_pending_since = Some(Instant::now());
            }
        }
        if self.job_state.work_cycle_started_at.is_none() {
            self.job_state.work_cycle_started_at = Some(Instant::now());
        }
        self.job_state.pending.sort_by_key(|request| {
            (
                priority_rank(request.priority),
                dependency_rank(request.kind),
            )
        });
        self.job_metrics.max_queue_depth = self
            .job_metrics
            .max_queue_depth
            .max(self.job_state.pending.len());
    }

    pub fn start_queued_job(&mut self) -> Option<RoiJobKind> {
        if self.job_state.running_request.is_some() {
            return None;
        }
        let request = self.job_state.pending.first().copied()?;
        self.job_state.pending.remove(0);
        if let Some(queued_at) = self.job_state.oldest_pending_since.take() {
            self.job_metrics.last_queue_delay_ms = queued_at.elapsed().as_secs_f32() * 1000.0;
        }
        if !self.job_state.pending.is_empty() {
            self.job_state.oldest_pending_since = Some(Instant::now());
        }
        self.job_state.running_request = Some(request);
        Some(request.kind)
    }

    pub fn finish_job(&mut self, kind: RoiJobKind) {
        if self.job_state.running_kind() == Some(kind) {
            self.job_state.running_request = None;
        }
    }

    pub fn has_queued_job(&self, kind: RoiJobKind) -> bool {
        self.job_state
            .pending
            .iter()
            .any(|request| request.kind == kind)
    }

    pub fn running_job_kind(&self) -> Option<RoiJobKind> {
        self.job_state.running_kind()
    }

    pub fn queued_job_kind(&self) -> Option<RoiJobKind> {
        self.job_state.queued_kind()
    }
}

fn priority_rank(priority: RoiJobPriority) -> u8 {
    match priority {
        RoiJobPriority::InteractivePreview => 0,
        RoiJobPriority::VisibleCommitted => 1,
        RoiJobPriority::Background => 2,
    }
}

fn dependency_rank(kind: RoiJobKind) -> u8 {
    match kind {
        RoiJobKind::RebuildVoxelCache => 0,
        RoiJobKind::RebuildContourCache => 1,
        RoiJobKind::RebuildMeshCache => 2,
    }
}

fn higher_priority(left: RoiJobPriority, right: RoiJobPriority) -> RoiJobPriority {
    if priority_rank(left) <= priority_rank(right) {
        left
    } else {
        right
    }
}

fn merge_dirty_regions(left: RoiDirtyRegion, right: RoiDirtyRegion) -> RoiDirtyRegion {
    match (left, right) {
        (RoiDirtyRegion::Full, _) | (_, RoiDirtyRegion::Full) => RoiDirtyRegion::Full,
        (
            RoiDirtyRegion::VoxelAabb {
                min: left_min,
                max: left_max,
            },
            RoiDirtyRegion::VoxelAabb {
                min: right_min,
                max: right_max,
            },
        ) => RoiDirtyRegion::VoxelAabb {
            min: std::array::from_fn(|axis| left_min[axis].min(right_min[axis])),
            max: std::array::from_fn(|axis| left_max[axis].max(right_max[axis])),
        },
        (
            RoiDirtyRegion::MeshChunkAabb {
                min: left_min,
                max: left_max,
            },
            RoiDirtyRegion::MeshChunkAabb {
                min: right_min,
                max: right_max,
            },
        ) => RoiDirtyRegion::MeshChunkAabb {
            min: std::array::from_fn(|axis| left_min[axis].min(right_min[axis])),
            max: std::array::from_fn(|axis| left_max[axis].max(right_max[axis])),
        },
        (left, right) if left == right => left,
        _ => RoiDirtyRegion::Full,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::components::{ContourData, RoiId};
    use crate::convert::PlaneFamily;

    #[test]
    fn test_starting_queued_work_records_queue_delay() {
        let mut roi = Roi::new_contour(
            RoiId(1),
            "test".to_string(),
            ContourData {
                active_plane_family: PlaneFamily::Axial,
                slices: Vec::new(),
            },
        );
        roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);

        assert!(roi.job_state.oldest_pending_since.is_some());
        assert_eq!(roi.start_queued_job(), Some(RoiJobKind::RebuildVoxelCache));
        assert!(roi.job_state.oldest_pending_since.is_none());
        assert!(roi.job_metrics.last_queue_delay_ms >= 0.0);
    }
}
