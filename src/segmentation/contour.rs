use crate::app::components::ViewMode;
use glam::Vec2;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Contour {
    pub points: Vec<Vec2>, // Ordered polyline vertices (uv coordinates 0..1)
    pub is_closed: bool,
    pub segment_id: Uuid, // For tracking across slices
    pub label_index: u8,  // Which label this contour represents
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
