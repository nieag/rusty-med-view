//! Pure data types shared by every layer.
//!
//! This module imports nothing else from the crate, so the dependency order is
//! `model` <- `convert` <- `app::roi` <- runtime, systems <- render, gui. A test
//! (`tests/layering.rs`) enforces it.

pub mod contour;
pub mod geometry;
pub mod mesh;
pub mod view;
pub mod volume;

pub use contour::{ContourData, ContourLoop, ContourPoint, ContourSlice};
pub use geometry::{
    GeometryIdentity, OrthogonalFamily, PlaneDefinition, PlaneFamily, VoxelData, VoxelGeometry,
    VoxelGeometryError,
};
pub use mesh::{MeshData, MeshFace, MeshVertex};
pub use view::ViewMode;
pub use volume::{LoadedLabel, VolumeData};
