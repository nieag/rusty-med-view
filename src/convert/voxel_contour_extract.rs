use crate::app::roi::{ContourData, ContourLoop, ContourPoint, ContourSlice, VoxelData};
use crate::convert::{
    orthogonal_plane_from_volume_uv, plane_local_mm_to_world_mm, voxel_index_to_world_mm,
    world_mm_to_plane_local_mm, world_mm_to_voxel_index, PlaneDefinition, PlaneFamily,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoxelContourExtractionError {
    UnsupportedPlaneFamily { family: PlaneFamily },
    UnsupportedPlaneGeometry,
}

pub fn extract_contour_slice_from_voxel_data(
    voxel_data: &VoxelData,
    requested_plane: PlaneDefinition,
) -> Result<ContourData, VoxelContourExtractionError> {
    let family = requested_plane.family;
    if family == PlaneFamily::Oblique {
        return extract_oblique_contour_slice(voxel_data, requested_plane);
    }

    let geometry = voxel_data.geometry;
    let (depth_axis, _, _) = family_axes(family);
    let reference_plane = orthogonal_plane_from_volume_uv(family, [0.5; 3], geometry)
        .ok_or(VoxelContourExtractionError::UnsupportedPlaneGeometry)?;
    let requested_normal = normalized(requested_plane.normal_mm)
        .ok_or(VoxelContourExtractionError::UnsupportedPlaneGeometry)?;
    let source_normal = normalized(reference_plane.normal_mm)
        .ok_or(VoxelContourExtractionError::UnsupportedPlaneGeometry)?;
    if requested_normal.dot(source_normal).abs() < 0.999 {
        return Err(VoxelContourExtractionError::UnsupportedPlaneGeometry);
    }

    let depth = world_mm_to_voxel_index(requested_plane.origin_mm, geometry)[depth_axis];
    let depth_len = geometry.dimensions[depth_axis];
    let mut slices = Vec::new();
    if depth.is_finite() && depth >= -0.5 && depth <= depth_len as f32 - 0.5 {
        let depth_index = depth.round().clamp(0.0, depth_len.saturating_sub(1) as f32) as u32;
        if let Some(slice) =
            extract_slice_at_depth(voxel_data, family, depth_index, requested_plane)
        {
            slices.push(slice);
        } else {
            slices.push(ContourSlice {
                plane: requested_plane,
                loops: Vec::new(),
            });
        }
    }

    Ok(ContourData {
        active_plane_family: family,
        slices,
    })
}

fn extract_oblique_contour_slice(
    voxel_data: &VoxelData,
    requested_plane: PlaneDefinition,
) -> Result<ContourData, VoxelContourExtractionError> {
    const MAX_PLANE_SAMPLES_PER_AXIS: u32 = 2048;

    let geometry = voxel_data.geometry;
    let u_axis = normalized(requested_plane.u_axis_mm)
        .ok_or(VoxelContourExtractionError::UnsupportedPlaneGeometry)?;
    let v_axis = normalized(requested_plane.v_axis_mm)
        .ok_or(VoxelContourExtractionError::UnsupportedPlaneGeometry)?;
    if u_axis.dot(v_axis).abs() > 1e-3 {
        return Err(VoxelContourExtractionError::UnsupportedPlaneGeometry);
    }
    let normal = u_axis.cross(v_axis).normalize_or_zero();
    if normal.length_squared() <= 1e-12 {
        return Err(VoxelContourExtractionError::UnsupportedPlaneGeometry);
    }

    let mut min_local = [f32::INFINITY; 2];
    let mut max_local = [f32::NEG_INFINITY; 2];
    let mut min_distance = f32::INFINITY;
    let mut max_distance = f32::NEG_INFINITY;
    let plane_origin = glam::Vec3::from_array(requested_plane.origin_mm);
    for z in [-0.5, geometry.dimensions[2] as f32 - 0.5] {
        for y in [-0.5, geometry.dimensions[1] as f32 - 0.5] {
            for x in [-0.5, geometry.dimensions[0] as f32 - 0.5] {
                let world = voxel_index_to_world_mm([x, y, z], geometry);
                let local = world_mm_to_plane_local_mm(world, requested_plane);
                min_local[0] = min_local[0].min(local[0]);
                min_local[1] = min_local[1].min(local[1]);
                max_local[0] = max_local[0].max(local[0]);
                max_local[1] = max_local[1].max(local[1]);
                let distance = (glam::Vec3::from_array(world) - plane_origin).dot(normal);
                min_distance = min_distance.min(distance);
                max_distance = max_distance.max(distance);
            }
        }
    }

    if min_distance > 0.0 || max_distance < 0.0 {
        return Ok(ContourData {
            active_plane_family: PlaneFamily::Oblique,
            slices: Vec::new(),
        });
    }

    let base_step = geometry
        .spacing
        .into_iter()
        .filter(|spacing| spacing.is_finite() && *spacing > 1e-6)
        .fold(f32::INFINITY, f32::min);
    if !base_step.is_finite() {
        return Err(VoxelContourExtractionError::UnsupportedPlaneGeometry);
    }
    let extent_u = (max_local[0] - min_local[0]).max(base_step);
    let extent_v = (max_local[1] - min_local[1]).max(base_step);
    let width = (extent_u / base_step)
        .ceil()
        .clamp(1.0, MAX_PLANE_SAMPLES_PER_AXIS as f32) as u32;
    let height = (extent_v / base_step)
        .ceil()
        .clamp(1.0, MAX_PLANE_SAMPLES_PER_AXIS as f32) as u32;
    let step_u = extent_u / width as f32;
    let step_v = extent_v / height as f32;
    let mut mask = vec![false; (width * height) as usize];

    for y in 0..height {
        for x in 0..width {
            let local = [
                min_local[0] + (x as f32 + 0.5) * step_u,
                min_local[1] + (y as f32 + 0.5) * step_v,
            ];
            let world = plane_local_mm_to_world_mm(local, requested_plane);
            let index = world_mm_to_voxel_index(world, geometry);
            let inside = (0..3).all(|axis| {
                index[axis] >= -0.5 && index[axis] <= geometry.dimensions[axis] as f32 - 0.5
            });
            if !inside {
                continue;
            }
            let nearest = [
                index[0]
                    .round()
                    .clamp(0.0, geometry.dimensions[0].saturating_sub(1) as f32)
                    as u32,
                index[1]
                    .round()
                    .clamp(0.0, geometry.dimensions[1].saturating_sub(1) as f32)
                    as u32,
                index[2]
                    .round()
                    .clamp(0.0, geometry.dimensions[2].saturating_sub(1) as f32)
                    as u32,
            ];
            mask[(y * width + x) as usize] = voxel_data
                .raw_data
                .get(voxel_index(nearest, geometry.dimensions))
                .copied()
                .unwrap_or(0)
                != 0;
        }
    }

    let mut loops = Vec::new();
    for component in connected_components_4n(&mask, width, height) {
        for boundary in component_boundary_loops(&component) {
            if boundary.len() < 4 {
                continue;
            }
            loops.push(ContourLoop {
                points: boundary
                    .into_iter()
                    .map(|[grid_x, grid_y]| ContourPoint {
                        local_mm: [
                            min_local[0] + grid_x as f32 * step_u,
                            min_local[1] + grid_y as f32 * step_v,
                        ],
                    })
                    .collect(),
                is_closed: true,
            });
        }
    }

    Ok(ContourData {
        active_plane_family: PlaneFamily::Oblique,
        slices: vec![ContourSlice {
            plane: requested_plane,
            loops,
        }],
    })
}

pub fn extract_contours_from_voxel_data(
    voxel_data: &VoxelData,
    family: PlaneFamily,
) -> Result<ContourData, VoxelContourExtractionError> {
    if family == PlaneFamily::Oblique {
        return Err(VoxelContourExtractionError::UnsupportedPlaneFamily { family });
    }

    let geometry = voxel_data.geometry;
    let (depth_axis, _, _) = family_axes(family);
    let dimensions = geometry.dimensions;
    let depth_len = dimensions[depth_axis];
    let mut slices = Vec::new();

    for depth_index in 0..depth_len {
        let plane = orthogonal_plane_from_volume_uv(
            family,
            family_slice_cursor_uv(family, depth_index, depth_len),
            geometry,
        )
        .expect("orthogonal plane should be resolvable from valid geometry");

        if let Some(slice) = extract_slice_at_depth(voxel_data, family, depth_index, plane) {
            slices.push(slice);
        }
    }

    Ok(ContourData {
        active_plane_family: family,
        slices,
    })
}

fn normalized(v: [f32; 3]) -> Option<glam::Vec3> {
    let v = glam::Vec3::from_array(v);
    let length_squared = v.length_squared();
    (length_squared.is_finite() && length_squared > 1e-12).then(|| v / length_squared.sqrt())
}

fn extract_slice_at_depth(
    voxel_data: &VoxelData,
    family: PlaneFamily,
    depth_index: u32,
    output_plane: PlaneDefinition,
) -> Option<ContourSlice> {
    let geometry = voxel_data.geometry;
    let (_, u_axis, v_axis) = family_axes(family);
    let width = geometry.dimensions[u_axis];
    let height = geometry.dimensions[v_axis];
    let mask = orthogonal_mask(voxel_data, family, depth_index);
    let components = connected_components_4n(&mask, width, height);
    let mut loops = Vec::new();

    for component in components {
        for boundary in component_boundary_loops(&component) {
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
                    ContourPoint {
                        local_mm: world_mm_to_plane_local_mm(world, output_plane),
                    }
                })
                .collect::<Vec<_>>();
            loops.push(ContourLoop {
                points,
                is_closed: true,
            });
        }
    }

    (!loops.is_empty()).then_some(ContourSlice {
        plane: output_plane,
        loops,
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
    let depth_uv = crate::convert::slice_center_uv(depth_index as i32, depth_len);
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

fn component_boundary_loops(component: &[[u32; 2]]) -> Vec<Vec<[u32; 2]>> {
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

    let mut unused_edges = BTreeSet::new();
    for (a, neighbors) in &adjacency {
        for b in neighbors {
            unused_edges.insert(ordered_edge(*a, *b));
        }
    }

    let mut loops = Vec::new();
    while let Some(&(start, next)) = unused_edges.iter().next() {
        unused_edges.remove(&ordered_edge(start, next));
        let mut loop_points = vec![start, next];
        let mut previous = start;
        let mut current = next;
        let max_steps = adjacency.len().saturating_mul(4).max(8);

        for _ in 0..max_steps {
            if current == start {
                loop_points.pop();
                if loop_points.len() >= 4 {
                    loops.push(loop_points);
                }
                break;
            }
            let Some(candidate) = adjacency.get(&current).and_then(|neighbors| {
                neighbors.iter().copied().find(|point| {
                    *point != previous && unused_edges.contains(&ordered_edge(current, *point))
                })
            }) else {
                break;
            };
            unused_edges.remove(&ordered_edge(current, candidate));
            previous = current;
            current = candidate;
            loop_points.push(current);
        }
    }
    loops
}

fn ordered_edge(a: [u32; 2], b: [u32; 2]) -> ([u32; 2], [u32; 2]) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
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
mod tests;
