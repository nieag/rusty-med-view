// src/geometry.rs
use crate::components::ViewState;
use glam::{Quat, Vec3};

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub tex_coords: [f32; 2],
}

impl Vertex {
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                },
            ],
        }
    }
}

// The Full-Screen Quad (used for the raymarching viewports)
pub const QUAD_VERTICES: &[Vertex] = &[
    Vertex {
        position: [-1.0, -1.0, 0.0],
        tex_coords: [0.0, 1.0],
    }, // Bottom-Left
    Vertex {
        position: [1.0, -1.0, 0.0],
        tex_coords: [1.0, 1.0],
    }, // Bottom-Right
    Vertex {
        position: [1.0, 1.0, 0.0],
        tex_coords: [1.0, 0.0],
    }, // Top-Right
    Vertex {
        position: [-1.0, 1.0, 0.0],
        tex_coords: [0.0, 0.0],
    }, // Top-Left
];

pub const QUAD_INDICES: &[u16] = &[
    0, 1, 2, // Triangle 1
    0, 2, 3, // Triangle 2
];

/// Project a world position (0..1) to Normalized Device Coordinates (0..1 relative to viewport)
pub fn world_to_ndc(
    pos: Vec3,
    viewport_idx: usize,
    view: &ViewState,
    aspect_ratios: [f32; 3],
    screen_aspect: f32,
) -> Option<[f32; 2]> {
    let zoom = view.zoom[viewport_idx];
    let pan = view.pan[viewport_idx];
    let pivot = view.pivot[viewport_idx];

    if viewport_idx > 0 {
        // --- 2D Viewports ---
        let slice_aspect = match viewport_idx {
            1 => aspect_ratios[0] / aspect_ratios[1], // Axial (X/Y)
            2 => aspect_ratios[0] / aspect_ratios[2], // Coronal (X/Z)
            3 => aspect_ratios[1] / aspect_ratios[2], // Sagittal (Y/Z)
            _ => 1.0,
        };

        let k = screen_aspect / slice_aspect;

        let (u, v) = match viewport_idx {
            1 => (pos.x, pos.y), // Axial
            2 => (pos.x, pos.z), // Coronal
            3 => (pos.y, pos.z), // Sagittal
            _ => return None,
        };

        let ndc_x = ((u - pivot[0] - pan[0]) * zoom / k) + pivot[0];
        let ndc_y = ((v - pivot[1] - pan[1]) * zoom) + pivot[1];

        Some([ndc_x, ndc_y])
    } else {
        // --- 3D Viewport ---
        let p_centered = pos - Vec3::new(0.5, 0.5, 0.5);
        let p_scaled = p_centered * Vec3::from(aspect_ratios);
        let rot_quat = Quat::from_array(view.rotation[0]);
        let p_rotated = rot_quat * p_scaled;
        let p_cam = p_rotated + Vec3::new(0.0, 0.0, 3.5);

        if p_cam.z <= 0.1 {
            return None;
        }

        let proj_x = p_cam.x / p_cam.z;
        let proj_y = p_cam.y / p_cam.z;

        let uv_centered_x = proj_x / screen_aspect;
        let uv_centered_y = proj_y;

        let zoomed_uv_x = uv_centered_x + 0.5;
        let zoomed_uv_y = uv_centered_y + 0.5;

        let final_u = ((zoomed_uv_x - pivot[0] - pan[0]) * zoom) + pivot[0];
        let final_v = ((zoomed_uv_y - pivot[1] - pan[1]) * zoom) + pivot[1];

        Some([final_u, final_v])
    }
}
