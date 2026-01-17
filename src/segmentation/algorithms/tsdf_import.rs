use crate::app::components::LabelmapData;
use crate::segmentation::tsdf::ChunkedTSDF;
use std::collections::VecDeque;

/// Converts a voxel-based labelmap into a ChunkedTSDF.
/// This implementation uses a 3D distance transform that accounts for voxel spacing.
pub fn voxel_to_tsdf(labelmap: &LabelmapData, spacing: [f32; 3], truncation: f32) -> ChunkedTSDF {
    let mut tsdf = ChunkedTSDF::new(truncation);
    let dims = labelmap.dimensions;
    let data = &labelmap.raw_data;

    let total_voxels = (dims[0] * dims[1] * dims[2]) as usize;
    let mut distances = vec![f32::MAX; total_voxels];

    let get_idx = |x: u32, y: u32, z: u32| (z * dims[0] * dims[1] + y * dims[0] + x) as usize;

    let neighbors = [
        (1, 0, 0),
        (-1, 0, 0),
        (0, 1, 0),
        (0, -1, 0),
        (0, 0, 1),
        (0, 0, -1),
    ];

    let mut queue = VecDeque::new();

    // 1. Identify boundary voxels
    // We initialize voxels with label > 0 that have a neighbor with label == 0 as 0.0.
    // This makes the "center" of the boundary voxel the 0-distance point.
    for z in 0..dims[2] {
        for y in 0..dims[1] {
            for x in 0..dims[0] {
                let idx = get_idx(x, y, z);
                let label = data[idx];
                if label == 0 {
                    continue;
                }

                let mut is_boundary = false;
                for (dx, dy, dz) in neighbors {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    let nz = z as i32 + dz;

                    if nx >= 0
                        && nx < dims[0] as i32
                        && ny >= 0
                        && ny < dims[1] as i32
                        && nz >= 0
                        && nz < dims[2] as i32
                    {
                        let n_idx = get_idx(nx as u32, ny as u32, nz as u32);
                        if data[n_idx] == 0 {
                            is_boundary = true;
                            break;
                        }
                    } else {
                        // Volume boundary counts as interface if label is > 0
                        is_boundary = true;
                        break;
                    }
                }

                if is_boundary {
                    distances[idx] = 0.0;
                    queue.push_back((x as i32, y as i32, z as i32));
                }
            }
        }
    }

    // 2. Propagation with physical spacing
    while let Some((x, y, z)) = queue.pop_front() {
        let current_dist = distances[get_idx(x as u32, y as u32, z as u32)];

        for (dx, dy, dz) in neighbors {
            let nx = x + dx;
            let ny = y + dy;
            let nz = z + dz;

            if nx >= 0
                && nx < dims[0] as i32
                && ny >= 0
                && ny < dims[1] as i32
                && nz >= 0
                && nz < dims[2] as i32
            {
                let n_idx = get_idx(nx as u32, ny as u32, nz as u32);

                // Euclidean step based on axis
                let step = if dx != 0 {
                    spacing[0]
                } else if dy != 0 {
                    spacing[1]
                } else {
                    spacing[2]
                };

                let new_dist = current_dist + step;
                if new_dist < distances[n_idx] {
                    distances[n_idx] = new_dist;
                    queue.push_back((nx, ny, nz));
                }
            }
        }
    }

    // 3. Populate TSDF with sign
    for z in 0..dims[2] {
        for y in 0..dims[1] {
            for x in 0..dims[0] {
                let idx = get_idx(x, y, z);
                let label = data[idx];
                let dist = distances[idx];

                let signed_dist = if label > 0 { -dist } else { dist };
                if signed_dist.abs() <= truncation {
                    tsdf.set_distance(x as i32, y as i32, z as i32, signed_dist);
                } else if signed_dist < 0.0 {
                    tsdf.set_distance(x as i32, y as i32, z as i32, -truncation);
                }
            }
        }
    }
    tsdf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_voxel_to_tsdf_simple() {
        let dims = [10, 10, 10];
        let mut raw_data = vec![0u8; 1000];
        // 4x4x4 cube in the middle
        for z in 3..7 {
            for y in 3..7 {
                for x in 3..7 {
                    raw_data[(z * 100 + y * 10 + x) as usize] = 1;
                }
            }
        }
        let labelmap = LabelmapData {
            dimensions: dims,
            raw_data,
        };
        let spacing = [1.0, 1.0, 1.0];
        let tsdf = voxel_to_tsdf(&labelmap, spacing, 4.0);

        // Center should be inside (negative)
        assert!(tsdf.get_distance(5, 5, 5) < 0.0);
        // Corner should be outside (positive)
        assert!(tsdf.get_distance(0, 0, 0) > 0.0);
        // Boundary should be near 0
        assert!(tsdf.get_distance(3, 5, 5).abs() < 1.0);
    }

    #[test]
    fn test_voxel_to_tsdf_anisotropic() {
        let dims = [10, 10, 10];
        let mut raw_data = vec![0u8; 1000];
        // Single voxel at center
        raw_data[555] = 1;
        let labelmap = LabelmapData {
            dimensions: dims,
            raw_data,
        };
        // Spacing: 1x, 2y, 4z
        let spacing = [1.0, 2.0, 4.0];
        let tsdf = voxel_to_tsdf(&labelmap, spacing, 10.0);

        // x neighbor (1.0 away)
        let dx = tsdf.get_distance(6, 5, 5);
        // y neighbor (2.0 away)
        let dy = tsdf.get_distance(5, 6, 5);
        // z neighbor (4.0 away)
        let dz = tsdf.get_distance(5, 5, 6);

        assert!(
            (dx - 1.0).abs() < 0.1,
            "X distance should be ~1.0, got {}",
            dx
        );
        assert!(
            (dy - 2.0).abs() < 0.1,
            "Y distance should be ~2.0, got {}",
            dy
        );
        assert!(
            (dz - 4.0).abs() < 0.1,
            "Z distance should be ~4.0, got {}",
            dz
        );
    }
}
