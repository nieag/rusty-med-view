//! A synthetic many-label case for scale checks (backlog 2b.10).
//!
//! The count and the sizes are real even though the shapes are not: a labelmap on a large grid
//! whose labels are copies of one real mask (clipped to its cell of a regular grid of cells).
//! It covers the same world extent as the template, so it lines up with the image it came from.

use crate::model::{LoadedLabel, VoxelGeometry};
use glam::{DMat4, DVec3};

/// A labelmap of `dimensions` voxels with `count` labels (1 to 255), each the bounding box of
/// `template`'s non-zero voxels copied into its own cell.
pub fn tiled_label_map(
    template: &LoadedLabel,
    dimensions: [u32; 3],
    count: usize,
) -> Result<LoadedLabel, String> {
    if count == 0 || count > 255 {
        return Err(format!("{count} labels do not fit an 8-bit labelmap"));
    }
    let source = template.geometry.dimensions().map(|value| value as usize);
    let at = |x: usize, y: usize, z: usize| template.data[(z * source[1] + y) * source[0] + x] != 0;
    let mut min = [usize::MAX; 3];
    let mut max = [0usize; 3];
    for z in 0..source[2] {
        for y in 0..source[1] {
            for x in 0..source[0] {
                if at(x, y, z) {
                    for (axis, coordinate) in [x, y, z].into_iter().enumerate() {
                        min[axis] = min[axis].min(coordinate);
                        max[axis] = max[axis].max(coordinate);
                    }
                }
            }
        }
    }
    if min[0] == usize::MAX {
        return Err("the template labelmap is empty".to_string());
    }

    let [width, height, depth] = dimensions.map(|value| value as usize);
    let grid = [
        count.div_ceil(25).max(1),
        5.min(count),
        5.min(count.div_ceil(5)),
    ];
    let cell = [width / grid[0], height / grid[1], depth / grid[2]];
    let mut data = vec![0_u8; width * height * depth];
    for index in 0..count {
        let cell_index = [
            index % grid[0],
            (index / grid[0]) % grid[1],
            index / (grid[0] * grid[1]),
        ];
        let label = (index + 1) as u8;
        for dz in 0..cell[2].min(max[2] - min[2] + 1) {
            for dy in 0..cell[1].min(max[1] - min[1] + 1) {
                for dx in 0..cell[0].min(max[0] - min[0] + 1) {
                    if at(min[0] + dx, min[1] + dy, min[2] + dz) {
                        let x = cell_index[0] * cell[0] + dx;
                        let y = cell_index[1] * cell[1] + dy;
                        let z = cell_index[2] * cell[2] + dz;
                        data[(z * height + y) * width + x] = label;
                    }
                }
            }
        }
    }

    // New voxel i' sits where old voxel s * i' + (s - 1) / 2 sat, with s = old / new per axis.
    let scale = DVec3::new(
        source[0] as f64 / width as f64,
        source[1] as f64 / height as f64,
        source[2] as f64 / depth as f64,
    );
    let resample =
        DMat4::from_scale(scale) * DMat4::from_translation((scale - DVec3::ONE) * 0.5 / scale);
    let geometry = VoxelGeometry::from_affine(
        dimensions,
        template.geometry.ijk_to_world_affine() * resample,
    )
    .map_err(|error| error.to_string())?;
    Ok(LoadedLabel {
        dimensions,
        geometry,
        data,
        filename: format!("synthetic_{count}.nii"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template() -> LoadedLabel {
        let mut data = vec![0_u8; 10 * 8 * 6];
        for z in 1..4 {
            for y in 2..6 {
                for x in 3..8 {
                    data[(z * 8 + y) * 10 + x] = 1;
                }
            }
        }
        LoadedLabel {
            dimensions: [10, 8, 6],
            geometry: VoxelGeometry::new(
                [10, 8, 6],
                [2.0, 2.0, 3.0],
                [5.0, -7.0, 11.0],
                [0.0, 0.0, 0.0, 1.0],
            )
            .unwrap(),
            data,
            filename: "t.nii".to_string(),
        }
    }

    #[test]
    fn test_the_tiled_map_has_the_labels_and_covers_the_templates_world_extent() {
        let template = template();
        let map = tiled_label_map(&template, [40, 30, 20], 12).unwrap();

        let mut labels: Vec<u8> = map.data.iter().copied().filter(|v| *v != 0).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels, (1..=12).collect::<Vec<u8>>());
        // Each label is the 5 x 4 x 3 template box, clipped to its cell if the cell is smaller.
        let count = |label: u8| map.data.iter().filter(|v| **v == label).count();
        assert!((1..=12).all(|label| count(label) > 0 && count(label) <= 5 * 4 * 3));

        // The outer corners of the grid are the same places in the world.
        for corner in [[-0.5, -0.5, -0.5], [9.5, 7.5, 5.5]] {
            let new_corner = [
                (corner[0] + 0.5) * 4.0 - 0.5,
                (corner[1] + 0.5) * 30.0 / 8.0 - 0.5,
                (corner[2] + 0.5) * 20.0 / 6.0 - 0.5,
            ];
            let old = template.geometry.ijk_to_world_mm(corner.map(f64::from));
            let new = map.geometry.ijk_to_world_mm(new_corner.map(f64::from));
            let error = (0..3).map(|a| (old[a] - new[a]).abs()).fold(0.0, f64::max);
            assert!(error < 1e-6, "{old:?} vs {new:?}");
        }
    }

    #[test]
    fn test_the_tiled_map_rejects_bad_requests() {
        assert!(tiled_label_map(&template(), [10, 10, 10], 0).is_err());
        assert!(tiled_label_map(&template(), [10, 10, 10], 256).is_err());
    }
}
