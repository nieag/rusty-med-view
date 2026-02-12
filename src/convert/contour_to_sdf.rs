//! Contour to SDF conversion algorithms.
//!
//! Phase 4 implementation focuses on robust axis-aligned contours (axial/coronal/sagittal)
//! with finite extrusion along slice normal. Oblique contours are intentionally ignored here
//! and can be added in a dedicated follow-up.

use crate::app::segment::{ContourSet, PlaneContour, SdfVolume};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SdfBlendMode {
    Union,
}

#[derive(Debug, Clone, Copy)]
pub struct SdfBuildConfig {
    pub resolution_multiplier: f32,
    pub extrusion_scale: f32,
    pub blend_mode: SdfBlendMode,
    pub clamp_distance_mm: f32,
}

impl Default for SdfBuildConfig {
    fn default() -> Self {
        Self {
            resolution_multiplier: 1.0,
            extrusion_scale: 1.0,
            blend_mode: SdfBlendMode::Union,
            clamp_distance_mm: 16.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    X,
    Y,
    Z,
}

#[derive(Clone, Debug)]
struct AxisContour2d {
    axis: Axis,
    slice_mm: f32,
    poly: Vec<[f32; 2]>,
    min_2d: [f32; 2],
    max_2d: [f32; 2],
    half_thickness_mm: f32,
}

#[derive(Clone, Copy, Debug)]
struct IndexRange {
    x0: u32,
    x1: u32,
    y0: u32,
    y1: u32,
    z0: u32,
    z1: u32,
}

pub fn signed_distance_to_segment_2d(point: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let ap = [point[0] - a[0], point[1] - a[1]];
    let denom = ab[0] * ab[0] + ab[1] * ab[1];
    if denom <= 1e-12 {
        return (ap[0] * ap[0] + ap[1] * ap[1]).sqrt();
    }
    let t = ((ap[0] * ab[0] + ap[1] * ab[1]) / denom).clamp(0.0, 1.0);
    let c = [a[0] + t * ab[0], a[1] + t * ab[1]];
    let dx = point[0] - c[0];
    let dy = point[1] - c[1];
    (dx * dx + dy * dy).sqrt()
}

pub fn is_inside_polygon(point: [f32; 2], polygon: &[[f32; 2]]) -> bool {
    if polygon.len() < 3 {
        return false;
    }

    let mut crossings = 0;
    for i in 0..polygon.len() {
        let a = polygon[i];
        let b = polygon[(i + 1) % polygon.len()];
        let y_in_range = (a[1] <= point[1] && b[1] > point[1]) || (b[1] <= point[1] && a[1] > point[1]);
        if y_in_range {
            let t = (point[1] - a[1]) / (b[1] - a[1]);
            let x_intersect = a[0] + t * (b[0] - a[0]);
            if point[0] < x_intersect {
                crossings += 1;
            }
        }
    }
    crossings % 2 == 1
}

pub fn signed_distance_to_polygon_2d(point: [f32; 2], polygon: &[[f32; 2]]) -> f32 {
    if polygon.len() < 3 {
        return f32::MAX;
    }

    let mut min_dist = f32::MAX;
    for i in 0..polygon.len() {
        let a = polygon[i];
        let b = polygon[(i + 1) % polygon.len()];
        min_dist = min_dist.min(signed_distance_to_segment_2d(point, a, b));
    }

    if is_inside_polygon(point, polygon) {
        -min_dist
    } else {
        min_dist
    }
}

fn axis_from_plane(contour: &PlaneContour) -> Option<Axis> {
    let n = contour.plane.normal;
    if n[0].abs() > 0.9 {
        Some(Axis::X)
    } else if n[1].abs() > 0.9 {
        Some(Axis::Y)
    } else if n[2].abs() > 0.9 {
        Some(Axis::Z)
    } else {
        None
    }
}

fn project_point_for_axis(axis: Axis, p: [f32; 3]) -> [f32; 2] {
    match axis {
        Axis::X => [p[1], p[2]],
        Axis::Y => [p[0], p[2]],
        Axis::Z => [p[0], p[1]],
    }
}

fn axis_coord(axis: Axis, p: [f32; 3]) -> f32 {
    match axis {
        Axis::X => p[0],
        Axis::Y => p[1],
        Axis::Z => p[2],
    }
}

fn spacing_for_axis(axis: Axis, spacing: [f32; 3]) -> f32 {
    match axis {
        Axis::X => spacing[0],
        Axis::Y => spacing[1],
        Axis::Z => spacing[2],
    }
}

fn contour_to_axis_2d(contour: &PlaneContour, spacing: [f32; 3], cfg: &SdfBuildConfig) -> Option<AxisContour2d> {
    if contour.points.len() < 3 || !contour.is_closed {
        return None;
    }

    let axis = axis_from_plane(contour)?;
    let mut poly = Vec::with_capacity(contour.points.len());
    let mut min_2d = [f32::MAX, f32::MAX];
    let mut max_2d = [f32::MIN, f32::MIN];

    for &p in &contour.points {
        let q = project_point_for_axis(axis, p);
        min_2d[0] = min_2d[0].min(q[0]);
        min_2d[1] = min_2d[1].min(q[1]);
        max_2d[0] = max_2d[0].max(q[0]);
        max_2d[1] = max_2d[1].max(q[1]);
        poly.push(q);
    }

    let first_axis_coord = axis_coord(axis, contour.points[0]);
    let half_thickness_mm = 0.5 * spacing_for_axis(axis, spacing) * cfg.extrusion_scale.max(0.05);

    Some(AxisContour2d {
        axis,
        slice_mm: first_axis_coord,
        poly,
        min_2d,
        max_2d,
        half_thickness_mm,
    })
}

fn to_index_range(min_w: [f32; 3], max_w: [f32; 3], sdf: &SdfVolume) -> Option<IndexRange> {
    if min_w[0] > max_w[0] || min_w[1] > max_w[1] || min_w[2] > max_w[2] {
        return None;
    }

    let mut lo = [0i32; 3];
    let mut hi = [0i32; 3];
    for i in 0..3 {
        let start = ((min_w[i] - sdf.origin[i]) / sdf.spacing[i]).floor() as i32;
        let end = ((max_w[i] - sdf.origin[i]) / sdf.spacing[i]).ceil() as i32;
        lo[i] = start.clamp(0, sdf.dimensions[i] as i32 - 1);
        hi[i] = end.clamp(0, sdf.dimensions[i] as i32 - 1);
        if lo[i] > hi[i] {
            return None;
        }
    }

    Some(IndexRange {
        x0: lo[0] as u32,
        x1: hi[0] as u32,
        y0: lo[1] as u32,
        y1: hi[1] as u32,
        z0: lo[2] as u32,
        z1: hi[2] as u32,
    })
}

fn contour_roi(contour: &AxisContour2d, sdf: &SdfVolume, clamp_mm: f32) -> Option<IndexRange> {
    let e = clamp_mm.max(0.0);
    let (min_w, max_w) = match contour.axis {
        Axis::X => {
            (
                [
                    contour.slice_mm - contour.half_thickness_mm - e,
                    contour.min_2d[0] - e,
                    contour.min_2d[1] - e,
                ],
                [
                    contour.slice_mm + contour.half_thickness_mm + e,
                    contour.max_2d[0] + e,
                    contour.max_2d[1] + e,
                ],
            )
        }
        Axis::Y => {
            (
                [
                    contour.min_2d[0] - e,
                    contour.slice_mm - contour.half_thickness_mm - e,
                    contour.min_2d[1] - e,
                ],
                [
                    contour.max_2d[0] + e,
                    contour.slice_mm + contour.half_thickness_mm + e,
                    contour.max_2d[1] + e,
                ],
            )
        }
        Axis::Z => {
            (
                [
                    contour.min_2d[0] - e,
                    contour.min_2d[1] - e,
                    contour.slice_mm - contour.half_thickness_mm - e,
                ],
                [
                    contour.max_2d[0] + e,
                    contour.max_2d[1] + e,
                    contour.slice_mm + contour.half_thickness_mm + e,
                ],
            )
        }
    };

    to_index_range(min_w, max_w, sdf)
}

fn sd_extruded_polygon(world: [f32; 3], contour: &AxisContour2d) -> f32 {
    let point_2d = project_point_for_axis(contour.axis, world);
    let d2 = signed_distance_to_polygon_2d(point_2d, &contour.poly);
    let d_plane = (axis_coord(contour.axis, world) - contour.slice_mm).abs() - contour.half_thickness_mm;

    let qx = d2;
    let qy = d_plane;
    let out_x = qx.max(0.0);
    let out_y = qy.max(0.0);
    let outside = (out_x * out_x + out_y * out_y).sqrt();
    let inside = qx.max(qy).min(0.0);
    outside + inside
}

fn blend_values(a: f32, b: f32, mode: SdfBlendMode) -> f32 {
    match mode {
        SdfBlendMode::Union => a.min(b),
    }
}

pub fn build_sdf_from_contours_with_config(
    contours: &ContourSet,
    volume_dims: [u32; 3],
    volume_spacing: [f32; 3],
    cfg: SdfBuildConfig,
) -> SdfVolume {
    let rm = cfg.resolution_multiplier.max(0.1);
    let sdf_dims = [
        (volume_dims[0] as f32 * rm).round().max(1.0) as u32,
        (volume_dims[1] as f32 * rm).round().max(1.0) as u32,
        (volume_dims[2] as f32 * rm).round().max(1.0) as u32,
    ];
    let sdf_spacing = [volume_spacing[0] / rm, volume_spacing[1] / rm, volume_spacing[2] / rm];

    let mut sdf = SdfVolume::new(sdf_dims, sdf_spacing, [0.0, 0.0, 0.0]);

    let usable: Vec<_> = contours
        .all_contours()
        .filter_map(|c| contour_to_axis_2d(c, sdf_spacing, &cfg))
        .collect();

    if usable.is_empty() {
        return sdf;
    }

    for c in &usable {
        let Some(roi) = contour_roi(c, &sdf, cfg.clamp_distance_mm) else {
            continue;
        };

        for z in roi.z0..=roi.z1 {
            for y in roi.y0..=roi.y1 {
                for x in roi.x0..=roi.x1 {
                    let world = sdf.index_to_world([x, y, z]);
                    let mut d = sd_extruded_polygon(world, c);
                    if cfg.clamp_distance_mm.is_finite() {
                        d = d.clamp(-cfg.clamp_distance_mm, cfg.clamp_distance_mm);
                    }
                    let current = sdf.get(x, y, z);
                    sdf.set(x, y, z, blend_values(current, d, cfg.blend_mode));
                }
            }
        }
    }

    sdf
}

pub fn build_sdf_from_contours(
    contours: &ContourSet,
    volume_dims: [u32; 3],
    volume_spacing: [f32; 3],
    resolution_multiplier: f32,
) -> SdfVolume {
    build_sdf_from_contours_with_config(
        contours,
        volume_dims,
        volume_spacing,
        SdfBuildConfig {
            resolution_multiplier,
            ..SdfBuildConfig::default()
        },
    )
}

pub fn build_sdf_from_contours_fast(
    contours: &ContourSet,
    volume_dims: [u32; 3],
    volume_spacing: [f32; 3],
    resolution_multiplier: f32,
) -> SdfVolume {
    build_sdf_from_contours(contours, volume_dims, volume_spacing, resolution_multiplier)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::segment::Plane3D;
    use crate::util::orientation::SlicePlane;

    #[test]
    fn test_signed_distance_to_polygon_inside_outside() {
        let poly = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        assert!(signed_distance_to_polygon_2d([5.0, 5.0], &poly) < 0.0);
        assert!(signed_distance_to_polygon_2d([20.0, 5.0], &poly) > 0.0);
    }

    #[test]
    fn test_blend_union_is_true_min() {
        assert_eq!(blend_values(-2.0, 1.0, SdfBlendMode::Union), -2.0);
        assert_eq!(blend_values(3.0, -0.5, SdfBlendMode::Union), -0.5);
    }

    #[test]
    fn test_extrusion_limited_along_slice_normal() {
        let mut contours = ContourSet::new();
        let square = PlaneContour::with_points(
            Plane3D::from_axial(5.0),
            vec![[2.0, 2.0, 5.0], [8.0, 2.0, 5.0], [8.0, 8.0, 5.0], [2.0, 8.0, 5.0]],
            true,
        );
        contours.add_contour(SlicePlane::Axial, 5, square);

        let cfg = SdfBuildConfig {
            resolution_multiplier: 1.0,
            extrusion_scale: 1.0,
            blend_mode: SdfBlendMode::Union,
            clamp_distance_mm: 32.0,
        };
        let sdf = build_sdf_from_contours_with_config(&contours, [12, 12, 12], [1.0, 1.0, 1.0], cfg);

        let near = sdf.get(5, 5, 5);
        let far = sdf.get(5, 5, 10);
        assert!(near < 0.0, "near plane expected inside: {near}");
        assert!(far > 0.0, "far from plane expected outside: {far}");
    }

    #[test]
    fn test_spacing_affects_extrusion_thickness() {
        let mut contours = ContourSet::new();
        let square = PlaneContour::with_points(
            Plane3D::from_axial(10.0),
            vec![[6.0, 6.0, 10.0], [14.0, 6.0, 10.0], [14.0, 14.0, 10.0], [6.0, 14.0, 10.0]],
            true,
        );
        contours.add_contour(SlicePlane::Axial, 5, square);

        let sdf_thick = build_sdf_from_contours_with_config(
            &contours,
            [24, 24, 24],
            [1.0, 1.0, 2.0],
            SdfBuildConfig {
                clamp_distance_mm: 64.0,
                ..SdfBuildConfig::default()
            },
        );

        let sdf_thin = build_sdf_from_contours_with_config(
            &contours,
            [24, 24, 24],
            [1.0, 1.0, 1.0],
            SdfBuildConfig {
                clamp_distance_mm: 64.0,
                ..SdfBuildConfig::default()
            },
        );

        // Compare roughly the same world-space Z (~12mm): thin idx=12 (1mm), thick idx=6 (2mm)
        let d_thin = sdf_thin.get(10, 10, 12);
        let d_thick = sdf_thick.get(10, 10, 6);
        assert!(
            d_thick < d_thin,
            "expected thicker slab with larger spacing: thick={d_thick} thin={d_thin}"
        );
    }

    #[test]
    fn test_oblique_ignored_in_p4() {
        let mut contours = ContourSet::new();
        contours.add_oblique_contour(PlaneContour::with_points(
            Plane3D {
                normal: [0.707, 0.0, 0.707],
                distance: 10.0,
            },
            vec![[1.0, 1.0, 1.0], [2.0, 1.0, 2.0], [2.0, 2.0, 2.0]],
            true,
        ));

        let sdf = build_sdf_from_contours(&contours, [8, 8, 8], [1.0, 1.0, 1.0], 1.0);
        assert_eq!(sdf.get(4, 4, 4), f32::MAX);
    }
}
