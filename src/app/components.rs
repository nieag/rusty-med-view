pub use crate::app::annotations::{Anchor, CommentView, EntityId, Label, Provenance, Text, Thread};
pub use crate::app::roi::{
    ContourBody, ContourData, ContourLoop, ContourMovePreview, ContourPoint, ContourSlice,
    MeshBody, MeshData, MeshEditPreview, MeshFace, MeshVertex, PrimaryRepresentation, RoiBody,
    RoiId, RoiMetadata, VoxelBody, VoxelData, VoxelGeometry,
};
use crate::convert::{ChunkedMeshData, GeometryIdentity, PlaneDefinition, PlaneFamily};
pub use crate::model::{LoadedLabel, ViewMode, VolumeData};
use web_time::Instant;

use winit::keyboard::ModifiersState;

mod roi_types;
pub use roi_types::*;

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

// --- Annotation panel state (the annotations themselves are entities: `app::annotations`) ---
#[derive(Clone, Debug, Default)]
pub struct AnnotationState {
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
    /// Display name recorded as the author of what this user creates (no accounts yet).
    pub user: String,
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
            user: "Reviewer".to_string(),
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
