use crate::components::{ContourData, VoxelData, VoxelGeometry};
use crate::convert::{voxel_index_to_world_mm, world_mm_to_plane_local_mm, PlaneDefinition};
use glam::Vec3;

const EPSILON: f32 = 1e-6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContourRasterizationError {
    InvalidTargetGeometry,
    InvalidSlicePlane { slice_index: usize },
}

struct RasterSlice {
    plane: PlaneDefinition,
    origin: Vec3,
    normal: Vec3,
    loops: Vec<Vec<[f32; 2]>>,
}

pub fn rasterize_contours_to_voxel_data(
    contour: &ContourData,
    target_geometry: VoxelGeometry,
) -> Result<VoxelData, ContourRasterizationError> {
    if target_geometry
        .spacing
        .iter()
        .any(|s| !s.is_finite() || *s <= 0.0)
    {
        return Err(ContourRasterizationError::InvalidTargetGeometry);
    }

    let voxel_count = voxel_count(target_geometry.dimensions)
        .ok_or(ContourRasterizationError::InvalidTargetGeometry)?;
    let mut raw_data = vec![0_u8; voxel_count];

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

    for z in 0..target_geometry.dimensions[2] {
        for y in 0..target_geometry.dimensions[1] {
            for x in 0..target_geometry.dimensions[0] {
                let world_center = Vec3::from_array(voxel_index_to_world_mm(
                    [x as f32, y as f32, z as f32],
                    target_geometry,
                ));
                let mut filled = false;

                for slice in &slices {
                    let signed_distance = (world_center - slice.origin).dot(slice.normal);
                    let tolerance = slice_slab_tolerance_mm(target_geometry, slice.normal);
                    if signed_distance.abs() > tolerance {
                        continue;
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

        slices.push(RasterSlice {
            plane: slice.plane,
            origin,
            normal,
            loops,
        });
    }
    Ok(slices)
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
mod tests {
    use super::*;
    use crate::components::{ContourLoop, ContourPoint, ContourSlice};
    use crate::convert::{orthogonal_plane_from_volume_uv, PlaneFamily};

    fn index(dimensions: [u32; 3], x: u32, y: u32, z: u32) -> usize {
        (z as usize * dimensions[1] as usize + y as usize) * dimensions[0] as usize + x as usize
    }

    fn identity_geometry(dimensions: [u32; 3]) -> VoxelGeometry {
        VoxelGeometry {
            dimensions,
            spacing: [1.0, 1.0, 1.0],
            origin: [0.0, 0.0, 0.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        }
    }

    fn square_loop(half_extent: f32) -> ContourLoop {
        ContourLoop {
            points: vec![
                ContourPoint {
                    local_mm: [-half_extent, -half_extent],
                },
                ContourPoint {
                    local_mm: [half_extent, -half_extent],
                },
                ContourPoint {
                    local_mm: [half_extent, half_extent],
                },
                ContourPoint {
                    local_mm: [-half_extent, half_extent],
                },
            ],
            is_closed: true,
        }
    }

    #[test]
    fn test_rasterize_simple_axial_square_fill() {
        let geometry = identity_geometry([5, 5, 3]);
        let plane = orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 0.5], geometry)
            .expect("axial plane should resolve");
        let contour = ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: vec![ContourSlice {
                plane,
                loops: vec![square_loop(1.4)],
            }],
        };

        let voxel = rasterize_contours_to_voxel_data(&contour, geometry).expect("raster succeeds");
        let occupied = voxel.raw_data.iter().filter(|v| **v != 0).count();

        assert_eq!(occupied, 9);
        assert_eq!(voxel.raw_data[index(geometry.dimensions, 2, 2, 1)], 1);
        assert_eq!(voxel.raw_data[index(geometry.dimensions, 0, 0, 1)], 0);
        assert_eq!(voxel.raw_data[index(geometry.dimensions, 2, 2, 0)], 0);
    }

    #[test]
    fn test_rasterize_empty_contours_returns_zero_filled_voxel_data() {
        let geometry = identity_geometry([4, 3, 2]);
        let contour = ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: Vec::new(),
        };

        let voxel = rasterize_contours_to_voxel_data(&contour, geometry).expect("raster succeeds");

        assert_eq!(voxel.geometry, geometry);
        assert_eq!(voxel.raw_data.len(), 24);
        assert!(voxel.raw_data.iter().all(|v| *v == 0));
    }

    #[test]
    fn test_rasterize_skips_invalid_open_loop() {
        let geometry = identity_geometry([5, 5, 1]);
        let plane = orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 0.0], geometry)
            .expect("axial plane should resolve");
        let contour = ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: vec![ContourSlice {
                plane,
                loops: vec![ContourLoop {
                    points: vec![
                        ContourPoint {
                            local_mm: [-2.0, -2.0],
                        },
                        ContourPoint {
                            local_mm: [2.0, -2.0],
                        },
                        ContourPoint {
                            local_mm: [2.0, 2.0],
                        },
                        ContourPoint {
                            local_mm: [-2.0, 2.0],
                        },
                    ],
                    is_closed: false,
                }],
            }],
        };

        let voxel = rasterize_contours_to_voxel_data(&contour, geometry).expect("raster succeeds");
        assert!(voxel.raw_data.iter().all(|v| *v == 0));
    }

    #[test]
    fn test_rasterize_multiple_loops_use_even_odd_fill() {
        let geometry = identity_geometry([7, 7, 1]);
        let plane = orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 0.0], geometry)
            .expect("axial plane should resolve");
        let contour = ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: vec![ContourSlice {
                plane,
                loops: vec![square_loop(2.4), square_loop(1.4)],
            }],
        };

        let voxel = rasterize_contours_to_voxel_data(&contour, geometry).expect("raster succeeds");
        let occupied = voxel.raw_data.iter().filter(|v| **v != 0).count();

        assert_eq!(occupied, 16);
        assert_eq!(voxel.raw_data[index(geometry.dimensions, 3, 3, 0)], 0);
        assert_eq!(voxel.raw_data[index(geometry.dimensions, 1, 3, 0)], 1);
    }

    #[test]
    fn test_rasterize_preserves_target_geometry_in_result() {
        let geometry = VoxelGeometry {
            dimensions: [3, 4, 2],
            spacing: [0.7, 1.1, 2.3],
            origin: [5.0, -2.0, 8.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let contour = ContourData {
            active_plane_family: PlaneFamily::Sagittal,
            slices: Vec::new(),
        };

        let voxel = rasterize_contours_to_voxel_data(&contour, geometry).expect("raster succeeds");
        assert_eq!(voxel.geometry, geometry);
    }
}
