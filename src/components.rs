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
    pub cursor_pos: [f32; 4], // 0
    pub resolution: [f32; 2], // 16
    pub mouse_uv: [f32; 2],   // 24
    pub pan: [f32; 2],        // 32
    pub zoom: f32,            // 40
    pub time: f32,            // 44
    pub view_mode: u32,       // 48
    pub _pad: [u32; 3],       // 52 (total 64)
}

pub struct ViewState {
    pub zoom: [f32; 4],
    pub pan: [[f32; 2]; 4], // NEW: Per-viewport pan offsets
}

pub struct VolumeData {
    pub size: u32,
    pub densities: Vec<u8>, // Store the raw density values for CPU-side picking
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
