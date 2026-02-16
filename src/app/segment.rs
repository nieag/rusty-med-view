//! Contour-based segmentation data structures.
//!
//! This module defines the core types for storing and manipulating
//! contour-based segmentations, including contours, SDF volumes, and meshes.

use crate::convert::contour_to_tsdf_chunks::TsdfChunk;
use crate::util::orientation::SlicePlane;
use std::collections::{HashMap, VecDeque};

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

    /// Point on the plane closest to the world origin.
    pub fn origin(&self) -> [f32; 3] {
        [
            self.normal[0] * self.distance,
            self.normal[1] * self.distance,
            self.normal[2] * self.distance,
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

/// Plane representation used by spatial contour workflows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpatialPlane {
    /// Unit normal of the contour plane.
    pub normal: [f32; 3],
    /// Plane equation distance: normal dot point = distance.
    pub distance: f32,
    /// In-plane U axis.
    pub u_axis: [f32; 3],
    /// In-plane V axis.
    pub v_axis: [f32; 3],
    /// A reference origin point on the plane.
    pub origin: [f32; 3],
}

impl SpatialPlane {
    /// Build a stable in-plane basis from an existing geometric plane.
    pub fn from_plane3d(plane: Plane3D) -> Self {
        let normal = normalize3(plane.normal);
        let up = if normal[2].abs() < 0.95 {
            [0.0, 0.0, 1.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let u_axis = normalize3(cross3(up, normal));
        let v_axis = normalize3(cross3(normal, u_axis));
        Self {
            normal,
            distance: plane.distance,
            u_axis,
            v_axis,
            origin: plane.origin(),
        }
    }
}

/// Spatial contour with plane metadata and cached in-plane points.
#[derive(Debug, Clone)]
pub struct SpatialContour {
    pub plane: SpatialPlane,
    pub points_3d: Vec<[f32; 3]>,
    pub points_2d: Vec<[f32; 2]>,
    pub is_closed: bool,
}

impl SpatialContour {
    pub fn from_plane_contour(contour: &PlaneContour) -> Self {
        let plane = SpatialPlane::from_plane3d(contour.plane);
        let mut points_2d = Vec::with_capacity(contour.points.len());
        for &p in &contour.points {
            let rel = [
                p[0] - plane.origin[0],
                p[1] - plane.origin[1],
                p[2] - plane.origin[2],
            ];
            points_2d.push([dot3(rel, plane.u_axis), dot3(rel, plane.v_axis)]);
        }
        Self {
            plane,
            points_3d: contour.points.clone(),
            points_2d,
            is_closed: contour.is_closed,
        }
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

    /// Check whether any authored contour exists on the requested axis-aligned plane.
    pub fn has_any_contours_on_plane(&self, plane: SlicePlane) -> bool {
        match plane {
            SlicePlane::Axial => !self.axial.is_empty(),
            SlicePlane::Coronal => !self.coronal.is_empty(),
            SlicePlane::Sagittal => !self.sagittal.is_empty(),
        }
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

    /// Iterate all contours as spatial contours.
    pub fn all_spatial_contours(&self) -> impl Iterator<Item = SpatialContour> + '_ {
        self.all_contours().map(SpatialContour::from_plane_contour)
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

#[inline]
fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[inline]
fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let len2 = dot3(v, v);
    if len2 > 1e-12 {
        let inv = len2.sqrt().recip();
        [v[0] * inv, v[1] * inv, v[2] * inv]
    } else {
        [0.0, 0.0, 1.0]
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
    /// Optional index-space bounds where meaningful SDF updates occurred: [x0, y0, z0, x1, y1, z1].
    /// Used to accelerate downstream meshing by avoiding full-volume scans.
    pub active_bounds: Option<[u32; 6]>,
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
            active_bounds: None,
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

/// Chunk key in index-space chunk grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkKey {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

/// Runtime chunk cache used by live meshing.
#[derive(Debug, Clone)]
pub struct SegmentRuntimeCache {
    pub chunk_size: u32,
    /// Persistent TSDF store: the authoritative volumetric representation.
    pub tsdf_chunks: HashMap<ChunkKey, TsdfChunk>,
    pub dirty_tsdf_chunks: VecDeque<ChunkKey>,
    pub dirty_mesh_chunks: VecDeque<ChunkKey>,
    pub mesh_chunks_cpu: HashMap<ChunkKey, MeshData>,
    pub mesh_chunks_gpu_revision: HashMap<ChunkKey, u64>,
}

impl Default for SegmentRuntimeCache {
    fn default() -> Self {
        Self {
            chunk_size: 32,
            tsdf_chunks: HashMap::new(),
            dirty_tsdf_chunks: VecDeque::new(),
            dirty_mesh_chunks: VecDeque::new(),
            mesh_chunks_cpu: HashMap::new(),
            mesh_chunks_gpu_revision: HashMap::new(),
        }
    }
}

impl SegmentRuntimeCache {
    fn enqueue_unique(queue: &mut VecDeque<ChunkKey>, key: ChunkKey) {
        if !queue.iter().any(|k| *k == key) {
            queue.push_back(key);
        }
    }

    pub fn enqueue_dirty_tsdf_chunks(&mut self, keys: impl IntoIterator<Item = ChunkKey>) {
        for key in keys {
            Self::enqueue_unique(&mut self.dirty_tsdf_chunks, key);
        }
    }

    pub fn enqueue_dirty_mesh_chunks(&mut self, keys: impl IntoIterator<Item = ChunkKey>) {
        for key in keys {
            Self::enqueue_unique(&mut self.dirty_mesh_chunks, key);
        }
    }
}

pub type SegmentChunkRuntime = SegmentRuntimeCache;

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
    /// Monotonic mesh revision used to gate GPU buffer rebuilds.
    pub mesh_revision: u64,

    // === Dirty Flags ===
    /// True if SDF needs regeneration
    pub sdf_dirty: bool,
    /// True if mesh needs regeneration
    pub mesh_dirty: bool,
    /// Monotonic revision for SDF updates (used by preview cache)
    pub sdf_revision: u64,
    /// Revision for live/interactive SDF updates.
    pub live_sdf_revision: u64,
    /// Revision for finalized full-resolution SDF updates.
    pub final_sdf_revision: u64,
    /// Dirty region in world coordinates [minx,miny,minz,maxx,maxy,maxz].
    pub dirty_roi_world: Option<[f32; 6]>,
    /// Runtime chunk caches for localized live meshing updates.
    pub chunk_runtime: SegmentRuntimeCache,

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
            mesh_revision: 0,
            sdf_dirty: true,
            mesh_dirty: true,
            sdf_revision: 0,
            live_sdf_revision: 0,
            final_sdf_revision: 0,
            dirty_roi_world: None,
            chunk_runtime: SegmentRuntimeCache::default(),
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

    /// Mark caches dirty and merge a world-space dirty ROI.
    pub fn mark_dirty_with_world_roi(&mut self, roi_world: [f32; 6]) {
        self.invalidate_caches();
        self.extend_dirty_roi_world(roi_world);
    }

    /// Merge a world-space dirty ROI into the segment's pending dirty region.
    pub fn extend_dirty_roi_world(&mut self, roi_world: [f32; 6]) {
        if roi_world[0] > roi_world[3] || roi_world[1] > roi_world[4] || roi_world[2] > roi_world[5]
        {
            return;
        }
        self.dirty_roi_world = Some(match self.dirty_roi_world {
            Some(prev) => [
                prev[0].min(roi_world[0]),
                prev[1].min(roi_world[1]),
                prev[2].min(roi_world[2]),
                prev[3].max(roi_world[3]),
                prev[4].max(roi_world[4]),
                prev[5].max(roi_world[5]),
            ],
            None => roi_world,
        });
    }

    /// Clear live chunk mesh cache and pending chunk queue.
    pub fn clear_chunk_runtime(&mut self) {
        self.chunk_runtime.tsdf_chunks.clear();
        self.chunk_runtime.dirty_tsdf_chunks.clear();
        self.chunk_runtime.dirty_mesh_chunks.clear();
        self.chunk_runtime.mesh_chunks_cpu.clear();
        self.chunk_runtime.mesh_chunks_gpu_revision.clear();
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

    #[test]
    fn test_spatial_plane_from_plane3d_has_orthonormal_basis() {
        let plane = Plane3D {
            normal: [0.0, 1.0, 0.0],
            distance: 12.5,
        };
        let sp = SpatialPlane::from_plane3d(plane);
        let uu = dot3(sp.u_axis, sp.u_axis);
        let vv = dot3(sp.v_axis, sp.v_axis);
        let uv = dot3(sp.u_axis, sp.v_axis);
        let un = dot3(sp.u_axis, sp.normal);
        let vn = dot3(sp.v_axis, sp.normal);
        assert!((uu - 1.0).abs() < 1e-5);
        assert!((vv - 1.0).abs() < 1e-5);
        assert!(uv.abs() < 1e-5);
        assert!(un.abs() < 1e-5);
        assert!(vn.abs() < 1e-5);
    }

    #[test]
    fn test_spatial_contour_from_plane_contour_projects_points() {
        let contour = PlaneContour::with_points(
            Plane3D::from_axial(5.0),
            vec![[1.0, 1.0, 5.0], [3.0, 1.0, 5.0], [3.0, 2.0, 5.0]],
            true,
        );
        let sc = SpatialContour::from_plane_contour(&contour);
        assert_eq!(sc.points_3d.len(), 3);
        assert_eq!(sc.points_2d.len(), 3);
        for p in &sc.points_3d {
            let on_plane = dot3(*p, sc.plane.normal) - sc.plane.distance;
            assert!(on_plane.abs() < 1e-4);
        }
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

    #[test]
    fn test_contour_set_all_spatial_contours() {
        let mut set = ContourSet::new();
        set.add_contour(
            SlicePlane::Sagittal,
            4,
            PlaneContour::with_points(
                Plane3D::from_sagittal(4.0),
                vec![[4.0, 0.0, 0.0], [4.0, 1.0, 0.0], [4.0, 1.0, 1.0]],
                true,
            ),
        );
        let spatial: Vec<_> = set.all_spatial_contours().collect();
        assert_eq!(spatial.len(), 1);
        assert!(spatial[0].is_closed);
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

    #[test]
    fn test_runtime_cache_enqueues_unique_chunk_keys() {
        let mut cache = SegmentRuntimeCache::default();
        let key = ChunkKey { x: 1, y: 2, z: 3 };

        cache.enqueue_dirty_tsdf_chunks([key, key]);
        cache.enqueue_dirty_mesh_chunks([key, key]);

        assert_eq!(cache.dirty_tsdf_chunks.len(), 1);
        assert_eq!(cache.dirty_mesh_chunks.len(), 1);
    }
}
