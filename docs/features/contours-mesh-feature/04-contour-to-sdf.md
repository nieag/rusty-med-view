# Phase 4: Contour → SDF Conversion

## Goal

Convert contours from any plane (axial, coronal, sagittal, oblique) into a 3D signed distance field.

## Files to Create

### [NEW] `src/convert/mod.rs`
### [NEW] `src/convert/contour_to_sdf.rs`

## Algorithm Overview

For each voxel in the SDF:
1. Compute signed distance to all contours
2. Use minimum distance (union operation)
3. Support configurable resolution (upsampling)

## Core Functions

### `src/convert/contour_to_sdf.rs`

```rust
use crate::app::segment::{ContourSet, SdfVolume, PlaneContour, Plane3D};

/// Check if 2D point is inside closed polygon using winding number
pub fn is_inside_polygon(point: [f32; 2], polygon: &[[f32; 2]]) -> bool {
    let mut winding = 0i32;
    let n = polygon.len();
    
    for i in 0..n {
        let p1 = polygon[i];
        let p2 = polygon[(i + 1) % n];
        
        if p1[1] <= point[1] {
            if p2[1] > point[1] {
                // Upward crossing
                if cross_2d(p2[0] - p1[0], p2[1] - p1[1],
                           point[0] - p1[0], point[1] - p1[1]) > 0.0 {
                    winding += 1;
                }
            }
        } else {
            if p2[1] <= point[1] {
                // Downward crossing
                if cross_2d(p2[0] - p1[0], p2[1] - p1[1],
                           point[0] - p1[0], point[1] - p1[1]) < 0.0 {
                    winding -= 1;
                }
            }
        }
    }
    
    winding != 0
}

fn cross_2d(ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    ax * by - ay * bx
}

/// Compute unsigned distance from 2D point to polyline
pub fn distance_to_polyline_2d(point: [f32; 2], polyline: &[[f32; 2]]) -> f32 {
    let mut min_dist = f32::MAX;
    let n = polyline.len();
    
    for i in 0..n {
        let p1 = polyline[i];
        let p2 = polyline[(i + 1) % n];
        
        let dist = distance_to_segment_2d(point, p1, p2);
        min_dist = min_dist.min(dist);
    }
    
    min_dist
}

fn distance_to_segment_2d(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let pa = [p[0] - a[0], p[1] - a[1]];
    let ba = [b[0] - a[0], b[1] - a[1]];
    
    let ba_len_sq = ba[0] * ba[0] + ba[1] * ba[1];
    if ba_len_sq < 1e-10 {
        return (pa[0] * pa[0] + pa[1] * pa[1]).sqrt();
    }
    
    let t = (pa[0] * ba[0] + pa[1] * ba[1]) / ba_len_sq;
    let t = t.clamp(0.0, 1.0);
    
    let closest = [a[0] + t * ba[0], a[1] + t * ba[1]];
    let dx = p[0] - closest[0];
    let dy = p[1] - closest[1];
    
    (dx * dx + dy * dy).sqrt()
}

/// Compute signed distance from 2D point to closed contour
pub fn signed_distance_2d(point: [f32; 2], contour: &[[f32; 2]]) -> f32 {
    let dist = distance_to_polyline_2d(point, contour);
    let inside = is_inside_polygon(point, contour);
    
    if inside { -dist } else { dist }
}

/// Build SDF from contours with configurable resolution
pub fn build_sdf_from_contours(
    contours: &ContourSet,
    volume_dims: [u32; 3],
    volume_spacing: [f32; 3],
    resolution_multiplier: f32,
) -> SdfVolume {
    let sdf_dims = [
        ((volume_dims[0] as f32) * resolution_multiplier) as u32,
        ((volume_dims[1] as f32) * resolution_multiplier) as u32,
        volume_dims[2],  // Keep Z same as slice count
    ];
    
    let sdf_spacing = [
        volume_spacing[0] / resolution_multiplier,
        volume_spacing[1] / resolution_multiplier,
        volume_spacing[2],
    ];
    
    let mut sdf = SdfVolume::new(sdf_dims, sdf_spacing, [0.0, 0.0, 0.0]);
    
    // Initialize with large positive values (outside)
    for v in sdf.data.iter_mut() {
        *v = 1e10;
    }
    
    // Process each contour
    for contour in contours.all_contours() {
        add_contour_to_sdf(&mut sdf, contour, volume_dims, volume_spacing);
    }
    
    sdf
}

/// Add a single contour's contribution to the SDF
fn add_contour_to_sdf(
    sdf: &mut SdfVolume,
    contour: &PlaneContour,
    volume_dims: [u32; 3],
    volume_spacing: [f32; 3],
) {
    if contour.points.len() < 3 {
        return;
    }
    
    // Determine which voxels are affected by this contour
    for z in 0..sdf.dimensions[2] {
        for y in 0..sdf.dimensions[1] {
            for x in 0..sdf.dimensions[0] {
                let world_pos = sdf.index_to_world([x, y, z]);
                
                let dist = distance_to_plane_contour(world_pos, contour);
                
                // Take minimum (union)
                let current = sdf.get(x, y, z);
                sdf.set(x, y, z, current.min(dist));
            }
        }
    }
}

/// Compute signed distance from 3D point to contour on arbitrary plane
pub fn distance_to_plane_contour(point: [f32; 3], contour: &PlaneContour) -> f32 {
    // 1. Distance from point to contour's plane
    let dist_to_plane = contour.plane.distance_to_point(point);
    
    // 2. Project point onto plane
    let projected = contour.plane.project_point(point);
    
    // 3. Convert to 2D coordinates on the plane
    let (u, v) = project_to_plane_2d(projected, &contour.plane);
    
    // 4. Convert contour points to 2D on same plane
    let contour_2d: Vec<[f32; 2]> = contour.points
        .iter()
        .map(|&p| {
            let (u, v) = project_to_plane_2d(p, &contour.plane);
            [u, v]
        })
        .collect();
    
    // 5. Compute 2D signed distance
    let dist_2d = signed_distance_2d([u, v], &contour_2d);
    
    // 6. Combine: 3D distance
    // If point is far from plane, use euclidean distance
    // If point is on plane, use 2D distance
    let combined = (dist_to_plane * dist_to_plane + dist_2d.abs() * dist_2d.abs()).sqrt();
    
    // Sign comes from 2D test (inside/outside contour)
    if dist_2d < 0.0 { -combined } else { combined }
}

/// Project 3D point to 2D coordinates on plane
fn project_to_plane_2d(point: [f32; 3], plane: &Plane3D) -> (f32, f32) {
    // Create orthonormal basis on plane
    let n = plane.normal;
    
    // Find a vector not parallel to normal
    let up = if n[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
    
    // u = normalize(up × n)
    let u = normalize(cross(up, n));
    // v = n × u
    let v = cross(n, u);
    
    // Project point onto u and v axes
    (dot(point, u), dot(point, v))
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len < 1e-10 { return [0.0, 0.0, 0.0]; }
    [v[0] / len, v[1] / len, v[2] / len]
}
```

## Multi-Axis Blending

When contours exist on multiple axes, their SDFs are combined:

```rust
/// Blend SDFs from different axis contours
pub fn blend_sdf_union(a: f32, b: f32) -> f32 {
    a.min(b)  // Union: minimum distance
}

pub fn blend_sdf_intersection(a: f32, b: f32) -> f32 {
    a.max(b)  // Intersection: maximum distance
}

pub fn blend_sdf_smooth_union(a: f32, b: f32, k: f32) -> f32 {
    let h = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
    b * (1.0 - h) + a * h - k * h * (1.0 - h)
}
```

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_inside_polygon_square() {
        let square = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        
        assert!(is_inside_polygon([0.5, 0.5], &square));  // Center
        assert!(!is_inside_polygon([2.0, 0.5], &square)); // Outside right
        assert!(!is_inside_polygon([-0.5, 0.5], &square)); // Outside left
    }

    #[test]
    fn test_is_inside_polygon_concave() {
        // L-shaped polygon
        let l_shape = [
            [0.0, 0.0], [2.0, 0.0], [2.0, 1.0],
            [1.0, 1.0], [1.0, 2.0], [0.0, 2.0],
        ];
        
        assert!(is_inside_polygon([0.5, 0.5], &l_shape));  // Inside
        assert!(is_inside_polygon([0.5, 1.5], &l_shape));  // Inside upper arm
        assert!(!is_inside_polygon([1.5, 1.5], &l_shape)); // Outside concave area
    }

    #[test]
    fn test_signed_distance_circle() {
        // Approximate circle with many points
        let n = 32;
        let radius = 1.0;
        let circle: Vec<[f32; 2]> = (0..n)
            .map(|i| {
                let theta = 2.0 * std::f32::consts::PI * i as f32 / n as f32;
                [radius * theta.cos(), radius * theta.sin()]
            })
            .collect();
        
        // Point at center: distance = -radius
        let d_center = signed_distance_2d([0.0, 0.0], &circle);
        assert!((d_center + radius).abs() < 0.1);
        
        // Point at 2x radius: distance = +radius
        let d_outside = signed_distance_2d([2.0, 0.0], &circle);
        assert!((d_outside - radius).abs() < 0.1);
    }

    #[test]
    fn test_distance_to_oblique_contour() {
        // 45-degree tilted plane
        let plane = Plane3D {
            normal: normalize([1.0, 0.0, 1.0]),
            distance: 0.0,
        };
        
        let contour = PlaneContour {
            plane,
            points: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, -1.0],
                [1.0, 1.0, -1.0],
                [0.0, 1.0, 0.0],
            ],
            is_closed: true,
        };
        
        // Point on plane, inside contour
        let dist = distance_to_plane_contour([0.5, 0.5, -0.5], &contour);
        assert!(dist < 0.0);  // Should be inside (negative)
    }

    #[test]
    fn test_build_sdf_single_slice() {
        let mut contours = ContourSet::new();
        
        // Square contour on middle axial slice
        let square = PlaneContour {
            plane: Plane3D::from_axial(25.0),
            points: vec![
                [10.0, 10.0, 25.0],
                [40.0, 10.0, 25.0],
                [40.0, 40.0, 25.0],
                [10.0, 40.0, 25.0],
            ],
            is_closed: true,
        };
        contours.add_contour(SlicePlane::Axial, 25, square);
        
        let sdf = build_sdf_from_contours(&contours, [50, 50, 50], [1.0, 1.0, 1.0], 1.0);
        
        // Center should be inside (negative)
        assert!(sdf.get(25, 25, 25) < 0.0);
        
        // Corner should be outside (positive)
        assert!(sdf.get(0, 0, 25) > 0.0);
    }
}
```

## Verification

```bash
# Run phase 4 tests
cargo test contour_to_sdf::

# Expected: all tests pass
```

## Acceptance Criteria

- [ ] 2D signed distance computation is correct
- [ ] Winding number algorithm handles concave polygons
- [ ] Oblique plane distance computation works
- [ ] Multi-axis contours blend correctly (union)
- [ ] SDF resolution is configurable
- [ ] Unit tests pass: `cargo test contour_to_sdf::`
