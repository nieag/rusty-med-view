use crate::components::{MeshData, MeshFace, MeshVertex, VoxelData};
use crate::convert::voxel_index_to_world_mm;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoxelMeshExtractionError {
    InvalidRawDataLength { expected: usize, actual: usize },
}

pub fn extract_mesh_from_voxel_data(
    voxel_data: &VoxelData,
) -> Result<MeshData, VoxelMeshExtractionError> {
    let dimensions = voxel_data.geometry.dimensions;
    let expected_len = voxel_cell_count(dimensions);
    let actual_len = voxel_data.raw_data.len();
    if actual_len != expected_len {
        return Err(VoxelMeshExtractionError::InvalidRawDataLength {
            expected: expected_len,
            actual: actual_len,
        });
    }

    let mut mesh = MeshData {
        vertices: Vec::new(),
        faces: Vec::new(),
    };

    if expected_len == 0 {
        return Ok(mesh);
    }

    for z in 0..dimensions[2] {
        for y in 0..dimensions[1] {
            for x in 0..dimensions[0] {
                if !is_occupied(voxel_data, dimensions, x, y, z) {
                    continue;
                }

                for face in FACE_DEFINITIONS {
                    let nx = x as i64 + face.neighbor_offset[0];
                    let ny = y as i64 + face.neighbor_offset[1];
                    let nz = z as i64 + face.neighbor_offset[2];

                    if is_occupied_i64(voxel_data, dimensions, nx, ny, nz) {
                        continue;
                    }

                    append_face(&mut mesh, voxel_data, x as f32, y as f32, z as f32, face);
                }
            }
        }
    }

    Ok(mesh)
}

#[derive(Debug, Clone, Copy)]
struct FaceDefinition {
    neighbor_offset: [i64; 3],
    corners: [[f32; 3]; 4],
    triangles: [[u32; 3]; 2],
}

const FACE_DEFINITIONS: [FaceDefinition; 6] = [
    // -X
    FaceDefinition {
        neighbor_offset: [-1, 0, 0],
        corners: [
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 1.0],
            [0.0, 1.0, 0.0],
        ],
        triangles: [[0, 1, 2], [0, 2, 3]],
    },
    // +X
    FaceDefinition {
        neighbor_offset: [1, 0, 0],
        corners: [
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 1.0, 1.0],
            [1.0, 0.0, 1.0],
        ],
        triangles: [[0, 1, 2], [0, 2, 3]],
    },
    // -Y
    FaceDefinition {
        neighbor_offset: [0, -1, 0],
        corners: [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
        ],
        triangles: [[0, 1, 2], [0, 2, 3]],
    },
    // +Y
    FaceDefinition {
        neighbor_offset: [0, 1, 0],
        corners: [
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            [1.0, 1.0, 0.0],
        ],
        triangles: [[0, 1, 2], [0, 2, 3]],
    },
    // -Z
    FaceDefinition {
        neighbor_offset: [0, 0, -1],
        corners: [
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
        ],
        triangles: [[0, 1, 2], [0, 2, 3]],
    },
    // +Z
    FaceDefinition {
        neighbor_offset: [0, 0, 1],
        corners: [
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
        ],
        triangles: [[0, 1, 2], [0, 2, 3]],
    },
];

fn append_face(
    mesh: &mut MeshData,
    voxel_data: &VoxelData,
    x: f32,
    y: f32,
    z: f32,
    face: FaceDefinition,
) {
    let base_vertex = mesh.vertices.len() as u32;
    let cell_min_index = [x - 0.5, y - 0.5, z - 0.5];

    for corner in face.corners {
        let index = [
            cell_min_index[0] + corner[0],
            cell_min_index[1] + corner[1],
            cell_min_index[2] + corner[2],
        ];
        let world_mm = voxel_index_to_world_mm(index, voxel_data.geometry);
        mesh.vertices.push(MeshVertex { world_mm });
    }

    for tri in face.triangles {
        mesh.faces.push(MeshFace {
            vertex_indices: [
                base_vertex + tri[0],
                base_vertex + tri[1],
                base_vertex + tri[2],
            ],
        });
    }
}

fn voxel_cell_count(dimensions: [u32; 3]) -> usize {
    dimensions[0] as usize * dimensions[1] as usize * dimensions[2] as usize
}

fn voxel_linear_index(dimensions: [u32; 3], x: u32, y: u32, z: u32) -> usize {
    (z as usize * dimensions[1] as usize + y as usize) * dimensions[0] as usize + x as usize
}

fn is_occupied(voxel_data: &VoxelData, dimensions: [u32; 3], x: u32, y: u32, z: u32) -> bool {
    voxel_data.raw_data[voxel_linear_index(dimensions, x, y, z)] != 0
}

fn is_occupied_i64(voxel_data: &VoxelData, dimensions: [u32; 3], x: i64, y: i64, z: i64) -> bool {
    if x < 0 || y < 0 || z < 0 {
        return false;
    }

    let xu = x as u32;
    let yu = y as u32;
    let zu = z as u32;
    if xu >= dimensions[0] || yu >= dimensions[1] || zu >= dimensions[2] {
        return false;
    }

    is_occupied(voxel_data, dimensions, xu, yu, zu)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{VoxelData, VoxelGeometry};
    use glam::{Quat, Vec3};

    fn geometry(
        dimensions: [u32; 3],
        spacing: [f32; 3],
        origin: [f32; 3],
        orientation: [f32; 4],
    ) -> VoxelGeometry {
        VoxelGeometry {
            dimensions,
            spacing,
            origin,
            orientation,
        }
    }

    fn voxel_data_with_single_occupied(
        dimensions: [u32; 3],
        occupied: [u32; 3],
        spacing: [f32; 3],
        origin: [f32; 3],
        orientation: [f32; 4],
    ) -> VoxelData {
        let len = voxel_cell_count(dimensions);
        let mut raw = vec![0u8; len];
        let idx = voxel_linear_index(dimensions, occupied[0], occupied[1], occupied[2]);
        raw[idx] = 1;
        VoxelData {
            geometry: geometry(dimensions, spacing, origin, orientation),
            raw_data: raw,
        }
    }

    #[test]
    fn test_empty_voxel_data_produces_empty_mesh() {
        let voxel = VoxelData {
            geometry: geometry(
                [0, 0, 0],
                [1.0, 1.0, 1.0],
                [0.0, 0.0, 0.0],
                Quat::IDENTITY.to_array(),
            ),
            raw_data: Vec::new(),
        };

        let mesh = extract_mesh_from_voxel_data(&voxel).expect("empty data should succeed");
        assert!(mesh.vertices.is_empty());
        assert!(mesh.faces.is_empty());
    }

    #[test]
    fn test_simple_occupied_shape_produces_non_empty_mesh() {
        let voxel = voxel_data_with_single_occupied(
            [2, 2, 2],
            [0, 0, 0],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            Quat::IDENTITY.to_array(),
        );

        let mesh = extract_mesh_from_voxel_data(&voxel)
            .expect("single occupied voxel should produce surface");
        assert!(!mesh.vertices.is_empty());
        assert!(!mesh.faces.is_empty());
    }

    #[test]
    fn test_roi_owned_geometry_controls_vertex_placement() {
        let rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2).to_array();
        let voxel = voxel_data_with_single_occupied(
            [2, 2, 2],
            [0, 0, 0],
            [2.0, 3.0, 4.0],
            [10.0, 20.0, 30.0],
            rotation,
        );

        let mesh = extract_mesh_from_voxel_data(&voxel)
            .expect("single occupied voxel should produce mesh");
        let expected_world = Vec3::from_array(voxel.geometry.origin)
            + (Quat::from_array(rotation) * Vec3::new(1.0, -1.5, -2.0));

        let found = mesh.vertices.iter().any(|vertex| {
            let v = Vec3::from_array(vertex.world_mm);
            v.distance(expected_world) < 1e-5
        });
        assert!(
            found,
            "expected to find world-space vertex at transformed +X corner: {:?}",
            expected_world
        );
    }

    #[test]
    fn test_single_occupied_voxel_vertices_match_cell_boundary_aabb() {
        let voxel = voxel_data_with_single_occupied(
            [5, 6, 7],
            [2, 3, 4],
            [2.0, 3.0, 4.0],
            [10.0, 20.0, 30.0],
            Quat::IDENTITY.to_array(),
        );

        let mesh = extract_mesh_from_voxel_data(&voxel).expect("mesh extraction should succeed");
        assert!(!mesh.vertices.is_empty());

        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for vertex in &mesh.vertices {
            for axis in 0..3 {
                min[axis] = min[axis].min(vertex.world_mm[axis]);
                max[axis] = max[axis].max(vertex.world_mm[axis]);
            }
        }

        assert_eq!(min, [13.0, 27.5, 44.0]);
        assert_eq!(max, [15.0, 30.5, 48.0]);
    }

    #[test]
    fn test_extraction_is_deterministic_for_same_voxel_input() {
        let voxel = voxel_data_with_single_occupied(
            [3, 3, 3],
            [1, 1, 1],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            Quat::IDENTITY.to_array(),
        );

        let first = extract_mesh_from_voxel_data(&voxel).expect("first extraction should succeed");
        let second =
            extract_mesh_from_voxel_data(&voxel).expect("second extraction should succeed");
        assert_eq!(first, second);
    }

    #[test]
    fn test_single_voxel_topology_has_12_triangles() {
        let voxel = voxel_data_with_single_occupied(
            [2, 2, 2],
            [0, 0, 0],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            Quat::IDENTITY.to_array(),
        );
        let mesh = extract_mesh_from_voxel_data(&voxel).expect("mesh extraction should succeed");
        assert_eq!(mesh.faces.len(), 12);
    }

    #[test]
    fn test_solid_2x2x2_has_no_internal_faces() {
        let dimensions = [2, 2, 2];
        let voxel = VoxelData {
            geometry: geometry(
                dimensions,
                [1.0, 1.0, 1.0],
                [0.0, 0.0, 0.0],
                Quat::IDENTITY.to_array(),
            ),
            raw_data: vec![1; 8],
        };
        let mesh = extract_mesh_from_voxel_data(&voxel).expect("mesh extraction should succeed");
        // 2x2x2 solid block: 24 exterior quads -> 48 triangles.
        assert_eq!(mesh.faces.len(), 48);
    }

    #[test]
    fn test_invalid_raw_data_length_returns_error() {
        let voxel = VoxelData {
            geometry: geometry(
                [2, 2, 2],
                [1.0, 1.0, 1.0],
                [0.0, 0.0, 0.0],
                Quat::IDENTITY.to_array(),
            ),
            raw_data: vec![1; 7],
        };
        let err =
            extract_mesh_from_voxel_data(&voxel).expect_err("should reject invalid raw length");
        assert_eq!(
            err,
            VoxelMeshExtractionError::InvalidRawDataLength {
                expected: 8,
                actual: 7
            }
        );
    }

    #[test]
    fn test_all_non_zero_values_are_occupied() {
        let voxel = VoxelData {
            geometry: geometry(
                [2, 1, 1],
                [1.0, 1.0, 1.0],
                [0.0, 0.0, 0.0],
                Quat::IDENTITY.to_array(),
            ),
            raw_data: vec![1, 2],
        };
        let mesh = extract_mesh_from_voxel_data(&voxel).expect("mesh extraction should succeed");
        // Two adjacent occupied voxels: 10 exposed faces -> 20 triangles.
        assert_eq!(mesh.faces.len(), 20);
    }
}
