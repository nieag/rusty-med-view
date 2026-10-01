use crate::convert::{
    orthogonal_plane_from_volume_uv, plane_local_mm_to_world_mm, voxel_index_to_world_mm,
    world_mm_to_plane_local_mm, world_mm_to_voxel_index, PlaneDefinition,
};
use crate::model::{
    ContourData, ContourLoop, ContourPoint, ContourSlice, OrthogonalFamily, VoxelData,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoxelContourExtractionError {
    UnsupportedPlaneGeometry,
}

/// The contours of one plane through the voxel data, for a derived view: orthogonal planes read
/// their voxel slab, oblique planes are sampled. The result is a list of slices (empty when the
/// plane misses the volume), not an authoritative contour set.
pub fn extract_contour_slice_from_voxel_data(
    voxel_data: &VoxelData,
    requested_plane: PlaneDefinition,
) -> Result<Vec<ContourSlice>, VoxelContourExtractionError> {
    let Some(family) = requested_plane.family.orthogonal() else {
        return extract_oblique_contour_slice(voxel_data, requested_plane);
    };

    let geometry = voxel_data.geometry;
    let (depth_axis, _, _) = family_axes(family);
    let reference_plane = orthogonal_plane_from_volume_uv(family.into(), [0.5; 3], geometry)
        .ok_or(VoxelContourExtractionError::UnsupportedPlaneGeometry)?;
    let requested_normal = normalized(requested_plane.normal_mm)
        .ok_or(VoxelContourExtractionError::UnsupportedPlaneGeometry)?;
    let source_normal = normalized(reference_plane.normal_mm)
        .ok_or(VoxelContourExtractionError::UnsupportedPlaneGeometry)?;
    if requested_normal.dot(source_normal).abs() < crate::convert::PLANE_NORMAL_ALIGNMENT_COS {
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

    Ok(slices)
}

fn extract_oblique_contour_slice(
    voxel_data: &VoxelData,
    requested_plane: PlaneDefinition,
) -> Result<Vec<ContourSlice>, VoxelContourExtractionError> {
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
        return Ok(Vec::new());
    }

    let base_step = geometry
        .spacing()
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

    Ok(vec![ContourSlice {
        plane: requested_plane,
        loops,
    }])
}

/// Every slice of one orthogonal family that holds voxels, as an authoritative contour set.
pub fn extract_contours_from_voxel_data(
    voxel_data: &VoxelData,
    family: OrthogonalFamily,
) -> Result<ContourData, VoxelContourExtractionError> {
    extract_contours_in_grid(voxel_data, family, voxel_data.geometry)
}

/// Like [`extract_contours_from_voxel_data`] for voxel data that covers only a box of
/// `reference`. The slice planes are the reference grid's, so a slice has the same plane (and
/// the same identity) whether it came from a box or from the whole grid.
pub fn extract_contours_in_grid(
    voxel_data: &VoxelData,
    family: OrthogonalFamily,
    reference: crate::model::VoxelGeometry,
) -> Result<ContourData, VoxelContourExtractionError> {
    let geometry = voxel_data.geometry;
    let (depth_axis, _, _) = family_axes(family);
    let dimensions = geometry.dimensions;
    let depth_len = dimensions[depth_axis];
    let offset = geometry
        .offset_in(reference)
        .ok_or(VoxelContourExtractionError::UnsupportedPlaneGeometry)?;
    let mut slices = Vec::new();

    for depth_index in 0..depth_len {
        let plane = reference_layer_plane(family, depth_index + offset[depth_axis], reference)
            .ok_or(VoxelContourExtractionError::UnsupportedPlaneGeometry)?;

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
    family: OrthogonalFamily,
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

fn family_axes(family: OrthogonalFamily) -> (usize, usize, usize) {
    match family {
        OrthogonalFamily::Axial => (2, 0, 1),
        OrthogonalFamily::Coronal => (1, 0, 2),
        OrthogonalFamily::Sagittal => (0, 1, 2),
    }
}

/// The plane of layer `layer` of `family` on the reference grid: through the layer's centre, with
/// the grid's centre in the other two directions. Extraction and cutting use the same planes, so
/// a slice has one plane however it was made.
pub(crate) fn reference_layer_plane(
    family: OrthogonalFamily,
    layer: u32,
    reference: crate::model::VoxelGeometry,
) -> Option<PlaneDefinition> {
    let (depth_axis, _, _) = family_axes(family);
    orthogonal_plane_from_volume_uv(
        family.into(),
        family_slice_cursor_uv(family, layer, reference.dimensions()[depth_axis]),
        reference,
    )
}

fn family_slice_cursor_uv(family: OrthogonalFamily, depth_index: u32, depth_len: u32) -> [f32; 3] {
    let depth_uv = crate::convert::slice_center_uv(depth_index as i32, depth_len);
    match family {
        OrthogonalFamily::Axial => [0.5, 0.5, depth_uv],
        OrthogonalFamily::Coronal => [0.5, depth_uv, 0.5],
        OrthogonalFamily::Sagittal => [depth_uv, 0.5, 0.5],
    }
}

fn orthogonal_vertex_to_voxel_index(
    family: OrthogonalFamily,
    grid_x: f32,
    grid_y: f32,
    depth_index: u32,
) -> [f32; 3] {
    match family {
        OrthogonalFamily::Axial => [grid_x - 0.5, grid_y - 0.5, depth_index as f32],
        OrthogonalFamily::Coronal => [grid_x - 0.5, depth_index as f32, grid_y - 0.5],
        OrthogonalFamily::Sagittal => [depth_index as f32, grid_x - 0.5, grid_y - 0.5],
    }
}

fn orthogonal_mask(
    voxel_data: &VoxelData,
    family: OrthogonalFamily,
    depth_index: u32,
) -> Vec<bool> {
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
    family: OrthogonalFamily,
    u: u32,
    v: u32,
    depth_index: u32,
) -> [u32; 3] {
    match family {
        OrthogonalFamily::Axial => [u, v, depth_index],
        OrthogonalFamily::Coronal => [u, depth_index, v],
        OrthogonalFamily::Sagittal => [depth_index, u, v],
    }
}

fn connected_components_4n(mask: &[bool], width: u32, height: u32) -> Vec<Vec<[u32; 2]>> {
    let mut visited = vec![false; mask.len()];
    let mut components = Vec::new();
    let mut stack = Vec::new();

    for y in 0..height {
        for x in 0..width {
            let start = (y * width + x) as usize;
            if visited[start] || !mask[start] {
                continue;
            }

            // Breadth-first order is part of the contract: the loops of a slice come out in the
            // order of each component's first cell, and the boundary tracer only looks at the
            // set of cells, so any traversal order of one component gives the same loops.
            let mut component = Vec::new();
            stack.push([x, y]);
            visited[start] = true;
            while let Some([cx, cy]) = stack.pop() {
                component.push([cx, cy]);
                let mut visit = |nx: u32, ny: u32| {
                    let index = (ny * width + nx) as usize;
                    if !visited[index] && mask[index] {
                        visited[index] = true;
                        stack.push([nx, ny]);
                    }
                };
                if cx > 0 {
                    visit(cx - 1, cy);
                }
                if cx + 1 < width {
                    visit(cx + 1, cy);
                }
                if cy > 0 {
                    visit(cx, cy - 1);
                }
                if cy + 1 < height {
                    visit(cx, cy + 1);
                }
            }
            components.push(component);
        }
    }

    components
}

// Directions from a boundary vertex, in the order their target vertices sort by `[x, y]`:
// (x-1, y) < (x, y-1) < (x, y+1) < (x+1, y). Neighbours are tried in this order, which fixes
// which way the tracer turns at a vertex where two loops touch.
const LEFT: usize = 0;
const UP: usize = 1;
const DOWN: usize = 2;
const RIGHT: usize = 3;
const DIRECTION_STEPS: [[i64; 2]; 4] = [[-1, 0], [0, -1], [0, 1], [1, 0]];
const OPPOSITE: [usize; 4] = [RIGHT, DOWN, UP, LEFT];

/// The outlines (outer boundaries and holes) of one 4-connected component, as loops of grid
/// vertices. Edges of the cells that face a non-member are linked into loops, starting from the
/// smallest unused edge and taking the smallest unused neighbour at each vertex.
fn component_boundary_loops(component: &[[u32; 2]]) -> Vec<Vec<[u32; 2]>> {
    let Some(first) = component.first() else {
        return Vec::new();
    };
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (first[0], first[1], first[0], first[1]);
    for [x, y] in component {
        min_x = min_x.min(*x);
        min_y = min_y.min(*y);
        max_x = max_x.max(*x);
        max_y = max_y.max(*y);
    }
    let cell_width = (max_x - min_x + 1) as usize;
    let cell_height = (max_y - min_y + 1) as usize;
    let mut member = vec![false; cell_width * cell_height];
    for [x, y] in component {
        member[(y - min_y) as usize * cell_width + (x - min_x) as usize] = true;
    }
    let contains = |x: i64, y: i64| {
        x >= min_x as i64
            && y >= min_y as i64
            && x <= max_x as i64
            && y <= max_y as i64
            && member[(y - min_y as i64) as usize * cell_width + (x - min_x as i64) as usize]
    };

    // Per vertex: which of the four directions have a boundary edge, and which are used up.
    let vertex_width = cell_width + 1;
    let vertex_height = cell_height + 1;
    let mut linked = vec![0u8; vertex_width * vertex_height];
    let mut used = vec![0u8; vertex_width * vertex_height];
    let vertex_index =
        |x: i64, y: i64| (y - min_y as i64) as usize * vertex_width + (x - min_x as i64) as usize;
    let mut vertex_count = 0usize;
    let mut add_edge = |linked: &mut Vec<u8>, a: [i64; 2], direction: usize| {
        let b = [
            a[0] + DIRECTION_STEPS[direction][0],
            a[1] + DIRECTION_STEPS[direction][1],
        ];
        for (from, toward) in [(a, direction), (b, OPPOSITE[direction])] {
            let entry = &mut linked[vertex_index(from[0], from[1])];
            if *entry == 0 {
                vertex_count += 1;
            }
            *entry |= 1 << toward;
        }
    };
    for [cx, cy] in component {
        let (x, y) = (*cx as i64, *cy as i64);
        if !contains(x, y - 1) {
            add_edge(&mut linked, [x, y], RIGHT);
        }
        if !contains(x + 1, y) {
            add_edge(&mut linked, [x + 1, y], DOWN);
        }
        if !contains(x, y + 1) {
            add_edge(&mut linked, [x + 1, y + 1], LEFT);
        }
        if !contains(x - 1, y) {
            add_edge(&mut linked, [x, y + 1], UP);
        }
    }

    let is_unused = |linked: &[u8], used: &[u8], vertex: [i64; 2], direction: usize| {
        let index = vertex_index(vertex[0], vertex[1]);
        linked[index] & (1 << direction) != 0 && used[index] & (1 << direction) == 0
    };
    let mark_used = |used: &mut Vec<u8>, vertex: [i64; 2], direction: usize| {
        let other = [
            vertex[0] + DIRECTION_STEPS[direction][0],
            vertex[1] + DIRECTION_STEPS[direction][1],
        ];
        used[vertex_index(vertex[0], vertex[1])] |= 1 << direction;
        used[vertex_index(other[0], other[1])] |= 1 << OPPOSITE[direction];
    };

    let max_steps = vertex_count.saturating_mul(4).max(8);
    let mut loops = Vec::new();
    // The smallest unused edge (a, b) with a < b: scan vertices by x then y and look down, then
    // right, the only directions that lead to a larger vertex.
    for x in min_x as i64..=max_x as i64 + 1 {
        for y in min_y as i64..=max_y as i64 + 1 {
            for first_direction in [DOWN, RIGHT] {
                if !is_unused(&linked, &used, [x, y], first_direction) {
                    continue;
                }
                mark_used(&mut used, [x, y], first_direction);
                let start = [x, y];
                let mut current = [
                    x + DIRECTION_STEPS[first_direction][0],
                    y + DIRECTION_STEPS[first_direction][1],
                ];
                let mut previous = start;
                let mut loop_points = vec![start, current];
                for _ in 0..max_steps {
                    if current == start {
                        loop_points.pop();
                        if loop_points.len() >= 4 {
                            loops.push(loop_points);
                        }
                        break;
                    }
                    let Some(direction) = (0..4).find(|direction| {
                        let target = [
                            current[0] + DIRECTION_STEPS[*direction][0],
                            current[1] + DIRECTION_STEPS[*direction][1],
                        ];
                        target != previous && is_unused(&linked, &used, current, *direction)
                    }) else {
                        break;
                    };
                    mark_used(&mut used, current, direction);
                    previous = current;
                    current = [
                        current[0] + DIRECTION_STEPS[direction][0],
                        current[1] + DIRECTION_STEPS[direction][1],
                    ];
                    loop_points.push(current);
                }
            }
        }
    }
    loops
        .into_iter()
        .map(|points: Vec<[i64; 2]>| {
            points
                .into_iter()
                .map(|[x, y]| [x as u32, y as u32])
                .collect()
        })
        .collect()
}

fn voxel_index(index: [u32; 3], dimensions: [u32; 3]) -> usize {
    (index[2] as usize * dimensions[1] as usize + index[1] as usize) * dimensions[0] as usize
        + index[0] as usize
}

#[cfg(test)]
mod tests;
