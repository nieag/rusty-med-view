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

    let mesh =
        extract_mesh_from_voxel_data(&voxel).expect("single occupied voxel should produce surface");
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
    let identity = voxel_data_with_single_occupied(
        [2, 2, 2],
        [0, 0, 0],
        [2.0, 3.0, 4.0],
        [0.0, 0.0, 0.0],
        Quat::IDENTITY.to_array(),
    );
    let mesh = extract_mesh_from_voxel_data(&voxel).unwrap();
    let identity_mesh = extract_mesh_from_voxel_data(&identity).unwrap();

    for (rotated, local) in mesh.vertices.iter().zip(&identity_mesh.vertices) {
        let expected = Vec3::from_array(voxel.geometry.origin)
            + Quat::from_array(rotation) * Vec3::from_array(local.world_mm);
        assert!(Vec3::from_array(rotated.world_mm).distance(expected) < 1e-5);
    }
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

    assert!(min.into_iter().zip([13.0, 27.5, 44.0]).all(|(a, b)| a >= b));
    assert!(max.into_iter().zip([15.0, 30.5, 48.0]).all(|(a, b)| a <= b));
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
    let second = extract_mesh_from_voxel_data(&voxel).expect("second extraction should succeed");
    assert_eq!(first, second);
}

#[test]
fn test_single_voxel_smooth_topology_has_no_cube_faces() {
    let voxel = voxel_data_with_single_occupied(
        [2, 2, 2],
        [0, 0, 0],
        [1.0, 1.0, 1.0],
        [0.0, 0.0, 0.0],
        Quat::IDENTITY.to_array(),
    );
    let mesh = extract_mesh_from_voxel_data(&voxel).expect("mesh extraction should succeed");
    assert_eq!(mesh.faces.len(), 8);
}

#[test]
fn test_solid_2x2x2_smooth_mesh_is_smaller_than_cube_faces() {
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
    assert!(mesh.faces.len() < 48);
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
    let err = extract_mesh_from_voxel_data(&voxel).expect_err("should reject invalid raw length");
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
    assert!(mesh.faces.len() < 20);
}

fn canonical_triangles(mesh: &MeshData) -> Vec<[[i32; 3]; 3]> {
    let mut triangles = mesh
        .faces
        .iter()
        .map(|face| {
            let mut points = face.vertex_indices.map(|index| {
                mesh.vertices[index as usize]
                    .world_mm
                    .map(|value| (value * 2.0).round() as i32)
            });
            points.sort();
            points
        })
        .collect::<Vec<_>>();
    triangles.sort();
    triangles
}

fn canonical_triangle_bits(mesh: &MeshData) -> Vec<[[u32; 3]; 3]> {
    let mut triangles = mesh
        .faces
        .iter()
        .map(|face| {
            let mut points = face
                .vertex_indices
                .map(|index| mesh.vertices[index as usize].world_mm.map(f32::to_bits));
            points.sort();
            points
        })
        .collect::<Vec<_>>();
    triangles.sort();
    triangles
}

#[test]
fn test_chunked_extraction_matches_full_mesh_across_chunk_boundaries() {
    let dimensions = [5, 3, 2];
    let mut raw_data = vec![0; voxel_cell_count(dimensions)];
    for occupied in [[1, 1, 0], [2, 1, 0], [4, 2, 1]] {
        raw_data[voxel_linear_index(dimensions, occupied[0], occupied[1], occupied[2])] = 1;
    }
    let voxel = VoxelData {
        geometry: geometry(
            dimensions,
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            Quat::IDENTITY.to_array(),
        ),
        raw_data,
    };

    let full = extract_mesh_from_voxel_data(&voxel).unwrap();
    let chunked = extract_chunked_mesh_from_voxel_data(&voxel, 2).unwrap();

    assert_eq!(
        canonical_triangles(&chunked.merged_mesh()),
        canonical_triangles(&full)
    );
}

#[test]
fn test_chunk_seams_share_exact_vertices_on_anisotropic_curved_surface() {
    let dimensions = [9; 3];
    let mut raw_data = vec![0; voxel_cell_count(dimensions)];
    for z in 0..9 {
        for y in 0..9 {
            for x in 0..9 {
                let radius_squared: i32 = [x, y, z].map(|v| (v as i32 - 4).pow(2)).iter().sum();
                raw_data[voxel_linear_index(dimensions, x, y, z)] = u8::from(radius_squared <= 12);
            }
        }
    }
    for spacing in [[0.7, 1.3, 2.1], [3.0, 0.5, 0.9]] {
        let voxel = VoxelData {
            geometry: geometry(
                dimensions,
                spacing,
                [0.0; 3],
                Quat::from_rotation_y(0.4).to_array(),
            ),
            raw_data: raw_data.clone(),
        };
        let full = extract_mesh_from_voxel_data(&voxel).unwrap();
        for chunk_size in [1, 2, 3, 4] {
            let merged = extract_chunked_mesh_from_voxel_data(&voxel, chunk_size)
                .unwrap()
                .merged_mesh();
            assert_eq!(
                merged.vertices.len(),
                full.vertices.len(),
                "spacing={spacing:?}, chunk={chunk_size}"
            );
            crate::convert::validate_mesh_for_voxelization(&merged).unwrap();
        }
    }
}

#[test]
#[ignore = "chunk topology stress QA"]
fn test_random_chunked_masks_match_full_mesh_and_roundtrip() {
    let dimensions = [3; 3];
    let mut seed = 0x5eed_u32;
    for case in 0..1024 {
        let mut raw_data = vec![0; voxel_cell_count(dimensions)];
        for voxel in &mut raw_data {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *voxel = u8::from(seed & 1 != 0);
        }
        let source = VoxelData {
            geometry: geometry(
                dimensions,
                [0.7, 1.3, 2.1],
                [10.0, 20.0, 30.0],
                Quat::from_rotation_y(0.4).to_array(),
            ),
            raw_data,
        };
        let full = extract_mesh_from_voxel_data(&source).unwrap();
        for chunk_size in [1, 2] {
            let merged = extract_chunked_mesh_from_voxel_data(&source, chunk_size)
                .unwrap()
                .merged_mesh();
            assert_eq!(
                canonical_triangle_bits(&merged),
                canonical_triangle_bits(&full),
                "case={case}, chunk={chunk_size}"
            );
            let rebuilt = crate::convert::voxelize_mesh_to_voxel_data(&merged, source.geometry)
                .unwrap_or_else(|error| panic!("case={case}, chunk={chunk_size}: {error:?}"));
            assert_eq!(
                rebuilt.raw_data, source.raw_data,
                "case={case}, chunk={chunk_size}"
            );
        }
    }
}

#[test]
#[ignore = "incremental chunk topology stress QA"]
fn test_random_single_voxel_updates_match_full_rebuild() {
    let dimensions = [5; 3];
    let changed = [2, 2, 2];
    let changed_index = voxel_linear_index(dimensions, changed[0], changed[1], changed[2]);
    let mut seed = 0x51ce_u32;
    for case in 0..256 {
        let mut raw_data = vec![0; voxel_cell_count(dimensions)];
        for voxel in &mut raw_data {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *voxel = u8::from(seed & if case % 2 == 0 { 1 } else { 7 } == 0);
        }
        let mut source = VoxelData {
            geometry: geometry(
                dimensions,
                [0.7, 1.3, 2.1],
                [10.0, 20.0, 30.0],
                Quat::from_rotation_y(0.4).to_array(),
            ),
            raw_data,
        };
        for chunk_size in [1, 2] {
            let mut chunked = extract_chunked_mesh_from_voxel_data(&source, chunk_size).unwrap();
            source.raw_data[changed_index] ^= 1;
            rebuild_chunked_mesh_for_voxel_aabb(
                &mut chunked,
                &source,
                changed,
                changed.map(|value| value + 1),
            )
            .unwrap();
            let full = extract_mesh_from_voxel_data(&source).unwrap();
            let actual = canonical_triangle_bits(&chunked.merged_mesh());
            let expected = canonical_triangle_bits(&full);
            assert!(
                actual == expected,
                "case={case}, chunk={chunk_size}, occupied={:?}, triangles={} vs {}",
                source
                    .raw_data
                    .iter()
                    .enumerate()
                    .filter_map(|(index, value)| (*value != 0).then_some(index))
                    .collect::<Vec<_>>(),
                actual.len(),
                expected.len()
            );
            source.raw_data[changed_index] ^= 1;
        }
    }
}

#[test]
fn test_dirty_sdf_rebuild_updates_chunks_beyond_one_voxel_halo() {
    let dimensions = [5; 3];
    let mut raw_data = vec![0; voxel_cell_count(dimensions)];
    for index in [
        7, 33, 34, 47, 57, 67, 73, 75, 81, 83, 86, 89, 100, 106, 115, 118, 123,
    ] {
        raw_data[index] = 1;
    }
    let mut source = VoxelData {
        geometry: geometry(
            dimensions,
            [0.7, 1.3, 2.1],
            [10.0, 20.0, 30.0],
            Quat::from_rotation_y(0.4).to_array(),
        ),
        raw_data,
    };
    let mut chunked = extract_chunked_mesh_from_voxel_data(&source, 1).unwrap();
    source.raw_data[voxel_linear_index(dimensions, 2, 2, 2)] = 1;
    rebuild_chunked_mesh_for_voxel_aabb(&mut chunked, &source, [2; 3], [3; 3]).unwrap();
    let full = extract_mesh_from_voxel_data(&source).unwrap();
    assert_eq!(
        canonical_triangle_bits(&chunked.merged_mesh()),
        canonical_triangle_bits(&full)
    );
}

#[test]
#[ignore = "liver dirty-rebuild timing QA"]
fn test_liver_dirty_mesh_rebuild_matches_clean_full_rebuild() {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/qa_samples/liver_0_label.nii"
    ))
    .unwrap();
    let label =
        crate::nifti_loader::load_label_from_bytes(&bytes, "liver_0_label.nii".into()).unwrap();
    let mut voxels = VoxelData {
        geometry: geometry(
            label.dimensions,
            label.spacing,
            label.origin,
            label.orientation,
        ),
        raw_data: label.data,
    };
    let base = extract_chunked_mesh_from_voxel_data(&voxels, DEFAULT_MESH_CHUNK_SIZE).unwrap();
    let changed_index = voxels
        .raw_data
        .iter()
        .position(|value| *value != 0)
        .unwrap();
    voxels.raw_data[changed_index] = 0;
    let [width, height, _] = voxels.geometry.dimensions;
    let changed = [
        changed_index as u32 % width,
        (changed_index as u32 / width) % height,
        changed_index as u32 / (width * height),
    ];
    let setup_started = std::time::Instant::now();
    let mut work = IncrementalChunkedMeshRebuild::begin_for_voxel_aabb(
        base,
        &voxels,
        changed,
        changed.map(|value| value + 1),
    )
    .unwrap();
    let setup_duration = setup_started.elapsed();
    let chunk_started = std::time::Instant::now();
    while !work.step(&voxels).unwrap() {}
    let chunk_duration = chunk_started.elapsed();
    let rebuilt = work.into_result().unwrap();
    eprintln!("liver dirty mesh rebuild: setup={setup_duration:?}, chunks={chunk_duration:?}");
    let clean = extract_chunked_mesh_from_voxel_data(&voxels, DEFAULT_MESH_CHUNK_SIZE).unwrap();
    assert_eq!(
        canonical_triangle_bits(&rebuilt.merged_mesh()),
        canonical_triangle_bits(&clean.merged_mesh())
    );
}

#[test]
fn test_incremental_chunk_rebuild_matches_clean_full_rebuild() {
    let dimensions = [6, 2, 1];
    let mut voxel = voxel_data_with_single_occupied(
        dimensions,
        [1, 0, 0],
        [1.0, 1.0, 1.0],
        [0.0, 0.0, 0.0],
        Quat::IDENTITY.to_array(),
    );
    voxel.raw_data[voxel_linear_index(dimensions, 5, 0, 0)] = 1;
    let mut chunked = extract_chunked_mesh_from_voxel_data(&voxel, 2).unwrap();
    voxel.raw_data[voxel_linear_index(dimensions, 2, 0, 0)] = 1;
    let rebuilt =
        rebuild_chunked_mesh_for_voxel_aabb(&mut chunked, &voxel, [2, 0, 0], [3, 1, 1]).unwrap();
    let full = extract_mesh_from_voxel_data(&voxel).unwrap();

    assert_eq!(
        rebuilt.iter().map(|key| key.index).collect::<Vec<_>>(),
        vec![[0, 0, 0], [1, 0, 0], [2, 0, 0]]
    );
    assert_eq!(
        canonical_triangles(&chunked.merged_mesh()),
        canonical_triangles(&full)
    );
}

#[test]
fn test_incremental_rebuild_advances_one_chunk_per_step() {
    let dimensions = [40, 2, 1];
    let mut voxel = voxel_data_with_single_occupied(
        dimensions,
        [1, 0, 0],
        [1.0, 1.0, 1.0],
        [0.0, 0.0, 0.0],
        Quat::IDENTITY.to_array(),
    );
    voxel.raw_data[voxel_linear_index(dimensions, 20, 0, 0)] = 1;
    voxel.raw_data[voxel_linear_index(dimensions, 39, 0, 0)] = 1;
    let mut rebuild = IncrementalChunkedMeshRebuild::begin_full(&voxel, 16).unwrap();

    assert_eq!(rebuild.remaining_chunks(), 3);
    assert!(!rebuild.step(&voxel).unwrap());
    assert_eq!(rebuild.remaining_chunks(), 2);
    assert!(!rebuild.step(&voxel).unwrap());
    assert_eq!(rebuild.remaining_chunks(), 1);
    assert!(rebuild.step(&voxel).unwrap());

    let incremental = rebuild.into_result().unwrap();
    let clean = extract_chunked_mesh_from_voxel_data(&voxel, 16).unwrap();
    assert_eq!(incremental, clean);
}

#[test]
fn test_incremental_dirty_rebuild_scans_all_chunks_for_global_sdf() {
    let dimensions = [48, 2, 1];
    let mut voxel = voxel_data_with_single_occupied(
        dimensions,
        [1, 0, 0],
        [1.0, 1.0, 1.0],
        [0.0, 0.0, 0.0],
        Quat::IDENTITY.to_array(),
    );
    voxel.raw_data[voxel_linear_index(dimensions, 40, 0, 0)] = 1;
    let base = extract_chunked_mesh_from_voxel_data(&voxel, 16).unwrap();
    voxel.raw_data[voxel_linear_index(dimensions, 16, 0, 0)] = 1;
    let mut rebuild =
        IncrementalChunkedMeshRebuild::begin_for_voxel_aabb(base, &voxel, [16, 0, 0], [17, 1, 1])
            .unwrap();
    assert_eq!(
        rebuild
            .changed_keys()
            .iter()
            .map(|key| key.index)
            .collect::<Vec<_>>(),
        vec![[0, 0, 0], [1, 0, 0], [2, 0, 0]]
    );
    while !rebuild.step(&voxel).unwrap() {}
    let result = rebuild.into_result().unwrap();
    assert_eq!(
        canonical_triangles(&result.merged_mesh()),
        canonical_triangles(&extract_mesh_from_voxel_data(&voxel).unwrap())
    );
}
