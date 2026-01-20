use crate::app::components::ViewMode;
use crate::segmentation::algorithms::surface_nets::DistanceSampler;
use crate::segmentation::algorithms::MarchingSquares;
use crate::segmentation::contour::{SpatialContour, VectorContourSet};
use crate::segmentation::tsdf::ChunkedTSDF;
use glam::{Vec2, Vec3};
use uuid::Uuid;

/// Extracts spatial contours from a ChunkedTSDF.
/// This fulfills the "Authority Handoff" by generating an initial set of vector constraints.
pub fn tsdf_to_spatial_contours(tsdf: &ChunkedTSDF, spacing: [f32; 3]) -> VectorContourSet {
    let mut contour_set = VectorContourSet::new();
    let (min, max) = tsdf.bounds();

    // We'll slice at every index to ensure smooth scrolling in all views.
    let slice_step = 1;

    // Axial (Z)
    for z in (min[2]..max[2]).step_by(slice_step) {
        extract_and_add(tsdf, ViewMode::Axial, z, spacing, &mut contour_set);
    }

    // Coronal (Y)
    for y in (min[1]..max[1]).step_by(slice_step) {
        extract_and_add(tsdf, ViewMode::Coronal, y, spacing, &mut contour_set);
    }

    // Sagittal (X)
    for x in (min[0]..max[0]).step_by(slice_step) {
        extract_and_add(tsdf, ViewMode::Sagittal, x, spacing, &mut contour_set);
    }

    contour_set
}

fn extract_and_add(
    tsdf: &ChunkedTSDF,
    view_mode: ViewMode,
    slice_idx: i32,
    spacing: [f32; 3],
    set: &mut VectorContourSet,
) {
    let (min, max) = tsdf.bounds();

    let (width, height) = match view_mode {
        ViewMode::Axial => (max[0] - min[0], max[1] - min[1]),
        ViewMode::Coronal => (max[0] - min[0], max[2] - min[2]),
        ViewMode::Sagittal => (max[1] - min[1], max[2] - min[2]),
        _ => return,
    };

    if width <= 0 || height <= 0 {
        return;
    }

    // Manually slice the TSDF
    let mut slice_values = vec![0.0f32; (width * height) as usize];
    for y in 0..height {
        for x in 0..width {
            let (wx, wy, wz) = match view_mode {
                ViewMode::Axial => (min[0] + x, min[1] + y, slice_idx),
                ViewMode::Coronal => (min[0] + x, slice_idx, min[2] + y),
                ViewMode::Sagittal => (slice_idx, min[1] + x, min[2] + y),
                _ => continue,
            };
            slice_values[(y * width + x) as usize] = tsdf.get_distance(wx, wy, wz);
        }
    }

    let ms = MarchingSquares::new(0.0); // Surface is at distance 0.0
    let raw_contours = ms.extract(&slice_values, (width as u32, height as u32), 1);

    for contour in raw_contours {
        if contour.points.len() < 3 {
            continue;
        }

        // Simplify via RDP (very tight to represent the voxel grid accurately)
        let simplified = rdp_simplify(&contour.points, 0.0001);

        // Convert to SpatialContour (using absolute VOXEL SPACE units)
        let spacing_v = Vec3::from(spacing);
        let origin = match view_mode {
            ViewMode::Axial => {
                Vec3::new(min[0] as f32, min[1] as f32, slice_idx as f32 + 0.5) * spacing_v
            }
            ViewMode::Coronal => {
                Vec3::new(min[0] as f32, slice_idx as f32 + 0.5, min[2] as f32) * spacing_v
            }
            ViewMode::Sagittal => {
                Vec3::new(slice_idx as f32 + 0.5, min[1] as f32, min[2] as f32) * spacing_v
            }
            _ => continue,
        };

        let (right, up) = match view_mode {
            ViewMode::Axial => (Vec3::X, Vec3::Y),
            ViewMode::Coronal => (Vec3::X, Vec3::Z),
            ViewMode::Sagittal => (Vec3::Y, Vec3::Z),
            _ => continue,
        };

        // Scale simplified 0..1 coordinates to physical Millimeters
        let (sp_right, sp_up) = match view_mode {
            ViewMode::Axial => (spacing[0], spacing[1]),
            ViewMode::Coronal => (spacing[0], spacing[2]),
            ViewMode::Sagittal => (spacing[1], spacing[2]),
            _ => (1.0, 1.0),
        };

        let mut voxel_points = simplified;
        for p in &mut voxel_points {
            p.x *= width as f32 * sp_right;
            p.y *= height as f32 * sp_up;
        }

        let mut sc = SpatialContour {
            id: Uuid::new_v4(),
            origin,
            normal: match view_mode {
                ViewMode::Axial => Vec3::Z,
                ViewMode::Coronal => Vec3::Y,
                ViewMode::Sagittal => Vec3::X,
                _ => Vec3::ZERO,
            },
            right,
            up,
            points: voxel_points,
            influence: match view_mode {
                ViewMode::Axial => spacing[2] * 2.0,
                ViewMode::Coronal => spacing[1] * 2.0,
                ViewMode::Sagittal => spacing[0] * 2.0,
                _ => 1.0,
            },
            is_closed: contour.is_closed,
            label_index: contour.label_index,
        };

        // Safeguard: If this is a closed loop, ensure first and last points are reasonably close
        // (after RDP they won't be identical, but they should be adjacent)
        if sc.is_closed && sc.points.len() >= 2 {
            let first = sc.points[0];
            let last = *sc.points.last().unwrap();
            let dist = (first - last).length();
            if dist > 0.02 {
                sc.is_closed = false;
            }
        }

        set.contours.push(sc);
    }
}

/// Ramer-Douglas-Peucker simplification
fn rdp_simplify(points: &[Vec2], epsilon: f32) -> Vec<Vec2> {
    if points.len() < 3 {
        return points.to_vec();
    }

    let mut dmax = 0.0;
    let mut index = 0;
    let end = points.len() - 1;

    for i in 1..end {
        let d = perpendicular_distance(points[i], points[0], points[end]);
        if d > dmax {
            index = i;
            dmax = d;
        }
    }

    if dmax > epsilon {
        let mut rec1 = rdp_simplify(&points[0..=index], epsilon);
        let rec2 = rdp_simplify(&points[index..], epsilon);
        rec1.pop();
        rec1.extend(rec2);
        rec1
    } else {
        vec![points[0], points[end]]
    }
}
fn perpendicular_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let ap = p - a;
    let ab_len_sq = ab.length_squared();
    if ab_len_sq == 0.0 {
        return ap.length();
    }
    let t = (ap.dot(ab) / ab_len_sq).clamp(0.0, 1.0);
    let projection = a + t * ab;
    (p - projection).length()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rdp_simplification() {
        let points = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(0.5, 0.01),
            Vec2::new(1.0, 0.0),
        ];
        let simplified = rdp_simplify(&points, 0.05);
        assert_eq!(simplified.len(), 2);
        assert_eq!(simplified[0], Vec2::new(0.0, 0.0));
        assert_eq!(simplified[1], Vec2::new(1.0, 0.0));
    }

    #[test]
    fn test_handoff_basic() {
        let mut tsdf = ChunkedTSDF::new(4.0);
        // Create a small sphere in TSDF
        for z in 10..20 {
            for y in 10..20 {
                for x in 10..20 {
                    let d = ((x as i32 - 15).pow(2)
                        + (y as i32 - 15).pow(2)
                        + (z as i32 - 15).pow(2)) as f32;
                    let dist = d.sqrt() - 4.0;
                    if dist < 4.0 {
                        tsdf.set_distance(x, y, z, dist);
                    }
                }
            }
        }

        let spacing = [1.0, 1.0, 1.0];
        let contours = tsdf_to_spatial_contours(&tsdf, spacing);

        // Should have extracted some contours
        assert!(!contours.contours.is_empty());

        // Check if origins are roughly correct (at least one coord should match slice_idx)
        for c in &contours.contours {
            let in_sphere = c.origin.x >= 0.0
                && c.origin.x <= 32.0
                && c.origin.y >= 0.0
                && c.origin.y <= 32.0
                && c.origin.z >= 0.0
                && c.origin.z <= 32.0;
            assert!(
                in_sphere,
                "Origin {:?} should be within volume bounds",
                c.origin
            );
        }
    }

    #[test]
    fn test_handoff_boundary_touching() {
        let mut tsdf = ChunkedTSDF::new(4.0);
        // Create a half-sphere that hits the Z=10 boundary (simulated start of volume)
        for z in 10..15 {
            for y in 10..20 {
                for x in 10..20 {
                    let d = ((x as i32 - 15).pow(2)
                        + (y as i32 - 15).pow(2)
                        + (z as i32 - 15).pow(2)) as f32;
                    let dist = d.sqrt() - 4.0;
                    if dist < 4.0 {
                        tsdf.set_distance(x, y, z, dist);
                    }
                }
            }
        }

        let spacing = [1.0, 1.0, 1.0];
        let contours = tsdf_to_spatial_contours(&tsdf, spacing);

        // Axial slice at z=10 should hit the boundary and be OPEN
        let axial_10 = contours
            .contours
            .iter()
            .find(|c| c.normal == glam::Vec3::Z && (c.origin.z - 10.0).abs() < 1e-5);

        if let Some(c) = axial_10 {
            assert!(
                !c.is_closed,
                "Contour at volume boundary should be open to avoid diagonal chords"
            );
        }
    }

    #[test]
    fn test_handoff_with_anisotropic_spacing() {
        let mut tsdf = ChunkedTSDF::new(4.0);
        // Single voxel at (15, 15, 15)
        tsdf.set_distance(15, 15, 15, -1.0);

        let spacing = [1.0, 2.0, 4.0];
        let contours = tsdf_to_spatial_contours(&tsdf, spacing);

        // Origin: (voxel_index + 0.5) * spacing
        // For Axial slice at Z=15
        let axial_15 = contours
            .contours
            .iter()
            .find(|c| c.normal == glam::Vec3::Z && (c.origin.z - (15.5 * 4.0)).abs() < 1e-4);

        assert!(
            axial_15.is_some(),
            "Should find axial slice at physical Z = 15.5 * 4.0 = 62.0"
        );
        let c = axial_15.unwrap();

        // Check that origin matches the chunk boundary (0,0 in this case) * spacing
        assert!((c.origin.x - 0.0).abs() < 1e-4);
        assert!((c.origin.y - 0.0).abs() < 1e-4);
        assert!((c.origin.z - 62.0).abs() < 1e-4);

        // Check that points are physical (relative to origin)
        // One of the points in the small loop around (15, 15) should be near (15*1.0, 15*2.0)
        let has_correct_pt = c
            .points
            .iter()
            .any(|p| (p.x - 15.0).abs() < 1.0 && (p.y - 30.0).abs() < 1.0);
        assert!(
            has_correct_pt,
            "Should find a point near physical (15.0, 30.0) relative to origin"
        );
    }
}
