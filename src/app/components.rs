pub use crate::app::roi::{
    ContourBody, ContourData, ContourLoop, ContourMovePreview, ContourPoint, ContourSlice,
    MeshBody, MeshData, MeshEditPreview, MeshFace, MeshVertex, PrimaryRepresentation, RoiBody,
    RoiId, RoiMetadata, VoxelBody, VoxelData, VoxelGeometry,
};
use crate::convert::{ChunkedMeshData, GeometryIdentity, PlaneDefinition, PlaneFamily};
pub use crate::model::{LoadedLabel, ViewMode, VolumeData};
use glam::Vec3;
use web_time::Instant;

use winit::keyboard::ModifiersState;

// --- Basic Tags ---
pub struct MainVolumeTag;
pub struct RoiTag;

// --- Window & View ---
pub struct WindowSettings {
    pub width: u32,
    pub height: u32,
    pub viewport_rect: [f32; 4],
}

pub struct Transform {
    pub position: [f32; 3],
}

pub struct Viewport {
    pub mode: ViewMode,
    pub rect: [f32; 4], // [x, y, w, h] in pixels (physical)
    pub uniform_index: u32,
}

#[derive(Clone, Copy)]
pub struct ViewportState {
    pub zoom: f32,
    pub pan: [f32; 2],
    pub pivot: [f32; 2],
    pub user_rotation: [f32; 4],
}

/// Viewport Layout (Normalized 0..1 coordinates)
pub struct ViewportLayout {
    pub relative_rect: [f32; 4], // [x, y, w, h]
}

/// Global Protocol State
pub struct ProtocolState {
    pub active_protocol: String,
    pub last_protocol: Option<String>,
}

impl Default for ProtocolState {
    fn default() -> Self {
        Self {
            active_protocol: "Standard 2x2".to_string(),
            last_protocol: None,
        }
    }
}

impl Default for ViewportState {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: [0.0, 0.0],
            pivot: [0.5, 0.5],
            user_rotation: [0.0, 0.0, 0.0, 1.0],
        }
    }
}

#[derive(Default)]
pub struct InputState {
    pub last_mouse_pos: [f64; 2],
    pub mouse_uv: [f32; 2],
    pub active_viewport: Option<hecs::Entity>,
    pub modifiers: ModifiersState,
    pub is_dragging: bool,
    pub is_panning: bool,
    pub drag_start_pos: [f32; 2],
    pub drag_start_pan: [f32; 2],
    pub is_rotating: bool,
    pub rotation_start_pos: [f32; 2],
    pub rotation_start_val: [f32; 4],
    pub egui_wants_input: bool,
    pub scroll_accumulator: [f32; 4], // Accumulate sub-slice deltas per viewport
    pub contour_move_pending_commit: bool,
    pub mesh_move_pending_commit: bool,
}

// --- GPU Resources ---
#[derive(Clone)]
pub struct GpuVolumeResources {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub bind_group: wgpu::BindGroup,
}

// --- Uniforms ---
pub const MAX_VOXEL_OVERLAY_SLOTS: usize = 8;

#[repr(C)]
#[derive(Debug, Copy, Clone, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VoxelOverlayUniform {
    pub dimensions: [u32; 4],
    pub opacity: [f32; 4],
    pub main_to_roi_row0: [f32; 4],
    pub main_to_roi_row1: [f32; 4],
    pub main_to_roi_row2: [f32; 4],
}

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniforms {
    pub cursor_pos: [f32; 4],
    pub volume_dims: [u32; 4],
    pub volume_spacing: [f32; 4],
    pub voxel_overlays: [VoxelOverlayUniform; MAX_VOXEL_OVERLAY_SLOTS],
    pub window_params: [f32; 4],
    pub resolution: [f32; 2],
    pub mouse_uv: [f32; 2],
    pub pan: [f32; 2],
    pub zoom_pivot: [f32; 2],
    pub rotation: [f32; 4], // Quaternion
    pub oblique_origin_uv: [f32; 4],
    /// Oblique reslice basis along the plane's `u` axis: `xyz` is the volume-UV displacement per
    /// millimetre, `w` is the view window length in millimetres (likewise for `v` below).
    pub oblique_u_dir_length: [f32; 4],
    pub oblique_v_dir_length: [f32; 4],
    pub zoom: f32,
    pub view_mode: u32,
    pub overlay_flags: u32,
    /// Raymarch steps of the 3D view; fewer while the camera moves (see `CameraMotion`).
    pub ray_steps: u32,
}

// --- GUI / Editor ---
#[derive(Clone)]
pub struct GuiState {
    pub status_message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditorTool {
    #[default]
    Navigation,
    ContourSelect,
    ContourDraw,
    MeshDeform,
}

pub struct EditorState {
    pub active_roi: Option<hecs::Entity>,
    pub active_tool: EditorTool,
    pub contour_draft: Option<ContourDraft>,
    pub contour_selection: Option<ContourSelection>,
    pub mesh_selection: Option<MeshSelection>,
    pub mesh_brush_radius_mm: f32,
    pub mesh_brush_strength: f32,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            active_roi: None,
            active_tool: EditorTool::Navigation,
            contour_draft: None,
            contour_selection: None,
            mesh_selection: None,
            mesh_brush_radius_mm: 12.0,
            mesh_brush_strength: 1.0,
        }
    }
}

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

#[derive(Clone, Copy)]
pub struct VolumeWindowing {
    pub center: f32,
    pub width: f32,
}

#[cfg(test)]
fn unit_test_roi_geometry() -> VoxelGeometry {
    VoxelGeometry::new([1, 1, 1], [1.0; 3], [0.0; 3], [0.0, 0.0, 0.0, 1.0])
        .expect("unit test geometry is valid")
}

impl Default for VolumeWindowing {
    fn default() -> Self {
        Self {
            center: 40.0,
            width: 400.0,
        }
    }
}

// --- Load Result ---
#[derive(Debug)]
pub enum LoadResult {
    Volume(crate::io::nifti::LoadedVolume),
    Label(LoadedLabel), // Changed to local LoadedLabel
}

// --- ANNOTATIONS (Threaded discussions and notes) ---
#[derive(Clone, Debug)]
pub struct Comment {
    pub author: String,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct Annotation {
    pub id: uuid::Uuid,
    pub world_pos: Vec3,
    pub label: String,
    pub note: String,
    pub comments: Vec<Comment>,
}

#[derive(Clone, Debug, Default)]
pub struct AnnotationState {
    pub annotations: Vec<Annotation>,
    pub focused_id: Option<uuid::Uuid>,
    pub show_right_sidebar: bool,
}

// --- Session: state that exists once ---
/// The state that exists once per running app, as plain fields. (Things that are many, such as
/// ROIs and viewports, are entities in the `hecs` world; see ADR 0005.)
pub struct Session {
    pub input: InputState,
    pub editor: EditorState,
    pub gui: GuiState,
    pub windowing: VolumeWindowing,
    pub annotations: AnnotationState,
    pub protocol: ProtocolState,
    pub cursor: Transform,
    pub window_settings: WindowSettings,
}

impl Session {
    /// What the user is looking at: the active ROI and the cursor.
    pub fn view_focus(&self) -> crate::app::roi_runtime::ViewFocus {
        crate::app::roi_runtime::ViewFocus {
            active_roi: self.editor.active_roi,
            cursor_uv: self.cursor.position,
        }
    }

    /// A session for a window of the given size, with the cursor at the volume centre.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            input: InputState::default(),
            editor: EditorState::default(),
            gui: GuiState {
                status_message: None,
            },
            windowing: VolumeWindowing::default(),
            annotations: AnnotationState::default(),
            protocol: ProtocolState::default(),
            cursor: Transform {
                position: [0.5, 0.5, 0.5],
            },
            window_settings: WindowSettings {
                width,
                height,
                viewport_rect: [0.0, 0.0, width as f32, height as f32],
            },
        }
    }
}

#[cfg(test)]
mod tests;
