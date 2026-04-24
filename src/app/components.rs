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
}

// --- Volume Data ---
pub struct VolumeData {
    pub dimensions: [u32; 3],
    pub spacing: [f32; 3],
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
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniforms {
    pub cursor_pos: [f32; 4],
    pub volume_dims: [u32; 4],
    pub volume_spacing: [f32; 4],
    pub overlay_opacities: [f32; 4],
    pub window_params: [f32; 4],
    pub resolution: [f32; 2],
    pub mouse_uv: [f32; 2],
    pub pan: [f32; 2],
    pub zoom_pivot: [f32; 2],
    pub rotation: [f32; 4], // Quaternion
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
}

#[derive(Default)]
pub struct EditorState {
    pub active_roi: Option<hecs::Entity>,
    pub active_tool: EditorTool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RoiId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimaryRepresentation {
    Voxel,
    Contour,
    Mesh,
}

#[derive(Debug, Clone)]
pub struct RoiMetadata {
    pub roi_id: RoiId,
    pub name: String,
    pub is_visible: bool,
    pub is_locked: bool,
    pub color: [f32; 4],
}

pub struct LayerSettings {
    pub opacity: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoxelData {
    pub dimensions: [u32; 3],
    pub raw_data: Vec<u8>,
}

pub enum RoiAuthoritativeData {
    Voxel(VoxelData),
    Contour,
    Mesh,
}

#[derive(Default)]
pub struct RoiSessionCaches {
    pub voxel: Option<GpuVolumeResources>,
    pub contour: Option<ContourCache>,
    pub mesh: Option<MeshCache>,
}

pub struct ContourCache;

pub struct MeshCache;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RoiJobStatus {
    #[default]
    Idle,
    Queued(RoiJobKind),
    Running(RoiJobKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RoiJobState {
    pub current: RoiJobStatus,
}

pub struct Roi {
    pub metadata: RoiMetadata,
    pub primary_representation: PrimaryRepresentation,
    pub authoritative_data: RoiAuthoritativeData,
    pub session_caches: RoiSessionCaches,
    pub dirty_state: RoiDirtyState,
    pub job_state: RoiJobState,
}

impl Roi {
    pub fn new_voxel(
        roi_id: RoiId,
        name: String,
        dimensions: [u32; 3],
        raw_data: Vec<u8>,
        gpu_resources: GpuVolumeResources,
    ) -> Self {
        Self::new_voxel_with_cache(roi_id, name, dimensions, raw_data, Some(gpu_resources))
    }

    pub fn new_voxel_with_cache(
        roi_id: RoiId,
        name: String,
        dimensions: [u32; 3],
        raw_data: Vec<u8>,
        gpu_resources: Option<GpuVolumeResources>,
    ) -> Self {
        Self {
            metadata: RoiMetadata {
                roi_id,
                name,
                is_visible: false,
                is_locked: false,
                color: [1.0, 0.2, 0.2, 1.0],
            },
            primary_representation: PrimaryRepresentation::Voxel,
            authoritative_data: RoiAuthoritativeData::Voxel(VoxelData {
                dimensions,
                raw_data,
            }),
            session_caches: RoiSessionCaches {
                voxel: gpu_resources,
                contour: None,
                mesh: None,
            },
            dirty_state: RoiDirtyState::default(),
            job_state: RoiJobState::default(),
        }
    }

    pub fn voxel_cache(&self) -> Option<&GpuVolumeResources> {
        self.session_caches.voxel.as_ref()
    }

    pub fn voxel_cache_mut(&mut self) -> Option<&mut GpuVolumeResources> {
        self.session_caches.voxel.as_mut()
    }
}

#[derive(Clone, Copy)]
pub struct VolumeWindowing {
    pub center: f32,
    pub width: f32,
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

    #[test]
    fn test_aspect_ratio_cubic() {
        let vol = VolumeData {
            dimensions: [100, 100, 100],
            spacing: [1.0, 1.0, 1.0],
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
            intensities: vec![],
            intensity_range: [0.0, 1.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let ar = vol.aspect_ratios();
        assert_eq!(ar, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn test_new_voxel_roi_initializes_voxel_primary_state() {
        let roi = Roi::new_voxel_with_cache(
            RoiId(7),
            "Liver".to_string(),
            [16, 16, 8],
            vec![1; 16 * 16 * 8],
            None,
        );

        assert_eq!(roi.metadata.roi_id, RoiId(7));
        assert_eq!(roi.metadata.name, "Liver");
        assert_eq!(roi.primary_representation, PrimaryRepresentation::Voxel);
        assert!(matches!(
            roi.authoritative_data,
            RoiAuthoritativeData::Voxel(VoxelData {
                dimensions: [16, 16, 8],
                ..
            })
        ));
        assert!(roi.voxel_cache().is_none());
        assert!(roi.session_caches.contour.is_none());
        assert!(roi.session_caches.mesh.is_none());
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
}
