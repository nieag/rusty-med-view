pub(crate) mod sample;
mod snapshot;

use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};

pub const QA_API_VERSION: u32 = 1;
const QA_LOG_CAPACITY: usize = 500;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QaLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct QaError {
    pub category: String,
    pub message: String,
    pub fields: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct QaLogEvent {
    pub seq: u64,
    pub frame: u64,
    pub level: QaLevel,
    pub category: String,
    pub message: String,
    pub fields: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct QaLogsSnapshot {
    pub events: Vec<QaLogEvent>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct QaSnapshot {
    pub qa: QaSnapshotQa,
    pub app: QaSnapshotApp,
    pub volume: QaSnapshotVolume,
    pub rois: Vec<QaSnapshotRoi>,
    pub viewports: Vec<QaSnapshotViewport>,
    pub render: QaSnapshotRender,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct QaSnapshotQa {
    pub enabled: bool,
    pub version: u32,
    pub requested_sample: Option<String>,
    pub requested_preset: Option<String>,
    pub sample_phase: QaSamplePhase,
    pub preset_phase: QaPresetPhase,
    pub readiness_blockers: Vec<String>,
    pub preset_applied_frame: Option<u64>,
    pub last_presented_frame: Option<u64>,
    pub active_qa_roi: Option<String>,
    pub ready: bool,
    pub last_error: Option<QaError>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct QaSnapshotApp {
    pub status: Option<String>,
    pub active_tool: Option<String>,
    pub active_roi_id: Option<u64>,
    pub active_roi_name: Option<String>,
    pub frame_counter: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct QaSnapshotVolume {
    pub loaded: bool,
    pub dimensions: [u32; 3],
    pub spacing: [f32; 3],
    pub origin: [f32; 3],
    pub orientation: [f32; 4],
    pub world_bounds: Option<[[f32; 3]; 2]>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct QaSnapshotRoi {
    pub id: u64,
    pub name: String,
    pub authority: String,
    pub visible: bool,
    pub active: bool,
    pub overlay_slot: Option<u32>,
    pub reference_geometry_dimensions: [u32; 3],
    pub reference_geometry_identity: String,
    pub voxel_cache_generation: u64,
    pub contour_cache_generation: u64,
    pub mesh_cache_generation: u64,
    pub voxel_cache_current: bool,
    pub contour_cache_current: bool,
    pub mesh_cache_current: bool,
    pub voxel_dimensions: Option<[u32; 3]>,
    pub non_empty_voxel_bounds: Option<[[u32; 3]; 2]>,
    pub preview_active: bool,
    pub preview_revision: u64,
    pub running_job: Option<String>,
    pub pending_jobs: Vec<String>,
    pub completed_job_count: u64,
    pub last_completed_job: Option<String>,
    pub discarded_job_count: u64,
    pub failed_job_count: u64,
    pub last_job_duration_ms: f32,
    pub last_queue_delay_ms: f32,
    pub last_contour_raster_ms: f32,
    pub last_mesh_voxelization_ms: f32,
    pub last_cpu_cache_install_ms: f32,
    pub last_gpu_upload_ms: f32,
    pub last_work_convergence_ms: f32,
    pub max_job_queue_depth: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct QaSnapshotViewport {
    pub mode: String,
    pub rect: [f32; 4],
    pub ready: bool,
    pub representation_requests: Vec<String>,
    pub voxel_cache_state: String,
    pub contour_view_cache_state: String,
    pub mesh_cache_state: String,
    pub contour_editable: bool,
    pub stale: bool,
    pub image_renderable: bool,
    pub overlay_renderable: bool,
    pub contour_renderable: bool,
    pub mesh_renderable: bool,
    pub volume_slice_in_bounds: Option<bool>,
    pub cursor_intersects_active_roi: Option<bool>,
    /// Anatomical letters of a 2D orthogonal view's left, right, top, and bottom edges, in that
    /// order (for example `"RLAP"`), derived from the main volume's affine.
    pub edge_letters: Option<String>,
    pub render_blockers: Vec<String>,
    pub readiness_blockers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct QaSnapshotRender {
    pub frame_counter: u64,
    pub last_presented_frame: Option<u64>,
    pub viewport_uniform_count: u32,
    pub overlay_slots_used: u32,
    pub overlay_slots_max: u32,
    pub contour_batch_count: u32,
    pub mesh_batch_count: u32,
    pub mesh_chunks_uploaded: u32,
    pub mesh_chunks_reused: u32,
    pub last_warning: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct QaMetricsSnapshot {
    pub frame_counter: u64,
    pub visible_rois: usize,
    pub overlay_slots_used: u32,
    pub overlay_slots_max: u32,
    pub warning_count: u64,
    pub error_count: u64,
}

#[derive(Debug, Clone)]
pub struct QaLogBuffer {
    cap: usize,
    next_seq: u64,
    events: VecDeque<QaLogEvent>,
}

impl QaLogBuffer {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            next_seq: 1,
            events: VecDeque::new(),
        }
    }

    pub fn push(
        &mut self,
        frame: u64,
        level: QaLevel,
        category: impl Into<String>,
        message: impl Into<String>,
        fields: BTreeMap<String, String>,
    ) -> QaLogEvent {
        let event = QaLogEvent {
            seq: self.next_seq,
            frame,
            level,
            category: category.into(),
            message: message.into(),
            fields,
        };
        self.next_seq += 1;
        self.events.push_back(event.clone());
        while self.events.len() > self.cap {
            let _ = self.events.pop_front();
        }
        event
    }

    pub fn snapshot(&self) -> QaLogsSnapshot {
        QaLogsSnapshot {
            events: self.events.iter().cloned().collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct QaRuntime {
    pub enabled: bool,
    pub requested_sample: Option<String>,
    pub requested_preset: Option<String>,
    pub log_buffer: QaLogBuffer,
    pub last_error: Option<QaError>,
    pub frame_counter: u64,
    pub sample_phase: QaSamplePhase,
    pub preset_phase: QaPresetPhase,
    pub preset_applied_frame: Option<u64>,
    pub last_presented_frame: Option<u64>,
    pub active_qa_roi: Option<String>,
    pub sample_bootstrap_started: bool,
    pub viewport_uniform_count: u32,
    pub contour_batch_count: u32,
    pub mesh_batch_count: u32,
    pub mesh_chunks_uploaded: u32,
    pub mesh_chunks_reused: u32,
    pub last_render_warning: Option<String>,
    pub last_render_error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QaSamplePhase {
    NotRequested,
    FetchingVolume,
    LoadingVolume,
    FetchingLabel,
    LoadingLabel,
    Loaded,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QaPresetPhase {
    NotRequested,
    PendingSample,
    Applying,
    Applied,
    Failed,
}

impl QaRuntime {
    pub fn new(
        enabled: bool,
        requested_sample: Option<String>,
        requested_preset: Option<String>,
    ) -> Self {
        Self {
            enabled,
            requested_sample,
            requested_preset,
            log_buffer: QaLogBuffer::new(QA_LOG_CAPACITY),
            last_error: None,
            frame_counter: 0,
            sample_phase: QaSamplePhase::NotRequested,
            preset_phase: QaPresetPhase::NotRequested,
            preset_applied_frame: None,
            last_presented_frame: None,
            active_qa_roi: None,
            sample_bootstrap_started: false,
            viewport_uniform_count: 0,
            contour_batch_count: 0,
            mesh_batch_count: 0,
            mesh_chunks_uploaded: 0,
            mesh_chunks_reused: 0,
            last_render_warning: None,
            last_render_error: None,
        }
    }

    pub fn ready(&self) -> bool {
        self.enabled
            && self.requested_sample.is_none()
            && self.requested_preset.is_none()
            && self.last_error.is_none()
    }

    pub fn log(
        &mut self,
        frame: u64,
        level: QaLevel,
        category: impl Into<String>,
        message: impl Into<String>,
        fields: BTreeMap<String, String>,
    ) -> QaLogEvent {
        self.log_buffer
            .push(frame, level, category, message, fields)
    }

    pub fn set_error(
        &mut self,
        category: impl Into<String>,
        message: impl Into<String>,
        fields: BTreeMap<String, String>,
    ) {
        self.last_error = Some(QaError {
            category: category.into(),
            message: message.into(),
            fields,
        });
    }

    pub fn logs_snapshot(&self) -> QaLogsSnapshot {
        self.log_buffer.snapshot()
    }
}

pub fn to_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_buffer_keeps_latest_500_events() {
        let mut buffer = QaLogBuffer::new(500);
        for idx in 0..550 {
            buffer.push(
                0,
                QaLevel::Info,
                "qa",
                format!("event-{idx}"),
                BTreeMap::new(),
            );
        }
        let snapshot = buffer.snapshot();
        assert_eq!(snapshot.events.len(), 500);
        assert_eq!(snapshot.events.first().map(|e| e.seq), Some(51));
        assert_eq!(snapshot.events.last().map(|e| e.seq), Some(550));
    }

    #[test]
    fn test_log_buffer_preserves_monotonic_sequence_after_drops() {
        let mut buffer = QaLogBuffer::new(2);
        buffer.push(0, QaLevel::Info, "qa", "a", BTreeMap::new());
        buffer.push(0, QaLevel::Info, "qa", "b", BTreeMap::new());
        buffer.push(0, QaLevel::Info, "qa", "c", BTreeMap::new());
        let snapshot = buffer.snapshot();
        assert_eq!(snapshot.events.len(), 2);
        assert_eq!(snapshot.events[0].seq, 2);
        assert_eq!(snapshot.events[1].seq, 3);
    }

    #[test]
    fn test_last_error_serializes() {
        let mut runtime = QaRuntime::new(true, None, None);
        let mut fields = BTreeMap::new();
        fields.insert("sample".to_string(), "liver_0".to_string());
        runtime.set_error("qa.sample", "missing sample", fields);
        let serialized = to_json(&runtime.last_error);
        assert!(serialized.contains("\"category\":\"qa.sample\""));
        assert!(serialized.contains("\"sample\":\"liver_0\""));
    }

    #[test]
    fn test_snapshot_uses_snake_case_levels() {
        let mut runtime = QaRuntime::new(true, None, None);
        runtime.log(1, QaLevel::Warn, "qa", "warn msg", BTreeMap::new());
        let serialized = to_json(&runtime.logs_snapshot());
        assert!(serialized.contains("\"level\":\"warn\""));
    }
}
