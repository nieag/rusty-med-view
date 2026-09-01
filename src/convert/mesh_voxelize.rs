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
    MeshBuildFailed,
}

pub fn voxelize_mesh_to_voxel_data(
    mesh: &MeshData,
    target_geometry: VoxelGeometry,
) -> Result<VoxelData, MeshVoxelizationError> {
    validate_target_geometry(target_geometry)?;
    let (welded_vertices, indices) = welded_closed_mesh(mesh)?;

    let vertices = welded_vertices
        .iter()
        .enumerate()
        .map(|(vertex_index, world_mm)| {
            let voxel = world_mm_to_voxel_index(*world_mm, target_geometry);
            if voxel.iter().any(|value| !value.is_finite()) {
                return Err(MeshVoxelizationError::InvalidVertex { vertex_index });
            }
            Ok(Vector::new(voxel[0], voxel[1], voxel[2]))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let trimesh = TriMesh::with_flags(vertices.clone(), indices, TriMeshFlags::ORIENTED)
        .map_err(|_| MeshVoxelizationError::MeshBuildFailed)?;

    let dimensions = target_geometry.dimensions;
    let voxel_count = (dimensions[0] as usize)
        .checked_mul(dimensions[1] as usize)
        .and_then(|count| count.checked_mul(dimensions[2] as usize))
        .ok_or(MeshVoxelizationError::InvalidTargetGeometry)?;
    let mut raw_data = vec![0_u8; voxel_count];
    let (min, max) = voxel_bounds(&vertices, dimensions);

    for z in min[2]..=max[2] {
        for y in min[1]..=max[1] {
            for x in min[0]..=max[0] {
                if trimesh.contains_local_point(Vector::new(x as f32, y as f32, z as f32)) {
                    raw_data[linear_index([x, y, z], dimensions)] = 1;
                }
            }
        }
    }

    Ok(VoxelData {
        geometry: target_geometry,
        raw_data,
    })
}

pub fn validate_mesh_for_voxelization(mesh: &MeshData) -> Result<(), MeshVoxelizationError> {
    welded_closed_mesh(mesh).map(|_| ())
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
    Ok((welded_vertices, welded_faces))
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
