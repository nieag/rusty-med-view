use hecs::Entity;
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
    pub start_time: std::time::Instant,
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
    pub resolution: [f32; 2],
    pub time: f32,
    pub view_mode: u32,
    pub cursor_pos: [f32; 4],
    pub zoom: f32,          // <--- NEW
    pub _padding: [f32; 3], // <--- NEW: Pad to 48 bytes (16-byte alignment rule)
}

pub struct ViewState {
    // Index 0=3D, 1=XY, 2=XZ, 3=YZ
    pub zoom: [f32; 4],
}

pub struct InputState {
    pub last_mouse_pos: [f64; 2],
    pub active_viewport: u8,       // 0=3D, 1=Top(XY), 2=Front(XZ), 3=Side(YZ)
    pub modifiers: ModifiersState, // <--- NEW FIELD
}
