//! ECS components for the medical imaging viewer.
//!
//! Components represent data attached to entities. Key component types:
//! - **Transform/View**: Position, zoom, pan, rotation state
//! - **Volume**: Main volume data and GPU resources
//! - **Segmentation**: Labelmap overlay layers
//! - **Input/GUI**: User interaction state

use winit::keyboard::ModifiersState;

/// 3D transform component with position and rotation.
#[derive(Debug, Copy, Clone)]
pub struct Transform {
    pub position: [f32; 3],
    pub rotation: [f32; 3],
}

/// Tag component marking an entity as the 3D cursor.
pub struct CursorTag;

/// Camera rig settings (used for animation timing).
pub struct CameraRig {
    pub radius: f32,
    pub speed: f32,
    pub start_time: web_time::Instant,
}

/// Window and viewport dimensions.
pub struct WindowSettings {
    pub width: u32,
    pub height: u32,
    /// Viewport rectangle [x, y, width, height] in pixels
    pub viewport_rect: [f32; 4],
}

/// GPU resources for a volume, stored as a component
pub struct GpuVolumeResources {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub bind_group: wgpu::BindGroup,
}

/// GPU-aligned uniform struct passed to shaders.
///
/// **Important**: This struct must be kept in sync with the WGSL `Uniforms`
/// struct in `shader.wgsl`. Fields are ordered to satisfy 16-byte alignment.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniforms {
    pub cursor_pos: [f32; 4],        // 0
    pub volume_dims: [u32; 4],       // 16
    pub volume_spacing: [f32; 4],    // 32
    pub overlay_opacities: [f32; 4], // 48
    pub window_params: [f32; 4],     // 64: [center, width, data_min, data_max]
    pub resolution: [f32; 2],        // 80
    pub mouse_uv: [f32; 2],          // 88
    pub pan: [f32; 2],               // 96
    pub zoom_pivot: [f32; 2],        // 104
    pub rotation: [f32; 4],          // 112 (quaternion x, y, z, w)
    pub zoom: f32,                   // 128
    pub time: f32,                   // 132
    pub view_mode: u32,              // 136
    pub overlay_flags: u32,          // 140 (total 144 bytes)
}

/// Per-viewport view state (zoom, pan, pivot, rotation).
///
/// Each array index corresponds to a viewport:
/// - 0: 3D view
/// - 1: Axial slice
/// - 2: Coronal slice
/// - 3: Sagittal slice
pub struct ViewState {
    pub zoom: [f32; 4],
    /// Per-viewport pan offsets in UV space
    pub pan: [[f32; 2]; 4],
    /// Per-viewport zoom pivot (where zoom centers)
    pub pivot: [[f32; 2]; 4],
    /// Per-viewport rotation as quaternion [x, y, z, w]
    pub rotation: [[f32; 4]; 4],
}

/// Volume data component - stores loaded volume information
pub struct VolumeData {
    /// Volume dimensions [width, height, depth]
    pub dimensions: [u32; 3],
    /// Voxel spacing in mm [x, y, z] - for anisotropic volumes
    pub spacing: [f32; 3],
    /// Raw intensity data for CPU-side picking (f32 values)
    pub intensities: Vec<f32>,
    /// Intensity range [min, max] for windowing
    pub intensity_range: [f32; 2],
}

impl VolumeData {
    /// Get aspect ratios relative to max physical dimension
    pub fn aspect_ratios(&self) -> [f32; 3] {
        let physical_size = [
            self.dimensions[0] as f32 * self.spacing[0],
            self.dimensions[1] as f32 * self.spacing[1],
            self.dimensions[2] as f32 * self.spacing[2],
        ];
        let max_phys = physical_size[0]
            .max(physical_size[1])
            .max(physical_size[2])
            .max(1e-6); // Avoid division by zero
        [
            physical_size[0] / max_phys,
            physical_size[1] / max_phys,
            physical_size[2] / max_phys,
        ]
    }
}

/// Tag to identify the primary volume
pub struct MainVolumeTag;

/// State for async volume loading operations
#[derive(Clone)]
pub enum VolumeLoadingState {
    /// Ready to load a new file
    Ready,
    /// Currently loading a file
    Loading,
    /// Successfully loaded a volume
    Loaded { filename: String },
    /// Loading failed with error
    Error { message: String },
}

#[derive(Debug, Default)]
pub struct GuiState {
    pub load_requested: bool,
    pub load_label_requested: bool, // NEW
    pub status_message: Option<String>,
}

/// Result of an async load operation (either volume or labelmap)
pub enum LoadResult {
    Volume(crate::nifti_loader::LoadedVolume),
    Label(LoadedLabel),
}

#[derive(Debug)]
pub struct LoadedLabel {
    pub dimensions: [u32; 3],
    pub spacing: [f32; 3],
    pub data: Vec<u8>,    // Raw R8Uint data
    pub filename: String, // Source filename for layer naming
}

/// Mouse and keyboard input state.
///
/// Tracks current mouse position, active viewport, modifier keys,
/// and drag/rotation gesture state.
pub struct InputState {
    pub last_mouse_pos: [f64; 2],
    pub mouse_uv: [f32; 2],
    pub active_viewport: u8,
    pub modifiers: ModifiersState,
    /// True while middle-click or Alt+left drag is active
    pub is_dragging: bool,
    pub drag_start_pos: [f32; 2],
    pub drag_start_pan: [f32; 2],
    /// True while right-click rotation drag is active
    pub is_rotating: bool,
    pub rotation_start_pos: [f32; 2],
    pub rotation_start_val: [f32; 4],
}

// --- NEW: Labelmap & Layering Components ---

/// Metadata for a segmentation layer
pub struct Segmentation {
    pub name: String,
    pub is_visible: bool,
}

/// Settings for how a layer is rendered
pub struct LayerSettings {
    pub opacity: f32,
    pub active_representation: usize, // Index into a list of representations (if we had a list)
}

/// Polymorphic representation of the segmentation data
pub enum Representation {
    Voxel(GpuVolumeResources),
    // Future: Mesh(MeshResources),
    // Future: Contour(ContourResources),
}

/// Tag to identify an entity as a Segmentation Layer
pub struct SegmentationTag;

/// Data for a Voxel Labelmap (CPU side)
pub struct LabelmapData {
    pub dimensions: [u32; 3],
    pub spacing: [f32; 3], // Added spacing
    pub raw_data: Vec<u8>, // R8Uint indices
}

// --- NEW: Editor Components ---

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorTool {
    Navigation,
    Brush,
    Eraser,
}

pub struct EditorState {
    pub active_tool: EditorTool,
    pub brush_size: f32,
    pub active_label_index: u8,
    pub active_layer: Option<hecs::Entity>,
    /// Last painted voxel position for stroke interpolation
    pub last_paint_voxel: Option<[u32; 3]>,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            active_tool: EditorTool::Navigation,
            brush_size: 5.0,
            active_label_index: 1,
            active_layer: None,
            last_paint_voxel: None,
        }
    }
}

// --- NEW: Windowing Component ---

/// Volume windowing/contrast settings for intensity mapping.
///
/// Controls how volume intensities are mapped to display values.
/// Uses normalized 0-1 range (since textures are pre-normalized).
pub struct VolumeWindowing {
    /// Window center (L) in Hounsfield Units
    pub center: f32,
    /// Window width (W) in Hounsfield Units
    pub width: f32,
}

impl Default for VolumeWindowing {
    fn default() -> Self {
        // Default to soft tissue window
        Self {
            center: 40.0, // Soft tissue center HU
            width: 400.0, // Soft tissue width HU
        }
    }
}
