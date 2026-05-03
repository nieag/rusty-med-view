use crate::components::{ContourData, ContourLoop, ContourPoint, ContourSlice, VoxelData};
use crate::convert::{
    orthogonal_plane_from_volume_uv, voxel_index_to_world_mm, world_mm_to_plane_local_mm,
    PlaneFamily,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoxelContourExtractionError {
    UnsupportedPlaneFamily { family: PlaneFamily },
}

pub fn extract_contours_from_voxel_data(
    voxel_data: &VoxelData,
    family: PlaneFamily,
) -> Result<ContourData, VoxelContourExtractionError> {
    if family == PlaneFamily::Oblique {
        return Err(VoxelContourExtractionError::UnsupportedPlaneFamily { family });
    }

    let geometry = voxel_data.geometry;
    let (depth_axis, u_axis, v_axis) = family_axes(family);
    let dimensions = geometry.dimensions;
    let depth_len = dimensions[depth_axis];
    let width = dimensions[u_axis];
    let height = dimensions[v_axis];
    let mut slices = Vec::new();

    for depth_index in 0..depth_len {
        let mask = orthogonal_mask(voxel_data, family, depth_index);
        if mask.iter().all(|filled| !*filled) {
            continue;
        }

        let components = connected_components_4n(&mask, width, height);
        if components.is_empty() {
            continue;
        }

        let plane = orthogonal_plane_from_volume_uv(
            family,
            family_slice_cursor_uv(family, depth_index, depth_len),
            geometry,
        )
        .expect("orthogonal plane should be resolvable from valid geometry");

        let mut loops = Vec::new();
        for component in components {
            let boundary = component_boundary_loop(&component);
            if boundary.len() < 4 {
                continue;
            }

            let points = boundary
                .iter()
                .map(|[gx, gy]| {
                    let voxel_index = orthogonal_vertex_to_voxel_index(
                        family,
                        *gx as f32,
                        *gy as f32,
                        depth_index,
                    );
                    let world = voxel_index_to_world_mm(voxel_index, geometry);
                    let local_mm = world_mm_to_plane_local_mm(world, plane);
                    ContourPoint { local_mm }
                })
                .collect::<Vec<_>>();

            if points.len() >= 3 {
                loops.push(ContourLoop {
                    points,
                    is_closed: true,
                });
            }
        }

        if loops.is_empty() {
            continue;
        }

        slices.push(ContourSlice { plane, loops });
    }

    Ok(ContourData {
        active_plane_family: family,
        slices,
    })
}

fn family_axes(family: PlaneFamily) -> (usize, usize, usize) {
    match family {
        PlaneFamily::Axial => (2, 0, 1),
        PlaneFamily::Coronal => (1, 0, 2),
        PlaneFamily::Sagittal => (0, 1, 2),
        PlaneFamily::Oblique => unreachable!("oblique family has no orthogonal axes"),
    }
}

fn family_slice_cursor_uv(family: PlaneFamily, depth_index: u32, depth_len: u32) -> [f32; 3] {
    let depth_uv = if depth_len <= 1 {
        0.0
    } else {
        depth_index as f32 / (depth_len - 1) as f32
    };
    match family {
        PlaneFamily::Axial => [0.5, 0.5, depth_uv],
        PlaneFamily::Coronal => [0.5, depth_uv, 0.5],
        PlaneFamily::Sagittal => [depth_uv, 0.5, 0.5],
        PlaneFamily::Oblique => unreachable!("oblique family has no orthogonal cursor mapping"),
    }
}

fn orthogonal_vertex_to_voxel_index(
    family: PlaneFamily,
    grid_x: f32,
    grid_y: f32,
    depth_index: u32,
) -> [f32; 3] {
    match family {
        PlaneFamily::Axial => [grid_x - 0.5, grid_y - 0.5, depth_index as f32],
        PlaneFamily::Coronal => [grid_x - 0.5, depth_index as f32, grid_y - 0.5],
        PlaneFamily::Sagittal => [depth_index as f32, grid_x - 0.5, grid_y - 0.5],
        PlaneFamily::Oblique => unreachable!("oblique family has no orthogonal vertex mapping"),
    }
}

fn orthogonal_mask(voxel_data: &VoxelData, family: PlaneFamily, depth_index: u32) -> Vec<bool> {
    let dimensions = voxel_data.geometry.dimensions;
    let (_, u_axis, v_axis) = family_axes(family);
    let width = dimensions[u_axis];
    let height = dimensions[v_axis];
    let mut mask = vec![false; (width * height) as usize];
    for v in 0..height {
        for u in 0..width {
            let index = voxel_index_from_family_coords(family, u, v, depth_index);
            let idx = voxel_index(index, dimensions);
            let mask_idx = (v * width + u) as usize;
            mask[mask_idx] = voxel_data.raw_data.get(idx).copied().unwrap_or(0) != 0;
        }
    }
    mask
}

fn voxel_index_from_family_coords(
    family: PlaneFamily,
    u: u32,
    v: u32,
    depth_index: u32,
) -> [u32; 3] {
    match family {
        PlaneFamily::Axial => [u, v, depth_index],
        PlaneFamily::Coronal => [u, depth_index, v],
        PlaneFamily::Sagittal => [depth_index, u, v],
        PlaneFamily::Oblique => unreachable!("oblique family has no orthogonal index mapping"),
    }
}

fn connected_components_4n(mask: &[bool], width: u32, height: u32) -> Vec<Vec<[u32; 2]>> {
    let mut visited = vec![false; mask.len()];
    let mut components = Vec::new();

    for y in 0..height {
        for x in 0..width {
            let start = (y * width + x) as usize;
            if visited[start] || !mask[start] {
                continue;
            }

            let mut queue = VecDeque::from([[x, y]]);
            let mut component = Vec::new();
            visited[start] = true;

            while let Some([cx, cy]) = queue.pop_front() {
                component.push([cx, cy]);
                for [nx, ny] in neighbors_4(cx, cy, width, height) {
                    let nidx = (ny * width + nx) as usize;
                    if !visited[nidx] && mask[nidx] {
                        visited[nidx] = true;
                        queue.push_back([nx, ny]);
                    }
                }
            }

            components.push(component);
        }
    }

    components
}

fn neighbors_4(x: u32, y: u32, width: u32, height: u32) -> Vec<[u32; 2]> {
    let mut result = Vec::with_capacity(4);
    if x > 0 {
        result.push([x - 1, y]);
    }
    if x + 1 < width {
        result.push([x + 1, y]);
    }
    if y > 0 {
        result.push([x, y - 1]);
    }
    if y + 1 < height {
        result.push([x, y + 1]);
    }
    result
}

fn component_boundary_loop(component: &[[u32; 2]]) -> Vec<[u32; 2]> {
    let component_set: BTreeSet<[u32; 2]> = component.iter().copied().collect();
    let mut adjacency: BTreeMap<[u32; 2], BTreeSet<[u32; 2]>> = BTreeMap::new();

    for [x, y] in component {
        if *y == 0 || !component_set.contains(&[*x, *y - 1]) {
            add_edge(&mut adjacency, [*x, *y], [*x + 1, *y]);
        }
        if !component_set.contains(&[*x + 1, *y]) {
            add_edge(&mut adjacency, [*x + 1, *y], [*x + 1, *y + 1]);
        }
        if !component_set.contains(&[*x, *y + 1]) {
            add_edge(&mut adjacency, [*x + 1, *y + 1], [*x, *y + 1]);
        }
        if *x == 0 || !component_set.contains(&[*x - 1, *y]) {
            add_edge(&mut adjacency, [*x, *y + 1], [*x, *y]);
        }
    }

    let Some((&start, _)) = adjacency.first_key_value() else {
        return Vec::new();
    };

    let mut loop_points = vec![start];
    let Some(next) = adjacency
        .get(&start)
        .and_then(|neighbors| neighbors.iter().next())
        .copied()
    else {
        return Vec::new();
    };

    let mut previous = start;
    let mut current = next;
    loop_points.push(current);

    let max_steps = adjacency.len().saturating_mul(4).max(8);
    for _ in 0..max_steps {
        if current == start {
            loop_points.pop();
            return loop_points;
        }

        let Some(neighbors) = adjacency.get(&current) else {
            return Vec::new();
        };

        let next = neighbors.iter().copied().find(|p| *p != previous);
        let Some(next) = next else {
            return Vec::new();
        };

        previous = current;
        current = next;
        loop_points.push(current);
    }

    Vec::new()
}

fn add_edge(adjacency: &mut BTreeMap<[u32; 2], BTreeSet<[u32; 2]>>, a: [u32; 2], b: [u32; 2]) {
    adjacency.entry(a).or_default().insert(b);
    adjacency.entry(b).or_default().insert(a);
}

fn voxel_index(index: [u32; 3], dimensions: [u32; 3]) -> usize {
    (index[2] as usize * dimensions[1] as usize + index[1] as usize) * dimensions[0] as usize
        + index[0] as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::VoxelGeometry;
    use crate::convert::{
        plane_local_mm_to_world_mm, rasterize_contours_to_voxel_data, world_mm_to_voxel_index,
    };
    use glam::Quat;

    fn identity_geometry(dimensions: [u32; 3]) -> VoxelGeometry {
        VoxelGeometry {
            dimensions,
            spacing: [1.0, 1.0, 1.0],
            origin: [0.0, 0.0, 0.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        }
    }

    fn voxel_data_with_fill(
        dimensions: [u32; 3],
        filled_indices: impl IntoIterator<Item = [u32; 3]>,
    ) -> VoxelData {
        let mut raw = vec![0_u8; (dimensions[0] * dimensions[1] * dimensions[2]) as usize];
        for [x, y, z] in filled_indices {
            let idx = voxel_index([x, y, z], dimensions);
            raw[idx] = 1;
        }
        VoxelData {
            geometry: identity_geometry(dimensions),
            raw_data: raw,
        }
    }

    fn voxel_data_with_geometry_fill(
        geometry: VoxelGeometry,
        filled_indices: impl IntoIterator<Item = [u32; 3]>,
    ) -> VoxelData {
        let mut raw = vec![
            0_u8;
            (geometry.dimensions[0] * geometry.dimensions[1] * geometry.dimensions[2])
                as usize
        ];
        for [x, y, z] in filled_indices {
            let idx = voxel_index([x, y, z], geometry.dimensions);
            raw[idx] = 1;
        }
        VoxelData {
            geometry,
            raw_data: raw,
        }
    }

    #[test]
    fn test_extract_empty_voxel_data_returns_empty_contours() {
        let voxel = voxel_data_with_fill([4, 4, 2], []);
        let contour = extract_contours_from_voxel_data(&voxel, PlaneFamily::Axial)
            .expect("axial extraction should succeed");

        assert_eq!(contour.active_plane_family, PlaneFamily::Axial);
        assert!(contour.slices.is_empty());
    }

    #[test]
    fn test_extract_single_component_on_one_axial_slice_returns_one_closed_loop() {
        let voxel = voxel_data_with_fill([5, 5, 2], [[2, 2, 1]]);
        let contour = extract_contours_from_voxel_data(&voxel, PlaneFamily::Axial)
            .expect("axial extraction should succeed");

        assert_eq!(contour.slices.len(), 1);
        assert_eq!(contour.slices[0].loops.len(), 1);
        let loop0 = &contour.slices[0].loops[0];
        assert!(loop0.is_closed);
        assert!(loop0.points.len() >= 4);
    }

    #[test]
    fn test_extract_multiple_disconnected_components_returns_multiple_loops() {
        let voxel = voxel_data_with_fill([6, 6, 1], [[1, 1, 0], [4, 4, 0]]);
        let contour = extract_contours_from_voxel_data(&voxel, PlaneFamily::Axial)
            .expect("axial extraction should succeed");

        assert_eq!(contour.slices.len(), 1);
        assert_eq!(contour.slices[0].loops.len(), 2);
        assert!(contour.slices[0]
            .loops
            .iter()
            .all(|loop_data| loop_data.is_closed));
    }

    #[test]
    fn test_extract_component_touching_left_top_border_still_emits_loop() {
        let voxel = voxel_data_with_fill([5, 5, 1], [[0, 0, 0], [1, 0, 0], [0, 1, 0]]);
        let contour = extract_contours_from_voxel_data(&voxel, PlaneFamily::Axial)
            .expect("axial extraction should succeed");

        assert_eq!(contour.slices.len(), 1);
        assert_eq!(contour.slices[0].loops.len(), 1);
        assert!(contour.slices[0].loops[0].is_closed);
        assert!(contour.slices[0].loops[0].points.len() >= 4);
    }

    #[test]
    fn test_extract_sets_active_plane_family_to_requested_family() {
        let voxel = voxel_data_with_fill([3, 3, 1], [[1, 1, 0]]);
        let contour = extract_contours_from_voxel_data(&voxel, PlaneFamily::Axial)
            .expect("axial extraction should succeed");
        assert_eq!(contour.active_plane_family, PlaneFamily::Axial);
    }

    #[test]
    fn test_extract_rejects_oblique_family_in_v1() {
        let voxel = voxel_data_with_fill([3, 3, 1], [[1, 1, 0]]);
        let result = extract_contours_from_voxel_data(&voxel, PlaneFamily::Oblique);
        assert_eq!(
            result,
            Err(VoxelContourExtractionError::UnsupportedPlaneFamily {
                family: PlaneFamily::Oblique
            })
        );
    }

    fn assert_loop_points_on_expected_slice(
        contour: &ContourData,
        family: PlaneFamily,
        depth: f32,
    ) {
        assert_eq!(contour.slices.len(), 1);
        assert_eq!(contour.slices[0].loops.len(), 1);
        let plane = contour.slices[0].plane;
        for point in &contour.slices[0].loops[0].points {
            let world = plane_local_mm_to_world_mm(point.local_mm, plane);
            let index = world_mm_to_voxel_index(world, identity_geometry([5, 5, 5]));
            let actual = match family {
                PlaneFamily::Axial => index[2],
                PlaneFamily::Coronal => index[1],
                PlaneFamily::Sagittal => index[0],
                PlaneFamily::Oblique => unreachable!(),
            };
            assert!((actual - depth).abs() < 1e-3);
        }
    }

    #[test]
    fn test_axial_extraction_geometry_preserves_slice_depth() {
        let voxel = voxel_data_with_fill([5, 5, 5], [[2, 2, 3]]);
        let contour = extract_contours_from_voxel_data(&voxel, PlaneFamily::Axial).unwrap();
        assert_loop_points_on_expected_slice(&contour, PlaneFamily::Axial, 3.0);
    }

    #[test]
    fn test_coronal_extraction_geometry_preserves_slice_depth() {
        let voxel = voxel_data_with_fill([5, 5, 5], [[2, 3, 2]]);
        let contour = extract_contours_from_voxel_data(&voxel, PlaneFamily::Coronal).unwrap();
        assert_loop_points_on_expected_slice(&contour, PlaneFamily::Coronal, 3.0);
    }

    #[test]
    fn test_sagittal_extraction_geometry_preserves_slice_depth() {
        let voxel = voxel_data_with_fill([5, 5, 5], [[3, 2, 2]]);
        let contour = extract_contours_from_voxel_data(&voxel, PlaneFamily::Sagittal).unwrap();
        assert_loop_points_on_expected_slice(&contour, PlaneFamily::Sagittal, 3.0);
    }

    #[test]
    fn test_extraction_preserves_origin_and_spacing_through_plane_local_points() {
        let geometry = VoxelGeometry {
            dimensions: [4, 4, 3],
            spacing: [1.5, 2.0, 2.5],
            origin: [10.0, -5.0, 3.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let mut raw = vec![
            0_u8;
            (geometry.dimensions[0] * geometry.dimensions[1] * geometry.dimensions[2])
                as usize
        ];
        raw[voxel_index([1, 2, 1], geometry.dimensions)] = 1;
        let voxel = VoxelData {
            geometry,
            raw_data: raw,
        };

        let contour = extract_contours_from_voxel_data(&voxel, PlaneFamily::Axial).unwrap();
        assert_eq!(contour.slices.len(), 1);
        let plane = contour.slices[0].plane;
        let point = contour.slices[0].loops[0].points[0];
        let world = plane_local_mm_to_world_mm(point.local_mm, plane);
        let index = world_mm_to_voxel_index(world, geometry);

        assert!((index[2] - 1.0).abs() < 1e-3);
        assert!((index[0] - index[0].round()).abs() <= 0.5 + 1e-3);
        assert!((index[1] - index[1].round()).abs() <= 0.5 + 1e-3);
    }

    #[test]
    fn test_axial_voxel_contour_voxel_roundtrip_preserves_mask_geometry() {
        let voxel = voxel_data_with_fill(
            [7, 6, 3],
            [[1, 1, 1], [2, 1, 1], [1, 2, 1], [4, 3, 1], [4, 4, 1]],
        );

        let contour = extract_contours_from_voxel_data(&voxel, PlaneFamily::Axial)
            .expect("axial extraction should succeed");
        let rebuilt = rasterize_contours_to_voxel_data(&contour, voxel.geometry)
            .expect("roundtrip rasterization should succeed");

        assert_eq!(rebuilt.geometry, voxel.geometry);
        assert_eq!(rebuilt.raw_data, voxel.raw_data);
    }

    #[test]
    fn test_non_identity_orientation_roundtrip_preserves_mask_geometry() {
        let orientation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2).to_array();
        let geometry = VoxelGeometry {
            dimensions: [6, 6, 3],
            spacing: [0.7, 1.3, 2.1],
            origin: [12.0, -9.5, 4.25],
            orientation,
        };
        let voxel = voxel_data_with_geometry_fill(
            geometry,
            [[1, 1, 1], [2, 1, 1], [1, 2, 1], [3, 4, 1], [4, 4, 1]],
        );

        let contour = extract_contours_from_voxel_data(&voxel, PlaneFamily::Axial)
            .expect("axial extraction should succeed");
        let rebuilt = rasterize_contours_to_voxel_data(&contour, voxel.geometry)
            .expect("roundtrip rasterization should succeed");

        assert_eq!(rebuilt.raw_data, voxel.raw_data);
    }

    #[test]
    fn test_extracted_points_map_back_to_expected_source_slice_and_component_bounds() {
        let voxel = voxel_data_with_fill([6, 6, 4], [[2, 2, 3], [3, 2, 3], [2, 3, 3], [3, 3, 3]]);
        let contour = extract_contours_from_voxel_data(&voxel, PlaneFamily::Axial)
            .expect("axial extraction should succeed");

        assert_eq!(contour.slices.len(), 1);
        assert_eq!(contour.slices[0].loops.len(), 1);
        let plane = contour.slices[0].plane;
        for point in &contour.slices[0].loops[0].points {
            let world = plane_local_mm_to_world_mm(point.local_mm, plane);
            let idx = world_mm_to_voxel_index(world, voxel.geometry);
            assert!((idx[2] - 3.0).abs() < 1e-3);
            assert!((1.5 - 1e-3..=3.5 + 1e-3).contains(&idx[0]));
            assert!((1.5 - 1e-3..=3.5 + 1e-3).contains(&idx[1]));
        }
    }

    #[test]
    fn test_coronal_and_sagittal_voxel_contour_voxel_roundtrip_preserve_mask_geometry() {
        let voxel = voxel_data_with_fill(
            [6, 5, 4],
            [[2, 2, 1], [2, 2, 2], [2, 3, 1], [3, 1, 2], [4, 1, 2]],
        );

        for family in [PlaneFamily::Coronal, PlaneFamily::Sagittal] {
            let contour = extract_contours_from_voxel_data(&voxel, family)
                .expect("orthogonal extraction should succeed");
            let rebuilt = rasterize_contours_to_voxel_data(&contour, voxel.geometry)
                .expect("roundtrip rasterization should succeed");
            assert_eq!(rebuilt.raw_data, voxel.raw_data);
        }
    }
}
