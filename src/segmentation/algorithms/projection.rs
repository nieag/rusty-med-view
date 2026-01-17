use crate::segmentation::contour::SpatialContour;
use glam::{Vec2, Vec3};

/// Result of a contour projection onto a plane
pub enum ProjectedResult {
    /// The contour is in (or parallel to) the plane, showing the full polyline
    Full(Vec<Vec2>),
    /// The plane intersects the contour's segments, showing discrete points
    Intersections(Vec<Vec2>),
}

/// Projects a 3D spatial contour onto a 2D viewing plane.
///
/// # Arguments
/// * `contour` - The 3D spatial contour to project
/// * `slice_origin` - The origin of the viewing plane (in world/voxel space)
/// * `slice_normal` - The normal of the viewing plane
/// * `slice_right` - The local right vector of the viewing plane
/// * `slice_up` - The local up vector of the viewing plane
pub fn project_to_plane(
    contour: &SpatialContour,
    slice_origin: Vec3,
    slice_normal: Vec3,
    slice_right: Vec3,
    slice_up: Vec3,
) -> Option<ProjectedResult> {
    let dist = (contour.origin - slice_origin).dot(slice_normal).abs();

    // 1. Check if planes are parallel
    let dot = contour.normal.dot(slice_normal).abs();
    if dot > 0.99 {
        // Parallel planes. Only show if within a small epsilon (or influence radius)
        if dist < 0.5 {
            // Roughly within 1 voxel
            // Transform 3D points to the slice's local 2D space
            let mut projected_points = Vec::with_capacity(contour.points.len());
            for &p in &contour.points {
                let world_p = contour.to_world(p);
                let local_v = world_p - slice_origin;
                let x2d = local_v.dot(slice_right);
                let y2d = local_v.dot(slice_up);
                projected_points.push(Vec2::new(x2d, y2d));
            }
            return Some(ProjectedResult::Full(projected_points));
        }
        return None;
    }

    // 2. Intersecting planes
    // We iterate through segments of the contour and find where they cross the plane
    let mut intersections = Vec::new();

    let segment_count = if contour.is_closed {
        contour.points.len()
    } else {
        contour.points.len().saturating_sub(1)
    };
    for i in 0..segment_count {
        let p_curr = contour.points[i];
        let p_next = if i + 1 < contour.points.len() {
            contour.points[i + 1]
        } else {
            contour.points[0]
        };

        let v0 = contour.to_world(p_curr);
        let v1 = contour.to_world(p_next);

        let d0 = (v0 - slice_origin).dot(slice_normal);
        let d1 = (v1 - slice_origin).dot(slice_normal);

        // Check if v0 is on the plane
        if d0.abs() < 1e-6 {
            // Project world intersection to slice local 2D space
            let local_v = v0 - slice_origin;
            let x2d = local_v.dot(slice_right);
            let y2d = local_v.dot(slice_up);
            intersections.push(Vec2::new(x2d, y2d));
        } else if d0.signum() != d1.signum() && d1.abs() > 1e-6 {
            // Crossing between v0 and v1 (v1 is not on the plane)
            let total_d = (d0 - d1).abs();
            let t = d0.abs() / total_d;
            let intersect_world = v0 + (v1 - v0) * t;

            // Project world intersection to slice local 2D space
            let local_v = intersect_world - slice_origin;
            let x2d = local_v.dot(slice_right);
            let y2d = local_v.dot(slice_up);
            intersections.push(Vec2::new(x2d, y2d));
        }
    }

    if !intersections.is_empty() {
        // Deduplicate close points
        let mut unique = Vec::new();
        for p in intersections {
            if !unique.iter().any(|u: &Vec2| (u - p).length() < 1e-4) {
                unique.push(p);
            }
        }
        Some(ProjectedResult::Intersections(unique))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec2, Vec3};
    use uuid::Uuid;

    #[test]
    fn test_parallel_axial_projection() {
        let contour = SpatialContour {
            id: Uuid::new_v4(),
            origin: Vec3::new(10.0, 10.0, 50.0),
            normal: Vec3::Z,
            right: Vec3::X,
            up: Vec3::Y,
            points: vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(5.0, 0.0),
                Vec2::new(5.0, 5.0),
            ],
            influence: 1.0,
            is_closed: true,
            label_index: 1,
        };

        let slice_origin = Vec3::new(0.0, 0.0, 50.0);
        let slice_normal = Vec3::Z;
        let slice_right = Vec3::X;
        let slice_up = Vec3::Y;

        let result = project_to_plane(&contour, slice_origin, slice_normal, slice_right, slice_up);
        assert!(matches!(result, Some(ProjectedResult::Full(_))));
        if let Some(ProjectedResult::Full(points)) = result {
            assert_eq!(points.len(), 3);
            assert!((points[0] - Vec2::new(10.0, 10.0)).length() < 1e-5);
            assert!((points[1] - Vec2::new(15.0, 10.0)).length() < 1e-5);
        }
    }

    #[test]
    fn test_perpendicular_axial_projection() {
        // A vertical contour in the Coronal plane (XZ)
        let contour = SpatialContour {
            id: Uuid::new_v4(),
            origin: Vec3::new(10.0, 20.0, 50.0), // Located at Y=20
            normal: Vec3::Y,
            right: Vec3::X,
            up: Vec3::Z,
            points: vec![Vec2::new(0.0, -10.0), Vec2::new(0.0, 10.0)], // Vertical line from Z=40 to Z=60
            influence: 1.0,
            is_closed: true,
            label_index: 1,
        };

        // Looking at an Axial slice at Z=50
        let slice_origin = Vec3::new(0.0, 0.0, 50.0);
        let slice_normal = Vec3::Z;
        let slice_right = Vec3::X;
        let slice_up = Vec3::Y;

        let result = project_to_plane(&contour, slice_origin, slice_normal, slice_right, slice_up);
        assert!(matches!(result, Some(ProjectedResult::Intersections(_))));
        if let Some(ProjectedResult::Intersections(points)) = result {
            assert_eq!(points.len(), 1);
            // It should intersect at X=10, Y=20 in world space
            // In Axial slice local (origin X=0, Y=0), that's X=10, Y=20
            assert!((points[0] - Vec2::new(10.0, 20.0)).length() < 1e-5);
        }
    }
}
