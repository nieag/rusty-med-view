use glam::Vec3;

use winit::keyboard::ModifiersState;
use web_time::Instant;

// --- Basic Tags ---
pub struct CursorTag;
pub struct MainVolumeTag;
pub struct SegmentationTag;

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
    // --- Brush preview ---
    pub brush_preview: [f32; 4], // [brush_size, active (0/1), viewport, _]
    pub brush_center_voxel: [f32; 4], // [voxel_x, voxel_y, voxel_z, valid (0/1)]
    // ---
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
    Brush,
    Eraser,
    ContourDraw,
}

pub struct EditorState {
    pub active_layer: Option<hecs::Entity>,
    pub active_tool: EditorTool,
    pub brush_size: f32,
    pub active_label_index: u8,
    pub last_paint_voxel: Option<[u32; 3]>,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            active_layer: None,
            active_tool: EditorTool::default(),
            brush_size: 5.0,
            active_label_index: 1,
            last_paint_voxel: None,
        }
    }
}

pub struct Segmentation {
    pub name: String,
    pub is_visible: bool,
}

pub struct LayerSettings {
    pub opacity: f32,
}

pub struct LabelmapData {
    pub dimensions: [u32; 3],
    pub raw_data: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub struct SdfPreviewState {
    pub enabled: bool,
    pub opacity: f32,
    pub show_zero_isoline: bool,
    pub value_window_mm: f32,
    pub show_3d_surface: bool,
}

impl Default for SdfPreviewState {
    fn default() -> Self {
        Self {
            enabled: false,
            opacity: 0.35,
            show_zero_isoline: true,
            value_window_mm: 8.0,
            show_3d_surface: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SegPerfConfig {
    pub live_enabled: bool,
    pub live_mesh_enabled: bool,
    pub webgpu_required: bool,
    pub fallback_active: bool,
    pub live_resolution_scale: f32,
    pub full_resolution_scale: f32,
    pub frame_budget_ms: f32,
    pub live_interval_ms: f32,
    pub live_mesh_interval_ms: f32,
    pub max_roi_margin_mm: f32,
    pub next_finalize_index: usize,
    pub last_live_update_at: Option<Instant>,
    pub last_live_mesh_at: Option<Instant>,
    pub last_sdf_ms: f32,
    pub last_mesh_ms: f32,
    pub queue_depth: u32,
}

impl Default for SegPerfConfig {
    fn default() -> Self {
        Self {
            live_enabled: true,
            live_mesh_enabled: false,
            webgpu_required: true,
            fallback_active: false,
            live_resolution_scale: 0.5,
            full_resolution_scale: 1.5,
            frame_budget_ms: 6.0,
            live_interval_ms: 80.0,
            live_mesh_interval_ms: 220.0,
            max_roi_margin_mm: 12.0,
            next_finalize_index: 0,
            last_live_update_at: None,
            last_live_mesh_at: None,
            last_sdf_ms: 0.0,
            last_mesh_ms: 0.0,
            queue_depth: 0,
        }
    }
}

pub enum Representation {
    Voxel(GpuVolumeResources),
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
    pub segments: hecs::Entity,
    pub sdf_preview: hecs::Entity,
    pub seg_perf: hecs::Entity,
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
}
