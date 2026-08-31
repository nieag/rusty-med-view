pub use crate::app::roi::{
    ContourData, ContourLoop, ContourPoint, ContourSlice, MeshData, MeshFace, MeshVertex,
    PrimaryRepresentation, RoiAuthoritativeData, RoiId, RoiMetadata, VoxelData, VoxelGeometry,
};
use crate::convert::{
    ChunkedMeshData, GeometryIdentity, PlaneDefinition, PlaneFamily, RoiGeometry,
};
use glam::Vec3;

use winit::keyboard::ModifiersState;

// --- Basic Tags ---
pub struct CursorTag;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    ThreeD = 0,
    Axial = 1,
    Coronal = 2,
    Sagittal = 3,
    Oblique = 4,
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

// --- Volume Data ---
pub struct VolumeData {
    pub dimensions: [u32; 3],
    pub spacing: [f32; 3],
    pub origin: [f32; 3],
    pub intensities: Vec<f32>,
    pub intensity_range: [f32; 2],
    pub orientation: [f32; 4], // Quaternion
}

impl VolumeData {
    pub fn aspect_ratios(&self) -> [f32; 3] {
        let d = self.dimensions;
        let s = self.spacing;
        // avoid div by zero if empty
        if d[0] == 0 || d[1] == 0 || d[2] == 0 {
            return [1.0, 1.0, 1.0];
        }
        let max_dim = (d[0] as f32 * s[0])
            .max(d[1] as f32 * s[1])
            .max(d[2] as f32 * s[2]);
        [
            (d[0] as f32 * s[0]) / max_dim,
            (d[1] as f32 * s[1]) / max_dim,
            (d[2] as f32 * s[2]) / max_dim,
        ]
    }
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
    pub oblique_u_dir_length: [f32; 4],
    pub oblique_v_dir_length: [f32; 4],
    // --- Overlay primitive fields ---
    pub overlay_mouse_uv: [f32; 2], // Mouse position for dragged primitive
    pub overlay_primitive_count: u32, // Number of active primitives
    pub overlay_dragging_idx: u32,  // Index being dragged (u32::MAX = none)
    pub zoom: f32,
    pub view_mode: u32,
    pub overlay_flags: u32,
    pub _padding: u32,
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
    pub roi_edit_preview: Option<RoiEditPreview>,
    pub mesh_selection: Option<MeshSelection>,
    pub mesh_brush_radius_mm: f32,
    pub mesh_brush_strength: f32,
    pub roi_undo_stack: Vec<RoiEditHistoryEntry>,
    pub roi_redo_stack: Vec<RoiEditHistoryEntry>,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            active_roi: None,
            active_tool: EditorTool::Navigation,
            contour_draft: None,
            contour_selection: None,
            roi_edit_preview: None,
            mesh_selection: None,
            mesh_brush_radius_mm: 12.0,
            mesh_brush_strength: 1.0,
            roi_undo_stack: Vec::new(),
            roi_redo_stack: Vec::new(),
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

#[derive(Debug, Clone, PartialEq)]
pub struct ContourMovePreview {
    pub roi_entity: hecs::Entity,
    pub contour_data: ContourData,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MeshEditPreview {
    pub roi_entity: hecs::Entity,
    pub mesh_data: MeshData,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RoiEditPreview {
    ContourMove(ContourMovePreview),
    MeshDeform(MeshEditPreview),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeshSelection {
    pub roi_entity: hecs::Entity,
    pub vertex_index: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RoiEditSnapshot {
    Contour(ContourData),
    Mesh(MeshData),
}

#[derive(Debug, Clone, PartialEq)]
pub struct RoiEditHistoryEntry {
    pub roi_entity: hecs::Entity,
    pub snapshot: RoiEditSnapshot,
    pub dirty_region: RoiDirtyRegion,
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
    pub data: ContourData,
    pub source_generation: u64,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheGeneration {
    pub authoritative: u64,
    pub voxel: u64,
    pub contour: u64,
    pub mesh: u64,
}

impl Default for CacheGeneration {
    fn default() -> Self {
        Self {
            authoritative: 1,
            voxel: 0,
            contour: 0,
            mesh: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RoiDirtyState {
    pub authoritative_dirty: bool,
    pub voxel_cache_dirty: bool,
    pub contour_cache_dirty: bool,
    pub mesh_cache_dirty: bool,
    pub generations: CacheGeneration,
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
    pub discarded_count: u64,
    pub failed_count: u64,
    pub last_duration_ms: f32,
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
}

pub struct Roi {
    pub metadata: RoiMetadata,
    /// Immutable reference grid for conversions involving this ROI.
    ///
    pub reference_geometry: RoiGeometry,
    pub authoritative_data: RoiAuthoritativeData,
    pub session_caches: RoiSessionCaches,
    pub dirty_state: RoiDirtyState,
    pub job_state: RoiJobState,
    pub job_metrics: RoiJobMetrics,
    pub preview_state: RoiPreviewState,
}

impl Roi {
    pub fn reference_geometry(&self) -> &RoiGeometry {
        &self.reference_geometry
    }

    pub fn primary_representation(&self) -> PrimaryRepresentation {
        match &self.authoritative_data {
            RoiAuthoritativeData::Voxel(_) => PrimaryRepresentation::Voxel,
            RoiAuthoritativeData::Contour(_) => PrimaryRepresentation::Contour,
            RoiAuthoritativeData::Mesh(_) => PrimaryRepresentation::Mesh,
        }
    }

    pub fn new_voxel(
        roi_id: RoiId,
        name: String,
        geometry: VoxelGeometry,
        raw_data: Vec<u8>,
        gpu_resources: GpuVolumeResources,
    ) -> Self {
        Self::new_voxel_with_cache(roi_id, name, geometry, raw_data, Some(gpu_resources))
    }

    pub fn new_voxel_with_cache(
        roi_id: RoiId,
        name: String,
        geometry: VoxelGeometry,
        raw_data: Vec<u8>,
        gpu_resources: Option<GpuVolumeResources>,
    ) -> Self {
        let voxel_data = VoxelData { geometry, raw_data };
        let reference_geometry = RoiGeometry::from_legacy_parts(
            voxel_data.geometry.dimensions,
            voxel_data.geometry.spacing,
            voxel_data.geometry.origin,
            voxel_data.geometry.orientation,
        )
        .expect("voxel ROI constructors require valid reference geometry");
        Self {
            metadata: RoiMetadata {
                roi_id,
                name,
                is_visible: true,
                is_locked: false,
                color: [1.0, 0.2, 0.2, 1.0],
            },
            reference_geometry,
            authoritative_data: RoiAuthoritativeData::Voxel(voxel_data.clone()),
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
                contour_cache_dirty: true,
                mesh_cache_dirty: true,
                generations: CacheGeneration {
                    voxel: 1,
                    ..CacheGeneration::default()
                },
                ..RoiDirtyState::default()
            },
            job_state: RoiJobState::default(),
            job_metrics: RoiJobMetrics::default(),
            preview_state: RoiPreviewState::default(),
        }
    }

    #[cfg(test)]
    pub fn new_contour(roi_id: RoiId, name: String, contour_data: ContourData) -> Self {
        Self::new_contour_with_geometry(roi_id, name, unit_test_roi_geometry(), contour_data)
    }

    pub fn new_contour_with_geometry(
        roi_id: RoiId,
        name: String,
        reference_geometry: RoiGeometry,
        contour_data: ContourData,
    ) -> Self {
        Self {
            metadata: RoiMetadata {
                roi_id,
                name,
                is_visible: true,
                is_locked: false,
                color: [1.0, 0.2, 0.2, 1.0],
            },
            reference_geometry,
            authoritative_data: RoiAuthoritativeData::Contour(contour_data),
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
                voxel_cache_dirty: true,
                mesh_cache_dirty: true,
                ..RoiDirtyState::default()
            },
            job_state: RoiJobState::default(),
            job_metrics: RoiJobMetrics::default(),
            preview_state: RoiPreviewState::default(),
        }
    }

    #[cfg(test)]
    pub fn new_mesh(roi_id: RoiId, name: String, mesh_data: MeshData) -> Self {
        Self::new_mesh_with_geometry(roi_id, name, unit_test_roi_geometry(), mesh_data)
    }

    pub fn new_mesh_with_geometry(
        roi_id: RoiId,
        name: String,
        reference_geometry: RoiGeometry,
        mesh_data: MeshData,
    ) -> Self {
        Self {
            metadata: RoiMetadata {
                roi_id,
                name,
                is_visible: true,
                is_locked: false,
                color: [1.0, 0.2, 0.2, 1.0],
            },
            reference_geometry,
            authoritative_data: RoiAuthoritativeData::Mesh(mesh_data),
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
                voxel_cache_dirty: true,
                contour_cache_dirty: true,
                ..RoiDirtyState::default()
            },
            job_state: RoiJobState::default(),
            job_metrics: RoiJobMetrics::default(),
            preview_state: RoiPreviewState::default(),
        }
    }

    pub fn contour_data(&self) -> Option<&ContourData> {
        match &self.authoritative_data {
            RoiAuthoritativeData::Contour(contour) => Some(contour),
            RoiAuthoritativeData::Voxel(_) | RoiAuthoritativeData::Mesh(_) => None,
        }
    }

    pub fn mesh_data(&self) -> Option<&MeshData> {
        match &self.authoritative_data {
            RoiAuthoritativeData::Mesh(mesh) => Some(mesh),
            RoiAuthoritativeData::Voxel(_) | RoiAuthoritativeData::Contour(_) => None,
        }
    }
}

#[derive(Clone, Copy)]
pub struct VolumeWindowing {
    pub center: f32,
    pub width: f32,
}

#[cfg(test)]
fn unit_test_roi_geometry() -> RoiGeometry {
    RoiGeometry::from_legacy_parts(
        [1, 1, 1],
        [1.0, 1.0, 1.0],
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    )
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

#[derive(Debug)]
pub struct LoadedLabel {
    pub dimensions: [u32; 3],
    pub spacing: [f32; 3],
    pub origin: [f32; 3],
    pub orientation: [f32; 4],
    pub data: Vec<u8>,
    pub filename: String,
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

// --- Singleton Entity Registry ---
#[derive(Clone, Copy)]
pub struct AppEntities {
    pub input: hecs::Entity,
    pub editor: hecs::Entity,
    pub gui_state: hecs::Entity,
    pub volume_windowing: hecs::Entity,
    pub annotations: hecs::Entity,
    pub overlay: hecs::Entity,
    pub protocol: hecs::Entity,
    pub cursor: hecs::Entity,
    pub window_settings: hecs::Entity,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_plane_definition(family: PlaneFamily) -> PlaneDefinition {
        PlaneDefinition {
            family,
            origin_mm: [10.0, 20.0, 30.0],
            u_axis_mm: [1.0, 0.0, 0.0],
            v_axis_mm: [0.0, 1.0, 0.0],
            normal_mm: [0.0, 0.0, 1.0],
        }
    }

    #[test]
    fn test_aspect_ratio_cubic() {
        let vol = VolumeData {
            dimensions: [100, 100, 100],
            spacing: [1.0, 1.0, 1.0],
            origin: [0.0, 0.0, 0.0],
            intensities: vec![],
            intensity_range: [0.0, 1.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let ar = vol.aspect_ratios();
        assert!((ar[0] - 1.0).abs() < 1e-6);
        assert!((ar[1] - 1.0).abs() < 1e-6);
        assert!((ar[2] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_aspect_ratio_anisotropic() {
        let vol = VolumeData {
            dimensions: [256, 256, 128],
            spacing: [1.0, 1.0, 2.0], // Physical size is 256, 256, 256
            origin: [0.0, 0.0, 0.0],
            intensities: vec![],
            intensity_range: [0.0, 1.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let ar = vol.aspect_ratios();
        assert!((ar[0] - 1.0).abs() < 1e-6);
        assert!((ar[1] - 1.0).abs() < 1e-6);
        assert!((ar[2] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_aspect_ratio_zero_dims() {
        let vol = VolumeData {
            dimensions: [0, 0, 0],
            spacing: [1.0, 1.0, 1.0],
            origin: [0.0, 0.0, 0.0],
            intensities: vec![],
            intensity_range: [0.0, 1.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let ar = vol.aspect_ratios();
        assert_eq!(ar, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn test_contour_view_key_from_plane_excludes_viewport_identity() {
        let plane = test_plane_definition(PlaneFamily::Axial);
        let key_a = ContourViewKey::from_plane(plane);
        let key_b = ContourViewKey::from_plane(plane);
        assert!(key_a.logical_eq(&key_b));
    }

    #[test]
    fn test_contour_view_key_lookup_uses_slice_key_not_exact_plane_float() {
        let contour = ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: Vec::new(),
        };
        let mut roi = Roi::new_contour(RoiId(12), "C".to_string(), contour.clone());
        let plane = test_plane_definition(PlaneFamily::Coronal);
        let key_a = ContourViewKey::from_plane(plane);
        roi.upsert_contour_view_cache(
            key_a.clone(),
            contour,
            roi.dirty_state.generations.authoritative,
            CacheViewState::Current,
        );

        let mut drifted = plane;
        drifted.normal_mm = [0.0, 0.0, 1.0 + 1e-8];
        let key_b = ContourViewKey::from_plane(drifted);
        assert!(roi.contour_view_cache(&key_b).is_some());
    }

    #[test]
    fn test_oblique_slice_key_distinguishes_same_origin_different_normal() {
        let contour = ContourData {
            active_plane_family: PlaneFamily::Oblique,
            slices: Vec::new(),
        };
        let mut roi = Roi::new_contour(RoiId(13), "Oblique".to_string(), contour.clone());
        let plane_a = PlaneDefinition {
            family: PlaneFamily::Oblique,
            origin_mm: [12.0, -3.0, 7.0],
            u_axis_mm: [1.0, 0.0, 0.0],
            v_axis_mm: [0.0, 1.0, 0.0],
            normal_mm: [0.0, 0.0, 1.0],
        };
        let plane_b = PlaneDefinition {
            family: PlaneFamily::Oblique,
            origin_mm: [12.0, -3.0, 7.0],
            u_axis_mm: [1.0, 0.0, 0.0],
            v_axis_mm: [0.0, 0.70710677, 0.70710677],
            normal_mm: [0.0, -0.70710677, 0.70710677],
        };
        let key_a = ContourViewKey::from_plane(plane_a);
        let key_b = ContourViewKey::from_plane(plane_b);
        assert!(!key_a.logical_eq(&key_b));

        let gen = roi.dirty_state.generations.authoritative;
        roi.upsert_contour_view_cache(key_a, contour.clone(), gen, CacheViewState::Current);
        roi.upsert_contour_view_cache(key_b.clone(), contour, gen, CacheViewState::Current);
        let cache = roi.contour_cache().unwrap();
        assert_eq!(cache.views.len(), 2);
        assert!(roi.contour_view_cache(&key_b).is_some());
    }

    #[test]
    fn test_mark_contour_authoritative_changed_marks_existing_derived_views_dirty() {
        let contour = ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: Vec::new(),
        };
        let mut roi = Roi::new_contour(RoiId(11), "C".to_string(), contour.clone());
        let plane = test_plane_definition(PlaneFamily::Coronal);
        roi.upsert_contour_view_cache(
            ContourViewKey::from_plane(plane),
            contour,
            roi.dirty_state.generations.authoritative,
            CacheViewState::Current,
        );
        roi.dirty_state.contour_cache_dirty = false;
        roi.dirty_state.generations.contour = roi.dirty_state.generations.authoritative;

        roi.mark_contour_authoritative_changed();
        roi.mark_all_contour_view_caches_stale();

        let cache = roi.contour_cache().unwrap();
        assert_eq!(cache.views.len(), 1);
        assert_eq!(cache.views[0].state, CacheViewState::Stale);
    }

    #[test]
    fn test_new_voxel_roi_initializes_voxel_primary_state() {
        let roi = Roi::new_voxel_with_cache(
            RoiId(7),
            "Liver".to_string(),
            VoxelGeometry {
                dimensions: [16, 16, 8],
                spacing: [1.0, 1.0, 1.0],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            vec![1; 16 * 16 * 8],
            None,
        );

        assert_eq!(roi.metadata.roi_id, RoiId(7));
        assert_eq!(roi.metadata.name, "Liver");
        assert_eq!(roi.primary_representation(), PrimaryRepresentation::Voxel);
        assert_eq!(roi.reference_geometry().dimensions(), [16, 16, 8]);
        assert!(matches!(
            roi.authoritative_data,
            RoiAuthoritativeData::Voxel(VoxelData {
                geometry: VoxelGeometry {
                    dimensions: [16, 16, 8],
                    spacing: [1.0, 1.0, 1.0],
                    origin: [0.0, 0.0, 0.0],
                    orientation: [0.0, 0.0, 0.0, 1.0],
                },
                ..
            })
        ));
        let voxel_cache = roi.voxel_cache().expect("voxel cache should exist");
        assert_eq!(voxel_cache.data.raw_data.len(), 16 * 16 * 8);
        assert_eq!(voxel_cache.data.geometry.dimensions, [16, 16, 8]);
        assert!(voxel_cache.gpu_resources.is_none());
        assert!(roi.voxel_gpu_cache().is_none());
        assert!(roi.session_caches.contour.is_none());
        assert!(roi.session_caches.mesh.is_none());
        assert!(!roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert!(roi.is_cache_current(RoiCacheKind::Voxel));
        assert!(roi.renderable_voxel_cache().is_none());
    }

    #[test]
    fn test_editor_tool_default_remains_navigation() {
        let editor = EditorState::default();
        assert_eq!(editor.active_tool, EditorTool::Navigation);
    }

    #[test]
    fn test_roi_dirty_state_defaults_match_clean_voxel_baseline() {
        let state = RoiDirtyState::default();

        assert!(!state.authoritative_dirty);
        assert!(!state.voxel_cache_dirty);
        assert!(!state.contour_cache_dirty);
        assert!(!state.mesh_cache_dirty);
        assert_eq!(state.generations.authoritative, 1);
        assert_eq!(state.generations.voxel, 0);
    }

    #[test]
    fn test_new_voxel_roi_without_gpu_still_has_current_cpu_voxel_cache() {
        let roi = Roi::new_voxel_with_cache(
            RoiId(8),
            "Kidney".to_string(),
            VoxelGeometry {
                dimensions: [8, 8, 8],
                spacing: [0.5, 0.5, 0.5],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            vec![1; 8 * 8 * 8],
            None,
        );

        assert!(!roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert!(roi.is_cache_current(RoiCacheKind::Voxel));
        assert!(roi.voxel_cache().is_some());
        assert!(roi.voxel_gpu_cache().is_none());
    }

    #[test]
    fn test_new_voxel_roi_copies_authoritative_data_into_session_voxel_cache() {
        let geometry = VoxelGeometry {
            dimensions: [6, 5, 4],
            spacing: [0.9, 1.1, 1.3],
            origin: [1.0, 2.0, 3.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let raw_data = vec![0, 1, 0, 1, 1, 0, 1, 0];
        let roi = Roi::new_voxel_with_cache(
            RoiId(77),
            "Cache Copy".to_string(),
            geometry,
            raw_data.clone(),
            None,
        );

        let authoritative = match &roi.authoritative_data {
            RoiAuthoritativeData::Voxel(voxel) => voxel,
            RoiAuthoritativeData::Contour(_) | RoiAuthoritativeData::Mesh(_) => {
                panic!("expected voxel-authoritative ROI");
            }
        };
        let cached = roi.voxel_cache().expect("voxel cache should exist");

        assert_eq!(cached.data.geometry, authoritative.geometry);
        assert_eq!(cached.data.raw_data, authoritative.raw_data);
        assert_eq!(cached.data.raw_data, raw_data);
    }

    #[test]
    fn test_renderable_voxel_cache_requires_gpu_resources_even_when_cache_current() {
        let mut roi = Roi::new_voxel_with_cache(
            RoiId(88),
            "No GPU".to_string(),
            VoxelGeometry {
                dimensions: [4, 4, 4],
                spacing: [1.0, 1.0, 1.0],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            vec![1; 64],
            None,
        );

        roi.metadata.is_visible = true;
        assert!(roi.is_cache_current(RoiCacheKind::Voxel));
        assert!(roi.voxel_gpu_cache().is_none());
        assert!(roi.renderable_voxel_cache().is_none());
    }

    #[test]
    fn test_cache_current_requires_matching_generation_and_clean_state() {
        let mut roi = Roi::new_voxel_with_cache(
            RoiId(12),
            "Aorta".to_string(),
            VoxelGeometry {
                dimensions: [8, 8, 8],
                spacing: [0.75, 0.75, 0.75],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            vec![1; 8 * 8 * 8],
            None,
        );

        roi.dirty_state.voxel_cache_dirty = false;
        roi.dirty_state.generations.voxel = roi.dirty_state.generations.authoritative;
        assert!(roi.is_cache_current(RoiCacheKind::Voxel));

        roi.dirty_state.generations.voxel -= 1;
        assert!(!roi.is_cache_current(RoiCacheKind::Voxel));
    }

    #[test]
    fn test_mark_authoritative_changed_invalidates_all_derived_caches() {
        let mut roi = Roi::new_voxel_with_cache(
            RoiId(9),
            "Spleen".to_string(),
            VoxelGeometry {
                dimensions: [4, 4, 4],
                spacing: [1.0, 1.0, 1.0],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            vec![1; 64],
            None,
        );

        roi.dirty_state.voxel_cache_dirty = false;
        roi.dirty_state.contour_cache_dirty = false;
        roi.dirty_state.mesh_cache_dirty = false;
        roi.dirty_state.generations.voxel = roi.dirty_state.generations.authoritative;
        roi.dirty_state.generations.contour = roi.dirty_state.generations.authoritative;
        roi.dirty_state.generations.mesh = roi.dirty_state.generations.authoritative;

        roi.mark_authoritative_changed();

        assert!(roi.dirty_state.authoritative_dirty);
        assert_eq!(roi.dirty_state.generations.authoritative, 2);
        assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert!(roi.is_cache_dirty(RoiCacheKind::Contour));
        assert!(roi.is_cache_dirty(RoiCacheKind::Mesh));
        assert!(!roi.is_cache_current(RoiCacheKind::Voxel));
    }

    #[test]
    fn test_mark_contour_authoritative_changed_invalidates_only_derived_by_default() {
        let mut roi = Roi::new_contour(
            RoiId(16),
            "CTV".to_string(),
            ContourData {
                active_plane_family: PlaneFamily::Axial,
                slices: Vec::new(),
            },
        );

        roi.dirty_state.voxel_cache_dirty = false;
        roi.dirty_state.contour_cache_dirty = false;
        roi.dirty_state.mesh_cache_dirty = false;
        roi.dirty_state.generations.voxel = roi.dirty_state.generations.authoritative;
        roi.dirty_state.generations.contour = roi.dirty_state.generations.authoritative;
        roi.dirty_state.generations.mesh = roi.dirty_state.generations.authoritative;

        roi.mark_contour_authoritative_changed();

        assert!(roi.dirty_state.authoritative_dirty);
        assert_eq!(roi.dirty_state.generations.authoritative, 2);
        assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert!(!roi.is_cache_dirty(RoiCacheKind::Contour));
        assert!(roi.is_cache_dirty(RoiCacheKind::Mesh));
    }

    #[test]
    fn test_enqueue_rebuild_preserves_multiple_representation_jobs() {
        let mut roi = Roi::new_voxel_with_cache(
            RoiId(10),
            "Pancreas".to_string(),
            VoxelGeometry {
                dimensions: [4, 4, 4],
                spacing: [1.0, 1.0, 1.0],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            vec![0; 64],
            None,
        );

        roi.enqueue_rebuild(RoiJobKind::RebuildContourCache);
        roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);

        assert_eq!(roi.job_state.pending.len(), 2);
        assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildVoxelCache));
        assert_eq!(roi.start_queued_job(), Some(RoiJobKind::RebuildVoxelCache));
        roi.finish_job(RoiJobKind::RebuildVoxelCache);
        assert_eq!(
            roi.start_queued_job(),
            Some(RoiJobKind::RebuildContourCache)
        );
        assert_eq!(
            roi.running_job_kind(),
            Some(RoiJobKind::RebuildContourCache)
        );
    }

    #[test]
    fn test_interactive_job_priority_and_preview_supersession() {
        let mut roi = Roi::new_voxel_with_cache(
            RoiId(20),
            "Priority".to_string(),
            VoxelGeometry {
                dimensions: [8, 8, 8],
                spacing: [1.0; 3],
                origin: [0.0; 3],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            vec![0; 512],
            None,
        );
        roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
        roi.enqueue_job(RoiJobRequest {
            kind: RoiJobKind::RebuildMeshCache,
            source_generation: 1,
            preview_revision: Some(1),
            priority: RoiJobPriority::InteractivePreview,
            dirty_region: RoiDirtyRegion::VoxelAabb {
                min: [2, 2, 2],
                max: [3, 3, 3],
            },
        });
        roi.enqueue_job(RoiJobRequest {
            kind: RoiJobKind::RebuildMeshCache,
            source_generation: 1,
            preview_revision: Some(2),
            priority: RoiJobPriority::InteractivePreview,
            dirty_region: RoiDirtyRegion::VoxelAabb {
                min: [4, 4, 4],
                max: [5, 5, 5],
            },
        });

        assert_eq!(roi.job_state.pending.len(), 2);
        let request = roi.job_state.pending[0];
        assert_eq!(request.kind, RoiJobKind::RebuildMeshCache);
        assert_eq!(request.preview_revision, Some(2));
        assert_eq!(
            request.dirty_region,
            RoiDirtyRegion::VoxelAabb {
                min: [2, 2, 2],
                max: [5, 5, 5]
            }
        );
        assert_eq!(roi.job_metrics.max_queue_depth, 2);
    }

    #[test]
    fn test_preview_revision_is_monotonic_and_explicitly_ends() {
        let mut roi = Roi::new_contour(
            RoiId(21),
            "Preview".to_string(),
            ContourData {
                active_plane_family: PlaneFamily::Axial,
                slices: Vec::new(),
            },
        );

        assert_eq!(roi.begin_preview(), 1);
        assert_eq!(roi.begin_preview(), 2);
        assert!(roi.preview_state.active);
        roi.end_preview();
        assert!(!roi.preview_state.active);
        assert_eq!(roi.preview_state.revision, 2);
    }

    #[test]
    fn test_contour_view_cache_is_bounded() {
        let contour = ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: Vec::new(),
        };
        let mut roi = Roi::new_contour(RoiId(22), "Bounded".to_string(), contour.clone());
        for index in 0..=MAX_CONTOUR_VIEW_CACHE_ENTRIES {
            let mut plane = test_plane_definition(PlaneFamily::Coronal);
            plane.origin_mm[1] = index as f32;
            roi.upsert_contour_view_cache(
                ContourViewKey::from_plane(plane),
                contour.clone(),
                1,
                CacheViewState::Current,
            );
        }

        let cache = roi.contour_cache().unwrap();
        assert_eq!(cache.views.len(), MAX_CONTOUR_VIEW_CACHE_ENTRIES);
        assert_eq!(cache.views[0].key.plane.origin_mm[1], 1.0);
    }

    #[test]
    fn test_finish_cache_rebuild_marks_cache_current_and_clears_job() {
        let mut roi = Roi::new_voxel_with_cache(
            RoiId(11),
            "Heart".to_string(),
            VoxelGeometry {
                dimensions: [4, 4, 4],
                spacing: [1.0, 1.0, 1.0],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            vec![0; 64],
            None,
        );

        roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
        assert_eq!(roi.start_queued_job(), Some(RoiJobKind::RebuildVoxelCache));

        roi.finish_cache_rebuild(RoiCacheKind::Voxel);

        assert!(!roi.dirty_state.authoritative_dirty);
        assert!(!roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert!(roi.is_cache_current(RoiCacheKind::Voxel));
        assert_eq!(roi.running_job_kind(), None);
    }

    #[test]
    fn test_voxel_geometry_is_preserved_on_constructor() {
        let geometry = VoxelGeometry {
            dimensions: [12, 10, 8],
            spacing: [0.8, 0.8, 1.5],
            origin: [0.0, 0.0, 0.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };

        let roi = Roi::new_voxel_with_cache(
            RoiId(13),
            "Gallbladder".to_string(),
            geometry,
            vec![0; 12 * 10 * 8],
            None,
        );

        match roi.authoritative_data {
            RoiAuthoritativeData::Voxel(voxel) => assert_eq!(voxel.geometry, geometry),
            _ => panic!("expected voxel roi"),
        }
    }

    #[test]
    fn test_contour_data_preserves_active_plane_family() {
        let contour = ContourData {
            active_plane_family: PlaneFamily::Oblique,
            slices: Vec::new(),
        };
        assert_eq!(contour.active_plane_family, PlaneFamily::Oblique);
        assert!(contour.is_empty());
        assert!(!contour.has_loops());
    }

    #[test]
    fn test_contour_slice_preserves_plane_definition() {
        let plane = test_plane_definition(PlaneFamily::Coronal);
        let slice = ContourSlice {
            plane,
            loops: vec![ContourLoop {
                points: vec![
                    ContourPoint {
                        local_mm: [0.0, 0.0],
                    },
                    ContourPoint {
                        local_mm: [2.0, 0.0],
                    },
                    ContourPoint {
                        local_mm: [2.0, 2.0],
                    },
                ],
                is_closed: true,
            }],
        };

        assert_eq!(slice.plane, plane);
        assert!(slice.loops[0].is_valid_closed_loop());
    }

    #[test]
    fn test_contour_loop_requires_three_points_when_closed() {
        let loop_with_two_points = ContourLoop {
            points: vec![
                ContourPoint {
                    local_mm: [0.0, 0.0],
                },
                ContourPoint {
                    local_mm: [1.0, 0.0],
                },
            ],
            is_closed: true,
        };
        let loop_with_three_points = ContourLoop {
            points: vec![
                ContourPoint {
                    local_mm: [0.0, 0.0],
                },
                ContourPoint {
                    local_mm: [1.0, 0.0],
                },
                ContourPoint {
                    local_mm: [0.0, 1.0],
                },
            ],
            is_closed: true,
        };

        assert!(!loop_with_two_points.is_valid_closed_loop());
        assert!(loop_with_three_points.is_valid_closed_loop());
    }

    #[test]
    fn test_new_contour_roi_initializes_contour_primary_state() {
        let contour_data = ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: vec![ContourSlice {
                plane: test_plane_definition(PlaneFamily::Axial),
                loops: vec![ContourLoop {
                    points: vec![
                        ContourPoint {
                            local_mm: [0.0, 0.0],
                        },
                        ContourPoint {
                            local_mm: [1.0, 0.0],
                        },
                        ContourPoint {
                            local_mm: [1.0, 1.0],
                        },
                    ],
                    is_closed: true,
                }],
            }],
        };
        let reference_geometry = RoiGeometry::from_legacy_parts(
            [16, 16, 8],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap();
        let expected_identity = reference_geometry.identity();
        let roi = Roi::new_contour_with_geometry(
            RoiId(14),
            "GTV".to_string(),
            reference_geometry,
            contour_data.clone(),
        );

        assert_eq!(roi.metadata.roi_id, RoiId(14));
        assert_eq!(roi.metadata.name, "GTV");
        assert_eq!(roi.primary_representation(), PrimaryRepresentation::Contour);
        assert_eq!(roi.reference_geometry().identity(), expected_identity);
        assert!(matches!(
            roi.authoritative_data,
            RoiAuthoritativeData::Contour(_)
        ));
        assert_eq!(roi.contour_data(), Some(&contour_data));
        assert!(roi.voxel_cache().is_none());
        assert!(roi.session_caches.contour.is_none());
        assert!(roi.session_caches.mesh.is_none());
        assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert!(!roi.is_cache_current(RoiCacheKind::Voxel));
    }

    #[test]
    fn test_contour_accessor_rejects_voxel_roi() {
        let voxel_roi = Roi::new_voxel_with_cache(
            RoiId(15),
            "Body".to_string(),
            VoxelGeometry {
                dimensions: [8, 8, 8],
                spacing: [1.0, 1.0, 1.0],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            vec![0; 8 * 8 * 8],
            None,
        );

        assert!(voxel_roi.contour_data().is_none());
    }

    #[test]
    fn test_new_mesh_roi_initializes_mesh_primary_state() {
        let mesh_data = MeshData {
            vertices: vec![
                MeshVertex {
                    world_mm: [0.0, 0.0, 0.0],
                },
                MeshVertex {
                    world_mm: [1.0, 0.0, 0.0],
                },
                MeshVertex {
                    world_mm: [0.0, 1.0, 0.0],
                },
            ],
            faces: vec![MeshFace {
                vertex_indices: [0, 1, 2],
            }],
        };
        let roi = Roi::new_mesh(RoiId(21), "Surface".to_string(), mesh_data.clone());

        assert_eq!(roi.metadata.roi_id, RoiId(21));
        assert_eq!(roi.metadata.name, "Surface");
        assert_eq!(roi.primary_representation(), PrimaryRepresentation::Mesh);
        assert!(matches!(
            roi.authoritative_data,
            RoiAuthoritativeData::Mesh(_)
        ));
        assert_eq!(roi.mesh_data(), Some(&mesh_data));
        assert!(roi.voxel_cache().is_none());
        assert!(roi.session_caches.contour.is_none());
        assert!(roi.session_caches.mesh.is_none());
        assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert!(roi.is_cache_dirty(RoiCacheKind::Contour));
        assert!(!roi.is_cache_dirty(RoiCacheKind::Mesh));
    }

    #[test]
    fn test_mesh_accessor_rejects_non_mesh_rois() {
        let voxel_roi = Roi::new_voxel_with_cache(
            RoiId(22),
            "Voxel".to_string(),
            VoxelGeometry {
                dimensions: [4, 4, 4],
                spacing: [1.0, 1.0, 1.0],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            vec![0; 4 * 4 * 4],
            None,
        );
        let contour_roi = Roi::new_contour(
            RoiId(23),
            "Contour".to_string(),
            ContourData {
                active_plane_family: PlaneFamily::Axial,
                slices: Vec::new(),
            },
        );

        assert!(voxel_roi.mesh_data().is_none());
        assert!(contour_roi.mesh_data().is_none());
    }

    #[test]
    fn test_mark_mesh_authoritative_changed_invalidates_voxel_and_contour_without_mesh_cache() {
        let mesh_data = MeshData {
            vertices: vec![
                MeshVertex {
                    world_mm: [0.0, 0.0, 0.0],
                },
                MeshVertex {
                    world_mm: [1.0, 0.0, 0.0],
                },
                MeshVertex {
                    world_mm: [0.0, 1.0, 0.0],
                },
            ],
            faces: vec![MeshFace {
                vertex_indices: [0, 1, 2],
            }],
        };
        let mut roi = Roi::new_mesh(RoiId(24), "Mesh".to_string(), mesh_data);

        roi.dirty_state.voxel_cache_dirty = false;
        roi.dirty_state.contour_cache_dirty = false;
        roi.dirty_state.mesh_cache_dirty = false;
        roi.dirty_state.generations.voxel = roi.dirty_state.generations.authoritative;
        roi.dirty_state.generations.contour = roi.dirty_state.generations.authoritative;
        roi.dirty_state.generations.mesh = roi.dirty_state.generations.authoritative;

        roi.mark_mesh_authoritative_changed();

        assert!(roi.dirty_state.authoritative_dirty);
        assert_eq!(roi.dirty_state.generations.authoritative, 2);
        assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert!(roi.is_cache_dirty(RoiCacheKind::Contour));
        assert!(!roi.is_cache_dirty(RoiCacheKind::Mesh));
        assert!(!roi.is_cache_current(RoiCacheKind::Voxel));
        assert!(!roi.is_cache_current(RoiCacheKind::Contour));
    }

    #[test]
    fn test_mark_mesh_authoritative_changed_invalidates_mesh_cache_when_present() {
        let mesh_data = MeshData {
            vertices: vec![
                MeshVertex {
                    world_mm: [0.0, 0.0, 0.0],
                },
                MeshVertex {
                    world_mm: [1.0, 0.0, 0.0],
                },
                MeshVertex {
                    world_mm: [0.0, 1.0, 0.0],
                },
            ],
            faces: vec![MeshFace {
                vertex_indices: [0, 1, 2],
            }],
        };
        let mut roi = Roi::new_mesh(RoiId(25), "Mesh Cached".to_string(), mesh_data.clone());
        roi.session_caches.mesh = Some(MeshCache {
            data: mesh_data,
            chunks: None,
        });
        roi.dirty_state.voxel_cache_dirty = false;
        roi.dirty_state.contour_cache_dirty = false;
        roi.dirty_state.mesh_cache_dirty = false;
        roi.dirty_state.generations.voxel = roi.dirty_state.generations.authoritative;
        roi.dirty_state.generations.contour = roi.dirty_state.generations.authoritative;
        roi.dirty_state.generations.mesh = roi.dirty_state.generations.authoritative;

        roi.mark_mesh_authoritative_changed();

        assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert!(roi.is_cache_dirty(RoiCacheKind::Contour));
        assert!(roi.is_cache_dirty(RoiCacheKind::Mesh));
        assert!(!roi.is_cache_current(RoiCacheKind::Mesh));
    }

    #[test]
    fn test_finish_mesh_cache_rebuild_marks_mesh_cache_current_to_authoritative_generation() {
        let mesh_data = MeshData {
            vertices: vec![
                MeshVertex {
                    world_mm: [0.0, 0.0, 0.0],
                },
                MeshVertex {
                    world_mm: [1.0, 0.0, 0.0],
                },
                MeshVertex {
                    world_mm: [0.0, 1.0, 0.0],
                },
            ],
            faces: vec![MeshFace {
                vertex_indices: [0, 1, 2],
            }],
        };
        let mut roi = Roi::new_mesh(RoiId(26), "Mesh Rebuild".to_string(), mesh_data.clone());
        roi.session_caches.mesh = Some(MeshCache {
            data: mesh_data,
            chunks: None,
        });
        roi.dirty_state.mesh_cache_dirty = true;
        roi.enqueue_rebuild(RoiJobKind::RebuildMeshCache);
        assert_eq!(roi.start_queued_job(), Some(RoiJobKind::RebuildMeshCache));

        roi.finish_cache_rebuild(RoiCacheKind::Mesh);

        assert!(!roi.is_cache_dirty(RoiCacheKind::Mesh));
        assert!(roi.is_cache_current(RoiCacheKind::Mesh));
        assert_eq!(
            roi.cache_generation(RoiCacheKind::Mesh),
            roi.dirty_state.generations.authoritative
        );
        assert_eq!(roi.running_job_kind(), None);
    }
}
