use crate::app::roi::{MeshData, VoxelData, VoxelGeometry};
use crate::convert::world_mm_to_voxel_index;
use parry3d::math::Vector;
use parry3d::query::PointQuery;
use parry3d::shape::{TriMesh, TriMeshFlags};
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

pub struct IncrementalMeshVoxelization {
    geometry: VoxelGeometry,
    mesh: TriMesh,
    raw_data: Vec<u8>,
    min: [u32; 3],
    max: [u32; 3],
    cursor: [u32; 3],
    complete: bool,
}

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
        let (welded_vertices, indices) = welded_closed_mesh(mesh, !prevalidated)?;
        let vertices = welded_vertices
            .iter()
            .enumerate()
            .map(|(vertex_index, world_mm)| {
                let voxel = world_mm_to_voxel_index(*world_mm, geometry);
                if voxel.iter().any(|value| !value.is_finite()) {
                    return Err(MeshVoxelizationError::InvalidVertex { vertex_index });
                }
                Ok(Vector::new(voxel[0], voxel[1], voxel[2]))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mesh = TriMesh::with_flags(vertices.clone(), indices, TriMeshFlags::ORIENTED)
            .map_err(|_| MeshVoxelizationError::MeshBuildFailed)?;
        let dimensions = geometry.dimensions;
        let voxel_count = (dimensions[0] as usize)
            .checked_mul(dimensions[1] as usize)
            .and_then(|count| count.checked_mul(dimensions[2] as usize))
            .ok_or(MeshVoxelizationError::InvalidTargetGeometry)?;
        let (min, max) = voxel_bounds(&vertices, dimensions);
        Ok(Self {
            geometry,
            mesh,
            raw_data: vec![0; voxel_count],
            min,
            max,
            cursor: min,
            complete: false,
        })
    }

    pub fn step(&mut self, max_voxels: usize) -> bool {
        for _ in 0..max_voxels {
            if self.complete {
                break;
            }
            let [x, y, z] = self.cursor;
            if self
                .mesh
                .contains_local_point(Vector::new(x as f32, y as f32, z as f32))
            {
                self.raw_data[linear_index(self.cursor, self.geometry.dimensions)] = 1;
            }
            if x < self.max[0] {
                self.cursor[0] += 1;
            } else if y < self.max[1] {
                self.cursor = [self.min[0], y + 1, z];
            } else if z < self.max[2] {
                self.cursor = [self.min[0], self.min[1], z + 1];
            } else {
                self.complete = true;
            }
        }
        self.complete
    }

    pub fn into_result(self) -> Option<VoxelData> {
        self.complete.then_some(VoxelData {
            geometry: self.geometry,
            raw_data: self.raw_data,
        })
    }
}

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

fn validate_target_geometry(geometry: VoxelGeometry) -> Result<(), MeshVoxelizationError> {
    if geometry.dimensions.contains(&0)
        || geometry
            .spacing
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
        || geometry.origin.iter().any(|value| !value.is_finite())
        || geometry.orientation.iter().any(|value| !value.is_finite())
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
    let mesh = TriMesh::new(
        vertices.iter().map(|p| Vector::from_array(*p)).collect(),
        faces.to_vec(),
    )
    .map_err(|_| MeshVoxelizationError::MeshBuildFailed)?;
    for (i, face) in faces.iter().enumerate() {
        let points = face.map(|v| Vector::from_array(vertices[v as usize]));
        let bounds = parry3d::bounding_volume::Aabb::new(
            points[0].min(points[1]).min(points[2]),
            points[0].max(points[1]).max(points[2]),
        );
        for j in mesh
            .bvh()
            .intersect_aabb(&bounds)
            .filter(|j| *j as usize > i)
        {
            if faces_overlap_beyond_shared_boundary(vertices, *face, faces[j as usize]) {
                return Err(MeshVoxelizationError::SelfIntersection {
                    faces: [i, j as usize],
                });
            }
        }
    }
    Ok(())
}

fn faces_overlap_beyond_shared_boundary(vertices: &[[f32; 3]], a: [u32; 3], b: [u32; 3]) -> bool {
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

fn voxel_bounds(vertices: &[Vector], dimensions: [u32; 3]) -> ([u32; 3], [u32; 3]) {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for vertex in vertices {
        for axis in 0..3 {
            min[axis] = min[axis].min(vertex[axis]);
            max[axis] = max[axis].max(vertex[axis]);
        }
    }
    (
        std::array::from_fn(|axis| {
            min[axis]
                .floor()
                .clamp(0.0, dimensions[axis].saturating_sub(1) as f32) as u32
        }),
        std::array::from_fn(|axis| {
            max[axis]
                .ceil()
                .clamp(0.0, dimensions[axis].saturating_sub(1) as f32) as u32
        }),
    )
}

fn linear_index(index: [u32; 3], dimensions: [u32; 3]) -> usize {
    (index[2] as usize * dimensions[1] as usize + index[1] as usize) * dimensions[0] as usize
        + index[0] as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{MeshFace, MeshVertex};
    use crate::convert::extract_mesh_from_voxel_data;
    use glam::Quat;

    fn geometry() -> VoxelGeometry {
        VoxelGeometry {
            dimensions: [4, 4, 4],
            spacing: [1.0, 2.0, 3.0],
            origin: [10.0, 20.0, 30.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        }
    }

    #[test]
    fn test_voxel_mesh_voxel_roundtrip_preserves_solid_block() {
        let geometry = geometry();
        let mut raw_data = vec![0_u8; 64];
        for z in 1..=2 {
            for y in 1..=2 {
                for x in 1..=2 {
                    raw_data[linear_index([x, y, z], geometry.dimensions)] = 1;
                }
            }
        }
        let source = VoxelData {
            geometry,
            raw_data: raw_data.clone(),
        };
        let mesh = extract_mesh_from_voxel_data(&source).unwrap();

        let rebuilt = voxelize_mesh_to_voxel_data(&mesh, geometry).unwrap();

        assert_eq!(rebuilt.geometry, geometry);
        assert_eq!(rebuilt.raw_data, raw_data);
    }

    #[test]
    fn test_incremental_voxelization_matches_solid_block_after_one_voxel_steps() {
        let geometry = geometry();
        let mut raw_data = vec![0_u8; 64];
        for z in 1..=2 {
            for y in 1..=2 {
                for x in 1..=2 {
                    raw_data[linear_index([x, y, z], geometry.dimensions)] = 1;
                }
            }
        }
        let mesh = extract_mesh_from_voxel_data(&VoxelData {
            geometry,
            raw_data: raw_data.clone(),
        })
        .unwrap();
        let mut work = IncrementalMeshVoxelization::begin(&mesh, geometry).unwrap();
        assert!(!work.step(1));
        assert!(!work.step(0));
        while !work.step(1) {}
        assert_eq!(work.into_result().unwrap().raw_data, raw_data);
    }

    #[test]
    fn test_smooth_mesh_roundtrip_preserves_rotated_anisotropic_grid() {
        let geometry = VoxelGeometry {
            dimensions: [5, 4, 3],
            spacing: [1.5, 2.0, 3.5],
            origin: [10.0, -4.0, 22.0],
            orientation: Quat::from_rotation_y(0.4).to_array(),
        };
        let mut raw_data = vec![0; 60];
        for z in 1..=2 {
            for y in 1..=2 {
                for x in 1..=3 {
                    raw_data[linear_index([x, y, z], geometry.dimensions)] = 1;
                }
            }
        }
        let source = VoxelData {
            geometry,
            raw_data: raw_data.clone(),
        };

        let mesh = extract_mesh_from_voxel_data(&source).unwrap();
        let rebuilt = voxelize_mesh_to_voxel_data(&mesh, geometry).unwrap();

        assert_eq!(rebuilt.raw_data, raw_data);
    }

    #[test]
    fn test_open_mesh_is_rejected() {
        let mesh = MeshData {
            vertices: vec![
                MeshVertex {
                    world_mm: [0.0, 0.0, 0.0],
                },
                MeshVertex {
                    world_mm: [1.0, 0.0, 0.0],
                },
                MeshVertex {
                    world_mm: [0.0, 1.0, 0.0],
                },
            ],
            faces: vec![MeshFace {
                vertex_indices: [0, 1, 2],
            }],
        };

        assert!(matches!(
            voxelize_mesh_to_voxel_data(&mesh, geometry()),
            Err(MeshVoxelizationError::OpenOrNonManifoldEdge { .. })
        ));
        assert!(matches!(
            validate_mesh_for_voxelization(&mesh),
            Err(MeshVoxelizationError::OpenOrNonManifoldEdge { .. })
        ));
    }

    #[test]
    fn test_surface_intersection_distinguishes_shared_boundaries_from_overlap() {
        let a = [0, 1, 2];
        let base = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]];
        // Common edge: a valid neighbor, coplanar fold-over, noncoplanar hinge.
        for (opposite, expected) in [
            ([1.0, -1.0, 0.0], false),
            ([0.5, 0.5, 0.0], true),
            ([0.5, 0.5, 1.0], false),
        ] {
            let vertices = [base[0], base[1], base[2], opposite];
            assert_eq!(
                faces_overlap_beyond_shared_boundary(&vertices, a, [0, 1, 3]),
                expected
            );
        }
        // Sharing one vertex is allowed, but does not excuse an overlapping face.
        for (p, q, expected) in [
            ([-1.0, 0.0, 0.0], [0.0, -1.0, 0.0], false),
            ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], true),
            ([0.5, 0.5, -1.0], [0.5, 0.5, 1.0], true),
        ] {
            let vertices = [base[0], base[1], base[2], p, q];
            assert_eq!(
                faces_overlap_beyond_shared_boundary(&vertices, a, [0, 3, 4]),
                expected
            );
        }
        // Disjoint indices: coplanar containment, crossing and a close parallel sheet.
        for (b, expected) in [
            ([[0.2, 0.2, 0.0], [0.8, 0.2, 0.0], [0.2, 0.8, 0.0]], true),
            ([[0.5, 0.5, -1.0], [0.5, 0.5, 1.0], [1.0, 0.5, 1.0]], true),
            (
                [[0.2, 0.2, 1e-6], [0.8, 0.2, 1e-6], [0.2, 0.8, 1e-6]],
                false,
            ),
        ] {
            let vertices = [base[0], base[1], base[2], b[0], b[1], b[2]];
            assert_eq!(
                faces_overlap_beyond_shared_boundary(&vertices, a, [3, 4, 5]),
                expected
            );
        }
        assert!(faces_overlap_beyond_shared_boundary(&base, a, [2, 1, 0]));
    }

    #[test]
    fn test_folded_closed_surface_is_rejected_before_voxelization() {
        let mut mesh = MeshData {
            vertices: [
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [-1.0, 0.0, 0.0],
                [0.0, -1.0, 0.0],
                [0.0, 0.0, -1.0],
            ]
            .map(|world_mm| MeshVertex { world_mm })
            .to_vec(),
            faces: [
                [0, 1, 2],
                [0, 2, 3],
                [0, 3, 4],
                [0, 4, 1],
                [5, 2, 1],
                [5, 3, 2],
                [5, 4, 3],
                [5, 1, 4],
            ]
            .map(|vertex_indices| MeshFace { vertex_indices })
            .to_vec(),
        };
        validate_mesh_for_voxelization(&mesh).unwrap();
        let original = mesh.clone();
        // Repeated edits can still fold the tighter adaptive brush. The
        // validator must catch that accumulated deformation before resampling.
        for step in 0..10 {
            let anchor = mesh.vertices[0].world_mm;
            mesh = crate::systems::mesh_editing::deform_mesh_surface_brush(
                &mesh,
                [0, 1, 2],
                anchor,
                [0.2, 0.0, -0.15],
                0.1,
                1.0,
            );
            if step < 3 {
                validate_mesh_for_voxelization(&mesh).unwrap();
            }
        }
        assert!(matches!(
            validate_mesh_for_voxelization(&mesh),
            Err(MeshVoxelizationError::SelfIntersection { .. })
        ));
        // Keep testing the rejection boundary with an explicitly folded mesh,
        // independent of whether the editing tool can still produce this fold.
        mesh = original;
        mesh.vertices[0].world_mm = [2.0, 0.0, -0.5];
        assert!(matches!(
            validate_mesh_for_voxelization(&mesh),
            Err(MeshVoxelizationError::SelfIntersection { .. })
        ));
        assert!(matches!(
            voxelize_mesh_to_voxel_data(&mesh, geometry()),
            Err(MeshVoxelizationError::SelfIntersection { .. })
        ));
    }

    #[test]
    fn test_collapsed_triangle_in_closed_mesh_is_rejected() {
        let mesh = MeshData {
            vertices: [
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [0.0, 1.0, 1.0],
            ]
            .map(|world_mm| MeshVertex { world_mm })
            .to_vec(),
            faces: [[0, 2, 1], [0, 1, 3], [1, 2, 3], [2, 0, 3]]
                .map(|vertex_indices| MeshFace { vertex_indices })
                .to_vec(),
        };
        assert_eq!(
            validate_mesh_for_voxelization(&mesh),
            Err(MeshVoxelizationError::InvalidFace { face_index: 0 })
        );
        assert_eq!(
            voxelize_mesh_to_voxel_data(&mesh, geometry()),
            Err(MeshVoxelizationError::InvalidFace { face_index: 0 })
        );
    }

    #[test]
    fn test_mesh_voxelization_is_deterministic() {
        let geometry = geometry();
        let source = VoxelData {
            geometry,
            raw_data: vec![1; 64],
        };
        let mesh = extract_mesh_from_voxel_data(&source).unwrap();

        let first = voxelize_mesh_to_voxel_data(&mesh, geometry).unwrap();
        let second = voxelize_mesh_to_voxel_data(&mesh, geometry).unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn test_edge_touching_voxel_shells_roundtrip() {
        let geometry = VoxelGeometry {
            dimensions: [5, 5, 4],
            spacing: [1.0; 3],
            origin: [0.0; 3],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let mut raw_data = vec![0_u8; 100];
        raw_data[linear_index([1, 1, 1], geometry.dimensions)] = 1;
        raw_data[linear_index([2, 2, 1], geometry.dimensions)] = 1;
        let source = VoxelData {
            geometry,
            raw_data: raw_data.clone(),
        };
        let mesh = extract_mesh_from_voxel_data(&source).unwrap();

        let rebuilt = voxelize_mesh_to_voxel_data(&mesh, geometry).unwrap();

        assert_eq!(rebuilt.raw_data, raw_data);
    }
}
