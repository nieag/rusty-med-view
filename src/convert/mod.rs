//! Conversion algorithms for segmentation representations.
//!
//! This module contains algorithms for converting between different
//! segmentation formats (contours, SDF, mesh).

pub mod contour_to_sdf;

pub use contour_to_sdf::*;
