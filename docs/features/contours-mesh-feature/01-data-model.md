# Phase 1: Data Model

## Goal

Define core data structures for contour-based segmentation.

## Files to Create/Modify

### [NEW] `src/app/segment.rs`

## Data Structures

### Plane3D

```rust
/// A 3D plane defined by normal and distance from origin
/// Equation: normal · point = distance
pub struct Plane3D {
    pub normal: [f32; 3],
    pub distance: f32,
}

impl Plane3D {
    /// Create plane at axial slice Z
    pub fn from_axial(z: f32) -> Self {
        Self { normal: [0.0, 0.0, 1.0], distance: z }
    }
    
    /// Create plane at coronal slice Y
    pub fn from_coronal(y: f32) -> Self {
        Self { normal: [0.0, 1.0, 0.0], distance: y }
    }
    
    /// Create plane at sagittal slice X
    pub fn from_sagittal(x: f32) -> Self {
        Self { normal: [1.0, 0.0, 0.0], distance: x }
    }
    
    /// Signed distance from point to plane
    pub fn distance_to_point(&self, point: [f32; 3]) -> f32;
    
    /// Project point onto plane
    pub fn project_point(&self, point: [f32; 3]) -> [f32; 3];
}
```

### PlaneContour

```rust
/// A closed contour on a specific plane
pub struct PlaneContour {
    pub plane: Plane3D,
    pub points: Vec<[f32; 3]>,  // 3D world-space points on the plane
    pub is_closed: bool,
}
```

### ContourSet

```rust
use std::collections::HashMap;

/// Collection of contours organized by plane type
pub struct ContourSet {
    pub axial: HashMap<i32, Vec<PlaneContour>>,    // slice_z → contours
    pub coronal: HashMap<i32, Vec<PlaneContour>>,  // slice_y → contours
    pub sagittal: HashMap<i32, Vec<PlaneContour>>, // slice_x → contours
    pub oblique: Vec<PlaneContour>,                // arbitrary planes
}

impl ContourSet {
    pub fn new() -> Self;
    pub fn add_contour(&mut self, slice_plane: SlicePlane, index: i32, contour: PlaneContour);
    pub fn add_oblique_contour(&mut self, contour: PlaneContour);
    pub fn contours_at_slice(&self, plane: SlicePlane, index: i32) -> Option<&Vec<PlaneContour>>;
    pub fn all_contours(&self) -> impl Iterator<Item = &PlaneContour>;
    pub fn is_empty(&self) -> bool;
    pub fn clear(&mut self);
}
```

### SdfVolume

```rust
/// 3D signed distance field with configurable resolution
pub struct SdfVolume {
    pub dimensions: [u32; 3],
    pub spacing: [f32; 3],
    pub origin: [f32; 3],
    pub data: Vec<f32>,  // Row-major: x + y*dim_x + z*dim_x*dim_y
}

impl SdfVolume {
    pub fn new(dimensions: [u32; 3], spacing: [f32; 3], origin: [f32; 3]) -> Self;
    pub fn get(&self, x: u32, y: u32, z: u32) -> f32;
    pub fn set(&mut self, x: u32, y: u32, z: u32, value: f32);
    pub fn world_to_index(&self, world_pos: [f32; 3]) -> Option<[u32; 3]>;
    pub fn index_to_world(&self, index: [u32; 3]) -> [f32; 3];
}
```

### MeshData

```rust
/// Triangle mesh with per-vertex normals
pub struct MeshData {
    pub vertices: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,  // Triangle indices (3 per face)
}

impl MeshData {
    pub fn new() -> Self;
    pub fn vertex_count(&self) -> usize;
    pub fn triangle_count(&self) -> usize;
    pub fn is_empty(&self) -> bool;
}
```

### Segment

```rust
/// A segmentation segment with ground truth contours and derived caches
pub struct Segment {
    pub id: uuid::Uuid,
    pub name: String,
    pub color: [f32; 4],
    pub visible: bool,
    
    // Ground truth
    pub contours: ContourSet,
    
    // Derived caches (regenerated when contours change)
    pub sdf: Option<SdfVolume>,
    pub mesh: Option<MeshData>,
    
    // Dirty flags
    pub sdf_dirty: bool,
    pub mesh_dirty: bool,
    
    // Configuration
    pub sdf_resolution_multiplier: f32,  // 1.0 = volume resolution, 2.0 = 2x
}

impl Segment {
    pub fn new(name: &str, color: [f32; 4]) -> Self;
    pub fn invalidate_caches(&mut self);
}
```

## Unit Tests

Add to `src/app/segment.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plane_from_axial() {
        let plane = Plane3D::from_axial(50.0);
        assert_eq!(plane.normal, [0.0, 0.0, 1.0]);
        assert_eq!(plane.distance, 50.0);
    }

    #[test]
    fn test_plane_distance_to_point() {
        let plane = Plane3D::from_axial(10.0);
        // Point at Z=15 is 5 units above plane
        let dist = plane.distance_to_point([0.0, 0.0, 15.0]);
        assert!((dist - 5.0).abs() < 1e-6);
    }

    #[test]
    fn test_plane_project_point() {
        let plane = Plane3D::from_axial(10.0);
        let point = [5.0, 7.0, 20.0];
        let projected = plane.project_point(point);
        assert_eq!(projected, [5.0, 7.0, 10.0]);
    }

    #[test]
    fn test_contour_set_add_and_retrieve() {
        let mut set = ContourSet::new();
        let contour = PlaneContour {
            plane: Plane3D::from_axial(50.0),
            points: vec![[0.0, 0.0, 50.0], [1.0, 0.0, 50.0]],
            is_closed: true,
        };
        set.add_contour(SlicePlane::Axial, 50, contour);
        
        let retrieved = set.contours_at_slice(SlicePlane::Axial, 50);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().len(), 1);
    }

    #[test]
    fn test_sdf_volume_indexing() {
        let sdf = SdfVolume::new([10, 10, 10], [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        let world = sdf.index_to_world([5, 5, 5]);
        assert_eq!(world, [5.0, 5.0, 5.0]);
    }
}
```

## Verification

```bash
# Run phase 1 tests
cargo test segment::

# Expected output: all tests pass
```

## Acceptance Criteria

- [ ] All data structures compile with no errors
- [ ] Unit tests pass: `cargo test segment::`
- [ ] `Segment` can be added as ECS component
