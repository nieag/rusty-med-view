//! Contour drawing system.
//!
//! This module provides freehand contour drawing in 2D views,
//! converting screen-space points to 3D world-space contours.

use crate::app::segment::{Plane3D, PlaneContour};
use crate::util::orientation::SlicePlane;

// ============================================================================
// Contour Drawing State
// ============================================================================

/// State machine for contour drawing interaction.
#[derive(Debug, Clone, Default)]
pub enum ContourDrawState {
    /// Not currently drawing
    #[default]
    Idle,
    /// Actively drawing a contour
    Drawing {
        /// Screen UV points being collected
        points: Vec<[f32; 2]>,
        /// Which slice plane we're drawing on
        slice_plane: SlicePlane,
        /// The slice index (voxel coordinate)
        slice_index: i32,
    },
}

impl ContourDrawState {
    /// Check if currently drawing
    pub fn is_drawing(&self) -> bool {
        matches!(self, ContourDrawState::Drawing { .. })
    }

    /// Get the current points if drawing
    pub fn points(&self) -> Option<&Vec<[f32; 2]>> {
        match self {
            ContourDrawState::Drawing { points, .. } => Some(points),
            ContourDrawState::Idle => None,
        }
    }
}

// ============================================================================
// Screen to World Conversion
// ============================================================================

/// Convert screen UV to world-space position on a slice plane.
///
/// Uses radiological convention (X flipped for axial/coronal).
pub fn screen_uv_to_world(
    uv: [f32; 2],
    slice_plane: SlicePlane,
    slice_index: i32,
    volume_dims: [u32; 3],
    volume_spacing: [f32; 3],
) -> [f32; 3] {
    // Get volume UV using radiological convention (from orientation.rs)
    let volume_uv = slice_plane.screen_uv_to_volume(
        uv,
        (slice_index as f32 + 0.5) / volume_dims[slice_plane.depth_axis()] as f32,
    );

    // Convert volume UV [0,1] to world coordinates
    [
        volume_uv[0] * (volume_dims[0] as f32 - 1.0) * volume_spacing[0],
        volume_uv[1] * (volume_dims[1] as f32 - 1.0) * volume_spacing[1],
        volume_uv[2] * (volume_dims[2] as f32 - 1.0) * volume_spacing[2],
    ]
}

/// Convert a sequence of screen UV points to a 3D PlaneContour.
pub fn screen_points_to_plane_contour(
    screen_points: &[[f32; 2]],
    slice_plane: SlicePlane,
    slice_index: i32,
    volume_dims: [u32; 3],
    volume_spacing: [f32; 3],
) -> PlaneContour {
    // Compute the plane position in world space
    let depth_axis = slice_plane.depth_axis();
    let plane_position = (slice_index as f32 + 0.5) * volume_spacing[depth_axis];

    let plane = Plane3D::from_slice_plane(slice_plane, plane_position);

    // Convert all screen points to 3D
    let points_3d: Vec<[f32; 3]> = screen_points
        .iter()
        .map(|&uv| screen_uv_to_world(uv, slice_plane, slice_index, volume_dims, volume_spacing))
        .collect();

    PlaneContour::with_points(plane, points_3d, true) // Default to closed
}

// ============================================================================
// Contour Processing
// ============================================================================

/// Smooth a contour using Catmull-Rom spline interpolation.
///
/// # Arguments
/// * `points` - Input 3D points (forms a closed loop)
/// * `segments_per_edge` - Number of interpolated points per original edge
///
/// # Returns
/// Smoothed contour with more points
pub fn smooth_contour(
    points: &[[f32; 3]],
    segments_per_edge: u32,
    is_closed: bool,
) -> Vec<[f32; 3]> {
    if points.len() < 3 {
        return points.to_vec();
    }

    let mut result = Vec::with_capacity(points.len() * segments_per_edge as usize);
    let n = points.len();

    // Loop through each edge
    // If closed: n edges (0->1, ..., n-1 -> 0)
    // If open: n-1 edges (0->1, ..., n-2 -> n-1)
    let edge_count = if is_closed { n } else { n - 1 };

    for i in 0..edge_count {
        let (p0, p1, p2, p3) = if is_closed {
            (
                points[(i + n - 1) % n],
                points[i],
                points[(i + 1) % n],
                points[(i + 2) % n],
            )
        } else {
            (
                if i == 0 { points[0] } else { points[i - 1] },
                points[i],
                points[i + 1],
                if i + 2 < n {
                    points[i + 2]
                } else {
                    points[n - 1]
                },
            )
        };

        for j in 0..segments_per_edge {
            let t = j as f32 / segments_per_edge as f32;
            result.push(catmull_rom(p0, p1, p2, p3, t));
        }
    }

    // For open paths, add the final point
    if !is_closed {
        if let Some(&last) = points.last() {
            result.push(last);
        }
    }

    result
}

/// Catmull-Rom spline interpolation between p1 and p2.
fn catmull_rom(p0: [f32; 3], p1: [f32; 3], p2: [f32; 3], p3: [f32; 3], t: f32) -> [f32; 3] {
    let t2 = t * t;
    let t3 = t2 * t;

    let mut result = [0.0; 3];
    for i in 0..3 {
        result[i] = 0.5
            * (2.0 * p1[i]
                + (-p0[i] + p2[i]) * t
                + (2.0 * p0[i] - 5.0 * p1[i] + 4.0 * p2[i] - p3[i]) * t2
                + (-p0[i] + 3.0 * p1[i] - 3.0 * p2[i] + p3[i]) * t3);
    }
    result
}

/// Close a contour if endpoints are within threshold distance.
///
/// Returns true if the contour was closed (last point removed).
pub fn maybe_close_contour(points: &mut Vec<[f32; 3]>, threshold: f32) -> bool {
    if points.len() < 3 {
        return false;
    }

    let first = points[0];
    let last = points[points.len() - 1];
    let dist = ((first[0] - last[0]).powi(2)
        + (first[1] - last[1]).powi(2)
        + (first[2] - last[2]).powi(2))
    .sqrt();

    if dist < threshold {
        points.pop();
        true
    } else {
        false
    }
}

/// Simplify a contour using the Ramer-Douglas-Peucker algorithm.
pub fn simplify_rdp(points: &[[f32; 3]], epsilon: f32) -> Vec<[f32; 3]> {
    if points.len() < 3 {
        return points.to_vec();
    }

    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;

    fn find_furthest(points: &[[f32; 3]], start: usize, end: usize, epsilon: f32) -> Option<usize> {
        let p0 = points[start];
        let p1 = points[end];
        let mut max_dist = 0.0;
        let mut index = None;

        let line_vec = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let line_len_sq =
            line_vec[0] * line_vec[0] + line_vec[1] * line_vec[1] + line_vec[2] * line_vec[2];

        for i in (start + 1)..end {
            let p = points[i];
            let dist = if line_len_sq < 1e-6 {
                // Points coincide, use distance to start
                let dx = p[0] - p0[0];
                let dy = p[1] - p0[1];
                let dz = p[2] - p0[2];
                (dx * dx + dy * dy + dz * dz).sqrt()
            } else {
                // Perpendicular distance to line
                let t = ((p[0] - p0[0]) * line_vec[0]
                    + (p[1] - p0[1]) * line_vec[1]
                    + (p[2] - p0[2]) * line_vec[2])
                    / line_len_sq;
                let t = t.clamp(0.0, 1.0);
                let proj = [
                    p0[0] + t * line_vec[0],
                    p0[1] + t * line_vec[1],
                    p0[2] + t * line_vec[2],
                ];
                let dx = p[0] - proj[0];
                let dy = p[1] - proj[1];
                let dz = p[2] - proj[2];
                (dx * dx + dy * dy + dz * dz).sqrt()
            };

            if dist > max_dist {
                max_dist = dist;
                index = Some(i);
            }
        }

        if max_dist > epsilon {
            index
        } else {
            None
        }
    }

    fn rdp_recursive(
        points: &[[f32; 3]],
        start: usize,
        end: usize,
        epsilon: f32,
        keep: &mut [bool],
    ) {
        if end <= start + 1 {
            return;
        }
        if let Some(furthest) = find_furthest(points, start, end, epsilon) {
            keep[furthest] = true;
            rdp_recursive(points, start, furthest, epsilon, keep);
            rdp_recursive(points, furthest, end, epsilon, keep);
        }
    }

    rdp_recursive(points, 0, points.len() - 1, epsilon, &mut keep);

    let mut result = Vec::with_capacity(points.len());
    for (i, &k) in keep.iter().enumerate() {
        if k {
            result.push(points[i]);
        }
    }
    result
}

/// Resample a contour to a fixed spatial interval.
pub fn resample_contour(points: &[[f32; 3]], interval: f32, is_closed: bool) -> Vec<[f32; 3]> {
    if points.len() < 2 || interval <= 0.0 {
        return points.to_vec();
    }

    let mut result = vec![points[0]];
    let mut current_pos = points[0];
    let mut next_point_idx = 1;
    let n = points.len();
    let count = if is_closed { n + 1 } else { n };

    while next_point_idx < count {
        let p_next = points[next_point_idx % n];
        let dx = p_next[0] - current_pos[0];
        let dy = p_next[1] - current_pos[1];
        let dz = p_next[2] - current_pos[2];
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();

        if dist >= interval {
            // Place a point at 'interval' distance along this segment
            let t = interval / dist;
            current_pos = [
                current_pos[0] + dx * t,
                current_pos[1] + dy * t,
                current_pos[2] + dz * t,
            ];
            result.push(current_pos);
            // Don't increment next_point_idx yet, might need another point on this same segment
        } else {
            // Skip to next segment
            current_pos = p_next;
            next_point_idx += 1;
        }
    }

    // Ensure the last point is exactly the original end point if it's not closed
    if !is_closed && result.len() > 1 {
        *result.last_mut().unwrap() = *points.last().unwrap();
    }

    result
}

/// Perform a full optimization pass to keep point counts stable and smooth.
/// Deduplicate -> Simplify(0.05mm) -> Smooth(4x) -> Resample(0.5mm)
pub fn restabilize_contour(points: &[[f32; 3]], is_closed: bool) -> Vec<[f32; 3]> {
    // 1. Remove micro-duplicates (<0.01mm)
    let dedup = remove_duplicates(points);
    // 2. Prune redundant points very close to the line (0.05mm tolerance)
    let simplified = simplify_rdp(&dedup, 0.05);
    // 3. Add curvature (4x interpolation)
    let smoothed = smooth_contour(&simplified, 4, is_closed);
    // 4. Resample to a fixed density (0.5mm points)
    resample_contour(&smoothed, 0.5, is_closed)
}

/// Less aggressive stabilization for sculpted contours to reduce spline overshoot spikes.
pub fn restabilize_contour_for_sculpt(points: &[[f32; 3]], is_closed: bool) -> Vec<[f32; 3]> {
    let dedup = remove_duplicates(points);
    let simplified = simplify_rdp(&dedup, 0.1);
    // Sculpt edits are spike-prone around welds; avoid spline overshoot here.
    resample_contour(&simplified, 0.75, is_closed)
}

/// Remove adjacent points that are extremely close to each other (<0.01mm)
pub fn remove_duplicates(points: &[[f32; 3]]) -> Vec<[f32; 3]> {
    if points.len() < 2 {
        return points.to_vec();
    }
    let mut result = Vec::with_capacity(points.len());
    result.push(points[0]);
    for i in 1..points.len() {
        let p1 = points[i - 1];
        let p2 = points[i];
        let dx = p2[0] - p1[0];
        let dy = p2[1] - p1[1];
        let dz = p2[2] - p1[2];
        let d_sq = dx * dx + dy * dy + dz * dz;
        // 0.01mm threshold (1e-4 distance)
        if d_sq > 1e-6 {
            result.push(p2);
        }
    }
    result
}

#[inline]
fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

fn turn_angle_deg(prev: [f32; 3], curr: [f32; 3], next: [f32; 3]) -> Option<f32> {
    let v1 = [prev[0] - curr[0], prev[1] - curr[1], prev[2] - curr[2]];
    let v2 = [next[0] - curr[0], next[1] - curr[1], next[2] - curr[2]];
    let len1 = (v1[0] * v1[0] + v1[1] * v1[1] + v1[2] * v1[2]).sqrt();
    let len2 = (v2[0] * v2[0] + v2[1] * v2[1] + v2[2] * v2[2]).sqrt();
    if len1 < 1e-6 || len2 < 1e-6 {
        return None;
    }
    let dot = (v1[0] * v2[0] + v1[1] * v2[1] + v1[2] * v2[2]) / (len1 * len2);
    Some(dot.clamp(-1.0, 1.0).acos().to_degrees())
}

fn has_sharp_turn(points: &[[f32; 3]], min_turn_angle_deg: f32) -> bool {
    if points.len() < 3 {
        return true;
    }
    let n = points.len();
    for i in 0..n {
        let prev = points[(i + n - 1) % n];
        let curr = points[i];
        let next = points[(i + 1) % n];
        if let Some(angle) = turn_angle_deg(prev, curr, next) {
            if angle < min_turn_angle_deg {
                return true;
            }
        }
    }
    false
}

fn sanitize_closed_contour(
    points: &[[f32; 3]],
    min_segment_len: f32,
    min_turn_angle_deg: f32,
) -> Vec<[f32; 3]> {
    if points.len() < 3 {
        return points.to_vec();
    }

    let mut out = points.to_vec();
    for _ in 0..3 {
        if out.len() < 3 {
            break;
        }
        let n = out.len();
        let mut keep = vec![true; n];
        let mut removed_any = false;
        for i in 0..n {
            let prev = out[(i + n - 1) % n];
            let curr = out[i];
            let next = out[(i + 1) % n];
            if dist3(prev, curr) < min_segment_len || dist3(curr, next) < min_segment_len {
                keep[i] = false;
                removed_any = true;
                continue;
            }
            if let Some(angle) = turn_angle_deg(prev, curr, next) {
                if angle < min_turn_angle_deg {
                    keep[i] = false;
                    removed_any = true;
                }
            }
        }
        if !removed_any {
            break;
        }
        let filtered: Vec<[f32; 3]> = out
            .iter()
            .enumerate()
            .filter_map(|(i, &p)| keep[i].then_some(p))
            .collect();
        if filtered.len() < 3 {
            break;
        }
        out = filtered;
    }
    out
}

fn preprocess_stroke_for_sculpt(stroke: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let dedup = remove_duplicates(stroke);
    if dedup.len() < 3 {
        return dedup;
    }
    simplify_rdp(&dedup, 0.25)
}

fn project_to_2d_by_normal(point: [f32; 3], normal: [f32; 3]) -> [f32; 2] {
    let ax = normal[0].abs();
    let ay = normal[1].abs();
    let az = normal[2].abs();
    if ax >= ay && ax >= az {
        [point[1], point[2]]
    } else if ay >= az {
        [point[0], point[2]]
    } else {
        [point[0], point[1]]
    }
}

fn orient2d(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn on_segment2d(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> bool {
    let min_x = a[0].min(b[0]) - 1e-6;
    let max_x = a[0].max(b[0]) + 1e-6;
    let min_y = a[1].min(b[1]) - 1e-6;
    let max_y = a[1].max(b[1]) + 1e-6;
    p[0] >= min_x && p[0] <= max_x && p[1] >= min_y && p[1] <= max_y
}

fn segments_intersect_2d(a1: [f32; 2], a2: [f32; 2], b1: [f32; 2], b2: [f32; 2]) -> bool {
    let o1 = orient2d(a1, a2, b1);
    let o2 = orient2d(a1, a2, b2);
    let o3 = orient2d(b1, b2, a1);
    let o4 = orient2d(b1, b2, a2);

    if o1.abs() < 1e-6 && on_segment2d(a1, a2, b1) {
        return true;
    }
    if o2.abs() < 1e-6 && on_segment2d(a1, a2, b2) {
        return true;
    }
    if o3.abs() < 1e-6 && on_segment2d(b1, b2, a1) {
        return true;
    }
    if o4.abs() < 1e-6 && on_segment2d(b1, b2, a2) {
        return true;
    }

    (o1 > 0.0) != (o2 > 0.0) && (o3 > 0.0) != (o4 > 0.0)
}

fn has_self_intersection_closed(points: &[[f32; 3]], normal: [f32; 3]) -> bool {
    if points.len() < 4 {
        return false;
    }
    let n = points.len();
    let p2d: Vec<[f32; 2]> = points
        .iter()
        .map(|&p| project_to_2d_by_normal(p, normal))
        .collect();
    for i in 0..n {
        let i2 = (i + 1) % n;
        for j in (i + 1)..n {
            let j2 = (j + 1) % n;
            if i == j || i == j2 || i2 == j || i2 == j2 {
                continue;
            }
            if i == 0 && j2 == 0 {
                continue;
            }
            if segments_intersect_2d(p2d[i], p2d[i2], p2d[j], p2d[j2]) {
                return true;
            }
        }
    }
    false
}

fn estimate_contour_normal(points: &[[f32; 3]]) -> [f32; 3] {
    if points.len() < 3 {
        return [0.0, 0.0, 1.0];
    }
    for i in 0..(points.len() - 2) {
        let a = points[i];
        let b = points[i + 1];
        let c = points[i + 2];
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - b[0], c[1] - b[1], c[2] - b[2]];
        let n = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if len > 1e-6 {
            return [n[0] / len, n[1] / len, n[2] / len];
        }
    }
    [0.0, 0.0, 1.0]
}

/// Check if a 2D screen point should be added to the current contour.
///
/// Returns true if the point is far enough from the last point.
pub fn should_add_point(last_point: [f32; 2], new_point: [f32; 2], min_distance: f32) -> bool {
    let dx = new_point[0] - last_point[0];
    let dy = new_point[1] - last_point[1];
    (dx * dx + dy * dy).sqrt() >= min_distance
}

/// Find the nearest point index on a contour to a target point.
pub fn find_nearest_point_on_contour(points: &[[f32; 3]], target: [f32; 3]) -> (usize, f32) {
    let mut min_dist_sq = f32::MAX;
    let mut best_idx = 0;
    for (i, &p) in points.iter().enumerate() {
        let dx = p[0] - target[0];
        let dy = p[1] - target[1];
        let dz = p[2] - target[2];
        let d_sq = dx * dx + dy * dy + dz * dz;
        if d_sq < min_dist_sq {
            min_dist_sq = d_sq;
            best_idx = i;
        }
    }
    (best_idx, min_dist_sq.sqrt())
}

/// Merge a new stroke with an existing closed contour.
///
/// If both endpoints of the stroke are within `snap_threshold` of the existing contour,
/// it will replace the shorter path between the snap points with the new stroke.
///
/// Returns true if sculpting was successful.
pub fn sculpt_contour(
    existing: &mut Vec<[f32; 3]>,
    stroke: &[[f32; 3]],
    snap_threshold: f32,
) -> bool {
    let stroke = preprocess_stroke_for_sculpt(stroke);
    let n = existing.len();
    if n < 3 || stroke.len() < 2 {
        return false;
    }

    // 1. Find the sub-stroke that enters and leaves proximity
    let mut stroke_start_idx = None;
    let mut stroke_end_idx = None;
    let mut loop_snap_start = 0;
    let mut loop_snap_end = 0;

    for (i, &s_point) in stroke.iter().enumerate() {
        let (best_idx, dist) = find_nearest_point_on_contour(existing, s_point);
        if dist <= snap_threshold {
            stroke_start_idx = Some(i);
            loop_snap_start = best_idx;
            break;
        }
    }

    for (i, &s_point) in stroke.iter().enumerate().rev() {
        let (best_idx, dist) = find_nearest_point_on_contour(existing, s_point);
        if dist <= snap_threshold {
            stroke_end_idx = Some(i);
            loop_snap_end = best_idx;
            break;
        }
    }

    let (s_i, s_j) = match (stroke_start_idx, stroke_end_idx) {
        (Some(i), Some(j)) if i < j => (i, j),
        _ => return false,
    };

    // 2. Determine which loop path to replace
    let (idx_start, idx_end) = (loop_snap_start, loop_snap_end);
    let path1_len = if idx_end >= idx_start {
        idx_end - idx_start
    } else {
        (n - idx_start) + idx_end
    };
    let path2_len = n - path1_len;
    if path1_len.min(path2_len) < 2 {
        return false;
    }

    let (start, end) = if path1_len <= path2_len {
        (idx_start, idx_end)
    } else {
        (idx_end, idx_start)
    };

    // 3. Reconstruct the contour with exact weld at loop vertices
    // We replace existing[start..end] with stroke[s_i..s_j]
    let mut new_points = Vec::with_capacity(n + (s_j - s_i + 1));

    // Add the "long" part of the existing loop
    let mut curr = end;
    while curr != start {
        new_points.push(existing[curr]);
        curr = (curr + 1) % n;
    }
    // EXACT WELD: Start the new segment exactly at the loop vertex
    new_points.push(existing[start]);

    // Insert the internal stroke points (excluding the ends to avoid double-overlap)
    let sub_stroke = &stroke[s_i..=s_j];
    let start_snap_dist = dist3(existing[start], sub_stroke[0])
        .min(dist3(existing[start], sub_stroke[sub_stroke.len() - 1]));
    let end_snap_dist =
        dist3(existing[end], sub_stroke[0]).min(dist3(existing[end], sub_stroke[sub_stroke.len() - 1]));
    if start_snap_dist > snap_threshold * 1.5 || end_snap_dist > snap_threshold * 1.5 {
        return false;
    }

    // Check winding/direction
    let d_start = {
        let p = new_points[new_points.len() - 1];
        let s = sub_stroke[0];
        (p[0] - s[0]).powi(2) + (p[1] - s[1]).powi(2) + (p[2] - s[2]).powi(2)
    };
    let d_end = {
        let p = new_points[new_points.len() - 1];
        let s = sub_stroke[sub_stroke.len() - 1];
        (p[0] - s[0]).powi(2) + (p[1] - s[1]).powi(2) + (p[2] - s[2]).powi(2)
    };

    if d_start <= d_end {
        if sub_stroke.len() > 2 {
            for &p in &sub_stroke[1..sub_stroke.len() - 1] {
                new_points.push(p);
            }
        }
    } else {
        if sub_stroke.len() > 2 {
            for &p in sub_stroke[1..sub_stroke.len() - 1].iter().rev() {
                new_points.push(p);
            }
        }
    }

    let sanitized = sanitize_closed_contour(&new_points, 0.2, 12.0);
    let normal = estimate_contour_normal(&sanitized);
    if sanitized.len() < 3
        || has_sharp_turn(&sanitized, 8.0)
        || has_self_intersection_closed(&sanitized, normal)
    {
        return false;
    }
    *existing = sanitized;
    true
}

/// Bridge two separate contours using a connecting stroke.
///
/// Reconstructs a single loop: A[start..start] -> Stroke -> B[end..end] -> Stroke(rev)
pub fn bridge_contours(
    contour_a: &mut Vec<[f32; 3]>,
    contour_b: &[[f32; 3]],
    stroke: &[[f32; 3]],
    snap_threshold: f32,
) -> bool {
    let stroke = preprocess_stroke_for_sculpt(stroke);
    let n_a = contour_a.len();
    let n_b = contour_b.len();
    if n_a < 3 || n_b < 3 || stroke.len() < 2 {
        return false;
    }

    // 1. Find best sub-stroke and loop snap indices
    let mut stroke_start_idx = None;
    let mut loop_a_idx = 0;
    for (i, &s_point) in stroke.iter().enumerate() {
        let (best_idx, dist) = find_nearest_point_on_contour(contour_a, s_point);
        if dist <= snap_threshold {
            stroke_start_idx = Some(i);
            loop_a_idx = best_idx;
            break;
        }
    }

    let mut stroke_end_idx = None;
    let mut loop_b_idx = 0;
    for (i, &s_point) in stroke.iter().enumerate().rev() {
        let (best_idx, dist) = find_nearest_point_on_contour(contour_b, s_point);
        if dist <= snap_threshold {
            stroke_end_idx = Some(i);
            loop_b_idx = best_idx;
            break;
        }
    }

    let (s_i, s_j) = match (stroke_start_idx, stroke_end_idx) {
        (Some(i), Some(j)) if i < j => (i, j),
        _ => return false,
    };

    // 2. Reconstruct: A(loop) -> Stroke -> B(loop) -> Stroke(rev)
    let sub_stroke = &stroke[s_i..=s_j];
    let mut new_points = Vec::with_capacity(n_a + n_b + sub_stroke.len() * 2);

    // Full loop A (starting/ending at snap)
    // The loop 0..n_a already includes loop_a_idx
    for i in 0..n_a {
        new_points.push(contour_a[(loop_a_idx + i) % n_a]);
    }

    // Forward stroke to B
    // Weld A[loop_a_idx] to B[loop_b_idx]
    if sub_stroke.len() > 2 {
        for &p in &sub_stroke[1..sub_stroke.len() - 1] {
            new_points.push(p);
        }
    }

    // Full loop B (starting/ending at snap)
    for i in 0..n_b {
        new_points.push(contour_b[(loop_b_idx + i) % n_b]);
    }

    // Backward stroke back to A
    if sub_stroke.len() > 2 {
        for &p in sub_stroke[1..sub_stroke.len() - 1].iter().rev() {
            new_points.push(p);
        }
    }

    let sanitized = sanitize_closed_contour(&new_points, 0.2, 12.0);
    let normal = estimate_contour_normal(&sanitized);
    if sanitized.len() < 3
        || has_sharp_turn(&sanitized, 8.0)
        || has_self_intersection_closed(&sanitized, normal)
    {
        return false;
    }
    *contour_a = sanitized;
    true
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contour_draw_state_default() {
        let state = ContourDrawState::default();
        assert!(!state.is_drawing());
        assert!(state.points().is_none());
    }

    #[test]
    fn test_contour_draw_state_drawing() {
        let state = ContourDrawState::Drawing {
            points: vec![[0.5, 0.5]],
            slice_plane: SlicePlane::Axial,
            slice_index: 25,
        };
        assert!(state.is_drawing());
        assert_eq!(state.points().unwrap().len(), 1);
    }

    #[test]
    fn test_screen_to_world_axial_center() {
        // Center of screen should map to center of volume
        let uv = [0.5, 0.5];
        let dims = [100, 100, 50];
        let spacing = [1.0, 1.0, 2.0];

        let world = screen_uv_to_world(uv, SlicePlane::Axial, 25, dims, spacing);

        // With radiological convention, center maps to center
        assert!((world[0] - 49.5).abs() < 1.0, "X: {}", world[0]);
        assert!((world[1] - 49.5).abs() < 1.0, "Y: {}", world[1]);
        // Z should be at slice 25 * spacing 2.0 = ~51
        assert!((world[2] - 51.0).abs() < 2.0, "Z: {}", world[2]);
    }

    #[test]
    fn test_screen_points_to_plane_contour() {
        let points = vec![[0.3, 0.3], [0.7, 0.3], [0.7, 0.7], [0.3, 0.7]];
        let dims = [100, 100, 50];
        let spacing = [1.0, 1.0, 1.0];

        let contour = screen_points_to_plane_contour(&points, SlicePlane::Axial, 25, dims, spacing);

        assert_eq!(contour.points.len(), 4);
        assert!(contour.is_closed);
        assert!(contour.is_valid());
    }

    #[test]
    fn test_smooth_contour_square() {
        let square = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        let smoothed = smooth_contour(&square, 4, true);

        // 4 edges * 4 segments = 16 points
        assert_eq!(smoothed.len(), 16);
    }

    #[test]
    fn test_smooth_contour_too_few_points() {
        let points = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let smoothed = smooth_contour(&points, 4, false);

        // Should return original points
        assert_eq!(smoothed.len(), 2);
    }

    #[test]
    fn test_catmull_rom_at_t0() {
        // At t=0, should return p1
        let result = catmull_rom(
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            0.0,
        );
        assert!((result[0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_catmull_rom_at_t1() {
        // At t=1, should return p2
        let result = catmull_rom(
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            1.0,
        );
        assert!((result[0] - 2.0).abs() < 1e-6);
    }

    #[test]
    fn test_maybe_close_contour_near() {
        let mut points = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.01, 0.01, 0.0], // Very close to first point
        ];
        let closed = maybe_close_contour(&mut points, 0.05);
        assert!(closed);
        assert_eq!(points.len(), 3);
    }

    #[test]
    fn test_maybe_close_contour_far() {
        let mut points = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.5, 0.5, 0.0], // Not close to first point
        ];
        let closed = maybe_close_contour(&mut points, 0.05);
        assert!(!closed);
        assert_eq!(points.len(), 4);
    }

    #[test]
    fn test_maybe_close_contour_threshold_edge_not_closed() {
        let mut points = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.05, 0.0, 0.0], // Exactly threshold away from first point
        ];
        let closed = maybe_close_contour(&mut points, 0.05);
        assert!(!closed);
        assert_eq!(points.len(), 4);
    }

    #[test]
    fn test_sanitize_closed_contour_removes_spike() {
        let points = vec![
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [2.001, 0.001, 0.0], // tiny near-duplicate spike
            [2.0, 2.0, 0.0],
            [0.0, 2.0, 0.0],
        ];
        let sanitized = sanitize_closed_contour(&points, 0.2, 12.0);
        assert!(sanitized.len() < points.len());
    }

    #[test]
    fn test_simplify_rdp() {
        let points = vec![
            [0.0, 0.0, 0.0],
            [0.5, 0.001, 0.0], // Nearly on the line, should be removed
            [1.0, 0.0, 0.0],
            [1.5, 0.5, 0.0], // Peak
            [2.0, 0.0, 0.0],
        ];
        let simplified = simplify_rdp(&points, 0.1);

        // Should keep 0, 2, 3, 4 (p1 is removed because it's only 0.001 away from the line p0-p1)
        assert_eq!(simplified.len(), 4);
        assert_eq!(simplified[0], [0.0, 0.0, 0.0]);
        assert_eq!(simplified[1], [1.0, 0.0, 0.0]);
    }

    #[test]
    fn test_should_add_point() {
        assert!(should_add_point([0.0, 0.0], [0.1, 0.0], 0.05));
        assert!(!should_add_point([0.0, 0.0], [0.01, 0.0], 0.05));
    }
}
