//! How a viewport looks at the volume.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    ThreeD = 0,
    Axial = 1,
    Coronal = 2,
    Sagittal = 3,
    Oblique = 4,
}
