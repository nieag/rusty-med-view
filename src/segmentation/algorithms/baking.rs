use crate::segmentation::{
    contour::SpatialContour,
    tsdf::{ChunkedTSDF, CHUNK_SIZE},
};
use glam::{Vec2, Vec3};

/// Bakes a set of 3D spatial contours into the TSDF.
///
/// This process involves:
/// 1. Identifying the bounds of each contour.
/// 2. Iterating over relevant TSDF chunks.
/// 3. For each voxel, computing the signed distance to the contour.
/// 4. Combining this distance with the existing field (using a minimum operation).
pub fn bake_contours_to_tsdf(tsdf: &mut ChunkedTSDF, contours: &[SpatialContour]) {
    for contour in contours {
        bake_single_contour(tsdf, contour);
    }
}

fn bake_single_contour(tsdf: &mut ChunkedTSDF, contour: &SpatialContour) {
    if contour.points.is_empty() {
        return;
    }

    // 1. Compute AABB of the contour in world space to limit the search
    // We add 'influence' padding + truncation to ensure smooth falloff
    let padding = contour.influence + tsdf.truncation;
    let (min_aabb, max_aabb) = compute_contour_aabb(contour, padding);

    // 2. Convert AABB to chunk coordinates
    let (min_cx, min_cy, min_cz, _, _, _) = tsdf.world_to_local(
        min_aabb.x.floor() as i32,
        min_aabb.y.floor() as i32,
        min_aabb.z.floor() as i32,
    );
    let (max_cx, max_cy, max_cz, _, _, _) = tsdf.world_to_local(
        max_aabb.x.ceil() as i32,
        max_aabb.y.ceil() as i32,
        max_aabb.z.ceil() as i32,
    );

    // 3. Iterate over potentially affected chunks
    for cz in min_cz..=max_cz {
        for cy in min_cy..=max_cy {
            for cx in min_cx..=max_cx {
                // Ensure chunk exists (or create it if we are imposing structure)
                let _chunk_key = (cx, cy, cz);
                
                // We use a manual iteration here because we need to update potentially many voxels
                // and we want to avoid repeated hash lookups for every voxel.
                // However, ChunkedTSDF abstraction hides the chunk logic. 
                // For performance, we might eventually want a "get_mut_chunk_iter" method.
                // For now, we'll iterate voxels in world space and use set_distance.
                
                let chunk_world_x = cx as i32 * CHUNK_SIZE as i32;
                let chunk_world_y = cy as i32 * CHUNK_SIZE as i32;
                let chunk_world_z = cz as i32 * CHUNK_SIZE as i32;

                for lz in 0..CHUNK_SIZE as i32 {
                    for ly in 0..CHUNK_SIZE as i32 {
                        for lx in 0..CHUNK_SIZE as i32 {
                            let wx = chunk_world_x + lx;
                            let wy = chunk_world_y + ly;
                            let wz = chunk_world_z + lz;

                            // Optimization: Check if voxel is within AABB before expensive math
                            if (wx as f32) < min_aabb.x || (wx as f32) > max_aabb.x ||
                               (wy as f32) < min_aabb.y || (wy as f32) > max_aabb.y ||
                               (wz as f32) < min_aabb.z || (wz as f32) > max_aabb.z {
                                continue;
                            }

                            let voxel_pos = Vec3::new(wx as f32, wy as f32, wz as f32);
                            let dist = compute_contour_distance(contour, voxel_pos);

                            // We only care if the distance is within our influence/truncation range
                            // AND if it improves the existing field (union operation).
                            // But since this is a "set" operation, we might want to overwrite?
                            // Standard CSG Union: min(d1, d2).
                            // Standard CSG Intersection: max(d1, d2).
                            
                            // For now, let's assume we are "unioning" this shape onto the field.
                            let current_dist = tsdf.get_distance(wx, wy, wz);
                            
                            // We only modify if the new distance is "better" (more inside) or close.
                            // Simply taking min(current, new) works for union.
                            if dist < current_dist {
                                tsdf.set_distance(wx, wy, wz, dist);
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Computes the signed distance from a 3D point to a SpatialContour.
/// 
/// The distance is composed of:
/// - d_plane: Perpendicular distance to the contour's plane
/// - d_poly:  Signed distance to the polygon within the plane
/// 
/// Final distance = sqrt(d_plane^2 + d_poly^2) (if outside)
///                  d_plane (if inside, but this is simplistic)
/// 
/// Improved logic for "Extruded" or "Thickened" feel:
/// If we treat the contour as a tube or slab, the logic changes.
/// But typically for segmentation, we treat the contour as an infinite cylinder 
/// clipped by the influence radius? Or a sphere-swept surface?
/// 
/// "Vector-Authoritative" usually implies the contour defines a boundary on that plane, 
/// and we interpolate between them. 
/// However, for a single contour bake, we treat it as a 3D shape defined by its influence.
fn compute_contour_distance(contour: &SpatialContour, p: Vec3) -> f32 {
    // 1. Project point onto the plane
    let v = p - contour.origin;
    let dist_plane = v.dot(contour.normal); // Signed distance to plane
    
    // Project p onto the plane basis to get 2D coords
    let x2d = v.dot(contour.right);
    let y2d = v.dot(contour.up);
    let p2d = Vec2::new(x2d, y2d);

    // 2. Compute 2D signed distance to polygon
    let dist_poly = signed_distance_polygon(p2d, &contour.points, contour.is_closed);

    // 3. Combine distances
    // We want to create a shape that is the polygon extruded by 'influence'
    // OR we want a smooth field.
    // 
    // If dist_poly < 0 (inside polygon):
    //   We are inside the "cylinder". 
    //   The distance depends on how far we are from the plane.
    //   Let's say the shape thickness is determined by 'influence'.
    //   d = sqrt(dist_plane^2 + 0) = |dist_plane|
    //   But we want it to be a signed distance field.
    // 
    // Simple approach: The SDF of a flat shape (disk/polygon) in 3D.
    // d = max( |dist_plane| - thickness, dist_poly )
    // 
    // If we assume the contour has NO thickness (infinitely thin sheet),
    // then d = sqrt(dist_plane^2 + max(0, dist_poly)^2)
    // 
    // But usually for segmentation, we want "interpolation".
    // For a single contour, let's treat it as a "blob" with radius `influence`.
    // Effectively a capsule/tube around the polygon border + a slab in the middle.
    
    // Let's use the standard 2D extruded shape logic.
    // Distance to the 2D shape in the plane:
    let d_2d = dist_poly; 
    
    // Perpendicular distance:
    let d_z = dist_plane.abs();

    // Combine (Exterior)
    // If d_2d > 0 (outside polygon), we are outside.
    // d = length( (d_2d, d_z) )
    if d_2d > 0.0 {
        // Outside the extruded prism
        // We calculate distance to the border of the prism
        let len = (d_2d * d_2d + d_z * d_z).sqrt();
        // Subtract influence to give it volume?
        // If we want the contour to represent a solid, we normally loft.
        // If this is just a constraint, maybe we return a distance that represents 
        // "distance to the constraint".
        // 
        // Let's stick to the architecture doc: "TSDF ... resolve correspondence ... influence radius"
        // This implies the contour emits a field up to `influence`.
        
        return len - contour.influence;
    } else {
        // Inside the polygon (d_2d < 0).
        // We are inside the infinite prism.
        // Distance is determined by the plane distance vs the polygon boundary.
        // distance to surface = max(d_z - half_thickness, d_2d)
        
        // Let's assume the "thickness" is also `influence`.
        let half_thickness = contour.influence;
        
        // Signed distance intersection of "Slab of thickness 2*influence" AND "Infinite Prism"
        // d = max( |z| - r, d_poly )
        // If d < 0, we are inside.
        
        return (d_z - half_thickness).max(d_2d);
    }
}

/// Computes the signed distance from point p to a 2D polygon.
/// Negative = Inside, Positive = Outside.
fn signed_distance_polygon(p: Vec2, points: &[Vec2], closed: bool) -> f32 {
    if points.len() < 2 {
        if let Some(pt) = points.first() {
            return (p - *pt).length();
        }
        return std::f32::MAX;
    }

    let mut min_dist_sq = f32::MAX;
    let mut winding_number = 0; // standard winding number method

    let count = points.len();
    let num_segments = if closed { count } else { count - 1 };

    for i in 0..num_segments {
        let v1 = points[i];
        let v2 = points[(i + 1) % count];

        // Distance to segment
        let d_sq = squared_dist_point_segment(p, v1, v2);
        if d_sq < min_dist_sq {
            min_dist_sq = d_sq;
        }

        // Winding number (only relevant for closed polygons)
        if closed {
            if v1.y <= p.y {
                if v2.y > p.y && is_left(v1, v2, p) > 0.0 {
                    winding_number += 1;
                }
            } else {
                if v2.y <= p.y && is_left(v1, v2, p) < 0.0 {
                    winding_number -= 1;
                }
            }
        }
    }

    let dist = min_dist_sq.sqrt();

    if closed {
        // If winding number is non-zero, we are inside
        if winding_number != 0 {
            return -dist;
        }
    } else {
        // Open polylines don't have an "inside", so always positive distance
        // (Unless we want to treat them as strokes? For now, assume simple distance)
        return dist;
    }

    dist
}

fn squared_dist_point_segment(p: Vec2, v1: Vec2, v2: Vec2) -> f32 {
    let diff = v2 - v1;
    let len_sq = diff.length_squared();
    if len_sq < 1e-6 {
        return (p - v1).length_squared();
    }
    
    let t = ((p - v1).dot(diff) / len_sq).clamp(0.0, 1.0);
    let projection = v1 + diff * t;
    (p - projection).length_squared()
}

// > 0 if p is to the left of directed line v1->v2
fn is_left(v1: Vec2, v2: Vec2, p: Vec2) -> f32 {
    (v2.x - v1.x) * (p.y - v1.y) - (p.x - v1.x) * (v2.y - v1.y)
}

fn compute_contour_aabb(contour: &SpatialContour, padding: f32) -> (Vec3, Vec3) {
    if contour.points.is_empty() {
        return (contour.origin, contour.origin);
    }
    
    // We need to transform all 2D points to 3D world space to get the AABB
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    
    for &p2d in &contour.points {
        let p3d = contour.to_world(p2d);
        min = min.min(p3d);
        max = max.max(p3d);
    }
    
    (min - Vec3::splat(padding), max + Vec3::splat(padding))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_signed_distance_polygon_square() {
        let points = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 10.0),
            Vec2::new(0.0, 10.0),
        ];

        // Center (inside) -> -5.0
        assert!((signed_distance_polygon(Vec2::new(5.0, 5.0), &points, true) + 5.0).abs() < 1e-5);
        
        // Outside X -> 5.0
        assert!((signed_distance_polygon(Vec2::new(15.0, 5.0), &points, true) - 5.0).abs() < 1e-5);
        
        // On edge -> 0.0
        assert!(signed_distance_polygon(Vec2::new(0.0, 5.0), &points, true).abs() < 1e-5);
    }

    #[test]
    fn test_bake_simple_contour() {
        let mut tsdf = ChunkedTSDF::new(5.0);
        
        // Create a contour at Z=10, 10x10 square
        let contour = SpatialContour {
            id: Uuid::new_v4(),
            origin: Vec3::new(0.0, 0.0, 10.0),
            normal: Vec3::Z,
            right: Vec3::X,
            up: Vec3::Y,
            points: vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(10.0, 0.0),
                Vec2::new(10.0, 10.0),
                Vec2::new(0.0, 10.0),
            ],
            influence: 2.0, // Thickness = 4.0 total ( +/- 2.0 )
            is_closed: true,
            label_index: 1,
        };

        bake_single_contour(&mut tsdf, &contour);
        
        // Check center point (5, 5, 10) -> Should be deeply inside
        // Distance to plane = 0.
        // Distance to poly border = -5.0 (inside).
        // d = max(0 - 2.0, -5.0) = -2.0.
        let d = tsdf.get_distance(5, 5, 10);
        assert!((d + 2.0).abs() < 0.5, "Expected -2.0 at center, got {}", d);
        
        // Check outside point (13, 5, 10)
        // Distance to plane = 0.
        // Distance to poly border (nearest is 10,5) = 3.0
        // d = 3.0 - 2.0 (influence) = 1.0
        let d_out = tsdf.get_distance(13, 5, 10);
        assert!((d_out - 1.0).abs() < 0.5, "Expected 1.0 outside, got {}", d_out);
        
        // Check Z offset (5, 5, 15) -> 5 units above plane
        // Plane dist = 5.
        // Poly dist = -5.
        // d = max(5 - 2, -5) = 3.
        let d_z = tsdf.get_distance(5, 5, 15);
        assert!((d_z - 3.0).abs() < 0.5, "Expected 3.0 above plane, got {}", d_z);
    }
}
