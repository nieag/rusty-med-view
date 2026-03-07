//! Labelmap → TSDF chunk conversion via separable 3-D Euclidean Distance Transform.
//!
//! Builds [`TsdfChunk`] maps directly from binary voxel masks, bypassing the
//! Contours → SDF → TSDF intermediate pipeline for voxel-driven segments.
//! Only chunks near the label boundary are allocated (sparse representation).
//!
//! Grid metadata (`tsdf_dims`, `tsdf_spacing`, `tsdf_origin`) returned alongside
//! the chunk map aligns 1:1 with the input labelmap and is stored verbatim in
//! [`SegmentRuntimeCache`].

use crate::app::segment::ChunkKey;
use crate::convert::chunk_grid::{chunk_bounds_for_key, chunk_keys_for_bounds};
use crate::convert::contour_to_tsdf_chunks::{quantize_tsdf, TsdfChunk};
use std::collections::HashMap;

/// Build TSDF chunks directly from a binary labelmap using a separable 3-D EDT.
///
/// Returns `(chunks, tsdf_dims, tsdf_spacing, tsdf_origin)`.
///
/// - `label == 0` treats every non-zero voxel as foreground.
/// - Chunks whose entire padded region lies outside the truncation band are
///   skipped (sparse interior/exterior chunks produce no allocation).
/// - Each allocated chunk carries +1 voxel padding on all sides, matching the
///   convention in `segment_system.rs` so Surface Nets can stitch across boundaries.
pub fn build_tsdf_chunks_from_labelmap(
    mask: &[u8],
    dims: [u32; 3],
    spacing: [f32; 3],
    label: u8,
    chunk_size: u32,
    truncation_mm: f32,
) -> (HashMap<ChunkKey, TsdfChunk>, [u32; 3], [f32; 3], [f32; 3]) {
    let mut chunks = HashMap::new();
    let origin = [0.0f32; 3];

    if dims.contains(&0) || mask.is_empty() || truncation_mm <= 0.0 {
        return (chunks, dims, spacing, origin);
    }

    // ---- 1. Bounding box of the foreground label. ----
    let (Some(fg_min), Some(fg_max)) = fg_bounding_box(mask, dims, label) else {
        return (chunks, dims, spacing, origin);
    };

    // Truncation in voxels per axis (overshoot by 1 to be safe near faces).
    let trunc_vox = [
        (truncation_mm / spacing[0].max(1e-6)).ceil() as u32 + 1,
        (truncation_mm / spacing[1].max(1e-6)).ceil() as u32 + 1,
        (truncation_mm / spacing[2].max(1e-6)).ceil() as u32 + 1,
    ];

    // ---- 2. Select chunks that intersect the narrow band. ----
    let roi = [
        fg_min[0].saturating_sub(trunc_vox[0]),
        fg_min[1].saturating_sub(trunc_vox[1]),
        fg_min[2].saturating_sub(trunc_vox[2]),
        (fg_max[0] + trunc_vox[0]).min(dims[0].saturating_sub(1)),
        (fg_max[1] + trunc_vox[1]).min(dims[1].saturating_sub(1)),
        (fg_max[2] + trunc_vox[2]).min(dims[2].saturating_sub(1)),
    ];

    let cs = chunk_size.max(4);
    for key in chunk_keys_for_bounds(roi, cs) {
        let Some(cb) = chunk_bounds_for_key(key, cs, dims) else {
            continue;
        };

        // +1 voxel padding on all sides (matches segment_system.rs convention).
        let padded = [
            cb[0].saturating_sub(1),
            cb[1].saturating_sub(1),
            cb[2].saturating_sub(1),
            (cb[3] + 1).min(dims[0].saturating_sub(1)),
            (cb[4] + 1).min(dims[1].saturating_sub(1)),
            (cb[5] + 1).min(dims[2].saturating_sub(1)),
        ];

        // Extended region for EDT accuracy: padded bounds + full truncation band.
        // Every voxel in `padded` will see seeds within `truncation_mm`.
        let ext = [
            padded[0].saturating_sub(trunc_vox[0]),
            padded[1].saturating_sub(trunc_vox[1]),
            padded[2].saturating_sub(trunc_vox[2]),
            (padded[3] + trunc_vox[0]).min(dims[0].saturating_sub(1)),
            (padded[4] + trunc_vox[1]).min(dims[1].saturating_sub(1)),
            (padded[5] + trunc_vox[2]).min(dims[2].saturating_sub(1)),
        ];

        let enx = (ext[3] - ext[0] + 1) as usize;
        let eny = (ext[4] - ext[1] + 1) as usize;
        let enz = (ext[5] - ext[2] + 1) as usize;

        // ---- 3. Extract inside/outside mask for the extended region. ----
        let mut inside = vec![false; enx * eny * enz];
        for ez in 0..enz {
            for ey in 0..eny {
                for ex in 0..enx {
                    let gx = ext[0] as usize + ex;
                    let gy = ext[1] as usize + ey;
                    let gz = ext[2] as usize + ez;
                    let mi = gx + gy * dims[0] as usize + gz * (dims[0] * dims[1]) as usize;
                    let v = mask[mi];
                    inside[ex + ey * enx + ez * enx * eny] =
                        if label == 0 { v != 0 } else { v == label };
                }
            }
        }

        // ---- 4. Separable EDT: squared distances to nearest outside and inside. ----
        let outside_seeds: Vec<bool> = inside.iter().map(|b| !b).collect();
        let edt_bg_sq = edt_3d_sq(&outside_seeds, [enx, eny, enz], spacing);
        let edt_fg_sq = edt_3d_sq(&inside, [enx, eny, enz], spacing);

        // ---- 5. Build TsdfChunk from the padded sub-region. ----
        let px0 = (padded[0] - ext[0]) as usize;
        let py0 = (padded[1] - ext[1]) as usize;
        let pz0 = (padded[2] - ext[2]) as usize;
        let cnx = (padded[3] - padded[0] + 1) as usize;
        let cny = (padded[4] - padded[1] + 1) as usize;
        let cnz = (padded[5] - padded[2] + 1) as usize;

        let mut values = Vec::with_capacity(cnx * cny * cnz);
        let mut weight = Vec::with_capacity(cnx * cny * cnz);
        let mut has_valid = false;

        for z in 0..cnz {
            for y in 0..cny {
                for x in 0..cnx {
                    let ei = (px0 + x) + (py0 + y) * enx + (pz0 + z) * enx * eny;

                    let is_in = inside[ei];
                    let (sq, w) = if is_in {
                        let d2 = edt_bg_sq[ei];
                        (d2, u8::from(d2 < f32::MAX))
                    } else {
                        let d2 = edt_fg_sq[ei];
                        (d2, u8::from(d2 < f32::MAX))
                    };

                    let dist_mm = if sq < f32::MAX {
                        sq.sqrt()
                    } else {
                        truncation_mm
                    };
                    let sdf = (if is_in { -dist_mm } else { dist_mm })
                        .clamp(-truncation_mm, truncation_mm);

                    values.push(quantize_tsdf(sdf, truncation_mm));
                    weight.push(w);
                    if w > 0 {
                        has_valid = true;
                    }
                }
            }
        }

        // Skip pure-interior / pure-exterior chunks (no mesh contribution).
        if !has_valid {
            continue;
        }

        chunks.insert(
            key,
            TsdfChunk {
                dims: [cnx as u32, cny as u32, cnz as u32],
                voxel_size: spacing,
                origin: [
                    padded[0] as f32 * spacing[0],
                    padded[1] as f32 * spacing[1],
                    padded[2] as f32 * spacing[2],
                ],
                truncation_mm,
                values,
                weight,
                revision: 1,
            },
        );
    }

    (chunks, dims, spacing, origin)
}

// ============================================================================
// Bounding box helper
// ============================================================================

/// Returns `(Some(min), Some(max))` in voxel indices for the foreground label,
/// or `(None, None)` if no matching voxel exists.
fn fg_bounding_box(mask: &[u8], dims: [u32; 3], label: u8) -> (Option<[u32; 3]>, Option<[u32; 3]>) {
    let mut lo = [u32::MAX; 3];
    let mut hi = [0u32; 3];
    let mut found = false;

    for z in 0..dims[2] {
        for y in 0..dims[1] {
            for x in 0..dims[0] {
                let i = (x + y * dims[0] + z * dims[0] * dims[1]) as usize;
                let is_fg = if label == 0 {
                    mask[i] != 0
                } else {
                    mask[i] == label
                };
                if is_fg {
                    lo[0] = lo[0].min(x);
                    lo[1] = lo[1].min(y);
                    lo[2] = lo[2].min(z);
                    hi[0] = hi[0].max(x);
                    hi[1] = hi[1].max(y);
                    hi[2] = hi[2].max(z);
                    found = true;
                }
            }
        }
    }

    if found {
        (Some(lo), Some(hi))
    } else {
        (None, None)
    }
}

// ============================================================================
// Separable 3-D Euclidean Distance Transform (Felzenszwalb & Huttenlocher)
// ============================================================================

/// 3-D squared Euclidean Distance Transform.
///
/// `seeds[i]` = `true` marks voxels from which distances are measured.
/// Returns squared distances (mm²) in `x + y*nx + z*nx*ny` layout.
/// Positions unreachable from any seed (e.g. all-false extended regions)
/// return `f32::MAX`.
fn edt_3d_sq(seeds: &[bool], dims: [usize; 3], spacing: [f32; 3]) -> Vec<f32> {
    let [nx, ny, nz] = dims;
    let n = nx * ny * nz;

    // ---- Phase 1: 1-D EDT along X for every (y, z) row. ----
    let mut g = vec![f32::MAX; n];
    for z in 0..nz {
        for y in 0..ny {
            let base = z * ny * nx + y * nx;
            let row = &seeds[base..base + nx];
            let sp = spacing[0];

            let mut last: i32 = -1;
            for x in 0..nx {
                if row[x] {
                    g[base + x] = 0.0;
                    last = x as i32;
                } else if last >= 0 {
                    let d = (x as i32 - last) as f32 * sp;
                    g[base + x] = d * d;
                }
            }
            let mut next = nx as i32;
            for x in (0..nx).rev() {
                if row[x] {
                    next = x as i32;
                } else if next < nx as i32 {
                    let d = (next - x as i32) as f32 * sp;
                    let d2 = d * d;
                    if d2 < g[base + x] {
                        g[base + x] = d2;
                    }
                }
            }
        }
    }

    // ---- Phase 2: combine with Y for every (x, z) column. ----
    let mut h = vec![f32::MAX; n];
    let mut col = vec![f32::MAX; ny.max(1)];
    for z in 0..nz {
        for x in 0..nx {
            for y in 0..ny {
                col[y] = g[z * ny * nx + y * nx + x];
            }
            let dt = edt1d_combine(&col, spacing[1]);
            for y in 0..ny {
                h[z * ny * nx + y * nx + x] = dt[y];
            }
        }
    }

    // ---- Phase 3: combine with Z for every (x, y) pillar. ----
    let mut result = vec![f32::MAX; n];
    let mut pillar = vec![f32::MAX; nz.max(1)];
    for y in 0..ny {
        for x in 0..nx {
            for z in 0..nz {
                pillar[z] = h[z * ny * nx + y * nx + x];
            }
            let dt = edt1d_combine(&pillar, spacing[2]);
            for z in 0..nz {
                result[z * ny * nx + y * nx + x] = dt[z];
            }
        }
    }

    result
}

/// 1-D EDT combination step (Felzenszwalb & Huttenlocher lower-envelope algorithm).
///
/// Given `g[s]` = accumulated squared distances from the previous phase,
/// computes `dt[q] = min_s { g[s] + (q − s)² · sp² }` for every `q`.
/// Positions with `g[s] == f32::MAX` contribute no parabola (no seed there).
/// O(n).
// `q` is used both as an index AND as a numeric value in arithmetic — can't replace with enumerate.
#[allow(clippy::needless_range_loop)]
fn edt1d_combine(g: &[f32], sp: f32) -> Vec<f32> {
    let n = g.len();
    if n == 0 {
        return vec![];
    }
    let sp2 = sp * sp;
    let mut dt = vec![f32::MAX; n];

    // Find the first finite entry — if none, the whole column has no seeds.
    let Some(first) = g.iter().position(|v| *v < f32::MAX) else {
        return dt;
    };

    // Parabola lower-envelope stacks.
    let mut v = vec![0usize; n]; // center positions
    let mut z = vec![f32::NEG_INFINITY; n + 1]; // transition boundaries
    v[0] = first;
    z[0] = f32::NEG_INFINITY;
    z[1] = f32::INFINITY;
    let mut k = 0usize;

    // Intersection of the parabola at `si` and the one at `ui`.
    // sep(si, ui) = (g[ui] - g[si] + sp² · (ui² - si²)) / (2·sp²·(ui - si))
    let sep = |si: usize, ui: usize| -> f32 {
        let s = si as f32;
        let u = ui as f32;
        (g[ui] - g[si] + sp2 * (u * u - s * s)) / (2.0 * sp2 * (u - s))
    };

    for q in (first + 1)..n {
        if g[q] >= f32::MAX {
            continue;
        }
        let mut s = sep(v[k], q);
        while k > 0 && s <= z[k] {
            k -= 1;
            s = sep(v[k], q);
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f32::INFINITY;
    }

    // Scan left-to-right and assign minimum-parabola value.
    let mut j = 0usize;
    for q in 0..n {
        while z[j + 1] < q as f32 {
            j += 1;
        }
        let vj = v[j];
        if g[vj] < f32::MAX {
            let diff = (q as f32 - vj as f32) * sp;
            dt[q] = g[vj] + diff * diff;
        }
    }

    dt
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sphere_mask(dims: [u32; 3], radius: f32, label: u8) -> Vec<u8> {
        let cx = (dims[0] as f32 - 1.0) * 0.5;
        let cy = (dims[1] as f32 - 1.0) * 0.5;
        let cz = (dims[2] as f32 - 1.0) * 0.5;
        let mut mask = vec![0u8; (dims[0] * dims[1] * dims[2]) as usize];
        for z in 0..dims[2] {
            for y in 0..dims[1] {
                for x in 0..dims[0] {
                    let d = ((x as f32 - cx).powi(2)
                        + (y as f32 - cy).powi(2)
                        + (z as f32 - cz).powi(2))
                    .sqrt();
                    if d <= radius {
                        mask[(x + y * dims[0] + z * dims[0] * dims[1]) as usize] = label;
                    }
                }
            }
        }
        mask
    }

    // ---- EDT unit tests ----

    #[test]
    fn test_edt1d_combine_two_seeds() {
        // Seeds at positions 1 and 4; spacing = 1.0.
        let g = [f32::MAX, 0.0, f32::MAX, f32::MAX, 0.0];
        let dt = edt1d_combine(&g, 1.0);
        assert!((dt[0] - 1.0).abs() < 1e-4, "dt[0]={}", dt[0]);
        assert!((dt[1] - 0.0).abs() < 1e-4, "dt[1]={}", dt[1]);
        assert!((dt[2] - 1.0).abs() < 1e-4, "dt[2]={}", dt[2]);
        assert!((dt[3] - 1.0).abs() < 1e-4, "dt[3]={}", dt[3]);
        assert!((dt[4] - 0.0).abs() < 1e-4, "dt[4]={}", dt[4]);
    }

    #[test]
    fn test_edt1d_combine_spacing_scales_distance() {
        // Single seed at position 0, spacing = 2.0 mm.
        let g = [0.0, f32::MAX, f32::MAX];
        let dt = edt1d_combine(&g, 2.0);
        // dt[1] = (1 * 2.0)^2 = 4.0, dt[2] = (2 * 2.0)^2 = 16.0
        assert!((dt[0] - 0.0).abs() < 1e-4);
        assert!((dt[1] - 4.0).abs() < 1e-4, "dt[1]={}", dt[1]);
        assert!((dt[2] - 16.0).abs() < 1e-4, "dt[2]={}", dt[2]);
    }

    #[test]
    fn test_edt_3d_single_seed_distances() {
        // 3x3x3 volume, single seed at corner (0,0,0).
        let mut seeds = vec![false; 27];
        seeds[0] = true; // (0,0,0)
        let sp = [1.0f32; 3];
        let sq = edt_3d_sq(&seeds, [3, 3, 3], sp);
        // (0,0,0) → 0, (1,0,0) → 1, (1,1,0) → 2, (1,1,1) → 3
        assert!((sq[0] - 0.0).abs() < 1e-4); // (0,0,0)
        assert!((sq[1] - 1.0).abs() < 1e-4); // (1,0,0)
        let idx_110 = 1 + 1 * 3; // x=1, y=1, z=0
        assert!(
            (sq[idx_110] - 2.0).abs() < 1e-4,
            "sq[1,1,0]={}",
            sq[idx_110]
        );
        let idx_111 = 1 + 1 * 3 + 1 * 9; // x=1, y=1, z=1
        assert!(
            (sq[idx_111] - 3.0).abs() < 1e-4,
            "sq[1,1,1]={}",
            sq[idx_111]
        );
    }

    // ---- labelmap_to_tsdf integration tests ----

    #[test]
    fn test_labelmap_to_tsdf_sphere_allocates_boundary_chunks() {
        let dims = [32u32; 3];
        let mask = make_sphere_mask(dims, 8.0, 1);
        let (chunks, tsdf_dims, tsdf_spacing, tsdf_origin) =
            build_tsdf_chunks_from_labelmap(&mask, dims, [1.0; 3], 1, 16, 12.0);

        // Grid metadata must match input.
        assert_eq!(tsdf_dims, dims);
        assert_eq!(tsdf_spacing, [1.0; 3]);
        assert_eq!(tsdf_origin, [0.0; 3]);

        // At least boundary chunks are allocated.
        assert!(!chunks.is_empty(), "sphere boundary must allocate chunks");

        // Every allocated chunk must have at least one valid voxel.
        for (key, chunk) in &chunks {
            let has_valid = chunk.weight.iter().any(|&w| w > 0);
            assert!(has_valid, "chunk {:?} has no valid voxels", key);
        }
    }

    #[test]
    fn test_tsdf_chunks_sparse_skips_pure_interior_exterior() {
        // Small sphere in the center of a large volume.
        let dims = [64u32; 3];
        let mask = make_sphere_mask(dims, 5.0, 1);
        let (chunks, _, _, _) = build_tsdf_chunks_from_labelmap(&mask, dims, [1.0; 3], 1, 16, 8.0);

        assert!(!chunks.is_empty(), "boundary chunks must exist");
        // (64/16)^3 = 64 total possible chunks; sparsity must skip most.
        assert!(
            chunks.len() < 64,
            "expected sparse allocation, got {} chunks",
            chunks.len()
        );
    }

    #[test]
    fn test_labelmap_to_tsdf_sign_correctness() {
        use crate::convert::contour_to_tsdf_chunks::dequantize_tsdf;

        // 16³ volume with a 4³ foreground block at the center.
        let dims = [16u32; 3];
        let mut mask = vec![0u8; 16 * 16 * 16];
        for z in 6..10u32 {
            for y in 6..10u32 {
                for x in 6..10u32 {
                    mask[(x + y * 16 + z * 256) as usize] = 1;
                }
            }
        }

        let trunc = 6.0f32;
        let (chunks, _, _, _) = build_tsdf_chunks_from_labelmap(&mask, dims, [1.0; 3], 1, 8, trunc);
        assert!(!chunks.is_empty());

        for chunk in chunks.values() {
            for (i, (&val, &w)) in chunk.values.iter().zip(chunk.weight.iter()).enumerate() {
                if w == 0 {
                    continue;
                }
                let lx = i as u32 % chunk.dims[0];
                let ly = (i as u32 / chunk.dims[0]) % chunk.dims[1];
                let lz = i as u32 / (chunk.dims[0] * chunk.dims[1]);
                let gx = (chunk.origin[0] + lx as f32).round() as i32;
                let gy = (chunk.origin[1] + ly as f32).round() as i32;
                let gz = (chunk.origin[2] + lz as f32).round() as i32;

                let sdf = dequantize_tsdf(val, trunc);

                // Voxels clearly inside (≥2 from any face of the 6³ block).
                let deep_inside =
                    (7..9).contains(&gx) && (7..9).contains(&gy) && (7..9).contains(&gz);
                // Voxels clearly outside (≥3 from the block surface).
                let deep_outside = gx < 3 || gx >= 13 || gy < 3 || gy >= 13 || gz < 3 || gz >= 13;

                if deep_inside {
                    assert!(
                        sdf < 0.0,
                        "deep-inside voxel ({gx},{gy},{gz}) should have negative SDF, got {sdf}"
                    );
                }
                if deep_outside {
                    assert!(
                        sdf > 0.0,
                        "deep-outside voxel ({gx},{gy},{gz}) should have positive SDF, got {sdf}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_lazy_contour_from_tsdf() {
        use crate::app::segment::SegmentRuntimeCache;
        use crate::convert::slice_isolines::extract_tsdf_isolines_for_slice;
        use crate::util::orientation::SlicePlane;

        let dims = [32u32; 3];
        let mask = make_sphere_mask(dims, 8.0, 1);
        let (chunks, tsdf_dims, tsdf_spacing, tsdf_origin) =
            build_tsdf_chunks_from_labelmap(&mask, dims, [1.0; 3], 1, 16, 12.0);

        let mut runtime = SegmentRuntimeCache {
            chunk_size: 16,
            tsdf_dims,
            tsdf_spacing,
            tsdf_origin,
            ..SegmentRuntimeCache::default()
        };
        runtime.tsdf_chunks = chunks;

        // The equatorial axial slice should intersect the sphere surface.
        let lines = extract_tsdf_isolines_for_slice(&runtime, SlicePlane::Axial, 16, 10_000);
        assert!(!lines.is_empty(), "expected contour at sphere equator");

        // All coordinates must be finite.
        for (a, b) in &lines {
            assert!(a.iter().all(|v| v.is_finite()), "non-finite point a={a:?}");
            assert!(b.iter().all(|v| v.is_finite()), "non-finite point b={b:?}");
        }

        // Rough sanity: a full circle at radius 8 in a 32³ grid has circumference
        // ~50 voxels; expect at least 20 line segments.
        assert!(
            lines.len() >= 20,
            "expected ≥20 contour segments, got {}",
            lines.len()
        );
    }

    #[test]
    fn test_empty_mask_returns_no_chunks() {
        let dims = [8u32; 3];
        let mask = vec![0u8; (dims[0] * dims[1] * dims[2]) as usize];
        let (chunks, _, _, _) = build_tsdf_chunks_from_labelmap(&mask, dims, [1.0; 3], 1, 8, 8.0);
        assert!(chunks.is_empty());
    }
}
