// src/segmentation/algorithms/snapping.rs
//
// Vertex snapping for Hybrid Reconstruction (Milestone 8).
// Snaps mesh vertices to authoritative vector contour boundaries.

use crate::segmentation::contour::SpatialContour;
use glam::{Vec2, Vec3};

/// Result of a vertex snap operation
#[derive(Debug, Clone)]
pub struct SnapResult {
    pub position: Vec3,
    pub normal: Vec3,
    pub was_snapped: bool,
}

/// Configuration for the snapping pass
#[derive(Debug, Clone)]
pub struct SnapConfig {
    /// Maximum distance (in voxel units) at which a vertex will snap to contour
    pub snap_threshold: f32,
}

impl Default for SnapConfig {
    fn default() -> Self {
        Self {
            snap_threshold: 0.5, // Half a voxel
        }
    }
}

/// Finds the nearest contour constraint to a world-space position.
/// Returns the contour index and signed distance if within max_radius.
pub fn find_nearest_constraint(
    pos: Vec3,
    contours: &[SpatialContour],
    max_radius: f32,
) -> Option<(usize, f32)> {
    let mut best: Option<(usize, f32)> = None;

    for (i, contour) in contours.iter().enumerate() {
        if contour.points.is_empty() {
            continue;
        }

        let dist = compute_contour_distance(contour, pos);
        let abs_dist = dist.abs();

        if abs_dist <= max_radius {
            match best {
                None => best = Some((i, dist)),
                Some((_, best_dist)) if abs_dist < best_dist.abs() => {
                    best = Some((i, dist));
                }
                _ => {}
            }
        }
    }

    best
}

/// Snaps a mesh vertex to the nearest contour boundary if within threshold.
///
/// The snapping projects the vertex onto the contour's plane, then onto the
/// nearest point on the polyline boundary.
pub fn snap_vertex(pos: Vec3, contours: &[SpatialContour], config: &SnapConfig) -> SnapResult {
    // Find nearest constraint
    let Some((contour_idx, dist)) = find_nearest_constraint(pos, contours, config.snap_threshold)
    else {
        return SnapResult {
            position: pos,
            normal: Vec3::ZERO,
            was_snapped: false,
        };
    };

    let contour = &contours[contour_idx];

    // Project vertex onto contour plane
    let v = pos - contour.origin;
    let dist_plane = v.dot(contour.normal);
    let p_on_plane = pos - contour.normal * dist_plane;

    // Get 2D coordinates on the plane
    let rel = p_on_plane - contour.origin;
    let p2d = Vec2::new(rel.dot(contour.right), rel.dot(contour.up));

    // Find nearest point on the polyline
    let (nearest_2d, _) = nearest_point_on_polyline(p2d, &contour.points, contour.is_closed);

    // Convert back to 3D (snapped to boundary on the contour's plane)
    let snapped_pos = contour.to_world(nearest_2d);

    // Use the contour's plane normal as the surface normal
    // (pointing outward from the contour center)
    let center_2d = compute_polygon_centroid(&contour.points);
    let to_vertex = nearest_2d - center_2d;
    let outward_dir_2d = to_vertex.normalize_or_zero();
    let outward_3d =
        contour.right * outward_dir_2d.x + contour.up * outward_dir_2d.y + contour.normal * 0.0;

    // Blend between contour normal and outward direction based on whether we're
    // on an edge or face. For now, use the plane normal as primary.
    let normal = if dist.abs() < 0.001 {
        // Very close to plane surface, use in-plane outward direction
        outward_3d.normalize_or_zero()
    } else {
        // Some distance from plane, blend with plane normal
        let blend = (dist.abs() / config.snap_threshold).clamp(0.0, 1.0);
        (contour.normal * blend + outward_3d * (1.0 - blend)).normalize_or_zero()
    };

    SnapResult {
        position: snapped_pos,
        normal,
        was_snapped: true,
    }
}

/// Finds the nearest point on a 2D polyline to point p.
/// Returns (nearest_point, squared_distance).
fn nearest_point_on_polyline(p: Vec2, points: &[Vec2], closed: bool) -> (Vec2, f32) {
    if points.is_empty() {
        return (p, 0.0);
    }
    if points.len() == 1 {
        return (points[0], (p - points[0]).length_squared());
    }

    let mut min_dist_sq = f32::MAX;
    let mut nearest = points[0];

    let num_segments = if closed {
        points.len()
    } else {
        points.len() - 1
    };

    for i in 0..num_segments {
        let v1 = points[i];
        let v2 = points[(i + 1) % points.len()];

        let (proj, dist_sq) = closest_point_on_segment(p, v1, v2);
        if dist_sq < min_dist_sq {
            min_dist_sq = dist_sq;
            nearest = proj;
        }
    }

    (nearest, min_dist_sq)
}

/// Returns the closest point on segment v1-v2 to point p, and its squared distance.
fn closest_point_on_segment(p: Vec2, v1: Vec2, v2: Vec2) -> (Vec2, f32) {
    let diff = v2 - v1;
    let len_sq = diff.length_squared();

    if len_sq < 1e-10 {
        return (v1, (p - v1).length_squared());
    }

    let t = ((p - v1).dot(diff) / len_sq).clamp(0.0, 1.0);
    let projection = v1 + diff * t;
    (projection, (p - projection).length_squared())
}

/// Computes the centroid of a polygon.
fn compute_polygon_centroid(points: &[Vec2]) -> Vec2 {
    if points.is_empty() {
        return Vec2::ZERO;
    }
    let sum: Vec2 = points.iter().copied().sum();
    sum / points.len() as f32
}

/// Computes the signed distance from a 3D point to a SpatialContour.
/// This is the same logic as in baking.rs but exposed for snapping.
fn compute_contour_distance(contour: &SpatialContour, p: Vec3) -> f32 {
    let v = p - contour.origin;
    let dist_plane = v.dot(contour.normal);

    let x2d = v.dot(contour.right);
    let y2d = v.dot(contour.up);
    let p2d = Vec2::new(x2d, y2d);

    let dist_poly = signed_distance_polygon(p2d, &contour.points, contour.is_closed);

    let d_2d = dist_poly;
    let d_z = dist_plane.abs();

    if d_2d > 0.0 {
        let len = (d_2d * d_2d + d_z * d_z).sqrt();
        len - contour.influence
    } else {
        let half_thickness = contour.influence;
        (d_z - half_thickness).max(d_2d)
    }
}

/// Signed distance from point to 2D polygon.
fn signed_distance_polygon(p: Vec2, points: &[Vec2], closed: bool) -> f32 {
    if points.len() < 2 {
        if let Some(pt) = points.first() {
            return (p - *pt).length();
        }
        return f32::MAX;
    }

    let mut min_dist_sq = f32::MAX;
    let mut winding_number = 0i32;

    let count = points.len();
    let num_segments = if closed { count } else { count - 1 };

    for i in 0..num_segments {
        let v1 = points[i];
        let v2 = points[(i + 1) % count];

        let (_, d_sq) = closest_point_on_segment(p, v1, v2);
        if d_sq < min_dist_sq {
            min_dist_sq = d_sq;
        }

        if closed {
            if v1.y <= p.y {
                if v2.y > p.y && is_left(v1, v2, p) > 0.0 {
                    winding_number += 1;
                }
            } else if v2.y <= p.y && is_left(v1, v2, p) < 0.0 {
                winding_number -= 1;
            }
        }
    }

    let dist = min_dist_sq.sqrt();

    if closed && winding_number != 0 {
        -dist
    } else {
        dist
    }
}

fn is_left(v1: Vec2, v2: Vec2, p: Vec2) -> f32 {
    (v2.x - v1.x) * (p.y - v1.y) - (p.x - v1.x) * (v2.y - v1.y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn make_square_contour(origin: Vec3, size: f32, influence: f32) -> SpatialContour {
        SpatialContour {
            id: Uuid::new_v4(),
            origin,
            normal: Vec3::Z,
            right: Vec3::X,
            up: Vec3::Y,
            points: vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(size, 0.0),
                Vec2::new(size, size),
                Vec2::new(0.0, size),
            ],
            influence,
            is_closed: true,
            label_index: 1,
        }
    }

    #[test]
    fn test_snap_to_axial_plane() {
        let contour = make_square_contour(Vec3::ZERO, 10.0, 1.0);
        let contours = vec![contour];
        let config = SnapConfig {
            snap_threshold: 2.0,
        };

        // Point slightly outside edge (should snap to edge)
        let pos = Vec3::new(5.0, -0.3, 0.0);
        let result = snap_vertex(pos, &contours, &config);

        assert!(result.was_snapped, "Should snap when within threshold");
        assert!(
            (result.position.y - 0.0).abs() < 0.01,
            "Should snap to Y=0 edge, got Y={}",
            result.position.y
        );
    }

    #[test]
    fn test_no_snap_outside_threshold() {
        let contour = make_square_contour(Vec3::ZERO, 10.0, 1.0);
        let contours = vec![contour];
        let config = SnapConfig {
            snap_threshold: 0.5,
        };

        // Point far from contour (outside threshold)
        let pos = Vec3::new(5.0, -5.0, 0.0);
        let result = snap_vertex(pos, &contours, &config);

        assert!(
            !result.was_snapped,
            "Should NOT snap when outside threshold"
        );
        assert_eq!(result.position, pos, "Position should be unchanged");
    }

    #[test]
    fn test_find_nearest_constraint() {
        let c1 = make_square_contour(Vec3::new(0.0, 0.0, 0.0), 10.0, 1.0);
        let c2 = make_square_contour(Vec3::new(50.0, 0.0, 0.0), 10.0, 1.0);
        let contours = vec![c1, c2];

        // Point near first contour
        let result = find_nearest_constraint(Vec3::new(5.0, 5.0, 0.0), &contours, 10.0);
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, 0, "Should find first contour");

        // Point near second contour
        let result2 = find_nearest_constraint(Vec3::new(55.0, 5.0, 0.0), &contours, 10.0);
        assert!(result2.is_some());
        assert_eq!(result2.unwrap().0, 1, "Should find second contour");
    }

    #[test]
    fn test_nearest_point_on_polyline() {
        let points = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 10.0),
            Vec2::new(0.0, 10.0),
        ];

        // Point projects onto bottom edge
        let (nearest, _) = nearest_point_on_polyline(Vec2::new(5.0, -2.0), &points, true);
        assert!((nearest.y - 0.0).abs() < 0.01, "Should project to Y=0");
        assert!((nearest.x - 5.0).abs() < 0.01, "Should project to X=5");

        // Point projects onto corner
        let (nearest_corner, _) = nearest_point_on_polyline(Vec2::new(-1.0, -1.0), &points, true);
        assert!(
            (nearest_corner - Vec2::new(0.0, 0.0)).length() < 0.01,
            "Should project to corner (0,0)"
        );
    }
}
