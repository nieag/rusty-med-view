//! Segment system for contour-based segmentation workflow.
//!
//! Manages the full pipeline: contour drawing → SDF generation → mesh rendering.

use crate::app::segment::Segment;
use crate::convert::{build_sdf_from_contours, marching_cubes};
use crate::render::mesh_pipeline::MeshResources;
use crate::systems::contour_draw::ContourDrawState;
use crate::util::orientation::SlicePlane;

// ============================================================================
// Segmentation Manager Component
// ============================================================================

/// Manages all segments in the application.
#[derive(Default)]
pub struct SegmentManager {
    /// All segments
    pub segments: Vec<Segment>,
    /// Currently active segment index (for editing)
    pub active_segment: Option<usize>,
    /// Contour drawing state
    pub draw_state: ContourDrawState,
}

impl SegmentManager {
    /// Create a new segment manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a new segment with default settings.
    pub fn add_segment(&mut self, name: &str, color: [f32; 4]) -> usize {
        let segment = Segment::new(name, color);
        self.segments.push(segment);
        let idx = self.segments.len() - 1;
        self.active_segment = Some(idx);
        idx
    }

    /// Get the active segment mutably.
    pub fn active_segment_mut(&mut self) -> Option<&mut Segment> {
        self.active_segment.and_then(|idx| self.segments.get_mut(idx))
    }

    /// Get the active segment.
    pub fn active_segment(&self) -> Option<&Segment> {
        self.active_segment.and_then(|idx| self.segments.get(idx))
    }

    /// Remove a segment by index.
    pub fn remove_segment(&mut self, idx: usize) -> Option<Segment> {
        if idx < self.segments.len() {
            let removed = self.segments.remove(idx);
            // Adjust active index
            if let Some(active) = self.active_segment {
                if active == idx {
                    self.active_segment = None;
                } else if active > idx {
                    self.active_segment = Some(active - 1);
                }
            }
            Some(removed)
        } else {
            None
        }
    }

    /// Find segment by ID.
    pub fn find_by_id(&self, id: uuid::Uuid) -> Option<usize> {
        self.segments.iter().position(|s| s.id == id)
    }

    /// Get segment count.
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }
}

// ============================================================================
// Segment Regeneration
// ============================================================================

/// Regenerate SDF and mesh for a segment if dirty.
///
/// Returns true if mesh was regenerated.
pub fn regenerate_segment_if_dirty(
    segment: &mut Segment,
    volume_dims: [u32; 3],
    volume_spacing: [f32; 3],
) -> bool {
    let mut mesh_changed = false;

    // Regenerate SDF if dirty
    if segment.sdf_dirty {
        let sdf = build_sdf_from_contours(
            &segment.contours,
            volume_dims,
            volume_spacing,
            segment.sdf_resolution_multiplier,
        );
        segment.sdf = Some(sdf);
        segment.sdf_dirty = false;
        segment.mesh_dirty = true; // SDF changed, mesh needs update
    }

    // Regenerate mesh if dirty
    if segment.mesh_dirty {
        if let Some(sdf) = &segment.sdf {
            let mesh = marching_cubes(sdf, 0.0);
            segment.mesh = Some(mesh);
            mesh_changed = true;
        }
        segment.mesh_dirty = false;
    }

    mesh_changed
}

/// Regenerate all dirty segments.
pub fn regenerate_all_dirty(
    manager: &mut SegmentManager,
    volume_dims: [u32; 3],
    volume_spacing: [f32; 3],
) -> Vec<usize> {
    let mut regenerated = Vec::new();

    for (idx, segment) in manager.segments.iter_mut().enumerate() {
        if regenerate_segment_if_dirty(segment, volume_dims, volume_spacing) {
            regenerated.push(idx);
        }
    }

    regenerated
}

// ============================================================================
// Contour Drawing Integration
// ============================================================================

/// Start drawing a contour on the active segment.
pub fn start_drawing(
    manager: &mut SegmentManager,
    slice_plane: SlicePlane,
    slice_index: i32,
    start_point: [f32; 2],
) -> bool {
    if manager.active_segment.is_none() {
        return false;
    }

    manager.draw_state = ContourDrawState::Drawing {
        points: vec![start_point],
        slice_plane,
        slice_index,
    };
    true
}

/// Add a point while drawing.
pub fn add_drawing_point(manager: &mut SegmentManager, point: [f32; 2]) {
    if let ContourDrawState::Drawing { points, .. } = &mut manager.draw_state {
        // Only add if sufficiently far from last point
        if let Some(last) = points.last() {
            let dx = point[0] - last[0];
            let dy = point[1] - last[1];
            let dist = (dx * dx + dy * dy).sqrt();
            if dist > 0.005 {
                // Min distance threshold in UV
                points.push(point);
            }
        } else {
            points.push(point);
        }
    }
}

/// Finish drawing and add contour to the active segment.
///
/// Returns true if a contour was successfully added.
pub fn finish_drawing(
    manager: &mut SegmentManager,
    volume_dims: [u32; 3],
    volume_spacing: [f32; 3],
) -> bool {
    use crate::systems::contour_draw::{maybe_close_contour, screen_points_to_plane_contour, smooth_contour};

    let draw_state = std::mem::take(&mut manager.draw_state);

    if let ContourDrawState::Drawing {
        points,
        slice_plane,
        slice_index,
    } = draw_state
    {
        if points.len() < 3 {
            return false; // Not enough points
        }

        // Convert screen points to 3D contour
        let mut contour = screen_points_to_plane_contour(
            &points,
            slice_plane,
            slice_index,
            volume_dims,
            volume_spacing,
        );

        // Smooth the contour
        let smoothed = smooth_contour(&contour.points, 4);
        contour.points = smoothed;

        // Try to close the contour
        contour.is_closed = maybe_close_contour(&mut contour.points, 2.0);

        // Add to active segment
        if let Some(segment) = manager.active_segment_mut() {
            segment.contours.add_contour(slice_plane, slice_index, contour);
            segment.mark_dirty();
            return true;
        }
    }

    false
}

/// Cancel the current drawing operation.
pub fn cancel_drawing(manager: &mut SegmentManager) {
    manager.draw_state = ContourDrawState::Idle;
}

/// Check if currently drawing.
pub fn is_drawing(manager: &SegmentManager) -> bool {
    matches!(manager.draw_state, ContourDrawState::Drawing { .. })
}

// ============================================================================
// GPU Resource Management
// ============================================================================

/// Cached GPU resources for a segment's mesh.
pub struct SegmentGpuResources {
    pub segment_id: uuid::Uuid,
    pub mesh_resources: Option<MeshResources>,
}

impl SegmentGpuResources {
    /// Create empty resources for a segment.
    pub fn new(segment_id: uuid::Uuid) -> Self {
        Self {
            segment_id,
            mesh_resources: None,
        }
    }

    /// Update mesh resources from segment.
    pub fn update_from_segment(&mut self, device: &wgpu::Device, segment: &Segment) {
        if let Some(mesh) = &segment.mesh {
            self.mesh_resources = MeshResources::from_mesh_data(device, mesh);
        } else {
            self.mesh_resources = None;
        }
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_segment_manager_new() {
        let manager = SegmentManager::new();
        assert!(manager.is_empty());
        assert_eq!(manager.len(), 0);
        assert!(manager.active_segment.is_none());
    }

    #[test]
    fn test_add_segment() {
        let mut manager = SegmentManager::new();
        let idx = manager.add_segment("Kidney", [1.0, 0.0, 0.0, 1.0]);
        
        assert_eq!(idx, 0);
        assert_eq!(manager.len(), 1);
        assert_eq!(manager.active_segment, Some(0));
        assert_eq!(manager.segments[0].name, "Kidney");
    }

    #[test]
    fn test_remove_segment() {
        let mut manager = SegmentManager::new();
        manager.add_segment("A", [1.0, 0.0, 0.0, 1.0]);
        manager.add_segment("B", [0.0, 1.0, 0.0, 1.0]);
        manager.add_segment("C", [0.0, 0.0, 1.0, 1.0]);
        manager.active_segment = Some(1); // B

        let removed = manager.remove_segment(0); // Remove A
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().name, "A");
        assert_eq!(manager.len(), 2);
        assert_eq!(manager.active_segment, Some(0)); // B is now at index 0
    }

    #[test]
    fn test_find_by_id() {
        let mut manager = SegmentManager::new();
        manager.add_segment("Test", [1.0, 0.0, 0.0, 1.0]);
        let id = manager.segments[0].id;

        assert_eq!(manager.find_by_id(id), Some(0));
        assert_eq!(manager.find_by_id(uuid::Uuid::new_v4()), None);
    }

    #[test]
    fn test_start_drawing_no_active() {
        let mut manager = SegmentManager::new();
        let result = start_drawing(&mut manager, SlicePlane::Axial, 50, [0.5, 0.5]);
        assert!(!result); // No active segment
    }

    #[test]
    fn test_start_drawing_with_active() {
        let mut manager = SegmentManager::new();
        manager.add_segment("Test", [1.0, 0.0, 0.0, 1.0]);
        
        let result = start_drawing(&mut manager, SlicePlane::Axial, 50, [0.5, 0.5]);
        assert!(result);
        assert!(is_drawing(&manager));
    }

    #[test]
    fn test_add_drawing_point() {
        let mut manager = SegmentManager::new();
        manager.add_segment("Test", [1.0, 0.0, 0.0, 1.0]);
        start_drawing(&mut manager, SlicePlane::Axial, 50, [0.0, 0.0]);

        add_drawing_point(&mut manager, [0.1, 0.0]); // Far enough
        add_drawing_point(&mut manager, [0.101, 0.0]); // Too close, should be ignored

        if let ContourDrawState::Drawing { points, .. } = &manager.draw_state {
            assert_eq!(points.len(), 2); // Only 2 points (start + 1 valid)
        } else {
            panic!("Expected Drawing state");
        }
    }

    #[test]
    fn test_cancel_drawing() {
        let mut manager = SegmentManager::new();
        manager.add_segment("Test", [1.0, 0.0, 0.0, 1.0]);
        start_drawing(&mut manager, SlicePlane::Axial, 50, [0.5, 0.5]);
        
        cancel_drawing(&mut manager);
        assert!(!is_drawing(&manager));
    }
}
