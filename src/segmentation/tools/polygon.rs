//! Polygon drawing tool with spline smoothing.
//!
//! Users click to place control points; the tool renders a smooth Catmull-Rom
//! spline through them. On close (double-click or connecting first point),
//! the spline is converted to a `SpatialContour`.

use crate::app::components::ViewMode;
use crate::segmentation::algorithms::spline::CatmullRomSpline;
use crate::segmentation::contour::SpatialContour;
use crate::util::orientation::SlicePlane;
use glam::{Vec2, Vec3};
use uuid::Uuid;

/// State for interactive polygon drawing tool.
#[derive(Debug, Clone)]
pub struct PolygonTool {
    /// Spline holding the control points (in screen UV coordinates).
    pub spline: CatmullRomSpline,
    /// The active viewing plane (Axial/Coronal/Sagittal).
    pub active_plane: Option<SlicePlane>,
    /// Normalized slice depth (0..1) in the volume.
    pub slice_depth: f32,
    /// Volume orientation quaternion for coordinate transforms.
    pub data_orientation: [f32; 4],
    /// Label index for the created contour.
    pub label_index: u8,
    /// Number of segments per control point for tessellation preview.
    pub tessellation_detail: usize,
}

impl Default for PolygonTool {
    fn default() -> Self {
        Self {
            spline: CatmullRomSpline::new(),
            active_plane: None,
            slice_depth: 0.5,
            data_orientation: [0.0, 0.0, 0.0, 1.0], // Identity
            label_index: 1,
            tessellation_detail: 8,
        }
    }
}

impl PolygonTool {
    /// Create a new polygon tool.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a control point in screen UV coordinates.
    pub fn add_point(&mut self, screen_uv: Vec2) {
        self.spline.add_point(screen_uv);
    }

    /// Remove the last control point.
    pub fn remove_last(&mut self) -> Option<Vec2> {
        self.spline.remove_last()
    }

    /// Clear all control points and reset state.
    pub fn clear(&mut self) {
        self.spline.clear();
        self.active_plane = None;
    }

    /// Check if the polygon is closed (enough points and first/last match).
    pub fn is_closed(&self) -> bool {
        self.spline.is_closed()
    }

    /// Get the tessellated spline preview (in screen UV for rendering).
    pub fn preview_polyline(&self) -> Vec<Vec2> {
        if self.spline.len() < 2 {
            return self.spline.control_points.clone();
        }
        self.spline.tessellate(self.tessellation_detail)
    }

    /// Get the raw control points (for rendering handles).
    pub fn control_points(&self) -> &[Vec2] {
        &self.spline.control_points
    }

    /// Close the polygon by connecting to the first point.
    pub fn close(&mut self) {
        if self.spline.len() >= 3 {
            if let Some(first) = self.spline.control_points.first().copied() {
                self.spline.add_point(first);
            }
        }
    }

    /// Convert the closed polygon to a SpatialContour.
    /// Uses the centralized SlicePlane API for orientation-aware coordinate conversion.
    ///
    /// Returns None if:
    /// - Not enough points
    /// - No active plane set
    pub fn to_spatial_contour(&self) -> Option<SpatialContour> {
        if self.spline.len() < 3 {
            return None;
        }
        let plane = self.active_plane?;

        // Get tessellated points in screen UV
        let screen_points = self.preview_polyline();

        // Convert each screen UV point to volume UV using SlicePlane
        let volume_points: Vec<Vec3> = screen_points
            .iter()
            .map(|uv| {
                let vol_uv = plane.screen_uv_to_volume(
                    [uv.x, uv.y],
                    self.slice_depth,
                    self.data_orientation,
                );
                Vec3::new(vol_uv[0], vol_uv[1], vol_uv[2])
            })
            .collect();

        // Compute plane basis from first three points
        let (origin, right, up, normal) = compute_plane_basis(&volume_points)?;

        // Project volume points to 2D plane coordinates
        let points_2d: Vec<Vec2> = volume_points
            .iter()
            .map(|p| {
                let rel = *p - origin;
                Vec2::new(rel.dot(right), rel.dot(up))
            })
            .collect();

        Some(SpatialContour {
            id: Uuid::new_v4(),
            origin,
            normal,
            right,
            up,
            points: points_2d,
            influence: 1.0, // Default influence radius (1 voxel)
            is_closed: self.is_closed(),
            label_index: self.label_index,
        })
    }
}

/// Compute the plane basis from a set of 3D points.
/// Returns (origin, right, up, normal).
fn compute_plane_basis(points: &[Vec3]) -> Option<(Vec3, Vec3, Vec3, Vec3)> {
    if points.len() < 3 {
        return None;
    }

    // Use centroid as origin
    let origin = points.iter().copied().sum::<Vec3>() / points.len() as f32;

    // Use first two edges to compute normal
    let v0 = points[1] - points[0];
    let v1 = points[2] - points[1];
    let normal = v0.cross(v1).normalize_or_zero();

    if normal.length_squared() < 1e-10 {
        // Degenerate case - points are collinear
        return None;
    }

    // Choose "right" as the direction from first to second point
    let right = v0.normalize_or_zero();
    let up = normal.cross(right).normalize_or_zero();

    Some((origin, right, up, normal))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_polygon_tool_add_remove() {
        let mut tool = PolygonTool::new();
        assert!(tool.spline.is_empty());

        tool.add_point(Vec2::new(0.1, 0.1));
        tool.add_point(Vec2::new(0.5, 0.1));
        tool.add_point(Vec2::new(0.5, 0.5));

        assert_eq!(tool.spline.len(), 3);

        tool.remove_last();
        assert_eq!(tool.spline.len(), 2);

        tool.clear();
        assert!(tool.spline.is_empty());
    }

    #[test]
    fn test_polygon_close() {
        let mut tool = PolygonTool::new();
        tool.add_point(Vec2::new(0.0, 0.0));
        tool.add_point(Vec2::new(1.0, 0.0));
        tool.add_point(Vec2::new(1.0, 1.0));

        assert!(!tool.is_closed());

        tool.close();
        assert!(tool.is_closed());
    }

    #[test]
    fn test_polygon_preview() {
        let mut tool = PolygonTool::new();
        tool.tessellation_detail = 4;

        tool.add_point(Vec2::new(0.0, 0.0));
        tool.add_point(Vec2::new(1.0, 0.0));
        tool.add_point(Vec2::new(1.0, 1.0));
        tool.add_point(Vec2::new(0.0, 1.0));

        let preview = tool.preview_polyline();
        assert!(!preview.is_empty());
    }

    #[test]
    fn test_compute_plane_basis() {
        let points = vec![
            Vec3::new(0.0, 0.0, 0.5),
            Vec3::new(1.0, 0.0, 0.5),
            Vec3::new(1.0, 1.0, 0.5),
            Vec3::new(0.0, 1.0, 0.5),
        ];

        let (origin, right, up, normal) = compute_plane_basis(&points).unwrap();

        // Normal should be +Z or -Z (axial plane)
        assert!(normal.z.abs() > 0.99);

        // Right and up should be orthogonal
        assert!(right.dot(up).abs() < 1e-5);
    }

    #[test]
    fn test_to_spatial_contour_needs_plane() {
        let mut tool = PolygonTool::new();
        tool.add_point(Vec2::new(0.0, 0.0));
        tool.add_point(Vec2::new(1.0, 0.0));
        tool.add_point(Vec2::new(1.0, 1.0));

        // No active plane set → should return None
        assert!(tool.to_spatial_contour().is_none());

        // Set active plane
        tool.active_plane = Some(SlicePlane::Axial);
        assert!(tool.to_spatial_contour().is_some());
    }
}
