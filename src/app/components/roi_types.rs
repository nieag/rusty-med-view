//! ROI data: the `Roi` component and the caches, revisions, jobs, history, and selections
//! around it.

use super::*;

pub struct LayerSettings {
    pub opacity: f32,
}

#[derive(Clone)]
pub struct VoxelCache {
    pub data: VoxelData,
    pub gpu_resources: Option<GpuVolumeResources>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContourDraft {
    pub roi_entity: hecs::Entity,
    pub plane: PlaneDefinition,
    pub points: Vec<ContourPoint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContourSelection {
    pub roi_entity: hecs::Entity,
    pub slice_index: usize,
    pub loop_index: usize,
    pub point_index: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshSelection {
    pub roi_entity: hecs::Entity,
    pub vertex_index: usize,
    pub triangle_vertex_indices: [u32; 3],
    pub anchor_world_mm: [f32; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub enum RoiEditSnapshot {
    // Boxed: a voxel body is far larger than a contour or mesh body.
    Voxel(Box<VoxelData>),
    Contour(ContourData),
    Mesh(MeshData),
}

#[derive(Debug, Clone, PartialEq)]
pub struct RoiEditHistoryEntry {
    pub snapshot: RoiEditSnapshot,
    pub dirty_region: RoiDirtyRegion,
}

/// Most undo (and redo) steps kept per ROI.
pub const MAX_ROI_EDIT_HISTORY: usize = 32;

/// Undo and redo steps of one ROI, oldest first. History belongs to the ROI, not to the editor:
/// undo acts on the active ROI and never changes which ROI is active, and an ROI's steps are
/// independent of every other ROI's.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RoiHistory {
    pub undo: Vec<RoiEditHistoryEntry>,
    pub redo: Vec<RoiEditHistoryEntry>,
}

impl RoiHistory {
    /// Records a new edit: it becomes the latest undo step and invalidates redo.
    pub fn record(&mut self, entry: RoiEditHistoryEntry) {
        self.undo.push(entry);
        Self::trim(&mut self.undo);
        self.redo.clear();
    }

    /// Moves the latest step of one stack to the other, swapping its snapshot for `current` (the
    /// state being replaced), so the step can be reversed.
    pub fn step(&mut self, undo: bool, current: RoiEditSnapshot) {
        let (source, destination) = if undo {
            (&mut self.undo, &mut self.redo)
        } else {
            (&mut self.redo, &mut self.undo)
        };
        if let Some(entry) = source.pop() {
            destination.push(RoiEditHistoryEntry {
                snapshot: current,
                dirty_region: entry.dirty_region,
            });
            Self::trim(destination);
        }
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    fn trim(stack: &mut Vec<RoiEditHistoryEntry>) {
        if stack.len() > MAX_ROI_EDIT_HISTORY {
            stack.remove(0);
        }
    }
}

#[derive(Default)]
pub struct RoiSessionCaches {
    pub voxel: Option<VoxelCache>,
    pub contour: Option<ContourCache>,
    pub mesh: Option<MeshCache>,
    pub preview_voxel: Option<PreviewVoxelCache>,
    pub preview_mesh: Option<PreviewMeshCache>,
    /// Geometry identity recorded when a mesh cache is accepted for this ROI.
    pub mesh_geometry_identity: Option<GeometryIdentity>,
    /// Geometry identity recorded when a preview mesh cache is accepted for this ROI.
    pub preview_mesh_geometry_identity: Option<GeometryIdentity>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreviewVoxelCache {
    pub data: VoxelData,
    pub source_generation: u64,
    pub preview_revision: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreviewMeshCache {
    pub data: MeshData,
    pub chunks: Option<ChunkedMeshData>,
    pub dirty_voxel_aabb: Option<([u32; 3], [u32; 3])>,
    pub source_generation: u64,
    pub preview_revision: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContourCache {
    pub views: Vec<ContourViewCache>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContourViewCache {
    pub key: ContourViewKey,
    /// The slices this derived view shows (one plane for each request), not an authoritative
    /// contour set.
    pub data: Vec<ContourSlice>,
    /// Authoritative revision this view was derived from.
    pub built_from: Revision,
    pub geometry_identity: GeometryIdentity,
    pub state: CacheViewState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContourViewKey {
    pub family: PlaneFamily,
    pub plane: PlaneDefinition,
    pub slice_key: ContourSliceKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContourSliceKey {
    pub origin_quantized_mm: [i32; 3],
    pub normal_quantized: [i32; 3],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheViewState {
    Current,
    Preview { revision: u64 },
    Stale,
    Queued,
    Rebuilding,
    Blocked { reason: String },
    Unsupported { reason: String },
}

pub const MAX_CONTOUR_VIEW_CACHE_ENTRIES: usize = 96;

impl ContourSliceKey {
    pub fn from_plane(plane: PlaneDefinition) -> Self {
        Self {
            origin_quantized_mm: plane.origin_mm.map(|v| (v * 1000.0).round() as i32),
            normal_quantized: plane.normal_mm.map(|v| (v * 1_000_000.0).round() as i32),
        }
    }
}

impl ContourViewKey {
    pub fn from_plane(plane: PlaneDefinition) -> Self {
        Self {
            family: plane.family,
            plane,
            slice_key: ContourSliceKey::from_plane(plane),
        }
    }

    pub fn logical_eq(&self, other: &Self) -> bool {
        self.family == other.family && self.slice_key == other.slice_key
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MeshCache {
    pub data: MeshData,
    pub chunks: Option<ChunkedMeshData>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoiCacheKind {
    Voxel,
    Contour,
    Mesh,
}

impl RoiCacheKind {
    /// Whether a cache of this kind must be rebuilt when only the authoritative representation
    /// changes but the shape is preserved (a lossless primary-view switch). Contour views are
    /// expressed in the authoritative family, so they follow it; the voxel and mesh caches
    /// describe the shape and stay current.
    pub const fn tracks_form(self) -> bool {
        matches!(self, Self::Contour)
    }

    /// The job that rebuilds a cache of this kind.
    pub const fn rebuild_job(self) -> RoiJobKind {
        match self {
            Self::Voxel => RoiJobKind::RebuildVoxelCache,
            Self::Contour => RoiJobKind::RebuildContourCache,
            Self::Mesh => RoiJobKind::RebuildMeshCache,
        }
    }
}

/// Version of an ROI's authoritative data.
///
/// `shape` changes when the ROI's shape changes (an edit, an undo, a lossy conversion), which
/// invalidates every derived cache. `form` changes when only the authoritative representation
/// changes while the shape is preserved (a lossless primary-view switch), which invalidates only
/// caches whose [`RoiCacheKind::tracks_form`] is true.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Revision {
    pub shape: u64,
    pub form: u64,
}

impl Revision {
    /// Nothing has been built from this revision; also the "never built" marker of a cache.
    pub const NEVER: Self = Self { shape: 0, form: 0 };
    /// Revision of a newly created ROI.
    pub const INITIAL: Self = Self { shape: 1, form: 0 };

    pub const fn from_shape(shape: u64) -> Self {
        Self { shape, form: 0 }
    }

    /// The revision after the shape changed.
    pub const fn next_shape(self) -> Self {
        Self {
            shape: self.shape.saturating_add(1),
            form: self.form,
        }
    }

    /// The revision after the representation changed without changing the shape.
    pub const fn next_form(self) -> Self {
        Self {
            shape: self.shape,
            form: self.form.saturating_add(1),
        }
    }
}

/// What one derived cache was built from, and whether it was explicitly invalidated since. This
/// is the single freshness rule for the voxel, contour, and mesh caches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheFreshness {
    pub dirty: bool,
    pub built_from: Revision,
}

impl Default for Revision {
    fn default() -> Self {
        Self::NEVER
    }
}

impl CacheFreshness {
    /// A cache that was explicitly invalidated and has not been rebuilt.
    pub const fn invalidated() -> Self {
        Self {
            dirty: true,
            built_from: Revision::NEVER,
        }
    }

    /// A cache freshly built from `revision`.
    pub const fn built_from(revision: Revision) -> Self {
        Self {
            dirty: false,
            built_from: revision,
        }
    }

    /// Whether the cache reflects `authoritative`. Caches that do not track the form only compare
    /// the shape.
    pub fn is_current(&self, authoritative: Revision, kind: RoiCacheKind) -> bool {
        !self.dirty
            && self.built_from.shape == authoritative.shape
            && (!kind.tracks_form() || self.built_from.form == authoritative.form)
    }
}

/// Freshness of the three derived caches against the authoritative revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoiDirtyState {
    pub authoritative_dirty: bool,
    pub authoritative: Revision,
    pub voxel: CacheFreshness,
    pub contour: CacheFreshness,
    pub mesh: CacheFreshness,
}

impl Default for RoiDirtyState {
    fn default() -> Self {
        Self {
            authoritative_dirty: false,
            authoritative: Revision::INITIAL,
            voxel: CacheFreshness::default(),
            contour: CacheFreshness::default(),
            mesh: CacheFreshness::default(),
        }
    }
}

impl RoiDirtyState {
    pub fn freshness(&self, kind: RoiCacheKind) -> CacheFreshness {
        match kind {
            RoiCacheKind::Voxel => self.voxel,
            RoiCacheKind::Contour => self.contour,
            RoiCacheKind::Mesh => self.mesh,
        }
    }

    pub fn freshness_mut(&mut self, kind: RoiCacheKind) -> &mut CacheFreshness {
        match kind {
            RoiCacheKind::Voxel => &mut self.voxel,
            RoiCacheKind::Contour => &mut self.contour,
            RoiCacheKind::Mesh => &mut self.mesh,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoiJobKind {
    RebuildVoxelCache,
    RebuildContourCache,
    RebuildMeshCache,
}

impl RoiJobKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RebuildVoxelCache => "rebuild_voxel_cache",
            Self::RebuildContourCache => "rebuild_contour_cache",
            Self::RebuildMeshCache => "rebuild_mesh_cache",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoiJobPriority {
    InteractivePreview,
    VisibleCommitted,
    Background,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoiDirtyRegion {
    Full,
    VoxelAabb { min: [u32; 3], max: [u32; 3] },
    ContourSlice(ContourSliceKey),
    MeshChunkAabb { min: [u32; 3], max: [u32; 3] },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoiJobRequest {
    pub kind: RoiJobKind,
    pub source_generation: u64,
    pub preview_revision: Option<u64>,
    pub priority: RoiJobPriority,
    pub dirty_region: RoiDirtyRegion,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RoiJobMetrics {
    pub completed_count: u64,
    pub last_completed_kind: Option<RoiJobKind>,
    pub discarded_count: u64,
    pub failed_count: u64,
    pub last_duration_ms: f32,
    pub last_queue_delay_ms: f32,
    pub last_contour_raster_ms: f32,
    pub last_mesh_voxelization_ms: f32,
    pub last_cpu_cache_install_ms: f32,
    pub last_gpu_upload_ms: f32,
    pub last_work_convergence_ms: f32,
    pub max_queue_depth: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RoiPreviewState {
    pub active: bool,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RoiJobState {
    pub running_request: Option<RoiJobRequest>,
    pub pending: Vec<RoiJobRequest>,
    // ponytail: one timestamp measures oldest pending work; add per-request timestamps only if
    // queue-delay attribution becomes a measured bottleneck.
    pub oldest_pending_since: Option<Instant>,
    // ponytail: one timestamp covers a coalesced ROI work cycle; split by representation only if
    // a measured bottleneck needs finer attribution.
    pub work_cycle_started_at: Option<Instant>,
    /// A representation switch waiting for derived data (see `app::roi::switch`).
    pub pending_switch: Option<crate::app::roi::switch::PendingSwitch>,
    /// Outcomes of this ROI's work for the user, drained by `advance_roi_work`.
    pub messages: Vec<String>,
}

pub struct Roi {
    /// Immutable reference grid for conversions involving this ROI.
    ///
    pub reference_geometry: VoxelGeometry,
    pub body: RoiBody,
    pub session_caches: RoiSessionCaches,
    pub dirty_state: RoiDirtyState,
    pub job_state: RoiJobState,
    pub job_metrics: RoiJobMetrics,
    pub preview_state: RoiPreviewState,
    /// Undo and redo steps of this ROI.
    pub history: RoiHistory,
    /// Current mesh generation already passed full voxelization validation.
    pub validated_mesh_generation: Option<u64>,
}

impl Roi {
    pub fn reference_geometry(&self) -> VoxelGeometry {
        self.reference_geometry
    }

    pub fn primary_representation(&self) -> PrimaryRepresentation {
        match &self.body {
            RoiBody::Voxel(_) => PrimaryRepresentation::Voxel,
            RoiBody::Contour(_) => PrimaryRepresentation::Contour,
            RoiBody::Mesh(_) => PrimaryRepresentation::Mesh,
        }
    }

    pub fn new_voxel(
        roi_id: RoiId,
        name: String,
        geometry: VoxelGeometry,
        raw_data: Vec<u8>,
        gpu_resources: GpuVolumeResources,
    ) -> (Self, RoiMetadata) {
        Self::new_voxel_with_cache(roi_id, name, geometry, raw_data, Some(gpu_resources))
    }

    pub fn new_voxel_with_cache(
        roi_id: RoiId,
        name: String,
        geometry: VoxelGeometry,
        raw_data: Vec<u8>,
        gpu_resources: Option<GpuVolumeResources>,
    ) -> (Self, RoiMetadata) {
        let voxel_data = VoxelData { geometry, raw_data };
        let reference_geometry = voxel_data.geometry;
        let roi = Self {
            reference_geometry,
            body: RoiBody::Voxel(VoxelBody {
                data: voxel_data.clone(),
            }),
            session_caches: RoiSessionCaches {
                voxel: Some(VoxelCache {
                    data: voxel_data,
                    gpu_resources,
                }),
                contour: None,
                mesh: None,
                preview_voxel: None,
                preview_mesh: None,
                mesh_geometry_identity: None,
                preview_mesh_geometry_identity: None,
            },
            dirty_state: RoiDirtyState {
                voxel: CacheFreshness::built_from(Revision::INITIAL),
                contour: CacheFreshness::invalidated(),
                mesh: CacheFreshness::invalidated(),
                ..RoiDirtyState::default()
            },
            job_state: RoiJobState::default(),
            job_metrics: RoiJobMetrics::default(),
            preview_state: RoiPreviewState::default(),
            history: RoiHistory::default(),
            validated_mesh_generation: None,
        };
        (roi, RoiMetadata::new(roi_id, name))
    }

    #[cfg(test)]
    pub fn new_contour(
        roi_id: RoiId,
        name: String,
        contour_data: ContourData,
    ) -> (Self, RoiMetadata) {
        Self::new_contour_with_geometry(roi_id, name, unit_test_roi_geometry(), contour_data)
    }

    pub fn new_contour_with_geometry(
        roi_id: RoiId,
        name: String,
        reference_geometry: VoxelGeometry,
        contour_data: ContourData,
    ) -> (Self, RoiMetadata) {
        let roi = Self {
            reference_geometry,
            body: RoiBody::Contour(ContourBody::new(contour_data)),
            session_caches: RoiSessionCaches {
                voxel: None,
                contour: None,
                mesh: None,
                preview_voxel: None,
                preview_mesh: None,
                mesh_geometry_identity: None,
                preview_mesh_geometry_identity: None,
            },
            dirty_state: RoiDirtyState {
                voxel: CacheFreshness::invalidated(),
                mesh: CacheFreshness::invalidated(),
                ..RoiDirtyState::default()
            },
            job_state: RoiJobState::default(),
            job_metrics: RoiJobMetrics::default(),
            preview_state: RoiPreviewState::default(),
            history: RoiHistory::default(),
            validated_mesh_generation: None,
        };
        (roi, RoiMetadata::new(roi_id, name))
    }

    #[cfg(test)]
    pub fn new_mesh(roi_id: RoiId, name: String, mesh_data: MeshData) -> (Self, RoiMetadata) {
        Self::new_mesh_with_geometry(roi_id, name, unit_test_roi_geometry(), mesh_data)
    }

    pub fn new_mesh_with_geometry(
        roi_id: RoiId,
        name: String,
        reference_geometry: VoxelGeometry,
        mesh_data: MeshData,
    ) -> (Self, RoiMetadata) {
        let roi = Self {
            reference_geometry,
            body: RoiBody::Mesh(MeshBody::new(mesh_data)),
            session_caches: RoiSessionCaches {
                voxel: None,
                contour: None,
                mesh: None,
                preview_voxel: None,
                preview_mesh: None,
                mesh_geometry_identity: None,
                preview_mesh_geometry_identity: None,
            },
            dirty_state: RoiDirtyState {
                voxel: CacheFreshness::invalidated(),
                contour: CacheFreshness::invalidated(),
                ..RoiDirtyState::default()
            },
            job_state: RoiJobState::default(),
            job_metrics: RoiJobMetrics::default(),
            preview_state: RoiPreviewState::default(),
            history: RoiHistory::default(),
            validated_mesh_generation: None,
        };
        (roi, RoiMetadata::new(roi_id, name))
    }

    pub fn contour_data(&self) -> Option<&ContourData> {
        match &self.body {
            RoiBody::Contour(ContourBody { data: contour, .. }) => Some(contour),
            RoiBody::Voxel(_) | RoiBody::Mesh(_) => None,
        }
    }

    pub fn mesh_data(&self) -> Option<&MeshData> {
        match &self.body {
            RoiBody::Mesh(MeshBody { data: mesh, .. }) => Some(mesh),
            RoiBody::Voxel(_) | RoiBody::Contour(_) => None,
        }
    }
}

#[cfg(test)]
fn unit_test_roi_geometry() -> VoxelGeometry {
    VoxelGeometry::new([1, 1, 1], [1.0; 3], [0.0; 3], [0.0, 0.0, 0.0, 1.0])
        .expect("unit test geometry is valid")
}
