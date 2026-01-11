// src/overlay/primitives.rs
use glam::Vec3;

/// Primitive types for GPU overlay rendering
#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum OverlayPrimitiveKind {
    Circle = 0, // Filled circle - annotations, measurement endpoints
    Ring = 1,   // Hollow circle - brush preview, selection highlights
}

/// A single GPU-rendered overlay primitive.
/// Layout is carefully aligned for GPU storage buffer.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct OverlayPrimitive {
    /// World position [x, y, z, w] - w unused, for 16-byte alignment
    pub world_pos: [f32; 4],
    /// RGBA color [r, g, b, a]
    pub color: [f32; 4],
    /// Parameters: [radius, thickness, viewport_mask, kind]
    /// - radius: size in world units (0.0-1.0 range)
    /// - thickness: for rings/lines (in world units)
    /// - viewport_mask: bitfield (1=3D, 2=Axial, 4=Coronal, 8=Sagittal, 15=all)
    /// - kind: OverlayPrimitiveKind as f32
    pub params: [f32; 4],
    /// Secondary position for lines: [x, y, z, flags]
    /// - For Line: end point
    /// - flags: reserved
    pub secondary_pos: [f32; 4],
}

impl OverlayPrimitive {
    /// Create a circle primitive at the given world position
    pub fn circle(pos: Vec3, radius: f32, color: [f32; 4], viewport_mask: u32) -> Self {
        Self {
            world_pos: [pos.x, pos.y, pos.z, 0.0],
            color,
            params: [
                radius,
                0.0,
                viewport_mask as f32,
                OverlayPrimitiveKind::Circle as u32 as f32,
            ],
            secondary_pos: [0.0; 4],
        }
    }

    /// Create a ring (hollow circle) primitive
    pub fn ring(
        pos: Vec3,
        radius: f32,
        thickness: f32,
        color: [f32; 4],
        viewport_mask: u32,
    ) -> Self {
        Self {
            world_pos: [pos.x, pos.y, pos.z, 0.0],
            color,
            params: [
                radius,
                thickness,
                viewport_mask as f32,
                OverlayPrimitiveKind::Ring as u32 as f32,
            ],
            secondary_pos: [0.0; 4],
        }
    }
}
