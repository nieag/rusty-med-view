//! Contour data: closed loops on slice planes.

use super::geometry::{PlaneDefinition, PlaneFamily};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContourPoint {
    pub local_mm: [f32; 2],
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContourLoop {
    pub points: Vec<ContourPoint>,
    pub is_closed: bool,
}

impl ContourLoop {
    pub fn is_valid_closed_loop(&self) -> bool {
        self.is_closed && self.points.len() >= 3
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContourSlice {
    pub plane: PlaneDefinition,
    pub loops: Vec<ContourLoop>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContourData {
    pub active_plane_family: PlaneFamily,
    pub slices: Vec<ContourSlice>,
}

impl ContourData {
    pub fn is_empty(&self) -> bool {
        self.slices.is_empty()
    }

    pub fn has_loops(&self) -> bool {
        self.slices.iter().any(|slice| !slice.loops.is_empty())
    }
}
