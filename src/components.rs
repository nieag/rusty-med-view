use glam::Vec3;

use web_time::Instant;
use winit::keyboard::ModifiersState;

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

pub struct CameraRig {
    pub speed: f32,
    pub start_time: Instant,
}

pub struct Transform {
    pub position: [f32; 3],
}

pub struct ViewState {
    pub zoom: [f32; 4],
    pub pan: [[f32; 2]; 4],
    pub pivot: [[f32; 2]; 4],
    pub rotation: [[f32; 4]; 4],
}

pub struct InputState {
    pub last_mouse_pos: [f64; 2],
    pub mouse_uv: [f32; 2],
    pub active_viewport: u32,
    pub modifiers: ModifiersState,
    pub is_dragging: bool,
    pub is_panning: bool,
    pub drag_start_pos: [f32; 2],
    pub drag_start_pan: [f32; 2],
    pub is_rotating: bool,
    pub rotation_start_pos: [f32; 2],
    pub rotation_start_val: [f32; 4],
    pub egui_wants_input: bool,
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
    pub time: f32,
    pub view_mode: u32,
    pub overlay_flags: u32,
}

// --- GUI / Editor ---
#[derive(Clone)]
pub struct GuiState {
    pub load_requested: bool,
    pub load_label_requested: bool,
    pub status_message: Option<String>,
    pub bind_group_needs_rebuild: bool,
}

#[derive(Clone, Copy)]
pub enum VolumeLoadingState {
    Ready,
    Loading,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditorTool {
    #[default]
    Navigation,
    Brush,
    Eraser,
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
    pub active_representation: u32,
}

pub struct LabelmapData {
    pub dimensions: [u32; 3],
    pub spacing: [f32; 3],
    pub raw_data: Vec<u8>,
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
use crate::nifti_loader; // Assuming crate root has nifti_loader mod

#[derive(Debug)]
pub enum LoadResult {
    Volume(nifti_loader::LoadedVolume),
    Label(LoadedLabel), // Changed to local LoadedLabel
}

#[derive(Debug)]
pub struct LoadedLabel {
    pub dimensions: [u32; 3],
    pub spacing: [f32; 3],
    pub data: Vec<u8>,
    pub filename: String,
}

// --- ANNOTATIONS (kept for labels/text, positions sync to OverlayState) ---
#[derive(Clone, Debug)]
pub struct Annotation {
    pub world_pos: Vec3,
    pub label: String,
}

#[derive(Clone, Debug, Default)]
pub struct AnnotationState {
    pub annotations: Vec<Annotation>,
}

// --- Singleton Entity Registry ---
#[derive(Clone, Copy)]
pub struct AppEntities {
    pub input: hecs::Entity,
    pub view: hecs::Entity,
    pub editor: hecs::Entity,
    pub gui_state: hecs::Entity,
    pub loading: hecs::Entity,
    pub volume_windowing: hecs::Entity,
    pub annotations: hecs::Entity,
    pub overlay: hecs::Entity,
    pub cursor: hecs::Entity,
    pub camera_rig: hecs::Entity,
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
}
