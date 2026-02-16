use crate::app::segment::{ChunkKey, SdfVolume, SegmentRuntimeCache, SpatialPlane};
use crate::convert::dequantize_tsdf;
use crate::util::orientation::SlicePlane;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IsolineParityStats {
    pub sdf_segments: usize,
    pub tsdf_segments: usize,
    pub abs_diff: usize,
    pub rel_diff: f32,
}

impl IsolineParityStats {
    pub fn from_counts(sdf_segments: usize, tsdf_segments: usize) -> Self {
        let abs_diff = sdf_segments.abs_diff(tsdf_segments);
        let denom = sdf_segments.max(tsdf_segments).max(1) as f32;
        Self {
            sdf_segments,
            tsdf_segments,
            abs_diff,
            rel_diff: abs_diff as f32 / denom,
        }
    }
}

#[inline]
fn crosses_iso_zero(a: f32, b: f32) -> bool {
    (a <= 0.0 && b > 0.0) || (a > 0.0 && b <= 0.0)
}

#[inline]
fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

#[inline]
fn plane_corner_index(plane: SlicePlane, slice: u32, u: u32, v: u32) -> [u32; 3] {
    match plane {
        SlicePlane::Axial => [u, v, slice],
        SlicePlane::Coronal => [u, slice, v],
        SlicePlane::Sagittal => [slice, u, v],
    }
}

#[inline]
fn index_to_center_uv(idx: u32, dim: u32) -> f32 {
    if dim == 0 {
        0.5
    } else {
        (idx as f32 + 0.5) / dim as f32
    }
}

#[inline]
fn corner_volume_uv_for_dims(
    dims: [u32; 3],
    plane: SlicePlane,
    slice: u32,
    u: u32,
    v: u32,
) -> [f32; 3] {
    match plane {
        SlicePlane::Axial => [
            index_to_center_uv(u, dims[0]),
            index_to_center_uv(v, dims[1]),
            index_to_center_uv(slice, dims[2]),
        ],
        SlicePlane::Coronal => [
            index_to_center_uv(u, dims[0]),
            index_to_center_uv(slice, dims[1]),
            index_to_center_uv(v, dims[2]),
        ],
        SlicePlane::Sagittal => [
            index_to_center_uv(slice, dims[0]),
            index_to_center_uv(u, dims[1]),
            index_to_center_uv(v, dims[2]),
        ],
    }
}

#[inline]
fn sample_sdf_at_index(sdf: &SdfVolume, idx: [u32; 3]) -> Option<f32> {
    if idx[0] >= sdf.dimensions[0] || idx[1] >= sdf.dimensions[1] || idx[2] >= sdf.dimensions[2] {
        return None;
    }
    Some(sdf.get(idx[0], idx[1], idx[2]))
}

#[inline]
fn chunk_origin_index_axis(chunk_origin: f32, tsdf_origin: f32, spacing: f32) -> i32 {
    ((chunk_origin - tsdf_origin) / spacing.max(1e-6)).round() as i32
}

#[inline]
fn sample_chunk_at_global_index(
    runtime: &SegmentRuntimeCache,
    key: ChunkKey,
    idx: [u32; 3],
) -> Option<f32> {
    let chunk = runtime.tsdf_chunks.get(&key)?;
    if chunk.dims.contains(&0) {
        return None;
    }

    let ox = chunk_origin_index_axis(
        chunk.origin[0],
        runtime.tsdf_origin[0],
        runtime.tsdf_spacing[0],
    );
    let oy = chunk_origin_index_axis(
        chunk.origin[1],
        runtime.tsdf_origin[1],
        runtime.tsdf_spacing[1],
    );
    let oz = chunk_origin_index_axis(
        chunk.origin[2],
        runtime.tsdf_origin[2],
        runtime.tsdf_spacing[2],
    );

    let lx = idx[0] as i32 - ox;
    let ly = idx[1] as i32 - oy;
    let lz = idx[2] as i32 - oz;
    if lx < 0
        || ly < 0
        || lz < 0
        || lx >= chunk.dims[0] as i32
        || ly >= chunk.dims[1] as i32
        || lz >= chunk.dims[2] as i32
    {
        return None;
    }

    let li = (lx as usize)
        + (ly as usize) * chunk.dims[0] as usize
        + (lz as usize) * (chunk.dims[0] * chunk.dims[1]) as usize;
    if chunk.weight.get(li).copied().unwrap_or(0) == 0 {
        return None;
    }
    let q = *chunk.values.get(li)?;
    Some(dequantize_tsdf(q, chunk.truncation_mm))
}

#[inline]
fn sample_tsdf_at_index(runtime: &SegmentRuntimeCache, idx: [u32; 3]) -> Option<f32> {
    let dims = runtime.tsdf_dims;
    if dims.contains(&0) || idx[0] >= dims[0] || idx[1] >= dims[1] || idx[2] >= dims[2] {
        return None;
    }
    let cs = runtime.chunk_size.max(4);
    let primary = ChunkKey {
        x: (idx[0] / cs) as i32,
        y: (idx[1] / cs) as i32,
        z: (idx[2] / cs) as i32,
    };
    if let Some(v) = sample_chunk_at_global_index(runtime, primary, idx) {
        return Some(v);
    }

    // Fallback on neighboring chunk keys in case sparse updates lag one frame.
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                if dx == 0 && dy == 0 && dz == 0 {
                    continue;
                }
                let key = ChunkKey {
                    x: primary.x + dx,
                    y: primary.y + dy,
                    z: primary.z + dz,
                };
                if let Some(v) = sample_chunk_at_global_index(runtime, key, idx) {
                    return Some(v);
                }
            }
        }
    }
    None
}

#[inline]
fn add_scaled3(base: [f32; 3], dir: [f32; 3], scale: f32) -> [f32; 3] {
    [
        base[0] + dir[0] * scale,
        base[1] + dir[1] * scale,
        base[2] + dir[2] * scale,
    ]
}

fn sample_tsdf_world_nearest(runtime: &SegmentRuntimeCache, world: [f32; 3]) -> Option<f32> {
    let dims = runtime.tsdf_dims;
    if dims.contains(&0) {
        return None;
    }
    let fx = (world[0] - runtime.tsdf_origin[0]) / runtime.tsdf_spacing[0].max(1e-6);
    let fy = (world[1] - runtime.tsdf_origin[1]) / runtime.tsdf_spacing[1].max(1e-6);
    let fz = (world[2] - runtime.tsdf_origin[2]) / runtime.tsdf_spacing[2].max(1e-6);
    let xi = fx.round() as i32;
    let yi = fy.round() as i32;
    let zi = fz.round() as i32;
    if xi < 0
        || yi < 0
        || zi < 0
        || xi >= dims[0] as i32
        || yi >= dims[1] as i32
        || zi >= dims[2] as i32
    {
        return None;
    }
    sample_tsdf_at_index(runtime, [xi as u32, yi as u32, zi as u32])
}

pub fn extract_sdf_isolines_for_slice(
    sdf: &SdfVolume,
    plane: SlicePlane,
    slice_index: i32,
    max_segments: usize,
) -> Vec<([f32; 3], [f32; 3])> {
    if slice_index < 0 {
        return Vec::new();
    }

    let dims = sdf.dimensions;
    let (u_dim, v_dim, depth_dim) = match plane {
        SlicePlane::Axial => (dims[0], dims[1], dims[2]),
        SlicePlane::Coronal => (dims[0], dims[2], dims[1]),
        SlicePlane::Sagittal => (dims[1], dims[2], dims[0]),
    };

    let slice = slice_index as u32;
    if slice >= depth_dim || u_dim < 2 || v_dim < 2 {
        return Vec::new();
    }

    let mut segments = Vec::with_capacity((u_dim as usize * v_dim as usize) / 4);
    for v in 0..(v_dim - 1) {
        for u in 0..(u_dim - 1) {
            if segments.len() >= max_segments {
                return segments;
            }

            let idx00 = plane_corner_index(plane, slice, u, v);
            let idx10 = plane_corner_index(plane, slice, u + 1, v);
            let idx11 = plane_corner_index(plane, slice, u + 1, v + 1);
            let idx01 = plane_corner_index(plane, slice, u, v + 1);

            let (Some(v00), Some(v10), Some(v11), Some(v01)) = (
                sample_sdf_at_index(sdf, idx00),
                sample_sdf_at_index(sdf, idx10),
                sample_sdf_at_index(sdf, idx11),
                sample_sdf_at_index(sdf, idx01),
            ) else {
                continue;
            };

            let val = [v00, v10, v11, v01];
            let pos = [
                corner_volume_uv_for_dims(dims, plane, slice, u, v),
                corner_volume_uv_for_dims(dims, plane, slice, u + 1, v),
                corner_volume_uv_for_dims(dims, plane, slice, u + 1, v + 1),
                corner_volume_uv_for_dims(dims, plane, slice, u, v + 1),
            ];

            let mut edges: [Option<[f32; 3]>; 4] = [None, None, None, None];
            let pairs = [(0usize, 1usize), (1, 2), (2, 3), (3, 0)];
            for (edge_idx, (a, b)) in pairs.iter().enumerate() {
                let va = val[*a];
                let vb = val[*b];
                if !crosses_iso_zero(va, vb) {
                    continue;
                }
                let denom = vb - va;
                let t = if denom.abs() > 1e-6 {
                    (-va / denom).clamp(0.0, 1.0)
                } else {
                    0.5
                };
                edges[edge_idx] = Some(lerp3(pos[*a], pos[*b], t));
            }

            let case = ((val[0] < 0.0) as u8)
                | (((val[1] < 0.0) as u8) << 1)
                | (((val[2] < 0.0) as u8) << 2)
                | (((val[3] < 0.0) as u8) << 3);

            let mut push_pair = |a: usize, b: usize| {
                if let (Some(p0), Some(p1)) = (edges[a], edges[b]) {
                    segments.push((p0, p1));
                }
            };

            match case {
                0 | 15 => {}
                1 | 14 => push_pair(3, 0),
                2 | 13 => push_pair(0, 1),
                3 | 12 => push_pair(3, 1),
                4 | 11 => push_pair(1, 2),
                5 => {
                    push_pair(3, 2);
                    push_pair(0, 1);
                }
                6 | 9 => push_pair(0, 2),
                7 | 8 => push_pair(3, 2),
                10 => {
                    push_pair(0, 3);
                    push_pair(1, 2);
                }
                _ => {}
            }
        }
    }
    segments
}

pub fn extract_sdf_isolines_for_slice_capped(
    sdf: &SdfVolume,
    plane: SlicePlane,
    slice_index: i32,
    max_segments: usize,
    max_samples_per_axis: u32,
) -> Vec<([f32; 3], [f32; 3])> {
    if slice_index < 0 {
        return Vec::new();
    }
    let dims = sdf.dimensions;
    let (u_dim, v_dim, depth_dim) = match plane {
        SlicePlane::Axial => (dims[0], dims[1], dims[2]),
        SlicePlane::Coronal => (dims[0], dims[2], dims[1]),
        SlicePlane::Sagittal => (dims[1], dims[2], dims[0]),
    };
    let slice = slice_index as u32;
    if slice >= depth_dim || u_dim < 2 || v_dim < 2 {
        return Vec::new();
    }
    let max_axis = max_samples_per_axis.max(2);
    let step_u = ((u_dim - 1) / max_axis).max(1);
    let step_v = ((v_dim - 1) / max_axis).max(1);
    let u_cells = ((u_dim - 1) / step_u).max(1);
    let v_cells = ((v_dim - 1) / step_v).max(1);

    let mut segments = Vec::with_capacity((u_cells as usize * v_cells as usize) / 3);
    for vc in 0..v_cells {
        for uc in 0..u_cells {
            if segments.len() >= max_segments {
                return segments;
            }
            let u0 = uc * step_u;
            let v0 = vc * step_v;
            let u1 = (u0 + step_u).min(u_dim - 1);
            let v1 = (v0 + step_v).min(v_dim - 1);
            if u1 == u0 || v1 == v0 {
                continue;
            }

            let idx00 = plane_corner_index(plane, slice, u0, v0);
            let idx10 = plane_corner_index(plane, slice, u1, v0);
            let idx11 = plane_corner_index(plane, slice, u1, v1);
            let idx01 = plane_corner_index(plane, slice, u0, v1);

            let (Some(v00), Some(v10), Some(v11), Some(v01)) = (
                sample_sdf_at_index(sdf, idx00),
                sample_sdf_at_index(sdf, idx10),
                sample_sdf_at_index(sdf, idx11),
                sample_sdf_at_index(sdf, idx01),
            ) else {
                continue;
            };

            let val = [v00, v10, v11, v01];
            let pos = [
                corner_volume_uv_for_dims(dims, plane, slice, u0, v0),
                corner_volume_uv_for_dims(dims, plane, slice, u1, v0),
                corner_volume_uv_for_dims(dims, plane, slice, u1, v1),
                corner_volume_uv_for_dims(dims, plane, slice, u0, v1),
            ];

            let mut edges: [Option<[f32; 3]>; 4] = [None, None, None, None];
            let pairs = [(0usize, 1usize), (1, 2), (2, 3), (3, 0)];
            for (edge_idx, (a, b)) in pairs.iter().enumerate() {
                let va = val[*a];
                let vb = val[*b];
                if !crosses_iso_zero(va, vb) {
                    continue;
                }
                let denom = vb - va;
                let t = if denom.abs() > 1e-6 {
                    (-va / denom).clamp(0.0, 1.0)
                } else {
                    0.5
                };
                edges[edge_idx] = Some(lerp3(pos[*a], pos[*b], t));
            }

            let case = ((val[0] < 0.0) as u8)
                | (((val[1] < 0.0) as u8) << 1)
                | (((val[2] < 0.0) as u8) << 2)
                | (((val[3] < 0.0) as u8) << 3);
            let mut push_pair = |a: usize, b: usize| {
                if let (Some(p0), Some(p1)) = (edges[a], edges[b]) {
                    segments.push((p0, p1));
                }
            };
            match case {
                0 | 15 => {}
                1 | 14 => push_pair(3, 0),
                2 | 13 => push_pair(0, 1),
                3 | 12 => push_pair(3, 1),
                4 | 11 => push_pair(1, 2),
                5 => {
                    push_pair(3, 2);
                    push_pair(0, 1);
                }
                6 | 9 => push_pair(0, 2),
                7 | 8 => push_pair(3, 2),
                10 => {
                    push_pair(0, 3);
                    push_pair(1, 2);
                }
                _ => {}
            }
        }
    }
    segments
}

pub fn extract_tsdf_isolines_for_slice(
    runtime: &SegmentRuntimeCache,
    plane: SlicePlane,
    slice_index: i32,
    max_segments: usize,
) -> Vec<([f32; 3], [f32; 3])> {
    if slice_index < 0 {
        return Vec::new();
    }

    let dims = runtime.tsdf_dims;
    let (u_dim, v_dim, depth_dim) = match plane {
        SlicePlane::Axial => (dims[0], dims[1], dims[2]),
        SlicePlane::Coronal => (dims[0], dims[2], dims[1]),
        SlicePlane::Sagittal => (dims[1], dims[2], dims[0]),
    };

    let slice = slice_index as u32;
    if slice >= depth_dim || u_dim < 2 || v_dim < 2 {
        return Vec::new();
    }

    let mut segments = Vec::with_capacity((u_dim as usize * v_dim as usize) / 4);
    for v in 0..(v_dim - 1) {
        for u in 0..(u_dim - 1) {
            if segments.len() >= max_segments {
                return segments;
            }

            let idx00 = plane_corner_index(plane, slice, u, v);
            let idx10 = plane_corner_index(plane, slice, u + 1, v);
            let idx11 = plane_corner_index(plane, slice, u + 1, v + 1);
            let idx01 = plane_corner_index(plane, slice, u, v + 1);

            let (Some(v00), Some(v10), Some(v11), Some(v01)) = (
                sample_tsdf_at_index(runtime, idx00),
                sample_tsdf_at_index(runtime, idx10),
                sample_tsdf_at_index(runtime, idx11),
                sample_tsdf_at_index(runtime, idx01),
            ) else {
                continue;
            };

            let val = [v00, v10, v11, v01];
            let pos = [
                corner_volume_uv_for_dims(dims, plane, slice, u, v),
                corner_volume_uv_for_dims(dims, plane, slice, u + 1, v),
                corner_volume_uv_for_dims(dims, plane, slice, u + 1, v + 1),
                corner_volume_uv_for_dims(dims, plane, slice, u, v + 1),
            ];

            let mut edges: [Option<[f32; 3]>; 4] = [None, None, None, None];
            let pairs = [(0usize, 1usize), (1, 2), (2, 3), (3, 0)];
            for (edge_idx, (a, b)) in pairs.iter().enumerate() {
                let va = val[*a];
                let vb = val[*b];
                if !crosses_iso_zero(va, vb) {
                    continue;
                }
                let denom = vb - va;
                let t = if denom.abs() > 1e-6 {
                    (-va / denom).clamp(0.0, 1.0)
                } else {
                    0.5
                };
                edges[edge_idx] = Some(lerp3(pos[*a], pos[*b], t));
            }

            let case = ((val[0] < 0.0) as u8)
                | (((val[1] < 0.0) as u8) << 1)
                | (((val[2] < 0.0) as u8) << 2)
                | (((val[3] < 0.0) as u8) << 3);

            let mut push_pair = |a: usize, b: usize| {
                if let (Some(p0), Some(p1)) = (edges[a], edges[b]) {
                    segments.push((p0, p1));
                }
            };

            match case {
                0 | 15 => {}
                1 | 14 => push_pair(3, 0),
                2 | 13 => push_pair(0, 1),
                3 | 12 => push_pair(3, 1),
                4 | 11 => push_pair(1, 2),
                5 => {
                    push_pair(3, 2);
                    push_pair(0, 1);
                }
                6 | 9 => push_pair(0, 2),
                7 | 8 => push_pair(3, 2),
                10 => {
                    push_pair(0, 3);
                    push_pair(1, 2);
                }
                _ => {}
            }
        }
    }
    segments
}

pub fn extract_tsdf_isolines_for_slice_capped(
    runtime: &SegmentRuntimeCache,
    plane: SlicePlane,
    slice_index: i32,
    max_segments: usize,
    max_samples_per_axis: u32,
) -> Vec<([f32; 3], [f32; 3])> {
    if slice_index < 0 {
        return Vec::new();
    }
    let dims = runtime.tsdf_dims;
    let (u_dim, v_dim, depth_dim) = match plane {
        SlicePlane::Axial => (dims[0], dims[1], dims[2]),
        SlicePlane::Coronal => (dims[0], dims[2], dims[1]),
        SlicePlane::Sagittal => (dims[1], dims[2], dims[0]),
    };
    let slice = slice_index as u32;
    if slice >= depth_dim || u_dim < 2 || v_dim < 2 {
        return Vec::new();
    }
    let max_axis = max_samples_per_axis.max(2);
    let step_u = ((u_dim - 1) / max_axis).max(1);
    let step_v = ((v_dim - 1) / max_axis).max(1);
    let u_cells = ((u_dim - 1) / step_u).max(1);
    let v_cells = ((v_dim - 1) / step_v).max(1);

    let mut segments = Vec::with_capacity((u_cells as usize * v_cells as usize) / 3);
    for vc in 0..v_cells {
        for uc in 0..u_cells {
            if segments.len() >= max_segments {
                return segments;
            }
            let u0 = uc * step_u;
            let v0 = vc * step_v;
            let u1 = (u0 + step_u).min(u_dim - 1);
            let v1 = (v0 + step_v).min(v_dim - 1);
            if u1 == u0 || v1 == v0 {
                continue;
            }

            let idx00 = plane_corner_index(plane, slice, u0, v0);
            let idx10 = plane_corner_index(plane, slice, u1, v0);
            let idx11 = plane_corner_index(plane, slice, u1, v1);
            let idx01 = plane_corner_index(plane, slice, u0, v1);
            let (Some(v00), Some(v10), Some(v11), Some(v01)) = (
                sample_tsdf_at_index(runtime, idx00),
                sample_tsdf_at_index(runtime, idx10),
                sample_tsdf_at_index(runtime, idx11),
                sample_tsdf_at_index(runtime, idx01),
            ) else {
                continue;
            };

            let val = [v00, v10, v11, v01];
            let pos = [
                corner_volume_uv_for_dims(dims, plane, slice, u0, v0),
                corner_volume_uv_for_dims(dims, plane, slice, u1, v0),
                corner_volume_uv_for_dims(dims, plane, slice, u1, v1),
                corner_volume_uv_for_dims(dims, plane, slice, u0, v1),
            ];

            let mut edges: [Option<[f32; 3]>; 4] = [None, None, None, None];
            let pairs = [(0usize, 1usize), (1, 2), (2, 3), (3, 0)];
            for (edge_idx, (a, b)) in pairs.iter().enumerate() {
                let va = val[*a];
                let vb = val[*b];
                if !crosses_iso_zero(va, vb) {
                    continue;
                }
                let denom = vb - va;
                let t = if denom.abs() > 1e-6 {
                    (-va / denom).clamp(0.0, 1.0)
                } else {
                    0.5
                };
                edges[edge_idx] = Some(lerp3(pos[*a], pos[*b], t));
            }

            let case = ((val[0] < 0.0) as u8)
                | (((val[1] < 0.0) as u8) << 1)
                | (((val[2] < 0.0) as u8) << 2)
                | (((val[3] < 0.0) as u8) << 3);
            let mut push_pair = |a: usize, b: usize| {
                if let (Some(p0), Some(p1)) = (edges[a], edges[b]) {
                    segments.push((p0, p1));
                }
            };
            match case {
                0 | 15 => {}
                1 | 14 => push_pair(3, 0),
                2 | 13 => push_pair(0, 1),
                3 | 12 => push_pair(3, 1),
                4 | 11 => push_pair(1, 2),
                5 => {
                    push_pair(3, 2);
                    push_pair(0, 1);
                }
                6 | 9 => push_pair(0, 2),
                7 | 8 => push_pair(3, 2),
                10 => {
                    push_pair(0, 3);
                    push_pair(1, 2);
                }
                _ => {}
            }
        }
    }
    segments
}

/// Extract TSDF iso-contours from an arbitrary spatial plane.
///
/// `extent_u_mm` and `extent_v_mm` are half-extents on the plane in world units.
/// `samples` controls the marching-squares lattice along (u,v).
pub fn extract_tsdf_isolines_for_spatial_plane(
    runtime: &SegmentRuntimeCache,
    plane: &SpatialPlane,
    extent_u_mm: f32,
    extent_v_mm: f32,
    samples: [u32; 2],
    max_segments: usize,
) -> Vec<([f32; 3], [f32; 3])> {
    let su = samples[0].max(2);
    let sv = samples[1].max(2);
    let du = (2.0 * extent_u_mm) / (su - 1) as f32;
    let dv = (2.0 * extent_v_mm) / (sv - 1) as f32;
    if !du.is_finite() || !dv.is_finite() {
        return Vec::new();
    }

    let mut segments = Vec::with_capacity(((su * sv) as usize) / 4);
    for v in 0..(sv - 1) {
        for u in 0..(su - 1) {
            if segments.len() >= max_segments {
                return segments;
            }

            let u0 = -extent_u_mm + u as f32 * du;
            let v0 = -extent_v_mm + v as f32 * dv;
            let u1 = u0 + du;
            let v1 = v0 + dv;

            let p00 = add_scaled3(
                add_scaled3(plane.origin, plane.u_axis, u0),
                plane.v_axis,
                v0,
            );
            let p10 = add_scaled3(
                add_scaled3(plane.origin, plane.u_axis, u1),
                plane.v_axis,
                v0,
            );
            let p11 = add_scaled3(
                add_scaled3(plane.origin, plane.u_axis, u1),
                plane.v_axis,
                v1,
            );
            let p01 = add_scaled3(
                add_scaled3(plane.origin, plane.u_axis, u0),
                plane.v_axis,
                v1,
            );

            let (Some(v00), Some(v10), Some(v11), Some(v01)) = (
                sample_tsdf_world_nearest(runtime, p00),
                sample_tsdf_world_nearest(runtime, p10),
                sample_tsdf_world_nearest(runtime, p11),
                sample_tsdf_world_nearest(runtime, p01),
            ) else {
                continue;
            };

            let val = [v00, v10, v11, v01];
            let pos = [p00, p10, p11, p01];

            let mut edges: [Option<[f32; 3]>; 4] = [None, None, None, None];
            let pairs = [(0usize, 1usize), (1, 2), (2, 3), (3, 0)];
            for (edge_idx, (a, b)) in pairs.iter().enumerate() {
                let va = val[*a];
                let vb = val[*b];
                if !crosses_iso_zero(va, vb) {
                    continue;
                }
                let denom = vb - va;
                let t = if denom.abs() > 1e-6 {
                    (-va / denom).clamp(0.0, 1.0)
                } else {
                    0.5
                };
                edges[edge_idx] = Some(lerp3(pos[*a], pos[*b], t));
            }

            let case = ((val[0] < 0.0) as u8)
                | (((val[1] < 0.0) as u8) << 1)
                | (((val[2] < 0.0) as u8) << 2)
                | (((val[3] < 0.0) as u8) << 3);

            let mut push_pair = |a: usize, b: usize| {
                if let (Some(p0), Some(p1)) = (edges[a], edges[b]) {
                    segments.push((p0, p1));
                }
            };

            match case {
                0 | 15 => {}
                1 | 14 => push_pair(3, 0),
                2 | 13 => push_pair(0, 1),
                3 | 12 => push_pair(3, 1),
                4 | 11 => push_pair(1, 2),
                5 => {
                    push_pair(3, 2);
                    push_pair(0, 1);
                }
                6 | 9 => push_pair(0, 2),
                7 | 8 => push_pair(3, 2),
                10 => {
                    push_pair(0, 3);
                    push_pair(1, 2);
                }
                _ => {}
            }
        }
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::segment::{SdfVolume, SegmentRuntimeCache};
    use crate::convert::{build_tsdf_chunk_from_sdf, chunk_bounds_for_key, chunk_keys_for_bounds};

    fn build_runtime_from_sdf(sdf: &SdfVolume, chunk_size: u32) -> SegmentRuntimeCache {
        let mut runtime = SegmentRuntimeCache {
            chunk_size,
            tsdf_dims: sdf.dimensions,
            tsdf_spacing: sdf.spacing,
            tsdf_origin: sdf.origin,
            ..SegmentRuntimeCache::default()
        };
        let bounds = [
            0,
            0,
            0,
            sdf.dimensions[0].saturating_sub(1),
            sdf.dimensions[1].saturating_sub(1),
            sdf.dimensions[2].saturating_sub(1),
        ];
        for key in chunk_keys_for_bounds(bounds, chunk_size) {
            if let Some(cb) = chunk_bounds_for_key(key, chunk_size, sdf.dimensions) {
                let padded = [
                    cb[0].saturating_sub(1),
                    cb[1].saturating_sub(1),
                    cb[2].saturating_sub(1),
                    (cb[3] + 1).min(sdf.dimensions[0].saturating_sub(1)),
                    (cb[4] + 1).min(sdf.dimensions[1].saturating_sub(1)),
                    (cb[5] + 1).min(sdf.dimensions[2].saturating_sub(1)),
                ];
                if let Some(tsdf) = build_tsdf_chunk_from_sdf(sdf, padded, 24.0, 1) {
                    runtime.tsdf_chunks.insert(key, tsdf);
                }
            }
        }
        runtime
    }

    fn make_sphere_sdf(dims: [u32; 3], radius: f32) -> SdfVolume {
        let mut sdf = SdfVolume::new(dims, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        let center = [
            (dims[0] as f32 - 1.0) * 0.5,
            (dims[1] as f32 - 1.0) * 0.5,
            (dims[2] as f32 - 1.0) * 0.5,
        ];
        for z in 0..dims[2] {
            for y in 0..dims[1] {
                for x in 0..dims[0] {
                    let dx = x as f32 - center[0];
                    let dy = y as f32 - center[1];
                    let dz = z as f32 - center[2];
                    let d = (dx * dx + dy * dy + dz * dz).sqrt() - radius;
                    sdf.set(x, y, z, d);
                }
            }
        }
        sdf
    }

    #[test]
    fn test_tsdf_slice_isolines_parity_with_sdf_counts() {
        let sdf = make_sphere_sdf([48, 48, 48], 14.0);
        let runtime = build_runtime_from_sdf(&sdf, 16);
        let slice = 24;

        let sdf_lines = extract_sdf_isolines_for_slice(&sdf, SlicePlane::Axial, slice, 50000);
        let tsdf_lines = extract_tsdf_isolines_for_slice(&runtime, SlicePlane::Axial, slice, 50000);
        let stats = IsolineParityStats::from_counts(sdf_lines.len(), tsdf_lines.len());

        assert!(
            stats.rel_diff <= 0.08,
            "Unexpected TSDF/SDF segment-count divergence: {:?}",
            stats
        );
    }

    #[test]
    fn test_tsdf_isoline_extraction_empty_without_runtime_dims() {
        let runtime = SegmentRuntimeCache::default();
        let lines = extract_tsdf_isolines_for_slice(&runtime, SlicePlane::Coronal, 10, 1000);
        assert!(lines.is_empty());
    }

    #[test]
    fn test_tsdf_spatial_plane_isolines_non_empty_on_sphere() {
        let sdf = make_sphere_sdf([48, 48, 48], 14.0);
        let runtime = build_runtime_from_sdf(&sdf, 16);
        let plane = SpatialPlane {
            normal: [0.0, 0.0, 1.0],
            distance: 24.0,
            u_axis: [1.0, 0.0, 0.0],
            v_axis: [0.0, 1.0, 0.0],
            origin: [24.0, 24.0, 24.0],
        };
        let lines =
            extract_tsdf_isolines_for_spatial_plane(&runtime, &plane, 20.0, 20.0, [64, 64], 50000);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_tsdf_spatial_plane_isolines_empty_for_empty_runtime() {
        let runtime = SegmentRuntimeCache::default();
        let plane = SpatialPlane {
            normal: [0.0, 1.0, 0.0],
            distance: 0.0,
            u_axis: [1.0, 0.0, 0.0],
            v_axis: [0.0, 0.0, 1.0],
            origin: [0.0, 0.0, 0.0],
        };
        let lines =
            extract_tsdf_isolines_for_spatial_plane(&runtime, &plane, 10.0, 10.0, [32, 32], 2000);
        assert!(lines.is_empty());
    }
}
