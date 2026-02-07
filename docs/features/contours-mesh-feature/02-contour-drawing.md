# Phase 2: Contour Drawing Tool

## Goal

Implement freehand contour drawing in 2D views with spline smoothing.

## Files to Create/Modify

### [NEW] `src/systems/contour_draw.rs`
### [MODIFY] `src/systems/input.rs`
### [MODIFY] `src/systems/mod.rs`

## State Machine

```
IDLE ──(mouse down + contour tool)──► DRAWING
                                          │
                                          ├──(mouse move)──► add point
                                          │
                                          └──(mouse up)──► finalize contour ──► IDLE
```

## Data Structures

### ContourDrawState

Add to `src/systems/input.rs`:

```rust
#[derive(Default)]
pub enum ContourDrawState {
    #[default]
    Idle,
    Drawing {
        points: Vec<[f32; 2]>,  // Screen UV points
        slice_plane: SlicePlane,
        slice_index: i32,
    },
}
```

Add field to `InputState`:

```rust
pub struct InputState {
    // ... existing fields ...
    pub contour_draw_state: ContourDrawState,
}
```

## Core Functions

### `src/systems/contour_draw.rs`

```rust
use crate::app::segment::{PlaneContour, Plane3D};
use crate::util::orientation::SlicePlane;

/// Convert screen UV points to 3D world-space contour
pub fn screen_points_to_plane_contour(
    screen_points: &[[f32; 2]],
    slice_plane: SlicePlane,
    slice_index: i32,
    volume_dims: [u32; 3],
    volume_spacing: [f32; 3],
) -> PlaneContour {
    let plane = match slice_plane {
        SlicePlane::Axial => Plane3D::from_axial(slice_index as f32 * volume_spacing[2]),
        SlicePlane::Coronal => Plane3D::from_coronal(slice_index as f32 * volume_spacing[1]),
        SlicePlane::Sagittal => Plane3D::from_sagittal(slice_index as f32 * volume_spacing[0]),
    };
    
    let points_3d: Vec<[f32; 3]> = screen_points
        .iter()
        .map(|uv| screen_uv_to_world(uv, slice_plane, slice_index, volume_dims, volume_spacing))
        .collect();
    
    PlaneContour {
        plane,
        points: points_3d,
        is_closed: true,
    }
}

/// Convert screen UV [0,1] to world-space position
fn screen_uv_to_world(
    uv: &[f32; 2],
    slice_plane: SlicePlane,
    slice_index: i32,
    volume_dims: [u32; 3],
    volume_spacing: [f32; 3],
) -> [f32; 3] {
    // Use radiological convention (flip X for axial/coronal)
    let volume_uv = slice_plane.screen_uv_to_volume(*uv, slice_index as f32 / volume_dims[2] as f32);
    [
        volume_uv[0] * (volume_dims[0] as f32 - 1.0) * volume_spacing[0],
        volume_uv[1] * (volume_dims[1] as f32 - 1.0) * volume_spacing[1],
        volume_uv[2] * (volume_dims[2] as f32 - 1.0) * volume_spacing[2],
    ]
}

/// Smooth contour using Catmull-Rom spline interpolation
pub fn smooth_contour(points: &[[f32; 3]], segments_per_edge: u32) -> Vec<[f32; 3]> {
    if points.len() < 3 {
        return points.to_vec();
    }
    
    let mut result = Vec::new();
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

/// Catmull-Rom spline interpolation
fn catmull_rom(p0: [f32; 3], p1: [f32; 3], p2: [f32; 3], p3: [f32; 3], t: f32) -> [f32; 3] {
    let t2 = t * t;
    let t3 = t2 * t;
    
    let mut result = [0.0; 3];
    for i in 0..3 {
        result[i] = 0.5 * (
            2.0 * p1[i] +
            (-p0[i] + p2[i]) * t +
            (2.0 * p0[i] - 5.0 * p1[i] + 4.0 * p2[i] - p3[i]) * t2 +
            (-p0[i] + 3.0 * p1[i] - 3.0 * p2[i] + p3[i]) * t3
        );
    }
    result
}

/// Close contour if endpoints are within threshold distance
pub fn maybe_close_contour(points: &mut Vec<[f32; 3]>, threshold: f32) -> bool {
    if points.len() < 3 {
        return false;
    }
    
    let first = points[0];
    let last = points[points.len() - 1];
    let dist = ((first[0] - last[0]).powi(2)
              + (first[1] - last[1]).powi(2)
              + (first[2] - last[2]).powi(2)).sqrt();
    
    if dist < threshold {
        points.pop();  // Remove last point (will connect to first)
        true
    } else {
        false
    }
}

/// Simplify contour using Ramer-Douglas-Peucker algorithm
pub fn simplify_contour(points: &[[f32; 3]], epsilon: f32) -> Vec<[f32; 3]> {
    if points.len() < 3 {
        return points.to_vec();
    }
    rdp_simplify(points, epsilon)
}

fn rdp_simplify(points: &[[f32; 3]], epsilon: f32) -> Vec<[f32; 3]> {
    // Ramer-Douglas-Peucker implementation
    // ... (standard algorithm)
    points.to_vec() // Placeholder
}
```

## Input Handling

Add to input processing (in `sys_handle_input` or equivalent):

```rust
// When contour tool is active and in 2D view
if tool == Tool::ContourDraw && view_state.view_mode != ViewMode::View3D {
    match &mut input.contour_draw_state {
        ContourDrawState::Idle => {
            if mouse_pressed {
                input.contour_draw_state = ContourDrawState::Drawing {
                    points: vec![mouse_uv],
                    slice_plane: current_slice_plane,
                    slice_index: current_slice_index,
                };
            }
        }
        ContourDrawState::Drawing { points, .. } => {
            if mouse_down {
                // Add point if moved enough
                if let Some(last) = points.last() {
                    let dist = ((mouse_uv[0] - last[0]).powi(2) 
                              + (mouse_uv[1] - last[1]).powi(2)).sqrt();
                    if dist > 0.005 {  // ~5 pixels at 1000px viewport
                        points.push(mouse_uv);
                    }
                }
            } else {
                // Mouse released - finalize contour
                finalize_contour(&input.contour_draw_state, segment, volume);
                input.contour_draw_state = ContourDrawState::Idle;
            }
        }
    }
}
```

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_screen_to_plane_basic() {
        // Center of screen should map to center of volume slice
        let uv = [0.5, 0.5];
        let dims = [100, 100, 50];
        let spacing = [1.0, 1.0, 2.0];
        
        let contour = screen_points_to_plane_contour(
            &[uv],
            SlicePlane::Axial,
            25,  // Middle slice
            dims,
            spacing,
        );
        
        // Check point is at center of XY plane at Z=25*2=50
        let point = contour.points[0];
        assert!((point[0] - 49.5).abs() < 1.0);  // ~center X
        assert!((point[1] - 49.5).abs() < 1.0);  // ~center Y
        assert!((point[2] - 50.0).abs() < 0.1);  // Z = slice * spacing
    }

    #[test]
    fn test_smooth_contour_preserves_points() {
        let square = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        let smoothed = smooth_contour(&square, 4);
        
        // Should have more points
        assert!(smoothed.len() > square.len());
        // 4 edges * 4 segments = 16 points
        assert_eq!(smoothed.len(), 16);
    }

    #[test]
    fn test_close_contour_near_endpoints() {
        let mut points = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.01, 0.01, 0.0],  // Very close to first point
        ];
        let closed = maybe_close_contour(&mut points, 0.05);
        assert!(closed);
        assert_eq!(points.len(), 3);  // Last point removed
    }

    #[test]
    fn test_close_contour_far_endpoints() {
        let mut points = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.5, 0.5, 0.0],  // Not close to first point
        ];
        let closed = maybe_close_contour(&mut points, 0.05);
        assert!(!closed);
        assert_eq!(points.len(), 4);  // Unchanged
    }

    #[test]
    fn test_catmull_rom_midpoint() {
        // At t=0, should return p1
        let result = catmull_rom(
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            0.0
        );
        assert!((result[0] - 1.0).abs() < 1e-6);
    }
}
```

## Verification

```bash
# Run phase 2 tests
cargo test contour_draw::

# Expected: all tests pass
```

## Acceptance Criteria

- [ ] Contour drawing state machine implemented
- [ ] Screen UV to world-space conversion works
- [ ] Catmull-Rom smoothing produces smooth curves
- [ ] Unit tests pass: `cargo test contour_draw::`
