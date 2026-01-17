use crate::app::components::ViewMode;
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contour {
    pub points: Vec<Vec2>, // Ordered polyline vertices (uv coordinates 0..1)
    pub is_closed: bool,
    pub segment_id: Uuid, // For tracking across slices
    pub label_index: u8,  // Which label this contour represents
}

/// A spatial constraint for the vector-authoritative system.
/// This represents a polyline in a specific 3D plane.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpatialContour {
    pub id: Uuid,
    pub origin: Vec3,      // Center of the plane
    pub normal: Vec3,      // Normal of the plane
    pub right: Vec3,       // "Right" vector for the 2D coordinate system in the plane
    pub up: Vec3,          // "Up" vector for the 2D coordinate system in the plane
    pub points: Vec<Vec2>, // 2D points relative to origin/right/up
    pub influence: f32,    // Spatial falloff radius (uncertainty)
    pub label_index: u8,
}

impl SpatialContour {
    pub fn to_world(&self, p: Vec2) -> Vec3 {
        self.origin + p.x * self.right + p.y * self.up
    }

    pub fn project_world(&self, world_p: Vec3) -> Vec2 {
        let rel = world_p - self.origin;
        Vec2::new(rel.dot(self.right), rel.dot(self.up))
    }
}

#[derive(Debug, Clone)]
pub struct SliceContours {
    pub contours: Vec<Contour>, // Multiple contours per slice (islands)
    pub slice_index: i32,
    pub view_axis: ViewMode, // Axial, Coronal, Sagittal
}

/// All contours for one segmentation layer
#[derive(Debug, Clone, Default)]
pub struct ContourSet {
    pub slices: HashMap<(ViewMode, i32), SliceContours>,
}

/// The authoritative vector layer for segmentation
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VectorContourSet {
    pub contours: Vec<SpatialContour>,
}

impl VectorContourSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.contours.clear();
    }
}

impl ContourSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_slice(&self, axis: ViewMode, index: i32) -> Option<&SliceContours> {
        self.slices.get(&(axis, index))
    }

    pub fn update_slice(&mut self, axis: ViewMode, index: i32, contours: Vec<Contour>) {
        self.slices.insert(
            (axis, index),
            SliceContours {
                contours,
                slice_index: index,
                view_axis: axis,
            },
        );
    }

    pub fn clear(&mut self) {
        self.slices.clear();
    }
}
