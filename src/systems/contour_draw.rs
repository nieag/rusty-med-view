//! Contour drawing system.
//!
//! This module provides freehand contour drawing in 2D views,
//! converting screen-space points to 3D world-space contours.

use crate::app::segment::{Plane3D, PlaneContour};
use crate::util::orientation::SlicePlane;

// ============================================================================
// Contour Drawing State
// ============================================================================

/// State machine for contour drawing interaction.
#[derive(Debug, Clone, Default)]
pub enum ContourDrawState {
    /// Not currently drawing
    #[default]
    Idle,
    /// Actively drawing a contour
    Drawing {
        /// Screen UV points being collected
        points: Vec<[f32; 2]>,
        /// Which slice plane we're drawing on
        slice_plane: SlicePlane,
        /// The slice index (voxel coordinate)
        slice_index: i32,
    },
}

impl ContourDrawState {
    /// Check if currently drawing
    pub fn is_drawing(&self) -> bool {
        matches!(self, ContourDrawState::Drawing { .. })
    }

    /// Get the current points if drawing
    pub fn points(&self) -> Option<&Vec<[f32; 2]>> {
        match self {
            ContourDrawState::Drawing { points, .. } => Some(points),
            ContourDrawState::Idle => None,
        }
    }
}

// ============================================================================
// Screen to World Conversion
// ============================================================================

/// Convert screen UV to world-space position on a slice plane.
///
/// Uses radiological convention (X flipped for axial/coronal).
pub fn screen_uv_to_world(
    uv: [f32; 2],
    slice_plane: SlicePlane,
    slice_index: i32,
    volume_dims: [u32; 3],
    volume_spacing: [f32; 3],
) -> [f32; 3] {
    // Get volume UV using radiological convention (from orientation.rs)
    let volume_uv = slice_plane.screen_uv_to_volume(
        uv,
        (slice_index as f32 + 0.5) / volume_dims[slice_plane.depth_axis()] as f32,
    );

    // Convert volume UV [0,1] to world coordinates
    [
        volume_uv[0] * (volume_dims[0] as f32 - 1.0) * volume_spacing[0],
        volume_uv[1] * (volume_dims[1] as f32 - 1.0) * volume_spacing[1],
        volume_uv[2] * (volume_dims[2] as f32 - 1.0) * volume_spacing[2],
    ]
}

/// Convert a sequence of screen UV points to a 3D PlaneContour.
pub fn screen_points_to_plane_contour(
    screen_points: &[[f32; 2]],
    slice_plane: SlicePlane,
    slice_index: i32,
    volume_dims: [u32; 3],
    volume_spacing: [f32; 3],
) -> PlaneContour {
    // Compute the plane position in world space
    let depth_axis = slice_plane.depth_axis();
    let plane_position =
        (slice_index as f32 + 0.5) * volume_spacing[depth_axis];

    let plane = Plane3D::from_slice_plane(slice_plane, plane_position);

    // Convert all screen points to 3D
    let points_3d: Vec<[f32; 3]> = screen_points
        .iter()
        .map(|&uv| screen_uv_to_world(uv, slice_plane, slice_index, volume_dims, volume_spacing))
        .collect();

    PlaneContour::with_points(plane, points_3d, true) // Default to closed
}

// ============================================================================
// Contour Processing
// ============================================================================

/// Smooth a contour using Catmull-Rom spline interpolation.
///
/// # Arguments
/// * `points` - Input 3D points (forms a closed loop)
/// * `segments_per_edge` - Number of interpolated points per original edge
///
/// # Returns
/// Smoothed contour with more points
pub fn smooth_contour(points: &[[f32; 3]], segments_per_edge: u32) -> Vec<[f32; 3]> {
    if points.len() < 3 {
        return points.to_vec();
    }

    let mut result = Vec::with_capacity(points.len() * segments_per_edge as usize);
    let n = points.len();

    for i in 0..n {
        let p0 = points[(i + n - 1) % n];
        let p1 = points[i];
        let p2 = points[(i + 1) % n];
        let p3 = points[(i + 2) % n];

        for j in 0..segments_per_edge {
            let t = j as f32 / segments_per_edge as f32;
            result.push(catmull_rom(p0, p1, p2, p3, t));
        }
    }

    result
}

/// Catmull-Rom spline interpolation between p1 and p2.
fn catmull_rom(p0: [f32; 3], p1: [f32; 3], p2: [f32; 3], p3: [f32; 3], t: f32) -> [f32; 3] {
    let t2 = t * t;
    let t3 = t2 * t;

    let mut result = [0.0; 3];
    for i in 0..3 {
        result[i] = 0.5
            * (2.0 * p1[i]
                + (-p0[i] + p2[i]) * t
                + (2.0 * p0[i] - 5.0 * p1[i] + 4.0 * p2[i] - p3[i]) * t2
                + (-p0[i] + 3.0 * p1[i] - 3.0 * p2[i] + p3[i]) * t3);
    }
    result
}

/// Close a contour if endpoints are within threshold distance.
///
/// Returns true if the contour was closed (last point removed).
pub fn maybe_close_contour(points: &mut Vec<[f32; 3]>, threshold: f32) -> bool {
    if points.len() < 3 {
        return false;
    }

    let first = points[0];
    let last = points[points.len() - 1];
    let dist = ((first[0] - last[0]).powi(2)
        + (first[1] - last[1]).powi(2)
        + (first[2] - last[2]).powi(2))
    .sqrt();

    if dist < threshold {
        points.pop(); // Remove last point (will connect to first implicitly)
        true
    } else {
        false
    }
}

/// Simplify contour by removing points that are too close together.
///
/// Returns a new contour with fewer points.
pub fn simplify_contour(points: &[[f32; 3]], min_distance: f32) -> Vec<[f32; 3]> {
    if points.is_empty() {
        return Vec::new();
    }

    let mut result = vec![points[0]];
    let min_dist_sq = min_distance * min_distance;

    for &point in &points[1..] {
        let last = result.last().unwrap();
        let dist_sq = (point[0] - last[0]).powi(2)
            + (point[1] - last[1]).powi(2)
            + (point[2] - last[2]).powi(2);

        if dist_sq >= min_dist_sq {
            result.push(point);
        }
    }

    result
}

/// Check if a 2D screen point should be added to the current contour.
///
/// Returns true if the point is far enough from the last point.
pub fn should_add_point(last_point: [f32; 2], new_point: [f32; 2], min_distance: f32) -> bool {
    let dx = new_point[0] - last_point[0];
    let dy = new_point[1] - last_point[1];
    (dx * dx + dy * dy).sqrt() >= min_distance
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contour_draw_state_default() {
        let state = ContourDrawState::default();
        assert!(!state.is_drawing());
        assert!(state.points().is_none());
    }

    #[test]
    fn test_contour_draw_state_drawing() {
        let state = ContourDrawState::Drawing {
            points: vec![[0.5, 0.5]],
            slice_plane: SlicePlane::Axial,
            slice_index: 25,
        };
        assert!(state.is_drawing());
        assert_eq!(state.points().unwrap().len(), 1);
    }

    #[test]
    fn test_screen_to_world_axial_center() {
        // Center of screen should map to center of volume
        let uv = [0.5, 0.5];
        let dims = [100, 100, 50];
        let spacing = [1.0, 1.0, 2.0];

        let world = screen_uv_to_world(uv, SlicePlane::Axial, 25, dims, spacing);

        // With radiological convention, center maps to center
        assert!((world[0] - 49.5).abs() < 1.0, "X: {}", world[0]);
        assert!((world[1] - 49.5).abs() < 1.0, "Y: {}", world[1]);
        // Z should be at slice 25 * spacing 2.0 = ~51
        assert!((world[2] - 51.0).abs() < 2.0, "Z: {}", world[2]);
    }

    #[test]
    fn test_screen_points_to_plane_contour() {
        let points = vec![[0.3, 0.3], [0.7, 0.3], [0.7, 0.7], [0.3, 0.7]];
        let dims = [100, 100, 50];
        let spacing = [1.0, 1.0, 1.0];

        let contour =
            screen_points_to_plane_contour(&points, SlicePlane::Axial, 25, dims, spacing);

        assert_eq!(contour.points.len(), 4);
        assert!(contour.is_closed);
        assert!(contour.is_valid());
    }

    #[test]
    fn test_smooth_contour_square() {
        let square = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        let smoothed = smooth_contour(&square, 4);

        // 4 edges * 4 segments = 16 points
        assert_eq!(smoothed.len(), 16);
    }

    #[test]
    fn test_smooth_contour_too_few_points() {
        let points = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let smoothed = smooth_contour(&points, 4);

        // Should return original points
        assert_eq!(smoothed.len(), 2);
    }

    #[test]
    fn test_catmull_rom_at_t0() {
        // At t=0, should return p1
        let result = catmull_rom(
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            0.0,
        );
        assert!((result[0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_catmull_rom_at_t1() {
        // At t=1, should return p2
        let result = catmull_rom(
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            1.0,
        );
        assert!((result[0] - 2.0).abs() < 1e-6);
    }

    #[test]
    fn test_maybe_close_contour_near() {
        let mut points = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.01, 0.01, 0.0], // Very close to first point
        ];
        let closed = maybe_close_contour(&mut points, 0.05);
        assert!(closed);
        assert_eq!(points.len(), 3);
    }

    #[test]
    fn test_maybe_close_contour_far() {
        let mut points = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.5, 0.5, 0.0], // Not close to first point
        ];
        let closed = maybe_close_contour(&mut points, 0.05);
        assert!(!closed);
        assert_eq!(points.len(), 4);
    }

    #[test]
    fn test_simplify_contour() {
        let points = vec![
            [0.0, 0.0, 0.0],
            [0.001, 0.0, 0.0], // Too close, should be removed
            [1.0, 0.0, 0.0],
            [1.001, 0.0, 0.0], // Too close, should be removed
            [2.0, 0.0, 0.0],
        ];
        let simplified = simplify_contour(&points, 0.1);

        assert_eq!(simplified.len(), 3);
        assert_eq!(simplified[0], [0.0, 0.0, 0.0]);
        assert_eq!(simplified[1], [1.0, 0.0, 0.0]);
        assert_eq!(simplified[2], [2.0, 0.0, 0.0]);
    }

    #[test]
    fn test_should_add_point() {
        assert!(should_add_point([0.0, 0.0], [0.1, 0.0], 0.05));
        assert!(!should_add_point([0.0, 0.0], [0.01, 0.0], 0.05));
    }
}
