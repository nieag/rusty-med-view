// use hecs::Entity;
use winit::keyboard::ModifiersState;
// use wgpu; // Implicitly available via Cargo.toml dependencies
// Standard Component used by almost everything
#[derive(Debug, Copy, Clone)]
pub struct Transform {
    pub position: [f32; 3],
    pub rotation: [f32; 3],
}

// Tag for the cursor
pub struct CursorTag;

// Camera settings
pub struct CameraRig {
    pub radius: f32,
    pub speed: f32,
    pub start_time: web_time::Instant,
}

// Window state
pub struct WindowSettings {
    pub width: u32,
    pub height: u32,
}

/// GPU resources for a volume, stored as a component
pub struct GpuVolumeResources {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub bind_group: wgpu::BindGroup,
}

// GPU Data Structures (Uniforms)
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniforms {
    pub cursor_pos: [f32; 4],        // 0
    pub volume_dims: [u32; 4],       // 16
    pub volume_spacing: [f32; 4],    // 32
    pub overlay_opacities: [f32; 4], // 48
    pub resolution: [f32; 2],        // 64
    pub mouse_uv: [f32; 2],          // 72
    pub pan: [f32; 2],               // 80
    pub zoom_pivot: [f32; 2],        // 88
    pub rotation: [f32; 4],          // 96 (Yaw, Pitch, Roll, Padding)
    pub zoom: f32,                   // 112
    pub time: f32,                   // 116
    pub view_mode: u32,              // 120
    pub overlay_flags: u32,          // 124 (total 128)
}

pub struct ViewState {
    pub zoom: [f32; 4],
    pub pan: [[f32; 2]; 4],      // Per-viewport pan offsets
    pub pivot: [[f32; 2]; 4],    // Per-viewport zoom pivot
    pub rotation: [[f32; 3]; 4], // NEW: Per-viewport rotation [yaw, pitch, roll]
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
    /// Get the largest dimension (used for normalization)
    pub fn max_dimension(&self) -> u32 {
        self.dimensions[0]
            .max(self.dimensions[1])
            .max(self.dimensions[2])
    }

    /// Get aspect ratios relative to max dimension
    pub fn aspect_ratios(&self) -> [f32; 3] {
        let max = self.max_dimension() as f32;
        [
            (self.dimensions[0] as f32 * self.spacing[0]) / max,
            (self.dimensions[1] as f32 * self.spacing[1]) / max,
            (self.dimensions[2] as f32 * self.spacing[2]) / max,
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
    pub data: Vec<u8>, // Raw R8Uint data
}

pub struct InputState {
    pub last_mouse_pos: [f64; 2],
    pub mouse_uv: [f32; 2],
    pub active_viewport: u8,
    pub modifiers: ModifiersState,
    pub is_dragging: bool,            // NEW: Dragging state
    pub drag_start_pos: [f32; 2],     // NEW: Screen pos at drag start
    pub drag_start_pan: [f32; 2],     // NEW: Pan offset at drag start
    pub is_rotating: bool,            // NEW: Rotating state (right click)
    pub rotation_start_pos: [f32; 2], // NEW: Screen pos at rotation start
    pub rotation_start_val: [f32; 3], // NEW: Rotation val at start [yaw, pitch, roll]
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
