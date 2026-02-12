//! Segment system for contour-based segmentation workflow.
//!
//! Manages the full pipeline: contour drawing → SDF generation → mesh rendering.

use crate::app::segment::{Plane3D, PlaneContour, Segment};
use crate::components::{AppEntities, VolumeData};
use crate::convert::{build_sdf_from_contours, marching_cubes};
use crate::render::mesh_pipeline::MeshResources;
use crate::systems::contour_draw::ContourDrawState;
use crate::util::orientation::SlicePlane;
use hecs::World;
use std::collections::HashSet;

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
        self.active_segment
            .and_then(|idx| self.segments.get_mut(idx))
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
        segment.sdf_revision = segment.sdf_revision.wrapping_add(1);
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

/// Regenerate segment derivatives (SDF + mesh) once per frame if needed.
pub fn sys_update_segment_derivatives(world: &mut World, entities: &AppEntities) {
    let mut volume_dims = [0u32; 3];
    let mut volume_spacing = [1.0f32; 3];
    for (_, vol) in world.query::<&VolumeData>().iter() {
        volume_dims = vol.dimensions;
        volume_spacing = vol.spacing;
        break;
    }

    if volume_dims.contains(&0) {
        return;
    }

    if let Ok(mut manager) = world.get::<&mut SegmentManager>(entities.segments) {
        regenerate_all_dirty(&mut manager, volume_dims, volume_spacing);
    }
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

fn project_point_for_slice(slice_plane: SlicePlane, p: [f32; 3]) -> [f32; 2] {
    match slice_plane {
        SlicePlane::Axial => [p[0], p[1]],
        SlicePlane::Coronal => [p[0], p[2]],
        SlicePlane::Sagittal => [p[1], p[2]],
    }
}

fn orient2d(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn on_segment2d(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> bool {
    let min_x = a[0].min(b[0]) - 1e-5;
    let max_x = a[0].max(b[0]) + 1e-5;
    let min_y = a[1].min(b[1]) - 1e-5;
    let max_y = a[1].max(b[1]) + 1e-5;
    p[0] >= min_x && p[0] <= max_x && p[1] >= min_y && p[1] <= max_y
}

fn segments_intersect_2d(a1: [f32; 2], a2: [f32; 2], b1: [f32; 2], b2: [f32; 2]) -> bool {
    let o1 = orient2d(a1, a2, b1);
    let o2 = orient2d(a1, a2, b2);
    let o3 = orient2d(b1, b2, a1);
    let o4 = orient2d(b1, b2, a2);

    if o1.abs() < 1e-6 && on_segment2d(a1, a2, b1) {
        return true;
    }
    if o2.abs() < 1e-6 && on_segment2d(a1, a2, b2) {
        return true;
    }
    if o3.abs() < 1e-6 && on_segment2d(b1, b2, a1) {
        return true;
    }
    if o4.abs() < 1e-6 && on_segment2d(b1, b2, a2) {
        return true;
    }

    (o1 > 0.0) != (o2 > 0.0) && (o3 > 0.0) != (o4 > 0.0)
}

fn point_in_polygon_2d(point: [f32; 2], poly: &[[f32; 2]]) -> bool {
    if poly.len() < 3 {
        return false;
    }
    let mut crossings = 0;
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        let y_in_range =
            (a[1] <= point[1] && b[1] > point[1]) || (b[1] <= point[1] && a[1] > point[1]);
        if y_in_range {
            let t = (point[1] - a[1]) / (b[1] - a[1]);
            let x_intersect = a[0] + t * (b[0] - a[0]);
            if point[0] < x_intersect {
                crossings += 1;
            }
        }
    }
    crossings % 2 == 1
}

fn contours_overlap_or_intersect_on_slice(
    a: &[[f32; 3]],
    b: &[[f32; 3]],
    slice_plane: SlicePlane,
) -> bool {
    if a.len() < 3 || b.len() < 3 {
        return false;
    }
    let a2: Vec<[f32; 2]> = a
        .iter()
        .map(|&p| project_point_for_slice(slice_plane, p))
        .collect();
    let b2: Vec<[f32; 2]> = b
        .iter()
        .map(|&p| project_point_for_slice(slice_plane, p))
        .collect();

    for i in 0..a2.len() {
        let a1 = a2[i];
        let a2n = a2[(i + 1) % a2.len()];
        for j in 0..b2.len() {
            let b1 = b2[j];
            let b2n = b2[(j + 1) % b2.len()];
            if segments_intersect_2d(a1, a2n, b1, b2n) {
                return true;
            }
        }
    }

    point_in_polygon_2d(a2[0], &b2) || point_in_polygon_2d(b2[0], &a2)
}

/// Finish drawing and add contour to the active segment.
///
/// Returns true if a contour was successfully added.
pub fn finish_drawing(
    manager: &mut SegmentManager,
    volume_dims: [u32; 3],
    volume_spacing: [f32; 3],
) -> bool {
    use crate::systems::contour_draw::{
        maybe_close_contour, restabilize_contour, restabilize_contour_for_sculpt,
        screen_points_to_plane_contour,
    };

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

        // 1. Convert to world points
        let stroke_points = screen_points_to_plane_contour(
            &points,
            slice_plane,
            slice_index,
            volume_dims,
            volume_spacing,
        )
        .points;

        // 2. Try to BRIDGE or SCULPT existing contours
        let mut processed = false;
        if let Some(segment) = manager.active_segment_mut() {
            if let Some(existing_list) = segment
                .contours
                .contours_at_slice_mut(slice_plane, slice_index)
            {
                // A. Check for BRIDGE (start/end on different contours)
                let mut bridge_candidates = Vec::new();
                for (i, contour) in existing_list.iter().enumerate() {
                    let (_, d_start) = crate::systems::contour_draw::find_nearest_point_on_contour(
                        &contour.points,
                        stroke_points[0],
                    );
                    let (_, d_end) = crate::systems::contour_draw::find_nearest_point_on_contour(
                        &contour.points,
                        stroke_points[stroke_points.len() - 1],
                    );

                    if d_start <= 15.0 {
                        bridge_candidates.push((i, true));
                    }
                    if d_end <= 15.0 {
                        bridge_candidates.push((i, false));
                    }
                }

                // If we found two different contours to bridge
                let unique_ids: HashSet<_> = bridge_candidates.iter().map(|(id, _)| *id).collect();
                if unique_ids.len() >= 2 {
                    let ids: Vec<_> = unique_ids.into_iter().collect();
                    let id_a = ids[0];
                    let id_b = ids[1];
                    let (first, second) = if id_a < id_b {
                        (id_a, id_b)
                    } else {
                        (id_b, id_a)
                    };

                    let contour_b_points = existing_list[second].points.clone();
                    if crate::systems::contour_draw::bridge_contours(
                        &mut existing_list[first].points,
                        &contour_b_points,
                        &stroke_points,
                        15.0,
                    ) {
                        existing_list[first].points =
                            restabilize_contour_for_sculpt(&existing_list[first].points, true);
                        existing_list.remove(second);
                        processed = true;
                    }
                }

                // B. Fallback to SCULPT (start/end on same contour)
                if !processed {
                    for existing_contour in existing_list.iter_mut() {
                        if crate::systems::contour_draw::sculpt_contour(
                            &mut existing_contour.points,
                            &stroke_points,
                            15.0,
                        ) {
                            existing_contour.points =
                                restabilize_contour_for_sculpt(&existing_contour.points, true);
                            existing_contour.is_closed = true;
                            processed = true;
                            break;
                        }
                    }
                }
            }
        }

        if processed {
            if let Some(segment) = manager.active_segment_mut() {
                segment.mark_dirty();
                return true;
            }
        }

        // 3. Fallback: Create a new closed contour
        let mut contour = PlaneContour::with_points(
            Plane3D::from_slice_plane(
                slice_plane,
                (slice_index as f32 + 0.5) * volume_spacing[slice_plane.depth_axis()],
            ),
            stroke_points,
            false,
        );

        let natural_close_threshold = 10.0;
        let is_natural_closure = maybe_close_contour(&mut contour.points, natural_close_threshold);

        contour.points = restabilize_contour(&contour.points, is_natural_closure);
        contour.is_closed = true;

        if let Some(segment) = manager.active_segment_mut() {
            // Suppress creating overlapping/intersecting loops on the same slice.
            if let Some(existing_list) =
                segment.contours.contours_at_slice(slice_plane, slice_index)
            {
                for existing in existing_list {
                    if !existing.points.is_empty() && !contour.points.is_empty() {
                        if contours_overlap_or_intersect_on_slice(
                            &existing.points,
                            &contour.points,
                            slice_plane,
                        ) {
                            return true;
                        }
                        let (_, d) = crate::systems::contour_draw::find_nearest_point_on_contour(
                            &existing.points,
                            contour.points[0],
                        );
                        if d < 10.0 {
                            return true;
                        } // suppress redundant loop
                    }
                }
            }
            segment
                .contours
                .add_contour(slice_plane, slice_index, contour);
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

    #[test]
    fn test_regenerate_segment_updates_sdf_revision() {
        let mut segment = Segment::new("Test", [1.0, 0.0, 0.0, 1.0]);
        let rev0 = segment.sdf_revision;
        let _ = regenerate_segment_if_dirty(&mut segment, [8, 8, 8], [1.0, 1.0, 1.0]);
        assert!(segment.sdf_revision > rev0);
    }

    #[test]
    fn test_contours_overlap_or_intersect_on_slice_intersection() {
        let a = vec![
            [0.0, 0.0, 10.0],
            [2.0, 0.0, 10.0],
            [2.0, 2.0, 10.0],
            [0.0, 2.0, 10.0],
        ];
        let b = vec![
            [1.0, -1.0, 10.0],
            [3.0, 1.0, 10.0],
            [1.0, 3.0, 10.0],
            [-1.0, 1.0, 10.0],
        ];
        assert!(contours_overlap_or_intersect_on_slice(
            &a,
            &b,
            SlicePlane::Axial
        ));
    }

    #[test]
    fn test_contours_overlap_or_intersect_on_slice_separate() {
        let a = vec![
            [0.0, 0.0, 10.0],
            [1.0, 0.0, 10.0],
            [1.0, 1.0, 10.0],
            [0.0, 1.0, 10.0],
        ];
        let b = vec![
            [3.0, 3.0, 10.0],
            [4.0, 3.0, 10.0],
            [4.0, 4.0, 10.0],
            [3.0, 4.0, 10.0],
        ];
        assert!(!contours_overlap_or_intersect_on_slice(
            &a,
            &b,
            SlicePlane::Axial
        ));
    }
}
