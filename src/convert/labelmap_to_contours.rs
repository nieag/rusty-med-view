//! Labelmap to contour extraction utilities.
//!
//! Converts binary/label voxel masks into editable axial contour loops.

use crate::app::segment::{ContourSet, Plane3D, PlaneContour};
use crate::util::orientation::SlicePlane;
use std::collections::HashMap;

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

fn add_edge(edges: &mut HashMap<GridPoint, Vec<GridPoint>>, a: GridPoint, b: GridPoint) {
    edges.entry(a).or_default().push(b);
}

fn pop_next_edge(edges: &mut HashMap<GridPoint, Vec<GridPoint>>, p: GridPoint) -> Option<GridPoint> {
    let next = edges.get_mut(&p).and_then(|v| v.pop());
    if let Some(v) = edges.get(&p) {
        if v.is_empty() {
            edges.remove(&p);
        }
    }
    next
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

fn grid_to_world_xy(p: GridPoint, dims: [u32; 3], spacing: [f32; 3]) -> [f32; 2] {
    let max_x = dims[0].saturating_sub(1) as f32;
    let max_y = dims[1].saturating_sub(1) as f32;
    let x = ((p.0 as f32) - 0.5).clamp(0.0, max_x) * spacing[0];
    let y = ((p.1 as f32) - 0.5).clamp(0.0, max_y) * spacing[1];
    [x, y]
}

fn extract_slice_loops(mask: &[u8], dims: [u32; 3], spacing: [f32; 3], z: u32, label: Option<u8>) -> Vec<PlaneContour> {
    let w = dims[0] as i32;
    let h = dims[1] as i32;
    let mut edges: HashMap<GridPoint, Vec<GridPoint>> = HashMap::new();

    for y in 0..h {
        for x in 0..w {
            if !is_foreground(mask, dims, x, y, z, label) {
                continue;
            }

            // Clockwise boundary edges around foreground voxels.
            if !is_foreground(mask, dims, x - 1, y, z, label) {
                add_edge(&mut edges, (x, y), (x, y + 1));
            }
            if !is_foreground(mask, dims, x + 1, y, z, label) {
                add_edge(&mut edges, (x + 1, y + 1), (x + 1, y));
            }
            if !is_foreground(mask, dims, x, y - 1, z, label) {
                add_edge(&mut edges, (x + 1, y), (x, y));
            }
            if !is_foreground(mask, dims, x, y + 1, z, label) {
                add_edge(&mut edges, (x, y + 1), (x + 1, y + 1));
            }
        }
    }

    let z_world = (z as f32 + 0.5) * spacing[2];
    let mut contours = Vec::new();

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
            .map(|&p| grid_to_world_xy(p, dims, spacing))
            .collect();
        let poly2d = simplify_collinear(&poly2d_raw);
        if poly2d.len() < 3 {
            continue;
        }

        let points3d: Vec<[f32; 3]> = poly2d
            .iter()
            .map(|p| [p[0], p[1], z_world])
            .collect();

        contours.push(PlaneContour::with_points(
            Plane3D::from_axial(z_world),
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
        let loops = extract_slice_loops(mask, dims, spacing, z, label);
        for contour in loops {
            set.add_contour(SlicePlane::Axial, z as i32, contour);
        }
    }

    set
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let slice = contours.contours_at_slice(SlicePlane::Axial, 1).cloned().unwrap_or_default();
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
}
