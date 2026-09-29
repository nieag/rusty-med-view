//! Shared mapping between voxel indices, normalized volume UV, and slices.
//!
//! Convention: cell-centered, edge-to-edge (see `docs/adr/0003-roi-owned-spatial-metadata.md`).
//!
//! - Continuous index coordinates put voxel centres at integers; voxel `i` covers
//!   `[i - 0.5, i + 0.5]`, so a volume spans `[-0.5, dim - 0.5]` and `dim * spacing` millimetres.
//! - Normalized volume UV is `(index + 0.5) / dim`. UV 0 and 1 are the outer voxel faces, which is
//!   the GPU texture-coordinate convention, so image sampling needs no correction.
//! - Voxel `k` has its centre at UV `(k + 0.5) / dim`, and the voxel containing a UV is
//!   `floor(uv * dim)`.
//!
//! A physical point keeps the same UV when a volume is resampled to another resolution over the
//! same field of view, which node-based (`index / (dim - 1)`) coordinates do not guarantee.
//! Every CPU and shader conversion between index space and UV must go through this module.

/// Continuous voxel index (centres at integers) for a normalized volume UV.
pub fn volume_uv_to_voxel_index(uv: [f32; 3], dimensions: [u32; 3]) -> [f32; 3] {
    let mut index = [0.0; 3];
    for axis in 0..3 {
        if dimensions[axis] > 0 {
            index[axis] = uv[axis] * dimensions[axis] as f32 - 0.5;
        }
    }
    index
}

/// Normalized volume UV for a continuous voxel index (centres at integers).
pub fn voxel_index_to_volume_uv(index: [f32; 3], dimensions: [u32; 3]) -> [f32; 3] {
    let mut uv = [0.0; 3];
    for axis in 0..3 {
        if dimensions[axis] > 0 {
            uv[axis] = (index[axis] + 0.5) / dimensions[axis] as f32;
        }
    }
    uv
}

/// Voxel that contains a normalized volume UV, clamped into the volume.
pub fn sample_index_from_volume_uv(uv: [f32; 3], dimensions: [u32; 3]) -> [u32; 3] {
    let mut sample = [0; 3];
    for axis in 0..3 {
        let dim = dimensions[axis];
        if dim == 0 {
            continue;
        }
        sample[axis] = (uv[axis] * dim as f32).floor().clamp(0.0, (dim - 1) as f32) as u32;
    }
    sample
}

/// Slice index containing a cursor UV, clamped into the volume.
pub fn slice_index_from_cursor_uv(cursor_uv: f32, dim: u32) -> i32 {
    if dim == 0 {
        return 0;
    }
    (cursor_uv * dim as f32)
        .floor()
        .clamp(0.0, (dim - 1) as f32) as i32
}

/// Cursor UV at the centre of a slice.
pub fn slice_center_uv(slice_index: i32, dim: u32) -> f32 {
    if dim == 0 {
        return 0.0;
    }
    let clamped = slice_index.clamp(0, dim as i32 - 1) as f32;
    (clamped + 0.5) / dim as f32
}

/// Cursor UV at the centre voxel of every axis, so slice planes start on voxel centres rather than
/// on a layer boundary (UV 0.5 lies exactly between two layers on even-sized axes).
pub fn centered_cursor_uv(dimensions: [u32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| slice_center_uv((dimensions[axis] / 2) as i32, dimensions[axis]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slice_index_from_cursor_uv_clamps() {
        assert_eq!(slice_index_from_cursor_uv(-1.0, 128), 0);
        assert_eq!(slice_index_from_cursor_uv(2.0, 128), 127);
        assert_eq!(slice_index_from_cursor_uv(0.0, 128), 0);
        assert_eq!(slice_index_from_cursor_uv(1.0, 128), 127);
        assert_eq!(slice_index_from_cursor_uv(0.5, 0), 0);
    }

    #[test]
    fn test_slice_center_roundtrips_for_every_slice() {
        for dim in [1_u32, 2, 3, 4, 125, 180] {
            for slice in 0..dim as i32 {
                let uv = slice_center_uv(slice, dim);

                assert_eq!(slice_index_from_cursor_uv(uv, dim), slice, "dim {dim}");
                assert!((0.0..=1.0).contains(&uv));
            }
        }
    }

    #[test]
    fn test_voxel_centres_sit_half_a_cell_inside_the_volume_faces() {
        let dims = [4, 8, 10];

        assert_eq!(
            voxel_index_to_volume_uv([0.0; 3], dims),
            [0.125, 0.0625, 0.05]
        );
        assert_eq!(voxel_index_to_volume_uv([-0.5; 3], dims), [0.0; 3]);
        assert_eq!(
            voxel_index_to_volume_uv([3.5, 7.5, 9.5], dims),
            [1.0, 1.0, 1.0]
        );
    }

    #[test]
    fn test_index_and_uv_are_inverse_mappings() {
        let dims = [180, 180, 125];
        for index in [[0.0, 0.0, 0.0], [89.5, 12.25, 124.0], [-0.5, 179.5, 62.0]] {
            let back = volume_uv_to_voxel_index(voxel_index_to_volume_uv(index, dims), dims);

            for axis in 0..3 {
                assert!((back[axis] - index[axis]).abs() < 1e-4);
            }
        }
    }

    #[test]
    fn test_uv_selects_the_same_cell_the_gpu_texture_would_sample() {
        // Nearest-texel sampling picks floor(uv * dim); the CPU sample index must agree so that
        // image, overlays, picking, and contours refer to the same voxel at every UV.
        let dims = [180_u32, 96, 125];
        for step in 0..=1000 {
            let uv = step as f32 / 1000.0;
            let sample = sample_index_from_volume_uv([uv; 3], dims);
            for axis in 0..3 {
                let texel = ((uv * dims[axis] as f32).floor() as u32).min(dims[axis] - 1);
                assert_eq!(sample[axis], texel, "axis {axis}, uv {uv}");
            }
        }
    }

    #[test]
    fn test_physical_point_keeps_its_uv_when_resampled_to_finer_resolution() {
        // Same field of view sampled with 2x the voxels: a physical position must not move.
        let coarse = [90_u32, 90, 60];
        let fine = [180_u32, 180, 120];
        let uv = [0.37, 0.62, 0.81];
        let coarse_index = volume_uv_to_voxel_index(uv, coarse);
        let fine_index = volume_uv_to_voxel_index(uv, fine);

        for axis in 0..3 {
            // Index scales with resolution about the shared outer face at -0.5.
            let coarse_from_face = coarse_index[axis] + 0.5;
            let fine_from_face = fine_index[axis] + 0.5;
            assert!((fine_from_face - 2.0 * coarse_from_face).abs() < 1e-3);
        }
    }

    #[test]
    fn test_sample_index_from_volume_uv_preserves_floor_and_clamp_behavior() {
        let sample = sample_index_from_volume_uv([1.2, -0.1, 0.999], [4, 4, 4]);
        assert_eq!(sample, [3, 0, 3]);

        let sample_with_zero_dim = sample_index_from_volume_uv([0.5, 0.5, 0.5], [0, 1, 2]);
        assert_eq!(sample_with_zero_dim, [0, 0, 1]);
    }

    #[test]
    fn test_sample_index_uses_voxel_cell_boundaries() {
        let dimensions = [5, 1, 1];
        // Index 0.51 lies inside voxel 1's cell: [0.5, 1.5).
        let uv = voxel_index_to_volume_uv([0.51, 0.0, 0.0], dimensions);

        assert_eq!(sample_index_from_volume_uv(uv, dimensions), [1, 0, 0]);
    }

    #[test]
    fn test_centered_cursor_lies_on_a_voxel_centre_for_even_and_odd_axes() {
        let dims = [180_u32, 125, 4];
        let uv = centered_cursor_uv(dims);

        for axis in 0..3 {
            let index = volume_uv_to_voxel_index(uv, dims)[axis];

            assert!(
                (index - index.round()).abs() < 1e-4,
                "axis {axis} cursor index {index} is not a voxel centre"
            );
        }
        assert_eq!(centered_cursor_uv([0, 1, 2])[0], 0.0);
    }

    /// Mirrors `get_overlay_color` in `src/shaders/shader.wgsl`: the overlay path turns the
    /// cell-centred UV back into a continuous index and rounds to the nearest cell. Keep the two
    /// in sync; this test fails if the CPU convention and that arithmetic ever disagree with the
    /// texel the image sampler fetches.
    fn shader_overlay_cell(uvw: f32, dim: u32) -> i32 {
        let index = uvw * dim as f32 - 0.5;
        (index + 0.5).floor() as i32
    }

    #[test]
    fn test_overlay_cell_matches_image_texel_and_cpu_sample_index_at_every_uv() {
        for dim in [1_u32, 2, 7, 125, 180] {
            for step in 0..=2000 {
                let uv = step as f32 / 2000.0;
                let texel = ((uv * dim as f32).floor() as i32).min(dim as i32 - 1);
                let overlay = shader_overlay_cell(uv, dim).min(dim as i32 - 1);
                let cpu = sample_index_from_volume_uv([uv; 3], [dim; 3])[0] as i32;

                assert_eq!(overlay, texel, "dim {dim}, uv {uv}: overlay vs image texel");
                assert_eq!(cpu, texel, "dim {dim}, uv {uv}: CPU vs image texel");
            }
        }
    }
}
