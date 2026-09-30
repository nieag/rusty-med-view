// src/geometry.rs
// use crate::components::ViewState; // Removed
use crate::components::Session;
use crate::components::VoxelGeometry;
use crate::components::{MainVolumeTag, Viewport, ViewportState, VolumeData};
use glam::Vec3;
use hecs::World;

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

#[derive(Debug, Clone, Copy)]
pub struct DisplayProjectionContext {
    pub main_geometry: VoxelGeometry,
    pub viewport_rect: [f32; 4],
    pub window_size: [f32; 2],
    pub screen_aspect: f32,
    pub zoom: f32,
    pub pan: [f32; 2],
    pub pivot: [f32; 2],
    pub cursor_pos: [f32; 3],
    pub display_aspect_ratios: [f32; 3],
    pub composed_rotation_3d: [f32; 4],
}

impl DisplayProjectionContext {
    pub fn view_projection_3d(self) -> ViewProjection {
        ViewProjection {
            zoom: self.zoom,
            pan: self.pan,
            pivot: self.pivot,
            rotation: self.composed_rotation_3d,
            aspect_ratios: self.display_aspect_ratios,
            geometry: self.main_geometry,
            cursor_pos: self.cursor_pos,
        }
    }
}

pub fn build_display_projection_context(
    world: &World,
    session: &Session,
    viewport: &Viewport,
    viewport_state: &ViewportState,
) -> Option<DisplayProjectionContext> {
    let mut volume_query = world.query::<&VolumeData>().with::<&MainVolumeTag>();
    let (_, main_volume) = volume_query.iter().next()?;
    let main_geometry = crate::app::roi_runtime::main_volume_geometry(world)?;

    let window_size = [
        session.window_settings.width as f32,
        session.window_settings.height as f32,
    ];
    let cursor_pos = {
        let cursor = &session.cursor;
        cursor.position
    };
    let screen_aspect = if viewport.rect[3] > 0.0 {
        viewport.rect[2] / viewport.rect[3]
    } else {
        1.0
    };

    Some(DisplayProjectionContext {
        main_geometry,
        viewport_rect: viewport.rect,
        window_size,
        screen_aspect,
        zoom: viewport_state.zoom,
        pan: viewport_state.pan,
        pivot: viewport_state.pivot,
        cursor_pos,
        display_aspect_ratios: main_volume.aspect_ratios(),
        composed_rotation_3d: crate::util::orientation::compose_view_rotation(
            main_volume.orientation(),
            viewport_state.user_rotation,
        ),
    })
}

pub fn project_world_mm_to_viewport_uv_3d(
    world_mm: [f32; 3],
    context: DisplayProjectionContext,
) -> Option<[f32; 2]> {
    let uv = crate::convert::world_mm_to_volume_uv(world_mm, context.main_geometry);
    world_to_ndc(
        Vec3::from_array(uv),
        crate::components::ViewMode::ThreeD,
        &context.view_projection_3d(),
        context.screen_aspect,
    )
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

        None
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
            geometry: VoxelGeometry::new(
                [64, 64, 64],
                [1.0, 1.0, 1.0],
                [0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            )
            .unwrap(),
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
