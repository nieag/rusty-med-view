use crate::app::roi::{ContourData, VoxelData, VoxelGeometry};
use crate::convert::{
    nearest_depth_layer, orthogonal_depth_axis, plane_local_mm_to_world_mm,
    voxel_index_to_world_mm, world_mm_to_plane_local_mm, world_mm_to_voxel_index, PlaneDefinition,
    PlaneFamily,
};
use glam::Vec3;

const EPSILON: f32 = 1e-6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContourRasterizationError {
    InvalidTargetGeometry,
    InvalidSlicePlane { slice_index: usize },
}

/// Returns max-exclusive voxel bounds affected by the supplied contour slices.
/// Orthogonal slices map to their exact voxel slabs. Oblique slices remain a
/// conservative full-volume invalidation until oblique rasterization is chunked.
pub fn contour_slices_voxel_aabb(
    contour: &ContourData,
    target_geometry: VoxelGeometry,
) -> Option<([u32; 3], [u32; 3])> {
    if contour.slices.is_empty() || target_geometry.dimensions.contains(&0) {
        return None;
    }
    if contour.active_plane_family == PlaneFamily::Oblique {
        return Some(([0, 0, 0], target_geometry.dimensions));
    }

    let depth_axis = match contour.active_plane_family {
        PlaneFamily::Axial => 2,
        PlaneFamily::Coronal => 1,
        PlaneFamily::Sagittal => 0,
        PlaneFamily::Oblique => unreachable!(),
    };
    let dimension = target_geometry.dimensions[depth_axis];
    let mut first_depth = dimension;
    let mut last_depth = 0;
    for slice in &contour.slices {
        let voxel_index = world_mm_to_voxel_index(slice.plane.origin_mm, target_geometry);
        let depth = voxel_index[depth_axis]
            .round()
            .clamp(0.0, dimension.saturating_sub(1) as f32) as u32;
        first_depth = first_depth.min(depth);
        last_depth = last_depth.max(depth);
    }

    let mut min = [0, 0, 0];
    let mut max = target_geometry.dimensions;
    min[depth_axis] = first_depth;
    max[depth_axis] = last_depth.saturating_add(1).min(dimension);
    Some((min, max))
}

/// Tight max-exclusive bounds for contour loop geometry. Callers rebuilding an
/// edit must merge bounds from both the committed and preview contours so erased
/// voxels are included.
pub fn contour_geometry_voxel_aabb(
    contour: &ContourData,
    target_geometry: VoxelGeometry,
) -> Option<([u32; 3], [u32; 3])> {
    let slab = contour_slices_voxel_aabb(contour, target_geometry)?;
    if contour.active_plane_family == PlaneFamily::Oblique {
        return Some(slab);
    }

    let depth_axis = match contour.active_plane_family {
        PlaneFamily::Axial => 2,
        PlaneFamily::Coronal => 1,
        PlaneFamily::Sagittal => 0,
        PlaneFamily::Oblique => unreachable!(),
    };
    let mut min = target_geometry.dimensions;
    let mut max = [0; 3];
    let mut found_point = false;
    for slice in &contour.slices {
        for contour_loop in &slice.loops {
            for point in &contour_loop.points {
                let world = plane_local_mm_to_world_mm(point.local_mm, slice.plane);
                let index = world_mm_to_voxel_index(world, target_geometry);
                if index.iter().any(|value| !value.is_finite()) {
                    continue;
                }
                found_point = true;
                for axis in 0..3 {
                    if axis == depth_axis {
                        continue;
                    }
                    let dimension = target_geometry.dimensions[axis];
                    let point_min = index[axis].floor().max(0.0) as u32;
                    let point_max = (index[axis].ceil() as u32).saturating_add(1).min(dimension);
                    min[axis] = min[axis].min(point_min.min(dimension));
                    max[axis] = max[axis].max(point_max);
                }
            }
        }
    }
    if !found_point {
        return Some(slab);
    }
    min[depth_axis] = slab.0[depth_axis];
    max[depth_axis] = slab.1[depth_axis];
    if (0..3).any(|axis| min[axis] >= max[axis]) {
        Some(slab)
    } else {
        Some((min, max))
    }
}

struct RasterSlice {
    plane: PlaneDefinition,
    origin: Vec3,
    normal: Vec3,
    slab_tolerance_mm: f32,
    /// Orthogonal slices fill exactly one voxel layer: `(depth axis, layer index)`. The layer is
    /// the nearest one, so a plane between two layers has a deterministic owner instead of
    /// filling both. Oblique slices keep the slab-distance rule.
    depth_layer: Option<(usize, i64)>,
    loops: Vec<Vec<[f32; 2]>>,
}

pub fn rasterize_contours_to_voxel_data(
    contour: &ContourData,
    target_geometry: VoxelGeometry,
) -> Result<VoxelData, ContourRasterizationError> {
    if target_geometry
        .spacing()
        .iter()
        .any(|s| !s.is_finite() || *s <= 0.0)
    {
        return Err(ContourRasterizationError::InvalidTargetGeometry);
    }

    let voxel_count = voxel_count(target_geometry.dimensions)
        .ok_or(ContourRasterizationError::InvalidTargetGeometry)?;
    let raw_data = vec![0_u8; voxel_count];

    if contour.slices.is_empty() || voxel_count == 0 {
        return Ok(VoxelData {
            geometry: target_geometry,
            raw_data,
        });
    }

    let slices = prepare_slices(contour, target_geometry)?;
    if slices.is_empty() {
        return Ok(VoxelData {
            geometry: target_geometry,
            raw_data,
        });
    }

    let raw_data = rasterize_slices_in_bounds(
        &slices,
        target_geometry,
        raster_bounds_for_slices(&slices, target_geometry),
        raw_data,
    );

    Ok(VoxelData {
        geometry: target_geometry,
        raw_data,
    })
}

fn rasterize_slices_in_bounds(
    slices: &[RasterSlice],
    target_geometry: VoxelGeometry,
    bounds: ([u32; 3], [u32; 3]),
    mut raw_data: Vec<u8>,
) -> Vec<u8> {
    for z in bounds.0[2]..bounds.1[2] {
        for y in bounds.0[1]..bounds.1[1] {
            for x in bounds.0[0]..bounds.1[0] {
                let world_center = Vec3::from_array(voxel_index_to_world_mm(
                    [x as f32, y as f32, z as f32],
                    target_geometry,
                ));
                let mut filled = false;

                for slice in slices {
                    if let Some((axis, layer)) = slice.depth_layer {
                        if [x, y, z][axis] as i64 != layer {
                            continue;
                        }
                    } else {
                        let signed_distance = (world_center - slice.origin).dot(slice.normal);
                        if signed_distance.abs() > slice.slab_tolerance_mm {
                            continue;
                        }
                    }

                    let local = world_mm_to_plane_local_mm(world_center.to_array(), slice.plane);
                    if point_in_loops_even_odd(local, &slice.loops) {
                        filled = true;
                        break;
                    }
                }

                if filled {
                    let idx = voxel_linear_index([x, y, z], target_geometry.dimensions);
                    raw_data[idx] = 1;
                }
            }
        }
    }
    raw_data
}

pub fn rasterize_contour_preview_slices_to_voxel_data(
    contour: &ContourData,
    base: &VoxelData,
) -> Result<VoxelData, ContourRasterizationError> {
    let target_geometry = base.geometry;
    let expected_len = voxel_count(target_geometry.dimensions)
        .ok_or(ContourRasterizationError::InvalidTargetGeometry)?;
    if base.raw_data.len() != expected_len {
        return Err(ContourRasterizationError::InvalidTargetGeometry);
    }
    if contour.active_plane_family == PlaneFamily::Oblique {
        return rasterize_contours_to_voxel_data(contour, target_geometry);
    }
    let slices = prepare_slices(contour, target_geometry)?;
    let mut raw_data = base.raw_data.clone();
    let dimensions = target_geometry.dimensions;

    for slice in slices {
        let Some((depth_axis, layer)) = slice.depth_layer else {
            continue;
        };
        // A plane outside the volume owns no layer; the full rasterizer fills nothing for it.
        if layer < 0 || layer >= i64::from(dimensions[depth_axis]) {
            continue;
        }
        let depth = layer as u32;

        let (width, height) = match contour.active_plane_family {
            PlaneFamily::Axial => (dimensions[0], dimensions[1]),
            PlaneFamily::Coronal => (dimensions[0], dimensions[2]),
            PlaneFamily::Sagittal => (dimensions[1], dimensions[2]),
            PlaneFamily::Oblique => unreachable!(),
        };
        for v in 0..height {
            for u in 0..width {
                let index = match contour.active_plane_family {
                    PlaneFamily::Axial => [u, v, depth],
                    PlaneFamily::Coronal => [u, depth, v],
                    PlaneFamily::Sagittal => [depth, u, v],
                    PlaneFamily::Oblique => unreachable!(),
                };
                let linear = voxel_linear_index(index, dimensions);
                raw_data[linear] = 0;
                let world_center = voxel_index_to_world_mm(
                    [index[0] as f32, index[1] as f32, index[2] as f32],
                    target_geometry,
                );
                let local = world_mm_to_plane_local_mm(world_center, slice.plane);
                if point_in_loops_even_odd(local, &slice.loops) {
                    raw_data[linear] = 1;
                }
            }
        }
    }

    Ok(VoxelData {
        geometry: target_geometry,
        raw_data,
    })
}

fn prepare_slices(
    contour: &ContourData,
    target_geometry: VoxelGeometry,
) -> Result<Vec<RasterSlice>, ContourRasterizationError> {
    let mut slices = Vec::new();
    for (slice_index, slice) in contour.slices.iter().enumerate() {
        let origin = Vec3::from_array(slice.plane.origin_mm);
        let normal = Vec3::from_array(slice.plane.normal_mm);
        let u_axis = Vec3::from_array(slice.plane.u_axis_mm);
        let v_axis = Vec3::from_array(slice.plane.v_axis_mm);

        let plane_has_non_finite = !origin.is_finite()
            || !normal.is_finite()
            || !u_axis.is_finite()
            || !v_axis.is_finite();
        if plane_has_non_finite || normal.length_squared() <= EPSILON {
            return Err(ContourRasterizationError::InvalidSlicePlane { slice_index });
        }
        let normal = normal.normalize();

        // Validate plane-local mapping is numerically usable for this target geometry.
        let probe = voxel_index_to_world_mm([0.0, 0.0, 0.0], target_geometry);
        let local_probe = world_mm_to_plane_local_mm(probe, slice.plane);
        if local_probe.iter().any(|v| !v.is_finite()) {
            return Err(ContourRasterizationError::InvalidSlicePlane { slice_index });
        }

        let mut loops = Vec::new();
        for contour_loop in &slice.loops {
            if !contour_loop.is_valid_closed_loop() {
                continue;
            }
            let points = contour_loop
                .points
                .iter()
                .map(|p| p.local_mm)
                .collect::<Vec<_>>();
            loops.push(points);
        }

        if loops.is_empty() {
            continue;
        }

        let depth_layer = match orthogonal_depth_axis(contour.active_plane_family) {
            Some(axis) => Some((
                axis,
                nearest_depth_layer(slice.plane.origin_mm, axis, target_geometry)
                    .ok_or(ContourRasterizationError::InvalidSlicePlane { slice_index })?,
            )),
            None => None,
        };

        slices.push(RasterSlice {
            plane: slice.plane,
            origin,
            normal,
            slab_tolerance_mm: slice_slab_tolerance_mm(target_geometry, normal),
            depth_layer,
            loops,
        });
    }
    Ok(slices)
}

fn raster_bounds_for_slices(
    slices: &[RasterSlice],
    geometry: VoxelGeometry,
) -> ([u32; 3], [u32; 3]) {
    let mut min = geometry.dimensions;
    let mut max = [0; 3];
    let mut found = false;

    for slice in slices {
        for contour_loop in &slice.loops {
            for point in contour_loop {
                let world = Vec3::from_array(plane_local_mm_to_world_mm(*point, slice.plane));
                for offset in [-slice.slab_tolerance_mm, slice.slab_tolerance_mm] {
                    let index = world_mm_to_voxel_index(
                        (world + slice.normal * offset).to_array(),
                        geometry,
                    );
                    if index.iter().any(|value| !value.is_finite()) {
                        return ([0; 3], geometry.dimensions);
                    }
                    found = true;
                    for axis in 0..3 {
                        min[axis] = min[axis].min(index[axis].floor().max(0.0) as u32);
                        max[axis] = max[axis].max(
                            (index[axis].ceil() as u32)
                                .saturating_add(1)
                                .min(geometry.dimensions[axis]),
                        );
                    }
                }
            }
        }
    }

    if !found || (0..3).any(|axis| min[axis] >= max[axis]) {
        return ([0; 3], geometry.dimensions);
    }

    (
        min.map(|value| value.saturating_sub(1)),
        std::array::from_fn(|axis| max[axis].saturating_add(1).min(geometry.dimensions[axis])),
    )
}

fn point_in_loops_even_odd(point: [f32; 2], loops: &[Vec<[f32; 2]>]) -> bool {
    let mut inside = false;
    for loop_points in loops {
        if point_in_polygon(point, loop_points.as_slice()) {
            inside = !inside;
        }
    }
    inside
}

fn point_in_polygon(point: [f32; 2], polygon: &[[f32; 2]]) -> bool {
    if polygon.len() < 3 {
        return false;
    }

    let (px, py) = (point[0], point[1]);
    let mut inside = false;
    let mut prev = polygon[polygon.len() - 1];

    for current in polygon {
        if point_on_segment(point, prev, *current) {
            return true;
        }

        let (x1, y1) = (prev[0], prev[1]);
        let (x2, y2) = (current[0], current[1]);
        let crosses = (y1 > py) != (y2 > py);
        if crosses {
            let x_at_y = ((x2 - x1) * (py - y1) / (y2 - y1)) + x1;
            if px < x_at_y {
                inside = !inside;
            }
        }
        prev = *current;
    }

    inside
}

fn point_on_segment(point: [f32; 2], a: [f32; 2], b: [f32; 2]) -> bool {
    let ap = Vec3::new(point[0] - a[0], point[1] - a[1], 0.0);
    let ab = Vec3::new(b[0] - a[0], b[1] - a[1], 0.0);
    let cross = ap.cross(ab).z.abs();
    if cross > 1e-5 {
        return false;
    }

    let dot = ap.dot(ab);
    if dot < -1e-5 {
        return false;
    }
    if dot > ab.length_squared() + 1e-5 {
        return false;
    }
    true
}

fn slice_slab_tolerance_mm(geometry: VoxelGeometry, plane_normal: Vec3) -> f32 {
    let origin = Vec3::from_array(voxel_index_to_world_mm([0.0, 0.0, 0.0], geometry));
    let dx = Vec3::from_array(voxel_index_to_world_mm([1.0, 0.0, 0.0], geometry)) - origin;
    let dy = Vec3::from_array(voxel_index_to_world_mm([0.0, 1.0, 0.0], geometry)) - origin;
    let dz = Vec3::from_array(voxel_index_to_world_mm([0.0, 0.0, 1.0], geometry)) - origin;

    (dx.dot(plane_normal).abs() + dy.dot(plane_normal).abs() + dz.dot(plane_normal).abs()) * 0.5
}

fn voxel_count(dimensions: [u32; 3]) -> Option<usize> {
    let xy = (dimensions[0] as usize).checked_mul(dimensions[1] as usize)?;
    xy.checked_mul(dimensions[2] as usize)
}

fn voxel_linear_index(index: [u32; 3], dimensions: [u32; 3]) -> usize {
    (index[2] as usize * dimensions[1] as usize + index[1] as usize) * dimensions[0] as usize
        + index[0] as usize
}

#[cfg(test)]
mod tests;
