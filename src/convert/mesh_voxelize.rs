use crate::convert::world_mm_to_voxel_index;
use crate::model::{MeshData, VoxelData, VoxelGeometry};
use std::collections::HashMap;

type WeldedVertices = Vec<[f32; 3]>;
type TriangleIndices = Vec<[u32; 3]>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshVoxelizationError {
    EmptyMesh,
    InvalidTargetGeometry,
    InvalidVertex { vertex_index: usize },
    InvalidFace { face_index: usize },
    OpenOrNonManifoldEdge { edge: [u32; 2], face_count: u32 },
    InconsistentWinding { edge: [u32; 2] },
    SelfIntersection { faces: [usize; 2] },
    MeshBuildFailed,
}

/// Voxelizes a closed mesh by scanline parity: for each row of voxel centres along x, the sorted
/// places where the row crosses the surface give the inside intervals. This replaces a
/// point-in-mesh query per voxel (2.5 s on the liver, about 40 ms now) and gives the same result.
///
/// Mesh vertices from voxel extraction lie on grid lines, so a row through voxel centres would
/// graze vertices and edges exactly. Every sample point is therefore shifted by a fixed offset
/// far below any voxel-scale feature and above float rounding, which puts it in general
/// position: no ties, and a centre that is within the offset of the surface is the only kind of
/// voxel that could come out differently from an exact test.
pub struct IncrementalMeshVoxelization {
    geometry: VoxelGeometry,
    raw_data: Vec<u8>,
    /// Surface crossings as (row, x in voxel-index units), sorted; row = z * dim_y + y.
    crossings: Vec<(u32, f64)>,
    next_crossing: usize,
}

const SAMPLE_OFFSET: [f64; 3] = [7.3e-5, 1.13e-4, 1.71e-4];

impl IncrementalMeshVoxelization {
    pub fn begin(mesh: &MeshData, geometry: VoxelGeometry) -> Result<Self, MeshVoxelizationError> {
        Self::begin_impl(mesh, geometry, false)
    }

    pub(crate) fn begin_prevalidated(
        mesh: &MeshData,
        geometry: VoxelGeometry,
    ) -> Result<Self, MeshVoxelizationError> {
        Self::begin_impl(mesh, geometry, true)
    }

    fn begin_impl(
        mesh: &MeshData,
        geometry: VoxelGeometry,
        prevalidated: bool,
    ) -> Result<Self, MeshVoxelizationError> {
        validate_target_geometry(geometry)?;
        // A mesh that was already validated skips the welding and the topology checks; the scan
        // only needs its triangles.
        let (source_vertices, indices) = if prevalidated {
            check_indices(mesh)?;
            (
                mesh.vertices.iter().map(|vertex| vertex.world_mm).collect(),
                mesh.faces.iter().map(|face| face.vertex_indices).collect(),
            )
        } else {
            welded_closed_mesh(mesh, true)?
        };
        let vertices = source_vertices
            .iter()
            .enumerate()
            .map(|(vertex_index, world_mm)| {
                let voxel = world_mm_to_voxel_index(*world_mm, geometry);
                if voxel.iter().any(|value| !value.is_finite()) {
                    return Err(MeshVoxelizationError::InvalidVertex { vertex_index });
                }
                Ok(voxel.map(f64::from))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let dimensions = geometry.dimensions;
        let voxel_count = (dimensions[0] as usize)
            .checked_mul(dimensions[1] as usize)
            .and_then(|count| count.checked_mul(dimensions[2] as usize))
            .ok_or(MeshVoxelizationError::InvalidTargetGeometry)?;
        let mut crossings = surface_crossings(&vertices, &indices, dimensions);
        crossings.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
        Ok(Self {
            geometry,
            raw_data: vec![0; voxel_count],
            crossings,
            next_crossing: 0,
        })
    }

    /// Fills rows until about `max_voxels` voxels' worth of work is done.
    pub fn step(&mut self, max_voxels: usize) -> bool {
        let row_width = self.geometry.dimensions[0] as usize;
        let mut budget = max_voxels;
        while self.next_crossing < self.crossings.len() && budget > 0 {
            let row = self.crossings[self.next_crossing].0;
            let end = self.crossings[self.next_crossing..]
                .iter()
                .position(|(other, _)| *other != row)
                .map_or(self.crossings.len(), |offset| self.next_crossing + offset);
            let row_start = row as usize * row_width;
            for pair in self.crossings[self.next_crossing..end].chunks_exact(2) {
                // Sample x of voxel i is i + offset; it is inside when it lies between the pair.
                let first = (pair[0].1 - SAMPLE_OFFSET[0]).floor() as i64 + 1;
                let last = (pair[1].1 - SAMPLE_OFFSET[0]).ceil() as i64 - 1;
                let first = first.max(0);
                let last = last.min(row_width as i64 - 1);
                if first <= last {
                    self.raw_data[row_start + first as usize..=row_start + last as usize].fill(1);
                }
            }
            self.next_crossing = end;
            budget = budget.saturating_sub(row_width.max(1));
        }
        self.is_complete()
    }

    fn is_complete(&self) -> bool {
        self.next_crossing >= self.crossings.len()
    }

    pub fn into_result(self) -> Option<VoxelData> {
        self.is_complete().then_some(VoxelData {
            geometry: self.geometry,
            raw_data: self.raw_data,
        })
    }
}

/// Where rows of samples along +x cross the triangles, for rows inside the volume.
fn surface_crossings(
    vertices: &[[f64; 3]],
    triangles: &[[u32; 3]],
    dimensions: [u32; 3],
) -> Vec<(u32, f64)> {
    let mut crossings = Vec::with_capacity(triangles.len() * 2);
    let [_, dim_y, dim_z] = dimensions;
    for triangle in triangles {
        let [a, b, c] = triangle.map(|index| vertices[index as usize]);
        // Twice the signed area of the triangle seen along x, in the (y, z) plane.
        let area = (b[1] - a[1]) * (c[2] - a[2]) - (c[1] - a[1]) * (b[2] - a[2]);
        if area == 0.0 {
            continue; // edge-on to the rays
        }
        let row_range = |axis: usize, dimension: u32| {
            let low = a[axis].min(b[axis]).min(c[axis]);
            let high = a[axis].max(b[axis]).max(c[axis]);
            let first = (low - SAMPLE_OFFSET[axis]).ceil().max(0.0) as i64;
            let last = (high - SAMPLE_OFFSET[axis])
                .floor()
                .min(dimension as f64 - 1.0) as i64;
            first..=last
        };
        for z in row_range(2, dim_z) {
            let sample_z = z as f64 + SAMPLE_OFFSET[2];
            for y in row_range(1, dim_y) {
                let sample_y = y as f64 + SAMPLE_OFFSET[1];
                let w1 =
                    ((sample_y - a[1]) * (c[2] - a[2]) - (c[1] - a[1]) * (sample_z - a[2])) / area;
                let w2 =
                    ((b[1] - a[1]) * (sample_z - a[2]) - (sample_y - a[1]) * (b[2] - a[2])) / area;
                let w0 = 1.0 - w1 - w2;
                if w0 > 0.0 && w1 > 0.0 && w2 > 0.0 {
                    let x = w0 * a[0] + w1 * b[0] + w2 * c[0];
                    crossings.push(((z as u32) * dim_y + y as u32, x));
                }
            }
        }
    }
    crossings
}

#[cfg(test)]
pub fn voxelize_mesh_to_voxel_data(
    mesh: &MeshData,
    target_geometry: VoxelGeometry,
) -> Result<VoxelData, MeshVoxelizationError> {
    let mut work = IncrementalMeshVoxelization::begin(mesh, target_geometry)?;
    work.step(usize::MAX);
    Ok(work.into_result().expect("full voxel scan must complete"))
}

pub fn validate_mesh_for_voxelization(mesh: &MeshData) -> Result<(), MeshVoxelizationError> {
    welded_closed_mesh(mesh, true).map(|_| ())
}

/// The checks that keep a prevalidated mesh from indexing out of range or carrying NaNs.
fn check_indices(mesh: &MeshData) -> Result<(), MeshVoxelizationError> {
    if mesh.vertices.is_empty() || mesh.faces.is_empty() {
        return Err(MeshVoxelizationError::EmptyMesh);
    }
    for (face_index, face) in mesh.faces.iter().enumerate() {
        if face
            .vertex_indices
            .iter()
            .any(|index| *index as usize >= mesh.vertices.len())
        {
            return Err(MeshVoxelizationError::InvalidFace { face_index });
        }
    }
    Ok(())
}

fn validate_target_geometry(geometry: VoxelGeometry) -> Result<(), MeshVoxelizationError> {
    if geometry.dimensions.contains(&0)
        || geometry
            .spacing()
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
        || geometry.origin().iter().any(|value| !value.is_finite())
        || geometry
            .orientation()
            .iter()
            .any(|value| !value.is_finite())
    {
        return Err(MeshVoxelizationError::InvalidTargetGeometry);
    }
    Ok(())
}

fn welded_closed_mesh(
    mesh: &MeshData,
    check_intersections: bool,
) -> Result<(WeldedVertices, TriangleIndices), MeshVoxelizationError> {
    if mesh.vertices.is_empty() || mesh.faces.is_empty() {
        return Err(MeshVoxelizationError::EmptyMesh);
    }
    for (vertex_index, vertex) in mesh.vertices.iter().enumerate() {
        if vertex.world_mm.iter().any(|value| !value.is_finite()) {
            return Err(MeshVoxelizationError::InvalidVertex { vertex_index });
        }
    }

    let mut welded_vertices = Vec::new();
    let mut welded_ids = HashMap::new();
    let mut source_to_welded = Vec::with_capacity(mesh.vertices.len());
    for vertex in &mesh.vertices {
        let key = vertex.world_mm.map(|value| {
            if value == 0.0 {
                0.0_f32.to_bits()
            } else {
                value.to_bits()
            }
        });
        let id = *welded_ids.entry(key).or_insert_with(|| {
            let id = welded_vertices.len() as u32;
            welded_vertices.push(vertex.world_mm);
            id
        });
        source_to_welded.push(id);
    }

    let mut edges: HashMap<[u32; 2], (u32, i32)> = HashMap::new();
    let mut welded_faces = Vec::with_capacity(mesh.faces.len());
    for (face_index, face) in mesh.faces.iter().enumerate() {
        if face
            .vertex_indices
            .iter()
            .any(|index| *index as usize >= mesh.vertices.len())
        {
            return Err(MeshVoxelizationError::InvalidFace { face_index });
        }
        let [a, b, c] = face
            .vertex_indices
            .map(|index| source_to_welded[index as usize]);
        if a == b || b == c || c == a {
            return Err(MeshVoxelizationError::InvalidFace { face_index });
        }
        // Distinct indices can still describe a collapsed triangle after a
        // deform. Use f64 here to avoid f32 overflow/underflow in the area test;
        // no tolerance that would discard small but valid world-mm triangles.
        let [pa, pb, pc] = [a, b, c]
            .map(|index| glam::DVec3::from_array(welded_vertices[index as usize].map(f64::from)));
        if (pb - pa).cross(pc - pa) == glam::DVec3::ZERO {
            return Err(MeshVoxelizationError::InvalidFace { face_index });
        }
        welded_faces.push([a, b, c]);
        for [from, to] in [[a, b], [b, c], [c, a]] {
            let edge = if from < to { [from, to] } else { [to, from] };
            let direction = if from < to { 1 } else { -1 };
            let entry = edges.entry(edge).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += direction;
        }
    }

    for (edge, (face_count, direction_sum)) in edges {
        // Voxel boundaries can contain multiple closed shells that touch along an
        // edge. Their shared edge has balanced, even incidence (typically four)
        // and still defines a closed solid for winding-based voxelization.
        if face_count < 2 || face_count % 2 != 0 {
            return Err(MeshVoxelizationError::OpenOrNonManifoldEdge { edge, face_count });
        }
        if direction_sum != 0 {
            return Err(MeshVoxelizationError::InconsistentWinding { edge });
        }
    }
    if check_intersections {
        validate_surface_intersections(&welded_vertices, &welded_faces)?;
    }
    Ok((welded_vertices, welded_faces))
}

fn validate_surface_intersections(
    vertices: &[[f32; 3]],
    faces: &[[u32; 3]],
) -> Result<(), MeshVoxelizationError> {
    // Bucket the triangles into a uniform grid sized to the typical triangle, so each triangle
    // is compared only with its neighbours (a bounding-volume query per face was 0.4 s on the
    // liver mesh).
    let bounds: Vec<([f32; 3], [f32; 3])> = faces
        .iter()
        .map(|face| {
            let points = face.map(|v| vertices[v as usize]);
            (
                std::array::from_fn(|axis| {
                    points.iter().map(|p| p[axis]).fold(f32::INFINITY, f32::min)
                }),
                std::array::from_fn(|axis| {
                    points
                        .iter()
                        .map(|p| p[axis])
                        .fold(f32::NEG_INFINITY, f32::max)
                }),
            )
        })
        .collect();
    let mean_extent = bounds
        .iter()
        .map(|(min, max)| {
            (0..3)
                .map(|axis| max[axis] - min[axis])
                .fold(0.0_f32, f32::max)
        })
        .sum::<f32>()
        / bounds.len().max(1) as f32;
    let cell = mean_extent.max(1e-3);
    let cell_of = |value: f32| (value / cell).floor() as i64;
    let mut grid: HashMap<[i64; 3], Vec<u32>> = HashMap::new();
    for (index, (min, max)) in bounds.iter().enumerate() {
        for x in cell_of(min[0])..=cell_of(max[0]) {
            for y in cell_of(min[1])..=cell_of(max[1]) {
                for z in cell_of(min[2])..=cell_of(max[2]) {
                    grid.entry([x, y, z]).or_default().push(index as u32);
                }
            }
        }
    }
    for (key, members) in &grid {
        for (position, &i) in members.iter().enumerate() {
            for &j in &members[position + 1..] {
                let (i, j) = (i.min(j) as usize, i.max(j) as usize);
                let (a, b) = (&bounds[i], &bounds[j]);
                if (0..3).any(|axis| a.1[axis] < b.0[axis] || b.1[axis] < a.0[axis]) {
                    continue;
                }
                // A pair shares many cells; test it only in the cell holding the corner of the
                // overlap of the two boxes.
                if (0..3).any(|axis| cell_of(a.0[axis].max(b.0[axis])) != key[axis]) {
                    continue;
                }
                if faces_overlap_beyond_shared_boundary(vertices, faces[i], faces[j]) {
                    return Err(MeshVoxelizationError::SelfIntersection { faces: [i, j] });
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn faces_overlap_beyond_shared_boundary(
    vertices: &[[f32; 3]],
    a: [u32; 3],
    b: [u32; 3],
) -> bool {
    use glam::DVec3;
    let point = |v: u32| DVec3::from_array(vertices[v as usize].map(f64::from));
    let pa = a.map(point);
    let pb = b.map(point);
    let shared: Vec<_> = a.into_iter().filter(|v| b.contains(v)).collect();
    match shared.as_slice() {
        [] => triangle_regions_intersect(pa, pb),
        [v] => {
            // Intersection beyond a shared vertex must reach an opposite edge.
            let ea: Vec<_> = a.into_iter().filter(|i| i != v).map(point).collect();
            let eb: Vec<_> = b.into_iter().filter(|i| i != v).map(point).collect();
            triangle_regions_intersect([ea[0], ea[1], ea[1]], pb)
                || triangle_regions_intersect(pa, [eb[0], eb[1], eb[1]])
        }
        [u, v] => {
            // Shared-edge faces intersect elsewhere only when coplanar and
            // their opposite vertices lie on the same side of that edge.
            let origin = point(*u);
            let edge = point(*v) - origin;
            let opposite_a = point(*a.iter().find(|i| !shared.contains(i)).unwrap()) - origin;
            let opposite_b = point(*b.iter().find(|i| !shared.contains(i)).unwrap()) - origin;
            let normal_a = edge.cross(opposite_a);
            normal_a.dot(opposite_b) == 0.0 && normal_a.dot(edge.cross(opposite_b)) > 0.0
        }
        _ => true, // Duplicate triangles do not define a solid boundary.
    }
}

// Separating-axis test for two triangles, also allowing a segment encoded as
// [p, q, q]. In-plane axes handle coplanar triangles and coplanar segments.
fn triangle_regions_intersect(a: [glam::DVec3; 3], b: [glam::DVec3; 3]) -> bool {
    let origin = a[0];
    let a = a.map(|p| p - origin);
    let b = b.map(|p| p - origin);
    let edges = |p: [glam::DVec3; 3]| [p[1] - p[0], p[2] - p[1], p[0] - p[2]];
    let ea = edges(a);
    let eb = edges(b);
    let na = ea[0].cross(ea[1]);
    let nb = eb[0].cross(eb[1]);
    let separated = |axis: glam::DVec3| {
        let ap = a.map(|p| p.dot(axis));
        let bp = b.map(|p| p.dot(axis));
        ap.into_iter().fold(f64::NEG_INFINITY, f64::max)
            < bp.into_iter().fold(f64::INFINITY, f64::min)
            || bp.into_iter().fold(f64::NEG_INFINITY, f64::max)
                < ap.into_iter().fold(f64::INFINITY, f64::min)
    };
    if separated(na) || separated(nb) {
        return false;
    }
    for edge in ea.into_iter().chain(eb) {
        if separated(na.cross(edge)) || separated(nb.cross(edge)) {
            return false;
        }
    }
    for edge_a in ea {
        for edge_b in eb {
            if separated(edge_a.cross(edge_b)) {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
fn linear_index(index: [u32; 3], dimensions: [u32; 3]) -> usize {
    (index[2] as usize * dimensions[1] as usize + index[1] as usize) * dimensions[0] as usize
        + index[0] as usize
}

#[cfg(test)]
mod tests;
