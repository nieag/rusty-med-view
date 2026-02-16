//! Labelmap to contour extraction utilities.
//!
//! Converts binary/label voxel masks into editable axial contour loops.

use crate::app::segment::{ContourSet, Plane3D, PlaneContour};
use crate::util::orientation::SlicePlane;
use std::collections::{HashMap, HashSet, VecDeque};

type GridPoint = (i32, i32);

fn idx(dims: [u32; 3], x: u32, y: u32, z: u32) -> usize {
    (x + y * dims[0] + z * dims[0] * dims[1]) as usize
}

fn is_foreground(mask: &[u8], dims: [u32; 3], x: i32, y: i32, z: u32, label: Option<u8>) -> bool {
    if x < 0 || y < 0 || x >= dims[0] as i32 || y >= dims[1] as i32 {
        return false;
    }
    let v = mask[idx(dims, x as u32, y as u32, z)];
    if let Some(l) = label {
        v == l
    } else {
        v != 0
    }
}

fn is_foreground_plane(
    mask: &[u8],
    dims: [u32; 3],
    plane: SlicePlane,
    slice: u32,
    u: i32,
    v: i32,
    label: Option<u8>,
) -> bool {
    match plane {
        SlicePlane::Axial => is_foreground(mask, dims, u, v, slice, label),
        SlicePlane::Coronal => {
            if u < 0 || v < 0 || u >= dims[0] as i32 || v >= dims[2] as i32 {
                return false;
            }
            let val = mask[idx(dims, u as u32, slice, v as u32)];
            label.map_or(val != 0, |l| val == l)
        }
        SlicePlane::Sagittal => {
            if u < 0 || v < 0 || u >= dims[1] as i32 || v >= dims[2] as i32 {
                return false;
            }
            let val = mask[idx(dims, slice, u as u32, v as u32)];
            label.map_or(val != 0, |l| val == l)
        }
    }
}

fn plane_uv_dims(dims: [u32; 3], plane: SlicePlane) -> [u32; 2] {
    match plane {
        SlicePlane::Axial => [dims[0], dims[1]],
        SlicePlane::Coronal => [dims[0], dims[2]],
        SlicePlane::Sagittal => [dims[1], dims[2]],
    }
}

fn add_edge(edges: &mut HashMap<GridPoint, Vec<GridPoint>>, a: GridPoint, b: GridPoint) {
    edges.entry(a).or_default().push(b);
}

fn pop_next_edge(
    edges: &mut HashMap<GridPoint, Vec<GridPoint>>,
    p: GridPoint,
) -> Option<GridPoint> {
    let next = edges.get_mut(&p).and_then(|v| v.pop());
    if let Some(v) = edges.get(&p) {
        if v.is_empty() {
            edges.remove(&p);
        }
    }
    next
}

fn connected_components(
    mask: &[u8],
    dims: [u32; 3],
    plane: SlicePlane,
    slice: u32,
    label: Option<u8>,
) -> Vec<Vec<GridPoint>> {
    let uv_dims = plane_uv_dims(dims, plane);
    let w = uv_dims[0] as i32;
    let h = uv_dims[1] as i32;
    if w <= 0 || h <= 0 {
        return Vec::new();
    }

    let mut visited = vec![false; (uv_dims[0] * uv_dims[1]) as usize];
    let mut out = Vec::new();
    let mut q = VecDeque::new();
    let neighbors = [(-1, 0), (1, 0), (0, -1), (0, 1)];

    for y in 0..h {
        for x in 0..w {
            let flat = (x as u32 + y as u32 * uv_dims[0]) as usize;
            if visited[flat] || !is_foreground_plane(mask, dims, plane, slice, x, y, label) {
                continue;
            }

            let mut comp = Vec::new();
            visited[flat] = true;
            q.push_back((x, y));

            while let Some((cx, cy)) = q.pop_front() {
                comp.push((cx, cy));
                for (dx, dy) in neighbors {
                    let nx = cx + dx;
                    let ny = cy + dy;
                    if nx < 0 || ny < 0 || nx >= w || ny >= h {
                        continue;
                    }
                    let nflat = (nx as u32 + ny as u32 * uv_dims[0]) as usize;
                    if visited[nflat]
                        || !is_foreground_plane(mask, dims, plane, slice, nx, ny, label)
                    {
                        continue;
                    }
                    visited[nflat] = true;
                    q.push_back((nx, ny));
                }
            }
            out.push(comp);
        }
    }

    out
}

fn simplify_collinear(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
    if points.len() < 4 {
        return points.to_vec();
    }

    let mut out = Vec::with_capacity(points.len());
    for i in 0..points.len() {
        let prev = points[(i + points.len() - 1) % points.len()];
        let cur = points[i];
        let next = points[(i + 1) % points.len()];

        let ax = cur[0] - prev[0];
        let ay = cur[1] - prev[1];
        let bx = next[0] - cur[0];
        let by = next[1] - cur[1];
        let cross = ax * by - ay * bx;
        if cross.abs() > 1e-6 {
            out.push(cur);
        }
    }

    if out.len() < 3 {
        points.to_vec()
    } else {
        out
    }
}

fn signed_area_2d(points: &[[f32; 2]]) -> f32 {
    if points.len() < 3 {
        return 0.0;
    }
    let mut area2 = 0.0f32;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        area2 += a[0] * b[1] - b[0] * a[1];
    }
    0.5 * area2
}

fn extract_slice_loops(
    mask: &[u8],
    dims: [u32; 3],
    spacing: [f32; 3],
    plane: SlicePlane,
    slice: u32,
    label: Option<u8>,
) -> Vec<PlaneContour> {
    let mut contours = Vec::new();
    let components = connected_components(mask, dims, plane, slice, label);
    for comp in components {
        let comp_set: HashSet<GridPoint> = comp.into_iter().collect();
        let mut edges: HashMap<GridPoint, Vec<GridPoint>> = HashMap::new();

        for &(x, y) in &comp_set {
            if !comp_set.contains(&(x - 1, y)) {
                add_edge(&mut edges, (x, y), (x, y + 1));
            }
            if !comp_set.contains(&(x + 1, y)) {
                add_edge(&mut edges, (x + 1, y + 1), (x + 1, y));
            }
            if !comp_set.contains(&(x, y - 1)) {
                add_edge(&mut edges, (x + 1, y), (x, y));
            }
            if !comp_set.contains(&(x, y + 1)) {
                add_edge(&mut edges, (x, y + 1), (x + 1, y + 1));
            }
        }

        let mut loops_2d: Vec<Vec<[f32; 2]>> = Vec::new();
        while let Some((&start, _)) = edges.iter().next() {
            let Some(mut cur) = pop_next_edge(&mut edges, start) else {
                continue;
            };

            let mut loop_pts = vec![start];
            let mut guard = 0usize;
            while cur != start && guard < 1_000_000 {
                loop_pts.push(cur);
                let Some(next) = pop_next_edge(&mut edges, cur) else {
                    break;
                };
                cur = next;
                guard += 1;
            }

            if cur != start || loop_pts.len() < 3 {
                continue;
            }

            let poly2d_raw: Vec<[f32; 2]> = loop_pts
                .iter()
                .map(|&p| {
                    // 2D metric space for area/simplification in current plane.
                    match plane {
                        SlicePlane::Axial => [
                            (p.0 as f32).clamp(0.0, dims[0] as f32) * spacing[0],
                            (p.1 as f32).clamp(0.0, dims[1] as f32) * spacing[1],
                        ],
                        SlicePlane::Coronal => [
                            (p.0 as f32).clamp(0.0, dims[0] as f32) * spacing[0],
                            (p.1 as f32).clamp(0.0, dims[2] as f32) * spacing[2],
                        ],
                        SlicePlane::Sagittal => [
                            (p.0 as f32).clamp(0.0, dims[1] as f32) * spacing[1],
                            (p.1 as f32).clamp(0.0, dims[2] as f32) * spacing[2],
                        ],
                    }
                })
                .collect();
            let poly2d = simplify_collinear(&poly2d_raw);
            if poly2d.len() >= 3 {
                loops_2d.push(poly2d);
            }
        }

        // Keep a single outer boundary per connected component for faithful
        // one-contour-per-component slice representation.
        let Some(outer) = loops_2d
            .into_iter()
            .max_by(|a, b| signed_area_2d(a).abs().total_cmp(&signed_area_2d(b).abs()))
        else {
            continue;
        };

        // Rebuild from chosen loop as grid-edge coordinates to preserve exact edges.
        // We need the corresponding loop in grid coordinates; choose by area and retrace.
        // Fallback: project metric loop back directly into 3D in plane axes.
        let points3d: Vec<[f32; 3]> = outer
            .iter()
            .map(|p| match plane {
                SlicePlane::Axial => [p[0], p[1], (slice as f32 + 0.5) * spacing[2]],
                SlicePlane::Coronal => [p[0], (slice as f32 + 0.5) * spacing[1], p[1]],
                SlicePlane::Sagittal => [(slice as f32 + 0.5) * spacing[0], p[0], p[1]],
            })
            .collect();
        let plane_pos = match plane {
            SlicePlane::Axial => (slice as f32 + 0.5) * spacing[2],
            SlicePlane::Coronal => (slice as f32 + 0.5) * spacing[1],
            SlicePlane::Sagittal => (slice as f32 + 0.5) * spacing[0],
        };
        contours.push(PlaneContour::with_points(
            Plane3D::from_slice_plane(plane, plane_pos),
            points3d,
            true,
        ));
    }

    contours
}

/// Extract editable axial contours from a loaded labelmap.
///
/// If `label` is `None`, all non-zero voxels are treated as foreground.
pub fn extract_axial_contours_from_labelmap(
    mask: &[u8],
    dims: [u32; 3],
    spacing: [f32; 3],
    label: Option<u8>,
) -> ContourSet {
    let mut set = ContourSet::new();
    if dims.contains(&0) || mask.is_empty() {
        return set;
    }

    for z in 0..dims[2] {
        let loops = extract_slice_loops(mask, dims, spacing, SlicePlane::Axial, z, label);
        for contour in loops {
            set.add_contour(SlicePlane::Axial, z as i32, contour);
        }
    }

    set
}

/// Extract editable contours in all axis-aligned planes from a loaded labelmap.
pub fn extract_axis_aligned_contours_from_labelmap(
    mask: &[u8],
    dims: [u32; 3],
    spacing: [f32; 3],
    label: Option<u8>,
) -> ContourSet {
    let mut set = ContourSet::new();
    if dims.contains(&0) || mask.is_empty() {
        return set;
    }

    for z in 0..dims[2] {
        for contour in extract_slice_loops(mask, dims, spacing, SlicePlane::Axial, z, label) {
            set.add_contour(SlicePlane::Axial, z as i32, contour);
        }
    }
    for y in 0..dims[1] {
        for contour in extract_slice_loops(mask, dims, spacing, SlicePlane::Coronal, y, label) {
            set.add_contour(SlicePlane::Coronal, y as i32, contour);
        }
    }
    for x in 0..dims[0] {
        for contour in extract_slice_loops(mask, dims, spacing, SlicePlane::Sagittal, x, label) {
            set.add_contour(SlicePlane::Sagittal, x as i32, contour);
        }
    }

    set
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::path::Path;

    #[test]
    fn test_extract_single_square_slice() {
        let dims = [8, 8, 4];
        let mut data = vec![0u8; (dims[0] * dims[1] * dims[2]) as usize];

        for y in 2..6 {
            for x in 2..6 {
                data[idx(dims, x, y, 1)] = 1;
            }
        }

        let contours = extract_axial_contours_from_labelmap(&data, dims, [1.0, 1.0, 1.0], Some(1));
        let slice = contours
            .contours_at_slice(SlicePlane::Axial, 1)
            .cloned()
            .unwrap_or_default();
        assert!(!slice.is_empty());
        assert!(slice[0].is_closed);
        assert!(slice[0].points.len() >= 4);
    }

    #[test]
    fn test_extract_all_nonzero() {
        let dims = [4, 4, 1];
        let mut data = vec![0u8; (dims[0] * dims[1] * dims[2]) as usize];
        data[idx(dims, 1, 1, 0)] = 2;

        let contours = extract_axial_contours_from_labelmap(&data, dims, [1.0, 1.0, 1.0], None);
        assert!(contours.count() >= 1);
    }

    fn has_foreground_on_slice(mask: &[u8], dims: [u32; 3], z: u32) -> bool {
        let start = (z * dims[0] * dims[1]) as usize;
        let end = start + (dims[0] * dims[1]) as usize;
        mask[start..end].iter().any(|&v| v != 0)
    }

    fn count_components_on_slice(mask: &[u8], dims: [u32; 3], z: u32) -> usize {
        let w = dims[0] as i32;
        let h = dims[1] as i32;
        let mut visited = vec![false; (dims[0] * dims[1]) as usize];
        let mut q = VecDeque::new();
        let mut components = 0usize;

        let neighbors = [(-1, 0), (1, 0), (0, -1), (0, 1)];
        for y in 0..h {
            for x in 0..w {
                let flat = (x as u32 + y as u32 * dims[0]) as usize;
                if visited[flat] {
                    continue;
                }
                if !is_foreground(mask, dims, x, y, z, None) {
                    continue;
                }

                components += 1;
                visited[flat] = true;
                q.push_back((x, y));

                while let Some((cx, cy)) = q.pop_front() {
                    for (dx, dy) in neighbors {
                        let nx = cx + dx;
                        let ny = cy + dy;
                        if nx < 0 || ny < 0 || nx >= w || ny >= h {
                            continue;
                        }
                        let nflat = (nx as u32 + ny as u32 * dims[0]) as usize;
                        if visited[nflat] || !is_foreground(mask, dims, nx, ny, z, None) {
                            continue;
                        }
                        visited[nflat] = true;
                        q.push_back((nx, ny));
                    }
                }
            }
        }

        components
    }

    #[test]
    #[ignore = "Local dataset regression: /Users/nieage/Downloads/liver_0_label.nii"]
    fn test_liver_0_label_component_contour_parity() {
        let path = "/Users/nieage/Downloads/liver_0_label.nii";
        if !Path::new(path).exists() {
            panic!("local dataset not found at {path}");
        }

        let data = std::fs::read(path).expect("failed to read liver_0_label.nii");
        let loaded = crate::io::nifti::load_label_from_bytes(&data, "liver_0_label.nii".into())
            .expect("failed to parse liver_0_label.nii");

        let contours = extract_axial_contours_from_labelmap(
            &loaded.data,
            loaded.dimensions,
            [1.0, 1.0, 1.0],
            None,
        );

        let mut mismatches = Vec::new();
        let mut roi_slices = 0usize;
        let mut total_components = 0usize;
        let mut total_loops = 0usize;

        for z in 0..loaded.dimensions[2] {
            if !has_foreground_on_slice(&loaded.data, loaded.dimensions, z) {
                continue;
            }
            roi_slices += 1;

            let components = count_components_on_slice(&loaded.data, loaded.dimensions, z);
            let loops = contours
                .contours_at_slice(SlicePlane::Axial, z as i32)
                .map(|v| v.len())
                .unwrap_or(0);

            total_components += components;
            total_loops += loops;
            if components != loops {
                mismatches.push((z, components, loops));
            }
        }

        assert!(
            mismatches.is_empty(),
            "component/contour mismatch on {}/{} ROI slices (components={}, contours={}). First mismatches: {:?}",
            mismatches.len(),
            roi_slices,
            total_components,
            total_loops,
            mismatches.iter().take(12).collect::<Vec<_>>()
        );
    }
}
