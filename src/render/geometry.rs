// src/geometry.rs
// use crate::components::ViewState; // Removed
use glam::Vec3;

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
    zoom: f32,
    pan: [f32; 2],
    pivot: [f32; 2],
    rotation: [f32; 4],         // User rotation
    data_orientation: [f32; 4], // NIfTI orientation
    aspect_ratios: [f32; 3],
    screen_aspect: f32,
) -> Option<[f32; 2]> {
    if viewport_idx > 0 {
        // --- 2D Viewports ---
        let plane = crate::util::orientation::SlicePlane::from_viewport(viewport_idx as u32)?;
        let [ndc_x_relative, ndc_y_relative] =
            plane.volume_to_screen_uv(pos.into(), data_orientation);
        let k = screen_aspect / plane.slice_aspect(aspect_ratios, data_orientation);

        let ndc_x = ((ndc_x_relative - pivot[0] - pan[0]) * zoom / k) + pivot[0];
        let ndc_y = ((ndc_y_relative - pivot[1] - pan[1]) * zoom) + pivot[1];

        Some([ndc_x, ndc_y])
    } else {
        // --- 3D Viewport ---
        crate::util::orientation::volume_to_screen_3d(
            pos.into(),
            rotation,
            aspect_ratios,
            zoom,
            pan,
            screen_aspect,
        )
    }
}
