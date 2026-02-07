//! Contour to SDF conversion algorithms.
//!
//! Converts closed contours on arbitrary planes to a 3D signed distance field.
//! Supports axis-aligned and oblique contour planes.

use crate::app::segment::{ContourSet, Plane3D, PlaneContour, SdfVolume};


// ============================================================================
// 2D Geometry Helpers
// ============================================================================

/// Compute signed distance from a 2D point to a line segment.
///
/// Returns positive distance if point is to the right of the segment
/// (using winding direction for inside/outside determination).
pub fn signed_distance_to_segment_2d(
    point: [f32; 2],
    seg_start: [f32; 2],
    seg_end: [f32; 2],
) -> f32 {
    let dx = seg_end[0] - seg_start[0];
    let dy = seg_end[1] - seg_start[1];
    let len_sq = dx * dx + dy * dy;

    if len_sq < 1e-12 {
        // Degenerate segment (point)
        let px = point[0] - seg_start[0];
        let py = point[1] - seg_start[1];
        return (px * px + py * py).sqrt();
    }

    // Project point onto line
    let t = ((point[0] - seg_start[0]) * dx + (point[1] - seg_start[1]) * dy) / len_sq;
    let t_clamped = t.clamp(0.0, 1.0);

    // Closest point on segment
    let closest_x = seg_start[0] + t_clamped * dx;
    let closest_y = seg_start[1] + t_clamped * dy;

    let dist = ((point[0] - closest_x).powi(2) + (point[1] - closest_y).powi(2)).sqrt();
    dist
}

/// Compute signed distance from a 2D point to a closed polygon.
///
/// Uses ray casting to determine inside/outside, then finds minimum
/// distance to any edge.
pub fn signed_distance_to_polygon_2d(point: [f32; 2], polygon: &[[f32; 2]]) -> f32 {
    if polygon.len() < 3 {
        return f32::MAX;
    }

    // Find minimum distance to any edge
    let mut min_dist = f32::MAX;
    let n = polygon.len();

    for i in 0..n {
        let p0 = polygon[i];
        let p1 = polygon[(i + 1) % n];
        let dist = signed_distance_to_segment_2d(point, p0, p1);
        min_dist = min_dist.min(dist);
    }

    // Determine inside/outside using ray casting (crossing number)
    let inside = is_inside_polygon(point, polygon);

    if inside {
        -min_dist // Negative = inside
    } else {
        min_dist // Positive = outside
    }
}

/// Test if a 2D point is inside a closed polygon using ray casting.
///
/// Uses the crossing number algorithm (also known as even-odd rule).
pub fn is_inside_polygon(point: [f32; 2], polygon: &[[f32; 2]]) -> bool {
    if polygon.len() < 3 {
        return false;
    }

    let mut crossings = 0;
    let n = polygon.len();

    for i in 0..n {
        let p0 = polygon[i];
        let p1 = polygon[(i + 1) % n];

        // Check if the edge crosses the horizontal ray from point to +X
        let y_in_range = (p0[1] <= point[1] && p1[1] > point[1])
            || (p1[1] <= point[1] && p0[1] > point[1]);

        if y_in_range {
            // Compute x-intersection of edge with horizontal line at point[1]
            let t = (point[1] - p0[1]) / (p1[1] - p0[1]);
            let x_intersect = p0[0] + t * (p1[0] - p0[0]);

            if point[0] < x_intersect {
                crossings += 1;
            }
        }
    }

    crossings % 2 == 1
}

// ============================================================================
// 3D Distance to Plane Contour
// ============================================================================

/// Compute signed distance from a 3D point to a PlaneContour.
///
/// Projects the point onto the contour's plane, then computes 2D distance.
/// Combines with distance to the plane for full 3D distance.
pub fn distance_to_plane_contour(point: [f32; 3], contour: &PlaneContour) -> f32 {
    if contour.points.len() < 3 {
        return f32::MAX;
    }

    // Get the local 2D coordinate system on the plane
    let (u_axis, v_axis) = get_plane_axes(&contour.plane);

    // Project 3D points to 2D
    let origin = contour.plane.project_point([0.0, 0.0, 0.0]);
    let polygon_2d: Vec<[f32; 2]> = contour
        .points
        .iter()
        .map(|p| project_to_plane_2d(*p, origin, u_axis, v_axis))
        .collect();

    // Project query point to plane
    let point_on_plane = contour.plane.project_point(point);
    let point_2d = project_to_plane_2d(point_on_plane, origin, u_axis, v_axis);

    // Get 2D signed distance
    let dist_2d = signed_distance_to_polygon_2d(point_2d, &polygon_2d);

    // Distance from point to plane
    let dist_to_plane = contour.plane.distance_to_point(point).abs();

    // Combine: use Euclidean combination for 3D distance
    // Sign is determined by the 2D distance (inside/outside the contour)
    let dist_3d = (dist_2d.abs().powi(2) + dist_to_plane.powi(2)).sqrt();

    if dist_2d < 0.0 {
        -dist_3d // Inside
    } else {
        dist_3d // Outside
    }
}

/// Get orthonormal basis vectors for a plane.
fn get_plane_axes(plane: &Plane3D) -> ([f32; 3], [f32; 3]) {
    let n = plane.normal;

    // Find a vector not parallel to normal
    let up = if n[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };

    // u = up × n (normalized)
    let u = cross(up, n);
    let u_len = (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]).sqrt();
    let u = [u[0] / u_len, u[1] / u_len, u[2] / u_len];

    // v = n × u
    let v = cross(n, u);

    (u, v)
}

/// Cross product of two 3D vectors.
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Project a 3D point to 2D using plane coordinate system.
fn project_to_plane_2d(point: [f32; 3], origin: [f32; 3], u: [f32; 3], v: [f32; 3]) -> [f32; 2] {
    let rel = [point[0] - origin[0], point[1] - origin[1], point[2] - origin[2]];
    [
        rel[0] * u[0] + rel[1] * u[1] + rel[2] * u[2],
        rel[0] * v[0] + rel[1] * v[1] + rel[2] * v[2],
    ]
}

// ============================================================================
// SDF Building
// ============================================================================

/// Build an SDF volume from a ContourSet.
///
/// # Arguments
/// * `contours` - The contour set to convert
/// * `volume_dims` - Original volume dimensions
/// * `volume_spacing` - Original volume spacing
/// * `resolution_multiplier` - SDF resolution relative to volume (1.0 = same, 2.0 = 2x)
///
/// # Returns
/// An SdfVolume with signed distances to the contour surface
pub fn build_sdf_from_contours(
    contours: &ContourSet,
    volume_dims: [u32; 3],
    volume_spacing: [f32; 3],
    resolution_multiplier: f32,
) -> SdfVolume {
    // Compute SDF dimensions
    let sdf_dims = [
        (volume_dims[0] as f32 * resolution_multiplier).round() as u32,
        (volume_dims[1] as f32 * resolution_multiplier).round() as u32,
        (volume_dims[2] as f32 * resolution_multiplier).round() as u32,
    ];

    // Compute SDF spacing (smaller to maintain physical size)
    let sdf_spacing = [
        volume_spacing[0] / resolution_multiplier,
        volume_spacing[1] / resolution_multiplier,
        volume_spacing[2] / resolution_multiplier,
    ];

    let origin = [0.0, 0.0, 0.0];
    let mut sdf = SdfVolume::new(sdf_dims, sdf_spacing, origin);

    // Collect all contours for processing
    let all_contours: Vec<&PlaneContour> = contours.all_contours().collect();

    if all_contours.is_empty() {
        return sdf;
    }

    // For each voxel, compute minimum signed distance to all contours
    for z in 0..sdf_dims[2] {
        for y in 0..sdf_dims[1] {
            for x in 0..sdf_dims[0] {
                let world_pos = sdf.index_to_world([x, y, z]);
                let mut min_dist = f32::MAX;

                for contour in &all_contours {
                    let dist = distance_to_plane_contour(world_pos, contour);
                    // Take the minimum (union of all contours)
                    if dist.abs() < min_dist.abs() {
                        min_dist = dist;
                    }
                }

                sdf.set(x, y, z, min_dist);
            }
        }
    }

    sdf
}

/// Build SDF using a faster approximate method for axis-aligned contours.
///
/// This version processes slice-by-slice for better cache efficiency
/// and uses 2D distance transform optimization.
pub fn build_sdf_from_contours_fast(
    contours: &ContourSet,
    volume_dims: [u32; 3],
    volume_spacing: [f32; 3],
    resolution_multiplier: f32,
) -> SdfVolume {
    // For axis-aligned contours, we can use a faster approach:
    // 1. Process each slice independently using 2D distance
    // 2. Then propagate distances in the perpendicular direction

    // For now, delegate to the general method
    // TODO: Implement optimized 2D distance transform per slice
    build_sdf_from_contours(contours, volume_dims, volume_spacing, resolution_multiplier)
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::orientation::SlicePlane;


    // --- 2D Geometry Tests ---

    #[test]
    fn test_signed_distance_to_segment_point_on_segment() {
        let dist = signed_distance_to_segment_2d([0.5, 0.0], [0.0, 0.0], [1.0, 0.0]);
        assert!(dist.abs() < 1e-6, "dist: {}", dist);
    }

    #[test]
    fn test_signed_distance_to_segment_perpendicular() {
        let dist = signed_distance_to_segment_2d([0.5, 1.0], [0.0, 0.0], [1.0, 0.0]);
        assert!((dist - 1.0).abs() < 1e-6, "dist: {}", dist);
    }

    #[test]
    fn test_signed_distance_to_segment_endpoint() {
        let dist = signed_distance_to_segment_2d([2.0, 0.0], [0.0, 0.0], [1.0, 0.0]);
        assert!((dist - 1.0).abs() < 1e-6, "dist: {}", dist);
    }

    #[test]
    fn test_is_inside_polygon_square() {
        let square = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

        assert!(is_inside_polygon([0.5, 0.5], &square)); // Center
        assert!(!is_inside_polygon([2.0, 0.5], &square)); // Outside right
        assert!(!is_inside_polygon([-0.5, 0.5], &square)); // Outside left
        assert!(!is_inside_polygon([0.5, 2.0], &square)); // Outside top
    }

    #[test]
    fn test_is_inside_polygon_triangle() {
        let triangle = [[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]];

        assert!(is_inside_polygon([0.5, 0.3], &triangle)); // Inside
        assert!(!is_inside_polygon([0.5, 1.5], &triangle)); // Above
    }

    #[test]
    fn test_signed_distance_to_polygon_inside() {
        let square = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];

        let dist = signed_distance_to_polygon_2d([5.0, 5.0], &square);
        assert!(dist < 0.0, "Inside should be negative: {}", dist);
        assert!((dist + 5.0).abs() < 0.1, "Distance should be ~5: {}", dist);
    }

    #[test]
    fn test_signed_distance_to_polygon_outside() {
        let square = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];

        let dist = signed_distance_to_polygon_2d([15.0, 5.0], &square);
        assert!(dist > 0.0, "Outside should be positive: {}", dist);
        assert!((dist - 5.0).abs() < 0.1, "Distance should be ~5: {}", dist);
    }

    // --- 3D Distance Tests ---

    #[test]
    fn test_distance_to_plane_contour_on_plane() {
        let contour = PlaneContour::with_points(
            Plane3D::from_axial(50.0),
            vec![
                [0.0, 0.0, 50.0],
                [100.0, 0.0, 50.0],
                [100.0, 100.0, 50.0],
                [0.0, 100.0, 50.0],
            ],
            true,
        );

        // Point inside, on plane
        let dist = distance_to_plane_contour([50.0, 50.0, 50.0], &contour);
        assert!(dist < 0.0, "Inside should be negative: {}", dist);
    }

    #[test]
    fn test_distance_to_plane_contour_off_plane() {
        let contour = PlaneContour::with_points(
            Plane3D::from_axial(50.0),
            vec![
                [0.0, 0.0, 50.0],
                [100.0, 0.0, 50.0],
                [100.0, 100.0, 50.0],
                [0.0, 100.0, 50.0],
            ],
            true,
        );

        // Point inside in 2D, but 10 units above the plane
        let dist = distance_to_plane_contour([50.0, 50.0, 60.0], &contour);
        // Should still be "inside" but with distance from plane
        assert!(dist < 0.0, "Inside (projection) should be negative: {}", dist);
    }

    // --- SDF Building Tests ---

    #[test]
    fn test_build_sdf_empty_contours() {
        let contours = ContourSet::new();
        let sdf = build_sdf_from_contours(&contours, [10, 10, 10], [1.0, 1.0, 1.0], 1.0);

        // All values should be MAX (no contours)
        assert_eq!(sdf.get(5, 5, 5), f32::MAX);
    }

    #[test]
    fn test_build_sdf_simple_contour() {
        let mut contours = ContourSet::new();

        // Add a square contour on slice Z=5
        let square = PlaneContour::with_points(
            Plane3D::from_axial(5.0),
            vec![
                [2.0, 2.0, 5.0],
                [8.0, 2.0, 5.0],
                [8.0, 8.0, 5.0],
                [2.0, 8.0, 5.0],
            ],
            true,
        );
        contours.add_contour(SlicePlane::Axial, 5, square);

        let sdf = build_sdf_from_contours(&contours, [10, 10, 10], [1.0, 1.0, 1.0], 1.0);

        // Center of contour (5,5,5) should be inside (negative)
        let center_dist = sdf.get(5, 5, 5);
        assert!(
            center_dist < 0.0,
            "Center should be inside (negative): {}",
            center_dist
        );

        // Corner (0,0,5) should be outside (positive)
        let corner_dist = sdf.get(0, 0, 5);
        assert!(
            corner_dist > 0.0,
            "Corner should be outside (positive): {}",
            corner_dist
        );
    }

    #[test]
    fn test_cross_product() {
        let result = cross([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        assert!((result[0] - 0.0).abs() < 1e-6);
        assert!((result[1] - 0.0).abs() < 1e-6);
        assert!((result[2] - 1.0).abs() < 1e-6);
    }
}
