//! Contour-based segmentation data structures.
//!
//! This module defines the core types for storing and manipulating
//! contour-based segmentations, including contours, SDF volumes, and meshes.

use crate::util::orientation::SlicePlane;
use std::collections::HashMap;

// ============================================================================
// Plane3D - Arbitrary 3D plane representation
// ============================================================================

/// A 3D plane defined by normal and distance from origin.
/// Plane equation: normal · point = distance
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plane3D {
    pub normal: [f32; 3],
    pub distance: f32,
}

impl Plane3D {
    /// Create plane at axial slice (Z = constant)
    pub fn from_axial(z: f32) -> Self {
        Self {
            normal: [0.0, 0.0, 1.0],
            distance: z,
        }
    }

    /// Create plane at coronal slice (Y = constant)
    pub fn from_coronal(y: f32) -> Self {
        Self {
            normal: [0.0, 1.0, 0.0],
            distance: y,
        }
    }

    /// Create plane at sagittal slice (X = constant)
    pub fn from_sagittal(x: f32) -> Self {
        Self {
            normal: [1.0, 0.0, 0.0],
            distance: x,
        }
    }

    /// Create plane from SlicePlane enum and world-space position
    pub fn from_slice_plane(plane: SlicePlane, position: f32) -> Self {
        match plane {
            SlicePlane::Axial => Self::from_axial(position),
            SlicePlane::Coronal => Self::from_coronal(position),
            SlicePlane::Sagittal => Self::from_sagittal(position),
        }
    }

    /// Signed distance from point to plane (positive = same side as normal)
    pub fn distance_to_point(&self, point: [f32; 3]) -> f32 {
        let dot = self.normal[0] * point[0] + self.normal[1] * point[1] + self.normal[2] * point[2];
        dot - self.distance
    }

    /// Project point onto plane
    pub fn project_point(&self, point: [f32; 3]) -> [f32; 3] {
        let d = self.distance_to_point(point);
        [
            point[0] - d * self.normal[0],
            point[1] - d * self.normal[1],
            point[2] - d * self.normal[2],
        ]
    }
}

// ============================================================================
// PlaneContour - A contour on a specific plane
// ============================================================================

/// A closed or open contour on a specific 3D plane.
#[derive(Debug, Clone)]
pub struct PlaneContour {
    /// The plane this contour lies on
    pub plane: Plane3D,
    /// 3D world-space points on the plane
    pub points: Vec<[f32; 3]>,
    /// Whether the contour is closed (last point connects to first)
    pub is_closed: bool,
}

impl PlaneContour {
    /// Create a new empty contour on the given plane
    pub fn new(plane: Plane3D) -> Self {
        Self {
            plane,
            points: Vec::new(),
            is_closed: false,
        }
    }

    /// Create contour with points
    pub fn with_points(plane: Plane3D, points: Vec<[f32; 3]>, is_closed: bool) -> Self {
        Self {
            plane,
            points,
            is_closed,
        }
    }

    /// Check if contour has enough points to be valid
    pub fn is_valid(&self) -> bool {
        self.points.len() >= 3
    }
}

// ============================================================================
// ContourSet - Collection of contours per axis
// ============================================================================

/// Collection of contours organized by slice plane.
#[derive(Debug, Clone, Default)]
pub struct ContourSet {
    /// Axial contours indexed by slice Z (integer index)
    pub axial: HashMap<i32, Vec<PlaneContour>>,
    /// Coronal contours indexed by slice Y (integer index)
    pub coronal: HashMap<i32, Vec<PlaneContour>>,
    /// Sagittal contours indexed by slice X (integer index)
    pub sagittal: HashMap<i32, Vec<PlaneContour>>,
    /// Oblique contours (arbitrary planes)
    pub oblique: Vec<PlaneContour>,
}

impl ContourSet {
    /// Create empty contour set
    pub fn new() -> Self {
        Self::default()
    }

    /// Add contour for an axis-aligned slice
    pub fn add_contour(&mut self, plane: SlicePlane, index: i32, contour: PlaneContour) {
        let map = match plane {
            SlicePlane::Axial => &mut self.axial,
            SlicePlane::Coronal => &mut self.coronal,
            SlicePlane::Sagittal => &mut self.sagittal,
        };
        map.entry(index).or_default().push(contour);
    }

    /// Add oblique contour
    pub fn add_oblique_contour(&mut self, contour: PlaneContour) {
        self.oblique.push(contour);
    }

    /// Get contours at a specific slice
    pub fn contours_at_slice(&self, plane: SlicePlane, index: i32) -> Option<&Vec<PlaneContour>> {
        let map = match plane {
            SlicePlane::Axial => &self.axial,
            SlicePlane::Coronal => &self.coronal,
            SlicePlane::Sagittal => &self.sagittal,
        };
        map.get(&index)
    }

    /// Get mutable contours at a specific slice
    pub fn contours_at_slice_mut(
        &mut self,
        plane: SlicePlane,
        index: i32,
    ) -> Option<&mut Vec<PlaneContour>> {
        let map = match plane {
            SlicePlane::Axial => &mut self.axial,
            SlicePlane::Coronal => &mut self.coronal,
            SlicePlane::Sagittal => &mut self.sagittal,
        };
        map.get_mut(&index)
    }

    /// Iterate over all contours
    pub fn all_contours(&self) -> impl Iterator<Item = &PlaneContour> {
        self.axial
            .values()
            .flat_map(|v| v.iter())
            .chain(self.coronal.values().flat_map(|v| v.iter()))
            .chain(self.sagittal.values().flat_map(|v| v.iter()))
            .chain(self.oblique.iter())
    }

    /// Check if set is empty
    pub fn is_empty(&self) -> bool {
        self.axial.is_empty()
            && self.coronal.is_empty()
            && self.sagittal.is_empty()
            && self.oblique.is_empty()
    }

    /// Total number of contours
    pub fn count(&self) -> usize {
        self.axial.values().map(|v| v.len()).sum::<usize>()
            + self.coronal.values().map(|v| v.len()).sum::<usize>()
            + self.sagittal.values().map(|v| v.len()).sum::<usize>()
            + self.oblique.len()
    }

    /// Clear all contours
    pub fn clear(&mut self) {
        self.axial.clear();
        self.coronal.clear();
        self.sagittal.clear();
        self.oblique.clear();
    }
}

// ============================================================================
// SdfVolume - 3D signed distance field
// ============================================================================

/// 3D signed distance field with configurable resolution.
#[derive(Debug, Clone)]
pub struct SdfVolume {
    /// Grid dimensions [x, y, z]
    pub dimensions: [u32; 3],
    /// Voxel spacing in world units
    pub spacing: [f32; 3],
    /// World-space origin of the grid
    pub origin: [f32; 3],
    /// Signed distance values (row-major: x + y*dim_x + z*dim_x*dim_y)
    pub data: Vec<f32>,
}

impl SdfVolume {
    /// Create new SDF volume initialized to large positive values
    pub fn new(dimensions: [u32; 3], spacing: [f32; 3], origin: [f32; 3]) -> Self {
        let size = (dimensions[0] * dimensions[1] * dimensions[2]) as usize;
        Self {
            dimensions,
            spacing,
            origin,
            data: vec![f32::MAX; size],
        }
    }

    /// Get value at grid index
    pub fn get(&self, x: u32, y: u32, z: u32) -> f32 {
        if x >= self.dimensions[0] || y >= self.dimensions[1] || z >= self.dimensions[2] {
            return f32::MAX;
        }
        let idx = self.index(x, y, z);
        self.data[idx]
    }

    /// Set value at grid index
    pub fn set(&mut self, x: u32, y: u32, z: u32, value: f32) {
        if x >= self.dimensions[0] || y >= self.dimensions[1] || z >= self.dimensions[2] {
            return;
        }
        let idx = self.index(x, y, z);
        self.data[idx] = value;
    }

    /// Convert grid index to flat array index
    #[inline]
    fn index(&self, x: u32, y: u32, z: u32) -> usize {
        (x + y * self.dimensions[0] + z * self.dimensions[0] * self.dimensions[1]) as usize
    }

    /// Convert grid index to world position
    pub fn index_to_world(&self, index: [u32; 3]) -> [f32; 3] {
        [
            self.origin[0] + index[0] as f32 * self.spacing[0],
            self.origin[1] + index[1] as f32 * self.spacing[1],
            self.origin[2] + index[2] as f32 * self.spacing[2],
        ]
    }

    /// Convert world position to grid index (clamped)
    pub fn world_to_index(&self, world: [f32; 3]) -> Option<[u32; 3]> {
        let x = ((world[0] - self.origin[0]) / self.spacing[0]).round() as i32;
        let y = ((world[1] - self.origin[1]) / self.spacing[1]).round() as i32;
        let z = ((world[2] - self.origin[2]) / self.spacing[2]).round() as i32;

        if x < 0
            || y < 0
            || z < 0
            || x >= self.dimensions[0] as i32
            || y >= self.dimensions[1] as i32
            || z >= self.dimensions[2] as i32
        {
            return None;
        }

        Some([x as u32, y as u32, z as u32])
    }
}

// ============================================================================
// MeshData - Triangle mesh with normals
// ============================================================================

/// Triangle mesh with per-vertex normals.
#[derive(Debug, Clone, Default)]
pub struct MeshData {
    /// Vertex positions
    pub vertices: Vec<[f32; 3]>,
    /// Vertex normals
    pub normals: Vec<[f32; 3]>,
    /// Triangle indices (3 per face)
    pub indices: Vec<u32>,
}

impl MeshData {
    /// Create empty mesh
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of vertices
    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    /// Number of triangles
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Check if mesh is empty
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty() || self.indices.is_empty()
    }

    /// Clear mesh data
    pub fn clear(&mut self) {
        self.vertices.clear();
        self.normals.clear();
        self.indices.clear();
    }
}

// ============================================================================
// Segment - A single segmentation with contours and derived data
// ============================================================================

/// A segmentation segment with ground truth contours and derived caches.
#[derive(Debug, Clone)]
pub struct Segment {
    /// Unique identifier
    pub id: uuid::Uuid,
    /// Display name
    pub name: String,
    /// RGBA color for rendering
    pub color: [f32; 4],
    /// Visibility flag
    pub visible: bool,

    // === Ground Truth ===
    /// User-drawn contours (the source of truth)
    pub contours: ContourSet,

    // === Derived Caches ===
    /// Signed distance field (regenerated when contours change)
    pub sdf: Option<SdfVolume>,
    /// Triangle mesh (regenerated when SDF changes)
    pub mesh: Option<MeshData>,

    // === Dirty Flags ===
    /// True if SDF needs regeneration
    pub sdf_dirty: bool,
    /// True if mesh needs regeneration
    pub mesh_dirty: bool,
    /// Monotonic revision for SDF updates (used by preview cache)
    pub sdf_revision: u64,

    // === Configuration ===
    /// SDF resolution relative to volume (1.0 = same, 2.0 = 2x)
    pub sdf_resolution_multiplier: f32,
}

impl Segment {
    /// Create a new segment with the given name and color
    pub fn new(name: &str, color: [f32; 4]) -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            name: name.to_string(),
            color,
            visible: true,
            contours: ContourSet::new(),
            sdf: None,
            mesh: None,
            sdf_dirty: true,
            mesh_dirty: true,
            sdf_revision: 0,
            // Slightly denser default SDF improves extracted mesh smoothness.
            sdf_resolution_multiplier: 1.5,
        }
    }

    /// Invalidate all caches (call when contours change)
    pub fn invalidate_caches(&mut self) {
        self.sdf_dirty = true;
        self.mesh_dirty = true;
    }

    /// Alias for invalidate_caches
    pub fn mark_dirty(&mut self) {
        self.invalidate_caches();
    }

    /// Check if segment has any contours
    pub fn has_contours(&self) -> bool {
        !self.contours.is_empty()
    }

    /// Check if segment has a valid mesh for rendering
    pub fn has_mesh(&self) -> bool {
        self.mesh.as_ref().map_or(false, |m| !m.is_empty())
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- Plane3D Tests ---

    #[test]
    fn test_plane_from_axial() {
        let plane = Plane3D::from_axial(50.0);
        assert_eq!(plane.normal, [0.0, 0.0, 1.0]);
        assert_eq!(plane.distance, 50.0);
    }

    #[test]
    fn test_plane_from_coronal() {
        let plane = Plane3D::from_coronal(30.0);
        assert_eq!(plane.normal, [0.0, 1.0, 0.0]);
        assert_eq!(plane.distance, 30.0);
    }

    #[test]
    fn test_plane_from_sagittal() {
        let plane = Plane3D::from_sagittal(20.0);
        assert_eq!(plane.normal, [1.0, 0.0, 0.0]);
        assert_eq!(plane.distance, 20.0);
    }

    #[test]
    fn test_plane_distance_to_point() {
        let plane = Plane3D::from_axial(10.0);
        // Point at Z=15 is 5 units above plane
        let dist = plane.distance_to_point([0.0, 0.0, 15.0]);
        assert!((dist - 5.0).abs() < 1e-6);

        // Point at Z=5 is 5 units below plane
        let dist = plane.distance_to_point([0.0, 0.0, 5.0]);
        assert!((dist + 5.0).abs() < 1e-6);

        // Point on plane
        let dist = plane.distance_to_point([100.0, 200.0, 10.0]);
        assert!(dist.abs() < 1e-6);
    }

    #[test]
    fn test_plane_project_point() {
        let plane = Plane3D::from_axial(10.0);
        let point = [5.0, 7.0, 20.0];
        let projected = plane.project_point(point);
        assert_eq!(projected[0], 5.0);
        assert_eq!(projected[1], 7.0);
        assert!((projected[2] - 10.0).abs() < 1e-6);
    }

    // --- ContourSet Tests ---

    #[test]
    fn test_contour_set_new() {
        let set = ContourSet::new();
        assert!(set.is_empty());
        assert_eq!(set.count(), 0);
    }

    #[test]
    fn test_contour_set_add_and_retrieve() {
        let mut set = ContourSet::new();
        let contour = PlaneContour::with_points(
            Plane3D::from_axial(50.0),
            vec![[0.0, 0.0, 50.0], [1.0, 0.0, 50.0], [1.0, 1.0, 50.0]],
            true,
        );
        set.add_contour(SlicePlane::Axial, 50, contour);

        assert!(!set.is_empty());
        assert_eq!(set.count(), 1);

        let retrieved = set.contours_at_slice(SlicePlane::Axial, 50);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().len(), 1);

        // Different slice should be empty
        let empty = set.contours_at_slice(SlicePlane::Axial, 51);
        assert!(empty.is_none());
    }

    #[test]
    fn test_contour_set_all_contours() {
        let mut set = ContourSet::new();

        // Add to different axes
        set.add_contour(
            SlicePlane::Axial,
            10,
            PlaneContour::new(Plane3D::from_axial(10.0)),
        );
        set.add_contour(
            SlicePlane::Coronal,
            20,
            PlaneContour::new(Plane3D::from_coronal(20.0)),
        );
        set.add_oblique_contour(PlaneContour::new(Plane3D {
            normal: [0.707, 0.0, 0.707],
            distance: 50.0,
        }));

        let all: Vec<_> = set.all_contours().collect();
        assert_eq!(all.len(), 3);
    }

    // --- SdfVolume Tests ---

    #[test]
    fn test_sdf_volume_new() {
        let sdf = SdfVolume::new([10, 10, 10], [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        assert_eq!(sdf.dimensions, [10, 10, 10]);
        assert_eq!(sdf.data.len(), 1000);
        assert_eq!(sdf.get(0, 0, 0), f32::MAX);
    }

    #[test]
    fn test_sdf_volume_get_set() {
        let mut sdf = SdfVolume::new([10, 10, 10], [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        sdf.set(5, 5, 5, -1.5);
        assert_eq!(sdf.get(5, 5, 5), -1.5);
    }

    #[test]
    fn test_sdf_volume_indexing() {
        let sdf = SdfVolume::new([10, 10, 10], [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        let world = sdf.index_to_world([5, 5, 5]);
        assert_eq!(world, [5.0, 5.0, 5.0]);
    }

    #[test]
    fn test_sdf_volume_world_to_index() {
        let sdf = SdfVolume::new([10, 10, 10], [2.0, 2.0, 2.0], [0.0, 0.0, 0.0]);
        let idx = sdf.world_to_index([6.0, 4.0, 2.0]);
        assert_eq!(idx, Some([3, 2, 1]));

        // Out of bounds
        let idx = sdf.world_to_index([100.0, 0.0, 0.0]);
        assert_eq!(idx, None);
    }

    // --- MeshData Tests ---

    #[test]
    fn test_mesh_data_new() {
        let mesh = MeshData::new();
        assert!(mesh.is_empty());
        assert_eq!(mesh.vertex_count(), 0);
        assert_eq!(mesh.triangle_count(), 0);
    }

    #[test]
    fn test_mesh_data_triangle() {
        let mesh = MeshData {
            vertices: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.5, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]],
            indices: vec![0, 1, 2],
        };
        assert!(!mesh.is_empty());
        assert_eq!(mesh.vertex_count(), 3);
        assert_eq!(mesh.triangle_count(), 1);
    }

    // --- Segment Tests ---

    #[test]
    fn test_segment_new() {
        let segment = Segment::new("Test", [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(segment.name, "Test");
        assert_eq!(segment.color, [1.0, 0.0, 0.0, 1.0]);
        assert!(segment.visible);
        assert!(!segment.has_contours());
        assert!(!segment.has_mesh());
        assert!(segment.sdf_dirty);
        assert!(segment.mesh_dirty);
    }

    #[test]
    fn test_segment_invalidate() {
        let mut segment = Segment::new("Test", [1.0, 0.0, 0.0, 1.0]);
        segment.sdf_dirty = false;
        segment.mesh_dirty = false;

        segment.invalidate_caches();

        assert!(segment.sdf_dirty);
        assert!(segment.mesh_dirty);
    }
}
