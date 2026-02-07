//! GPU resources for contour rendering.
//!
//! This module manages the GPU buffers and bind groups needed to render
//! contour outlines in 2D views using SDF-based line rendering.

use crate::app::segment::PlaneContour;
use crate::util::orientation::SlicePlane;
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

// ============================================================================
// Constants
// ============================================================================

/// Maximum number of contour line segments per frame
pub const MAX_CONTOUR_SEGMENTS: usize = 2048;

/// Line width in UV space (roughly 2 pixels at 1000px viewport)
pub const DEFAULT_LINE_WIDTH: f32 = 0.002;

// ============================================================================
// GPU Data Structures (must match shader)
// ============================================================================

/// A single line segment for GPU rendering.
/// Matches the WGSL struct layout.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct ContourSegmentGpu {
    /// Start point in screen UV [0,1]
    pub p0: [f32; 2],
    /// End point in screen UV [0,1]
    pub p1: [f32; 2],
}

/// Contour rendering parameters.
/// Matches the WGSL uniform layout.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct ContourParamsGpu {
    /// Number of active segments
    pub segment_count: u32,
    /// Line width in UV space
    pub line_width: f32,
    /// Padding to align to 16 bytes
    pub _padding: [f32; 2],
    /// RGBA color for rendering
    pub color: [f32; 4],
}

impl Default for ContourParamsGpu {
    fn default() -> Self {
        Self {
            segment_count: 0,
            line_width: DEFAULT_LINE_WIDTH,
            _padding: [0.0; 2],
            color: [1.0, 1.0, 0.0, 1.0], // Yellow
        }
    }
}

// ============================================================================
// ContourResources
// ============================================================================

/// GPU resources for rendering contour line segments.
pub struct ContourResources {
    /// Storage buffer for line segments
    pub segment_buffer: wgpu::Buffer,
    /// Uniform buffer for contour parameters
    pub params_buffer: wgpu::Buffer,
    /// Bind group layout
    pub bind_group_layout: wgpu::BindGroupLayout,
    /// Bind group
    pub bind_group: wgpu::BindGroup,
    /// Current segment count (for stats/debugging)
    pub segment_count: u32,
}

impl ContourResources {
    /// Create new contour resources.
    pub fn new(device: &wgpu::Device) -> Self {
        // Create segment storage buffer (empty but sized)
        let segment_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Contour Segment Buffer"),
            size: (MAX_CONTOUR_SEGMENTS * std::mem::size_of::<ContourSegmentGpu>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Create params uniform buffer
        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Contour Params Buffer"),
            contents: bytemuck::cast_slice(&[ContourParamsGpu::default()]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Create bind group layout
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Contour Bind Group Layout"),
            entries: &[
                // Segment storage buffer
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Params uniform buffer
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        // Create bind group
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Contour Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: segment_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params_buffer.as_entire_binding(),
                },
            ],
        });

        Self {
            segment_buffer,
            params_buffer,
            bind_group_layout,
            bind_group,
            segment_count: 0,
        }
    }

    /// Update GPU buffers with contour segments for the current view.
    ///
    /// # Arguments
    /// * `queue` - WGPU queue for buffer writes
    /// * `segments` - Screen-space line segments to render
    /// * `color` - RGBA color for the contour
    /// * `line_width` - Line width in UV space
    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        segments: &[ContourSegmentGpu],
        color: [f32; 4],
        line_width: f32,
    ) {
        let segment_count = segments.len().min(MAX_CONTOUR_SEGMENTS);
        self.segment_count = segment_count as u32;

        // Update segment buffer
        if segment_count > 0 {
            queue.write_buffer(
                &self.segment_buffer,
                0,
                bytemuck::cast_slice(&segments[..segment_count]),
            );
        }

        // Update params buffer
        let params = ContourParamsGpu {
            segment_count: segment_count as u32,
            line_width,
            _padding: [0.0; 2],
            color,
        };
        queue.write_buffer(&self.params_buffer, 0, bytemuck::cast_slice(&[params]));
    }

    /// Clear all contours (set count to 0).
    pub fn clear(&mut self, queue: &wgpu::Queue) {
        self.segment_count = 0;
        let params = ContourParamsGpu {
            segment_count: 0,
            ..Default::default()
        };
        queue.write_buffer(&self.params_buffer, 0, bytemuck::cast_slice(&[params]));
    }
}

// ============================================================================
// Coordinate Conversion Helpers
// ============================================================================

/// Convert a 3D PlaneContour to screen-space line segments.
///
/// # Arguments
/// * `contour` - The 3D contour to convert
/// * `slice_plane` - Current view's slice plane
/// * `volume_dims` - Volume dimensions
/// * `volume_spacing` - Volume spacing
/// * `zoom` - Current zoom level
/// * `pan` - Current pan offset
///
/// # Returns
/// Vector of screen-space line segments
pub fn contour_to_screen_segments(
    contour: &PlaneContour,
    slice_plane: SlicePlane,
    volume_dims: [u32; 3],
    volume_spacing: [f32; 3],
    zoom: f32,
    pan: [f32; 2],
) -> Vec<ContourSegmentGpu> {
    if contour.points.len() < 2 {
        return Vec::new();
    }

    let mut segments = Vec::with_capacity(contour.points.len());
    let n = contour.points.len();

    // Determine loop count based on closed/open
    let loop_count = if contour.is_closed { n } else { n - 1 };

    for i in 0..loop_count {
        let p0 = contour.points[i];
        let p1 = contour.points[(i + 1) % n];

        // Convert world to screen UV
        let uv0 = world_to_screen_uv(p0, slice_plane, volume_dims, volume_spacing, zoom, pan);
        let uv1 = world_to_screen_uv(p1, slice_plane, volume_dims, volume_spacing, zoom, pan);

        segments.push(ContourSegmentGpu { p0: uv0, p1: uv1 });
    }

    segments
}

/// Convert world-space position to screen UV.
fn world_to_screen_uv(
    world: [f32; 3],
    slice_plane: SlicePlane,
    volume_dims: [u32; 3],
    volume_spacing: [f32; 3],
    zoom: f32,
    pan: [f32; 2],
) -> [f32; 2] {
    // Convert world to volume UV [0,1]
    let volume_uv = [
        world[0] / ((volume_dims[0] as f32 - 1.0) * volume_spacing[0]),
        world[1] / ((volume_dims[1] as f32 - 1.0) * volume_spacing[1]),
        world[2] / ((volume_dims[2] as f32 - 1.0) * volume_spacing[2]),
    ];

    // Project to screen UV based on slice plane (with radiological convention)
    let screen_uv = match slice_plane {
        SlicePlane::Axial => {
            // RADIOLOGICAL: Patient Right (x=1) on Screen Left (u=0)
            [1.0 - volume_uv[0], 1.0 - volume_uv[1]]
        }
        SlicePlane::Coronal => {
            // RADIOLOGICAL: Patient Right (x=1) on Screen Left (u=0)
            [1.0 - volume_uv[0], 1.0 - volume_uv[2]]
        }
        SlicePlane::Sagittal => {
            // Anterior is LEFT (u=0), Superior is UP (v=0)
            [1.0 - volume_uv[1], 1.0 - volume_uv[2]]
        }
    };

    // Apply zoom and pan (inverse of shader transform)
    let pivot = [0.5, 0.5];
    [
        (screen_uv[0] - pivot[0] - pan[0]) * zoom + pivot[0],
        (screen_uv[1] - pivot[1] - pan[1]) * zoom + pivot[1],
    ]
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::segment::Plane3D;

    #[test]
    fn test_contour_segment_gpu_size() {
        // Verify struct is exactly 16 bytes (4 floats)
        assert_eq!(std::mem::size_of::<ContourSegmentGpu>(), 16);
    }

    #[test]
    fn test_contour_params_gpu_size() {
        // Verify struct is 32 bytes (2 vec4s aligned)
        assert_eq!(std::mem::size_of::<ContourParamsGpu>(), 32);
    }

    #[test]
    fn test_contour_params_default() {
        let params = ContourParamsGpu::default();
        assert_eq!(params.segment_count, 0);
        assert_eq!(params.line_width, DEFAULT_LINE_WIDTH);
        assert_eq!(params.color, [1.0, 1.0, 0.0, 1.0]); // Yellow
    }

    #[test]
    fn test_contour_to_screen_segments_triangle() {
        let contour = PlaneContour::with_points(
            Plane3D::from_axial(50.0),
            vec![
                [25.0, 25.0, 50.0],
                [75.0, 25.0, 50.0],
                [50.0, 75.0, 50.0],
            ],
            true,
        );

        let segments = contour_to_screen_segments(
            &contour,
            SlicePlane::Axial,
            [100, 100, 100],
            [1.0, 1.0, 1.0],
            1.0, // No zoom
            [0.0, 0.0], // No pan
        );

        // Closed triangle has 3 segments
        assert_eq!(segments.len(), 3);
    }

    #[test]
    fn test_contour_to_screen_segments_open() {
        let contour = PlaneContour {
            plane: Plane3D::from_axial(50.0),
            points: vec![
                [0.0, 0.0, 50.0],
                [50.0, 0.0, 50.0],
                [100.0, 0.0, 50.0],
            ],
            is_closed: false,
        };

        let segments = contour_to_screen_segments(
            &contour,
            SlicePlane::Axial,
            [100, 100, 100],
            [1.0, 1.0, 1.0],
            1.0,
            [0.0, 0.0],
        );

        // Open line with 3 points has 2 segments
        assert_eq!(segments.len(), 2);
    }

    #[test]
    fn test_world_to_screen_uv_center() {
        // Center of volume should map to center of screen (with radiological flip)
        let uv = world_to_screen_uv(
            [49.5, 49.5, 50.0],
            SlicePlane::Axial,
            [100, 100, 100],
            [1.0, 1.0, 1.0],
            1.0,
            [0.0, 0.0],
        );

        // Should be near center
        assert!((uv[0] - 0.5).abs() < 0.02, "UV[0]: {}", uv[0]);
        assert!((uv[1] - 0.5).abs() < 0.02, "UV[1]: {}", uv[1]);
    }
}
