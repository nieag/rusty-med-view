// use hecs::Entity;
use winit::keyboard::ModifiersState;
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

// GPU Data Structures (Uniforms)
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniforms {
    pub cursor_pos: [f32; 4],
    pub resolution: [f32; 2],
    pub mouse_uv: [f32; 2],
    pub pan: [f32; 2],
    pub zoom: f32,
    pub time: f32,
    pub view_mode: u32,
    pub _pad_a: u32,
    pub _pad_b: u32,
    pub _pad_c: u32,
    pub volume_dims: [u32; 4],
    pub volume_spacing: [f32; 4],
}

pub struct ViewState {
    pub zoom: [f32; 4],
    pub pan: [[f32; 2]; 4], // NEW: Per-viewport pan offsets
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
            self.dimensions[0] as f32 / max,
            self.dimensions[1] as f32 / max,
            self.dimensions[2] as f32 / max,
        ]
    }
}

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

pub struct InputState {
    pub last_mouse_pos: [f64; 2],
    pub mouse_uv: [f32; 2],
    pub active_viewport: u8,
    pub modifiers: ModifiersState,
    pub is_dragging: bool,        // NEW: Dragging state
    pub drag_start_pos: [f32; 2], // NEW: Screen pos at drag start
    pub drag_start_pan: [f32; 2], // NEW: Pan offset at drag start
}
