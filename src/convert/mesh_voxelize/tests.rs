use super::*;
use crate::convert::extract_mesh_from_voxel_data;
use crate::model::{MeshFace, MeshVertex};
use glam::Quat;
use parry3d::math::Vector;
use parry3d::shape::TriMesh;

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
        mesh = crate::convert::deform_mesh_surface_brush(
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

/// The point-in-mesh query per voxel that the scanline replaced, kept as the reference.
fn voxelize_by_point_queries(mesh: &MeshData, geometry: VoxelGeometry) -> Vec<u8> {
    use parry3d::query::PointQuery;
    use parry3d::shape::TriMeshFlags;
    let (vertices, indices) = welded_closed_mesh(mesh, false).unwrap();
    let vertices: Vec<Vector> = vertices
        .iter()
        .map(|world_mm| {
            let voxel = world_mm_to_voxel_index(*world_mm, geometry);
            Vector::new(voxel[0], voxel[1], voxel[2])
        })
        .collect();
    let tri_mesh = TriMesh::with_flags(vertices, indices, TriMeshFlags::ORIENTED).unwrap();
    let dimensions = geometry.dimensions;
    let mut raw = vec![0_u8; dimensions.iter().map(|d| *d as usize).product()];
    for z in 0..dimensions[2] {
        for y in 0..dimensions[1] {
            for x in 0..dimensions[0] {
                if tri_mesh.contains_local_point(Vector::new(x as f32, y as f32, z as f32)) {
                    raw[linear_index([x, y, z], dimensions)] = 1;
                }
            }
        }
    }
    raw
}

#[test]
fn test_scanline_voxelization_matches_point_queries_on_random_surfaces() {
    let dimensions = [11, 9, 8];
    let mut seed = 0x1234_5678_u32;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    for case in 0..80 {
        let geometry = VoxelGeometry::new(
            dimensions,
            [[1.0, 1.0, 1.0], [0.7, 1.3, 2.1], [2.0, 2.0, 3.0]][case % 3],
            [10.0, 20.0, 30.0],
            Quat::from_rotation_y(0.4).to_array(),
        )
        .unwrap();
        let raw_data: Vec<u8> = (0..dimensions.iter().product::<u32>())
            .map(|_| u8::from(next() % 3 == 0))
            .collect();
        if raw_data.iter().all(|value| *value == 0) {
            continue;
        }
        let mesh = extract_mesh_from_voxel_data(&VoxelData { geometry, raw_data }).unwrap();
        // The surface of a random mask passes through grid lines everywhere: the hard case.
        let fast = voxelize_mesh_to_voxel_data(&mesh, geometry)
            .unwrap()
            .raw_data;
        let reference = voxelize_by_point_queries(&mesh, geometry);
        assert_eq!(fast, reference, "case {case}");
    }
}

#[test]
fn test_scanline_voxelization_matches_point_queries_on_a_deformed_surface() {
    let dimensions = [16, 16, 16];
    let geometry =
        VoxelGeometry::new(dimensions, [1.0, 1.5, 2.0], [0.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap();
    let mut raw_data = vec![0_u8; 16 * 16 * 16];
    for z in 4..12 {
        for y in 3..13 {
            for x in 2..14 {
                raw_data[linear_index([x, y, z], dimensions)] = 1;
            }
        }
    }
    let mut mesh = extract_mesh_from_voxel_data(&VoxelData { geometry, raw_data }).unwrap();
    // Push vertices off the grid lines by smooth, non-uniform amounts, as a mesh edit would.
    for (index, vertex) in mesh.vertices.iter_mut().enumerate() {
        let t = index as f32 * 0.37;
        vertex.world_mm[0] += 0.4 * t.sin();
        vertex.world_mm[1] += 0.3 * (t * 1.3).cos();
        vertex.world_mm[2] += 0.5 * (t * 0.7).sin();
    }
    let fast = voxelize_mesh_to_voxel_data(&mesh, geometry)
        .unwrap()
        .raw_data;
    let reference = voxelize_by_point_queries(&mesh, geometry);
    let differing = fast.iter().zip(&reference).filter(|(a, b)| a != b).count();
    // A sample closer to the surface than the tiny sample offset may differ; none is expected.
    assert_eq!(differing, 0);
}
