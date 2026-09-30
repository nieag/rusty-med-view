use super::*;
use crate::convert::{extract_mesh_from_voxel_data, validate_mesh_for_voxelization};
use crate::model::{VoxelData, VoxelGeometry};

/// A 6 x 6 x 2 voxel slab: closed, and thin enough that pushing one face in reaches the other.
fn slab() -> MeshData {
    let geometry = VoxelGeometry::new([8, 8, 4], [1.0; 3], [0.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap();
    let mut raw_data = vec![0_u8; 8 * 8 * 4];
    for z in 1..3 {
        for y in 1..7 {
            for x in 1..7 {
                raw_data[(z * 8 + y) * 8 + x] = 1;
            }
        }
    }
    let mesh = extract_mesh_from_voxel_data(&VoxelData { geometry, raw_data }).unwrap();
    validate_mesh_for_voxelization(&mesh).expect("the slab is a valid closed surface");
    mesh
}

/// A triangle of the slab's top face nearest its centre, and a point on it.
fn top_face_seed(mesh: &MeshData) -> ([u32; 3], [f32; 3]) {
    let top = mesh
        .vertices
        .iter()
        .map(|vertex| vertex.world_mm[2])
        .fold(f32::MIN, f32::max);
    mesh.faces
        .iter()
        .filter(|face| {
            face.vertex_indices
                .iter()
                .all(|index| (mesh.vertices[*index as usize].world_mm[2] - top).abs() < 1e-4)
        })
        .min_by(|a, b| {
            let distance = |face: &crate::model::MeshFace| {
                let centre = face.vertex_indices.iter().fold(Vec3::ZERO, |sum, index| {
                    sum + Vec3::from_array(mesh.vertices[*index as usize].world_mm)
                }) / 3.0;
                centre.distance(Vec3::new(4.0, 4.0, top))
            };
            distance(a).total_cmp(&distance(b))
        })
        .map(|face| {
            let anchor = face.vertex_indices.iter().fold(Vec3::ZERO, |sum, index| {
                sum + Vec3::from_array(mesh.vertices[*index as usize].world_mm)
            }) / 3.0;
            (face.vertex_indices, anchor.to_array())
        })
        .expect("the slab has a top face")
}

#[test]
fn test_pushing_a_face_through_the_opposite_face_is_limited_to_stay_valid() {
    let mesh = slab();
    let (seeds, anchor) = top_face_seed(&mesh);
    let base = MeshDeformBase::new(&mesh).unwrap();
    let displacement = brush_displacement(&base, seeds, anchor, [0.0, 0.0, -6.0], 3.0, 1.0);

    let unlimited = apply_displacement(&base, &mesh, &displacement, 1.0);
    assert!(
        validate_mesh_for_voxelization(&unlimited).is_err(),
        "the full push must fold the slab through itself for this test to mean anything"
    );

    let (limited, fraction) =
        deform_mesh_surface_brush_limited(&base, &mesh, seeds, anchor, [0.0, 0.0, -6.0], 3.0, 1.0);
    assert!(fraction > 0.0 && fraction < 1.0, "fraction {fraction}");
    validate_mesh_for_voxelization(&limited).expect("the limited push keeps the surface valid");
    assert_ne!(limited, mesh, "the surface still moved as far as it could");
}

#[test]
fn test_a_drag_away_from_the_surface_is_not_limited() {
    let mesh = slab();
    let (seeds, anchor) = top_face_seed(&mesh);
    let base = MeshDeformBase::new(&mesh).unwrap();

    let (deformed, fraction) =
        deform_mesh_surface_brush_limited(&base, &mesh, seeds, anchor, [0.0, 0.0, 1.5], 3.0, 1.0);

    assert_eq!(fraction, 1.0);
    validate_mesh_for_voxelization(&deformed).expect("an outward pull stays valid");
}

#[test]
fn test_coincident_vertices_move_together() {
    let mesh = MeshData {
        vertices: [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0], // coincides with vertex 0
            [1.0, 1.0, 0.0],
        ]
        .map(|world_mm| MeshVertex { world_mm })
        .to_vec(),
        faces: [[0, 1, 2], [3, 4, 1]]
            .map(|vertex_indices| crate::model::MeshFace { vertex_indices })
            .to_vec(),
    };
    let base = MeshDeformBase::new(&mesh).unwrap();
    let displacement = brush_displacement(&base, [0, 1, 2], [0.0; 3], [0.0, 0.0, 1.0], 2.0, 1.0);
    let moved = apply_displacement(&base, &mesh, &displacement, 1.0);

    assert_eq!(moved.vertices[0], moved.vertices[3]);
    assert!(moved.vertices[0].world_mm[2] > 0.0);
}

#[test]
fn test_a_face_with_a_missing_vertex_has_no_deform_base() {
    let mesh = MeshData {
        vertices: vec![MeshVertex { world_mm: [0.0; 3] }],
        faces: vec![crate::model::MeshFace {
            vertex_indices: [0, 1, 2],
        }],
    };
    assert!(MeshDeformBase::new(&mesh).is_none());
}
