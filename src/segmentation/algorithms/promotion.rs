//! Incremental contour promotion for real-time brush feedback.
//!
//! Unlike the full `handoff.rs` (which scans the entire TSDF on load),
//! this module extracts contours from just the region modified by a brush stroke.

use crate::app::components::ViewMode;
use crate::segmentation::algorithms::MarchingSquares;
use crate::segmentation::contour::{SpatialContour, VectorContourSet};
use crate::segmentation::tsdf::ChunkedTSDF;
use crate::util::orientation::SlicePlane;
use glam::{Vec2, Vec3};
use uuid::Uuid;

/// Configuration for incremental contour promotion.
#[derive(Debug, Clone)]
pub struct PromotionConfig {
    /// RDP simplification epsilon (in normalized 0..1 space).
    pub rdp_epsilon: f32,
    /// Contour influence radius (spatial falloff).
    pub influence: f32,
}

impl Default for PromotionConfig {
    fn default() -> Self {
        Self {
            rdp_epsilon: 0.001,
            influence: 1.0,
        }
    }
}

/// Extract contours from a specific slice of the TSDF within an AABB.
///
/// This is the incremental version - instead of scanning the whole volume,
/// it only looks at the region that was just modified by a brush stroke.
///
/// # Arguments
/// * `tsdf` - The TSDF to extract from
/// * `view_mode` - Which plane to slice (Axial/Coronal/Sagittal)
/// * `slice_idx` - The slice index in voxel space
/// * `aabb_min` - Minimum corner of the modified region
/// * `aabb_max` - Maximum corner of the modified region
/// * `spacing` - Volume spacing for physical units
/// * `config` - Promotion configuration
///
/// # Returns
/// A vector of `SpatialContour`s extracted from this slice within the AABB.
pub fn extract_contours_in_aabb(
    tsdf: &ChunkedTSDF,
    view_mode: ViewMode,
    slice_idx: i32,
    aabb_min: [i32; 3],
    aabb_max: [i32; 3],
    spacing: [f32; 3],
    config: &PromotionConfig,
) -> Vec<SpatialContour> {
    // Calculate the 2D slice bounds based on view mode
    let (x_range, y_range, depth_check) = match view_mode {
        ViewMode::Axial => (
            aabb_min[0]..=aabb_max[0],
            aabb_min[1]..=aabb_max[1],
            slice_idx >= aabb_min[2] && slice_idx <= aabb_max[2],
        ),
        ViewMode::Coronal => (
            aabb_min[0]..=aabb_max[0],
            aabb_min[2]..=aabb_max[2],
            slice_idx >= aabb_min[1] && slice_idx <= aabb_max[1],
        ),
        ViewMode::Sagittal => (
            aabb_min[1]..=aabb_max[1],
            aabb_min[2]..=aabb_max[2],
            slice_idx >= aabb_min[0] && slice_idx <= aabb_max[0],
        ),
        _ => return vec![],
    };

    // If the slice doesn't intersect the AABB, nothing to extract
    if !depth_check {
        return vec![];
    }

    let width = (x_range.end() - x_range.start() + 1) as usize;
    let height = (y_range.end() - y_range.start() + 1) as usize;

    if width < 2 || height < 2 {
        return vec![];
    }

    // Extract the 2D slice values from TSDF
    let mut slice_values = vec![0.0f32; width * height];
    for (yi, wy) in y_range.clone().enumerate() {
        for (xi, wx) in x_range.clone().enumerate() {
            let (vx, vy, vz) = match view_mode {
                ViewMode::Axial => (wx, wy, slice_idx),
                ViewMode::Coronal => (wx, slice_idx, wy),
                ViewMode::Sagittal => (slice_idx, wx, wy),
                _ => continue,
            };
            slice_values[yi * width + xi] = tsdf.get_distance(vx, vy, vz);
        }
    }

    // Run Marching Squares
    let ms = MarchingSquares::new(0.0);
    let raw_contours = ms.extract(&slice_values, (width as u32, height as u32), 1);

    let mut result = Vec::new();

    for contour in raw_contours {
        if contour.points.len() < 3 {
            continue;
        }

        // Simplify via RDP
        let simplified = rdp_simplify(&contour.points, config.rdp_epsilon);
        if simplified.len() < 3 {
            continue;
        }

        // Convert to physical space
        let spacing_v = Vec3::from(spacing);
        let origin = match view_mode {
            ViewMode::Axial => Vec3::new(
                *x_range.start() as f32 * spacing[0],
                *y_range.start() as f32 * spacing[1],
                (slice_idx as f32 + 0.5) * spacing[2],
            ),
            ViewMode::Coronal => Vec3::new(
                *x_range.start() as f32 * spacing[0],
                (slice_idx as f32 + 0.5) * spacing[1],
                *y_range.start() as f32 * spacing[2],
            ),
            ViewMode::Sagittal => Vec3::new(
                (slice_idx as f32 + 0.5) * spacing[0],
                *x_range.start() as f32 * spacing[1],
                *y_range.start() as f32 * spacing[2],
            ),
            _ => continue,
        };

        let (right, up, normal) = match view_mode {
            ViewMode::Axial => (Vec3::X, Vec3::Y, Vec3::Z),
            ViewMode::Coronal => (Vec3::X, Vec3::Z, Vec3::Y),
            ViewMode::Sagittal => (Vec3::Y, Vec3::Z, Vec3::X),
            _ => continue,
        };

        // Scale points from 0..1 to physical coordinates
        let (sp_right, sp_up) = match view_mode {
            ViewMode::Axial => (spacing[0], spacing[1]),
            ViewMode::Coronal => (spacing[0], spacing[2]),
            ViewMode::Sagittal => (spacing[1], spacing[2]),
            _ => (1.0, 1.0),
        };

        let physical_points: Vec<Vec2> = simplified
            .iter()
            .map(|p| Vec2::new(p.x * width as f32 * sp_right, p.y * height as f32 * sp_up))
            .collect();

        result.push(SpatialContour {
            id: Uuid::new_v4(),
            origin,
            normal,
            right,
            up,
            points: physical_points,
            influence: config.influence,
            is_closed: contour.is_closed,
            label_index: contour.label_index,
        });
    }

    result
}

/// Update the VectorContourSet with contours from a modified AABB.
/// This replaces any existing contours in the affected slices.
///
/// # Arguments
/// * `contour_set` - The contour set to update
/// * `tsdf` - The TSDF to extract from
/// * `view_mode` - Which plane was modified
/// * `aabb_min` - Minimum corner of the modified region
/// * `aabb_max` - Maximum corner of the modified region
/// * `spacing` - Volume spacing
/// * `config` - Promotion configuration
pub fn update_contours_in_region(
    contour_set: &mut VectorContourSet,
    tsdf: &ChunkedTSDF,
    view_mode: ViewMode,
    aabb_min: [i32; 3],
    aabb_max: [i32; 3],
    spacing: [f32; 3],
    config: &PromotionConfig,
) {
    // Determine which slice indices to update based on view mode
    let slice_range = match view_mode {
        ViewMode::Axial => aabb_min[2]..=aabb_max[2],
        ViewMode::Coronal => aabb_min[1]..=aabb_max[1],
        ViewMode::Sagittal => aabb_min[0]..=aabb_max[0],
        _ => return,
    };

    let normal = match view_mode {
        ViewMode::Axial => Vec3::Z,
        ViewMode::Coronal => Vec3::Y,
        ViewMode::Sagittal => Vec3::X,
        _ => return,
    };

    // Remove old contours in the affected slice range
    let depth_range = match view_mode {
        ViewMode::Axial => (
            aabb_min[2] as f32 * spacing[2],
            (aabb_max[2] + 1) as f32 * spacing[2],
        ),
        ViewMode::Coronal => (
            aabb_min[1] as f32 * spacing[1],
            (aabb_max[1] + 1) as f32 * spacing[1],
        ),
        ViewMode::Sagittal => (
            aabb_min[0] as f32 * spacing[0],
            (aabb_max[0] + 1) as f32 * spacing[0],
        ),
        _ => return,
    };

    contour_set.contours.retain(|c| {
        // Keep contours that don't match this view or are outside the depth range
        if c.normal != normal {
            return true;
        }
        let depth = match view_mode {
            ViewMode::Axial => c.origin.z,
            ViewMode::Coronal => c.origin.y,
            ViewMode::Sagittal => c.origin.x,
            _ => return true,
        };
        depth < depth_range.0 || depth > depth_range.1
    });

    // Extract new contours for each slice
    for slice_idx in slice_range {
        let new_contours = extract_contours_in_aabb(
            tsdf, view_mode, slice_idx, aabb_min, aabb_max, spacing, config,
        );
        contour_set.contours.extend(new_contours);
    }

    contour_set.mark_dirty();
}

/// Ramer-Douglas-Peucker simplification.
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
    fn test_extract_contours_in_aabb() {
        let mut tsdf = ChunkedTSDF::new(4.0);

        // Create a small blob in the TSDF
        for z in 10..15 {
            for y in 10..15 {
                for x in 10..15 {
                    let cx = 12.5;
                    let cy = 12.5;
                    let cz = 12.5;
                    let d = ((x as f32 - cx).powi(2)
                        + (y as f32 - cy).powi(2)
                        + (z as f32 - cz).powi(2))
                    .sqrt()
                        - 2.0;
                    tsdf.set_distance(x, y, z, d);
                }
            }
        }

        let config = PromotionConfig::default();
        let contours = extract_contours_in_aabb(
            &tsdf,
            ViewMode::Axial,
            12,
            [10, 10, 10],
            [15, 15, 15],
            [1.0, 1.0, 1.0],
            &config,
        );

        // Should extract at least one contour
        assert!(!contours.is_empty(), "Should extract contours from blob");
    }

    #[test]
    fn test_extract_empty_region() {
        let tsdf = ChunkedTSDF::new(4.0);
        let config = PromotionConfig::default();

        let contours = extract_contours_in_aabb(
            &tsdf,
            ViewMode::Axial,
            50,
            [0, 0, 0],
            [10, 10, 10],
            [1.0, 1.0, 1.0],
            &config,
        );

        // Empty TSDF = no contours
        assert!(contours.is_empty());
    }

    #[test]
    fn test_update_contours_in_region() {
        let mut tsdf = ChunkedTSDF::new(4.0);
        let mut contour_set = VectorContourSet::new();

        // Add some geometry
        tsdf.apply_brush([15, 15, 15], 3.0);

        let config = PromotionConfig::default();
        update_contours_in_region(
            &mut contour_set,
            &tsdf,
            ViewMode::Axial,
            [12, 12, 12],
            [18, 18, 18],
            [1.0, 1.0, 1.0],
            &config,
        );

        assert!(!contour_set.contours.is_empty());
        assert!(contour_set.dirty);
    }
}
