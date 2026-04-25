// src/geometry.rs
// use crate::components::ViewState; // Removed
use crate::components::VoxelGeometry;
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

/// View projection parameters for 2D/3D viewport coordinate mapping.
pub struct ViewProjection {
    pub zoom: f32,
    pub pan: [f32; 2],
    pub pivot: [f32; 2],
    pub rotation: [f32; 4],
    pub aspect_ratios: [f32; 3],
    pub geometry: VoxelGeometry,
    pub cursor_pos: [f32; 3],
}

/// Project a world position (0..1) to Normalized Device Coordinates (0..1 relative to viewport)
pub fn world_to_ndc(
    pos: Vec3,
    view_mode: crate::components::ViewMode,
    proj: &ViewProjection,
    screen_aspect: f32,
) -> Option<[f32; 2]> {
    if view_mode != crate::components::ViewMode::ThreeD {
        // --- 2D Viewports ---
        if view_mode == crate::components::ViewMode::Oblique {
            let plane = crate::convert::oblique_plane_from_view_rotation(
                proj.cursor_pos,
                proj.rotation,
                proj.geometry,
            )?;
            let mapping = crate::convert::ViewportMapping {
                zoom: proj.zoom,
                pan: proj.pan,
                pivot: proj.pivot,
                screen_aspect,
            };
            return crate::convert::volume_uv_to_viewport_uv(
                pos.into(),
                plane,
                proj.geometry,
                mapping,
            );
        }

        let plane = crate::util::orientation::SlicePlane::from_mode(view_mode)?;
        if let Some(plane_definition) = crate::convert::orthogonal_plane_from_volume_uv(
            plane.to_plane_family(),
            pos.into(),
            proj.geometry,
        ) {
            let mapping = crate::convert::ViewportMapping {
                zoom: proj.zoom,
                pan: proj.pan,
                pivot: proj.pivot,
                screen_aspect,
            };
            if let Some(viewport_uv) = crate::convert::volume_uv_to_viewport_uv(
                pos.into(),
                plane_definition,
                proj.geometry,
                mapping,
            ) {
                return Some(viewport_uv);
            }
        }

        let [ndc_x_relative, ndc_y_relative] = plane.volume_to_screen_uv(pos.into());
        let k = screen_aspect / plane.slice_aspect(proj.aspect_ratios);

        let ndc_x =
            ((ndc_x_relative - proj.pivot[0] - proj.pan[0]) * proj.zoom / k) + proj.pivot[0];
        let ndc_y = ((ndc_y_relative - proj.pivot[1] - proj.pan[1]) * proj.zoom) + proj.pivot[1];

        Some([ndc_x, ndc_y])
    } else {
        // --- 3D Viewport ---
        crate::util::orientation::volume_to_screen_3d(
            pos.into(),
            proj.rotation,
            proj.aspect_ratios,
            proj.zoom,
            proj.pan,
            screen_aspect,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{ViewMode, VoxelGeometry};

    fn test_projection() -> ViewProjection {
        ViewProjection {
            zoom: 1.0,
            pan: [0.0, 0.0],
            pivot: [0.5, 0.5],
            rotation: [0.0, 0.0, 0.0, 1.0],
            aspect_ratios: [1.0, 1.0, 1.0],
            geometry: VoxelGeometry {
                dimensions: [64, 64, 64],
                spacing: [1.0, 1.0, 1.0],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            cursor_pos: [0.5, 0.5, 0.5],
        }
    }

    #[test]
    fn test_oblique_world_to_ndc_projects_cursor_center() {
        let proj = test_projection();
        let ndc = world_to_ndc(Vec3::new(0.5, 0.5, 0.5), ViewMode::Oblique, &proj, 1.0)
            .expect("expected oblique projection");

        assert!((ndc[0] - 0.5).abs() < 1e-6);
        assert!((ndc[1] - 0.5).abs() < 1e-6);
    }
}
