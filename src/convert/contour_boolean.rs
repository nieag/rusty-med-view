use crate::components::{ContourLoop, ContourPoint, ContourSlice};
use geo::{BooleanOps, Contains, Coord, LineString, MultiPolygon, Point, Polygon};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContourBooleanError {
    InvalidAdditiveLoop,
    EmptyUnion,
}

pub fn contour_slice_contains_point(slice: &ContourSlice, local_mm: [f32; 2]) -> bool {
    contour_slice_region(slice).contains(&Point::new(local_mm[0] as f64, local_mm[1] as f64))
        || slice
            .loops
            .iter()
            .filter(|contour_loop| contour_loop.is_valid_closed_loop())
            .any(|contour_loop| point_on_loop_boundary(local_mm, contour_loop))
}

pub fn union_contour_slice_with_loop(
    slice: &ContourSlice,
    additive_loop: &ContourLoop,
) -> Result<ContourSlice, ContourBooleanError> {
    let additive_polygon =
        loop_polygon(additive_loop).ok_or(ContourBooleanError::InvalidAdditiveLoop)?;
    let current = contour_slice_region(slice);
    let union = if current.0.is_empty() {
        MultiPolygon(vec![additive_polygon])
    } else {
        current.union(&additive_polygon)
    };
    let loops = multi_polygon_loops(&union);
    if loops.is_empty() {
        return Err(ContourBooleanError::EmptyUnion);
    }
    Ok(ContourSlice {
        plane: slice.plane,
        loops,
    })
}

fn contour_slice_region(slice: &ContourSlice) -> MultiPolygon<f64> {
    let mut polygons = slice.loops.iter().filter_map(loop_polygon);
    let Some(first) = polygons.next() else {
        return MultiPolygon(Vec::new());
    };
    polygons.fold(MultiPolygon(vec![first]), |region, polygon| {
        region.xor(&polygon)
    })
}

fn loop_polygon(contour_loop: &ContourLoop) -> Option<Polygon<f64>> {
    if !contour_loop.is_valid_closed_loop() {
        return None;
    }
    let mut coordinates = contour_loop
        .points
        .iter()
        .map(|point| Coord {
            x: point.local_mm[0] as f64,
            y: point.local_mm[1] as f64,
        })
        .collect::<Vec<_>>();
    if coordinates
        .iter()
        .any(|coord| !coord.x.is_finite() || !coord.y.is_finite())
    {
        return None;
    }
    if coordinates.first() != coordinates.last() {
        coordinates.push(*coordinates.first()?);
    }
    Some(Polygon::new(LineString::new(coordinates), Vec::new()))
}

fn multi_polygon_loops(multi_polygon: &MultiPolygon<f64>) -> Vec<ContourLoop> {
    let mut loops = Vec::new();
    for polygon in &multi_polygon.0 {
        if let Some(exterior) = line_string_loop(polygon.exterior()) {
            loops.push(exterior);
        }
        loops.extend(polygon.interiors().iter().filter_map(line_string_loop));
    }
    loops
}

fn line_string_loop(line_string: &LineString<f64>) -> Option<ContourLoop> {
    let mut coordinates = line_string.0.as_slice();
    if coordinates.first() == coordinates.last() {
        coordinates = &coordinates[..coordinates.len().saturating_sub(1)];
    }
    if coordinates.len() < 3 {
        return None;
    }
    Some(ContourLoop {
        points: coordinates
            .iter()
            .map(|coord| ContourPoint {
                local_mm: [coord.x as f32, coord.y as f32],
            })
            .collect(),
        is_closed: true,
    })
}

fn point_on_loop_boundary(point: [f32; 2], contour_loop: &ContourLoop) -> bool {
    let points = &contour_loop.points;
    for index in 0..points.len() {
        let a = points[index].local_mm;
        let b = points[(index + 1) % points.len()].local_mm;
        let ab = [b[0] - a[0], b[1] - a[1]];
        let ap = [point[0] - a[0], point[1] - a[1]];
        let cross = ab[0] * ap[1] - ab[1] * ap[0];
        let dot = ap[0] * ab[0] + ap[1] * ab[1];
        let length_squared = ab[0] * ab[0] + ab[1] * ab[1];
        if cross.abs() <= 1e-4 && dot >= -1e-4 && dot <= length_squared + 1e-4 {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{ContourData, VoxelGeometry};
    use crate::convert::{
        orthogonal_plane_from_volume_uv, rasterize_contours_to_voxel_data, PlaneFamily,
    };

    fn square(min: [f32; 2], max: [f32; 2]) -> ContourLoop {
        ContourLoop {
            points: vec![
                ContourPoint { local_mm: min },
                ContourPoint {
                    local_mm: [max[0], min[1]],
                },
                ContourPoint { local_mm: max },
                ContourPoint {
                    local_mm: [min[0], max[1]],
                },
            ],
            is_closed: true,
        }
    }

    fn test_slice(loops: Vec<ContourLoop>) -> ContourSlice {
        let geometry = VoxelGeometry {
            dimensions: [9, 9, 1],
            spacing: [1.0; 3],
            origin: [0.0; 3],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        ContourSlice {
            plane: orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 0.0], geometry)
                .unwrap(),
            loops,
        }
    }

    #[test]
    fn test_union_merges_overlapping_additive_loop_without_clearing_overlap() {
        let slice = test_slice(vec![square([-2.0, -2.0], [1.0, 2.0])]);
        let merged =
            union_contour_slice_with_loop(&slice, &square([0.0, -1.0], [3.0, 1.0])).expect("union");

        assert_eq!(merged.loops.len(), 1);
        assert!(contour_slice_contains_point(&merged, [-1.0, 0.0]));
        assert!(contour_slice_contains_point(&merged, [0.5, 0.0]));
        assert!(contour_slice_contains_point(&merged, [2.0, 0.0]));
    }

    #[test]
    fn test_union_inside_existing_loop_keeps_existing_boundary() {
        let slice = test_slice(vec![square([-3.0, -3.0], [3.0, 3.0])]);
        let merged = union_contour_slice_with_loop(&slice, &square([-1.0, -1.0], [1.0, 1.0]))
            .expect("union");

        assert_eq!(merged.loops.len(), 1);
        assert!(contour_slice_contains_point(&merged, [0.0, 0.0]));
    }

    #[test]
    fn test_union_preserves_existing_hole_and_raster_topology() {
        let geometry = VoxelGeometry {
            dimensions: [9, 9, 1],
            spacing: [1.0; 3],
            origin: [0.0; 3],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let slice = test_slice(vec![
            square([-3.0, -3.0], [3.0, 3.0]),
            square([-1.0, -1.0], [1.0, 1.0]),
        ]);
        let merged =
            union_contour_slice_with_loop(&slice, &square([4.0, -1.0], [5.0, 1.0])).expect("union");
        let contour = ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: vec![merged],
        };

        let voxel = rasterize_contours_to_voxel_data(&contour, geometry).unwrap();
        let center = (4 * 9 + 4) as usize;
        assert_eq!(voxel.raw_data[center], 0);
    }
}
