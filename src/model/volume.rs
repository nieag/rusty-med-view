//! The loaded image volume and imported labelmaps, as plain data.

use super::geometry::VoxelGeometry;

pub struct VolumeData {
    pub dimensions: [u32; 3],
    /// Validated grid with the full IJK-to-world affine; `None` for the empty placeholder volume.
    pub geometry: Option<VoxelGeometry>,
    pub intensities: Vec<f32>,
    pub intensity_range: [f32; 2],
}

impl VolumeData {
    /// Millimetre voxel size per axis, or unit spacing for the empty placeholder volume.
    pub fn spacing(&self) -> [f32; 3] {
        self.geometry.map_or([1.0; 3], VoxelGeometry::spacing)
    }

    /// Proper-rotation view of the grid axes (identity for the empty placeholder volume).
    pub fn orientation(&self) -> [f32; 4] {
        self.geometry
            .map_or([0.0, 0.0, 0.0, 1.0], VoxelGeometry::orientation)
    }

    pub fn aspect_ratios(&self) -> [f32; 3] {
        let d = self.dimensions;
        let s = self.spacing();
        // avoid div by zero if empty
        if d[0] == 0 || d[1] == 0 || d[2] == 0 {
            return [1.0, 1.0, 1.0];
        }
        let max_dim = (d[0] as f32 * s[0])
            .max(d[1] as f32 * s[1])
            .max(d[2] as f32 * s[2]);
        [
            (d[0] as f32 * s[0]) / max_dim,
            (d[1] as f32 * s[1]) / max_dim,
            (d[2] as f32 * s[2]) / max_dim,
        ]
    }
}

#[derive(Debug)]
pub struct LoadedLabel {
    pub dimensions: [u32; 3],
    /// Validated grid with the full IJK-to-world affine from the NIfTI sform or qform.
    pub geometry: VoxelGeometry,
    pub data: Vec<u8>,
    pub filename: String,
}
