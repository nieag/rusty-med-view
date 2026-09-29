use super::*;
use crate::components::{MeshFace, MeshVertex};
use crate::convert::extract_mesh_from_voxel_data;
use glam::Quat;

fn geometry() -> VoxelGeometry {
    VoxelGeometry::new(
        [4, 4, 4],
        [1.0, 2.0, 3.0],
        [10.0, 20.0, 30.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap()
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
    let geometry = VoxelGeometry::new(
        [5, 4, 3],
        [1.5, 2.0, 3.5],
        [10.0, -4.0, 22.0],
        Quat::from_rotation_y(0.4).to_array(),
    )
    .unwrap();
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
    let geometry = VoxelGeometry::new([5, 5, 4], [1.0; 3], [0.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap();
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
