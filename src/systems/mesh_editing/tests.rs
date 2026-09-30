use super::*;
use crate::app::roi::VoxelGeometry;
use crate::components::{
    EditorState, InputState, MainVolumeTag, MeshFace, MeshVertex, RoiId, VolumeData,
};

fn spawn_mesh_edit_world() -> (World, Session, hecs::Entity) {
    let mut world = World::new();
    world.spawn((
        VolumeData {
            dimensions: [10, 10, 10],
            geometry: Some(
                VoxelGeometry::new(
                    [10, 10, 10],
                    [1.0, 1.0, 1.0],
                    [0.0, 0.0, 0.0],
                    [0.0, 0.0, 0.0, 1.0],
                )
                .unwrap(),
            ),
            intensities: Vec::new(),
            intensity_range: [0.0, 1.0],
        },
        MainVolumeTag,
    ));
    let viewport = world.spawn((
        Viewport {
            mode: ViewMode::ThreeD,
            rect: [0.0, 0.0, 800.0, 600.0],
            uniform_index: 0,
        },
        ViewportState::default(),
    ));
    let mesh = MeshData {
        vertices: vec![
            MeshVertex {
                world_mm: [4.0, 4.0, 4.0],
            },
            MeshVertex {
                world_mm: [5.0, 4.0, 4.0],
            },
            MeshVertex {
                world_mm: [4.0, 5.0, 4.0],
            },
            MeshVertex {
                world_mm: [4.0, 4.0, 5.0],
            },
        ],
        faces: vec![
            MeshFace {
                vertex_indices: [0, 2, 1],
            },
            MeshFace {
                vertex_indices: [0, 1, 3],
            },
            MeshFace {
                vertex_indices: [0, 3, 2],
            },
            MeshFace {
                vertex_indices: [1, 2, 3],
            },
        ],
    };
    let roi_entity = world.spawn((Roi::new_mesh(RoiId(1), "Mesh".to_string(), mesh),));
    let mut session = Session::new(800, 600);
    session.editor = EditorState {
        active_roi: Some(roi_entity),
        active_tool: EditorTool::MeshDeform,
        ..EditorState::default()
    };
    session.input = InputState {
        active_viewport: Some(viewport),
        ..InputState::default()
    };
    (world, session, roi_entity)
}

#[test]
fn test_surface_brush_does_not_cross_disconnected_nearby_surface() {
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
            MeshVertex {
                world_mm: [0.0, 0.0, 0.2],
            },
            MeshVertex {
                world_mm: [1.0, 0.0, 0.2],
            },
            MeshVertex {
                world_mm: [0.0, 1.0, 0.2],
            },
        ],
        faces: vec![
            MeshFace {
                vertex_indices: [0, 1, 2],
            },
            MeshFace {
                vertex_indices: [3, 4, 5],
            },
        ],
    };

    let deformed =
        deform_mesh_surface_brush(&mesh, [0, 1, 2], [0.0, 0.0, 0.0], [0.0, 0.0, 1.0], 2.0, 1.0);

    assert!(deformed.vertices[0].world_mm[2] > mesh.vertices[0].world_mm[2]);
    assert_eq!(deformed.vertices[3], mesh.vertices[3]);
    assert_eq!(deformed.vertices[4], mesh.vertices[4]);
    assert_eq!(deformed.vertices[5], mesh.vertices[5]);
}

#[test]
fn test_surface_brush_expands_area_with_strength_scaled_drag() {
    let mesh = MeshData {
        vertices: [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [4.0, 0.0, 0.0],
        ]
        .map(|world_mm| MeshVertex { world_mm })
        .to_vec(),
        faces: [[0, 1, 2], [1, 3, 2]]
            .map(|vertex_indices| MeshFace { vertex_indices })
            .to_vec(),
    };
    let short = deform_mesh_surface_brush(&mesh, [0, 1, 2], [0.0; 3], [0.0, 0.0, 0.1], 2.0, 1.0);
    assert_eq!(short.vertices[3], mesh.vertices[3]);
    let long = deform_mesh_surface_brush(&mesh, [0, 1, 2], [0.0; 3], [0.0, 0.0, 4.0], 2.0, 1.0);
    assert!(
        long.vertices[3].world_mm[2] > 0.0,
        "long drag must reach outside base radius"
    );
    assert!(
        long.vertices[3].world_mm[2] < 2.0,
        "outer influence should be weaker than the initial aggressive growth"
    );
    assert_eq!(
        long.vertices[0].world_mm,
        [0.0, 0.0, 4.0],
        "do not clamp requested displacement"
    );
    let stronger = deform_mesh_surface_brush(&mesh, [0, 1, 2], [0.0; 3], [0.0, 0.0, 2.0], 2.0, 2.0);
    assert_eq!(long, stronger);
    let zero = deform_mesh_surface_brush(&mesh, [0, 1, 2], [0.0; 3], [0.0; 3], 2.0, 1.0);
    assert_eq!(zero, mesh);
}

#[test]
fn test_chunked_surface_stays_closed_after_deformation() {
    use crate::convert::{extract_chunked_mesh_from_voxel_data, validate_mesh_for_voxelization};
    let voxels = crate::components::VoxelData {
        geometry: crate::components::VoxelGeometry::new(
            [4; 3],
            [1.0; 3],
            [0.0; 3],
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap(),
        raw_data: vec![1; 64],
    };
    let chunks = extract_chunked_mesh_from_voxel_data(&voxels, 2).unwrap();
    assert!(chunks.chunks.len() > 1);
    let mesh = chunks.merged_mesh();
    validate_mesh_for_voxelization(&mesh).unwrap();
    let seeds = mesh.faces[0].vertex_indices;
    let anchor = mesh.vertices[seeds[0] as usize].world_mm;
    let deformed = deform_mesh_surface_brush(&mesh, seeds, anchor, [0.1, 0.2, 0.1], 20.0, 1.0);
    assert_ne!(deformed.vertices, mesh.vertices);
    validate_mesh_for_voxelization(&deformed).unwrap();
    let long_drag = deform_mesh_surface_brush(&mesh, seeds, anchor, [8.0, 0.0, -6.0], 1.0, 1.0);
    assert!(
        long_drag
            .vertices
            .iter()
            .zip(&mesh.vertices)
            .all(|(after, before)| after != before),
        "adaptive influence must spread this long drag across the small connected solid"
    );
    validate_mesh_for_voxelization(&long_drag).unwrap();
}

#[test]
fn test_deformed_chunked_mesh_resamples_on_rotated_anisotropic_grid() {
    use crate::convert::{extract_chunked_mesh_from_voxel_data, voxelize_mesh_to_voxel_data};
    let geometry = crate::components::VoxelGeometry::new(
        [8; 3],
        [1.0, 2.0, 3.0],
        [10.0, -20.0, 30.0],
        glam::Quat::from_rotation_y(0.4).to_array(),
    )
    .unwrap();
    let mut voxels = crate::components::VoxelData {
        geometry,
        raw_data: vec![0; 512],
    };
    for z in 2..6 {
        for y in 2..6 {
            for x in 2..6 {
                voxels.raw_data[(z * 8 + y) * 8 + x] = 1;
            }
        }
    }
    let mesh = extract_chunked_mesh_from_voxel_data(&voxels, 2)
        .unwrap()
        .merged_mesh();
    let seeds = mesh.faces[0].vertex_indices;
    let anchor = mesh.vertices[seeds[0] as usize].world_mm;
    let small_drag = deform_mesh_surface_brush(&mesh, seeds, anchor, [0.1, 0.1, 0.1], 5.0, 1.0);
    assert_ne!(small_drag.vertices, mesh.vertices);
    assert_eq!(
        voxelize_mesh_to_voxel_data(&small_drag, geometry)
            .unwrap()
            .raw_data,
        voxels.raw_data
    );

    // A broad brush moving one IJK step must shift occupancy in the owned
    // grid, not world X, and must not leave detached voxels at chunk seams.
    let delta = glam::Quat::from_array(geometry.orientation()) * Vec3::X * geometry.spacing()[0];
    let translated =
        deform_mesh_surface_brush(&mesh, seeds, anchor, delta.to_array(), 10000.0, 1.0);
    let actual = voxelize_mesh_to_voxel_data(&translated, geometry).unwrap();
    let mut expected = vec![0; 512];
    for z in 2..6 {
        for y in 2..6 {
            for x in 3..7 {
                expected[(z * 8 + y) * 8 + x] = 1;
            }
        }
    }
    assert_eq!(actual.raw_data, expected);
}

#[test]
#[ignore = "full liver extraction and voxelization; run explicitly for milestone QA"]
fn test_liver_deformation_preserves_closed_surface_and_local_voxel_changes() {
    use crate::convert::{
        extract_chunked_mesh_from_voxel_data, validate_mesh_for_voxelization,
        voxel_index_to_world_mm, voxelize_mesh_to_voxel_data, DEFAULT_MESH_CHUNK_SIZE,
    };
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/qa_samples/liver_0_label.nii"
    ))
    .unwrap();
    let label =
        crate::nifti_loader::load_label_from_bytes(&bytes, "liver_0_label.nii".into()).unwrap();
    let geometry = label.geometry;
    let voxels = crate::components::VoxelData {
        geometry,
        raw_data: label.data,
    };
    let started = std::time::Instant::now();
    let mesh = extract_chunked_mesh_from_voxel_data(&voxels, DEFAULT_MESH_CHUNK_SIZE)
        .unwrap()
        .merged_mesh();
    eprintln!(
        "liver extraction: {:?}, {} vertices, {} faces",
        started.elapsed(),
        mesh.vertices.len(),
        mesh.faces.len()
    );
    let started = std::time::Instant::now();
    validate_mesh_for_voxelization(&mesh).expect("undeformed liver must be closed");
    eprintln!("liver baseline validation: {:?}", started.elapsed());
    let started = std::time::Instant::now();
    let baseline = voxelize_mesh_to_voxel_data(&mesh, geometry).unwrap();
    eprintln!("liver baseline resample: {:?}", started.elapsed());
    assert_eq!(
        baseline
            .raw_data
            .iter()
            .zip(&voxels.raw_data)
            .filter(|(a, b)| (**a != 0) != (**b != 0))
            .count(),
        0,
        "undeformed liver roundtrip"
    );

    let seeds = mesh.faces[mesh.faces.len() / 2].vertex_indices;
    let anchor = mesh.vertices[seeds[0] as usize].world_mm;
    for (radius, delta, expected_valid) in [
        (20.0_f32, [2.0, 1.0, -1.0], true),
        (12.0, [24.0, 12.0, -12.0], false),
    ] {
        let deformed = deform_mesh_surface_brush(&mesh, seeds, anchor, delta, radius, 1.0);
        let started = std::time::Instant::now();
        if !expected_valid {
            // Adaptive area does not prevent every collision. Preserve this
            // observed large-drag failure as a voxelization rejection case.
            assert!(matches!(
                validate_mesh_for_voxelization(&deformed),
                Err(crate::convert::MeshVoxelizationError::SelfIntersection { .. })
            ));
            assert!(matches!(
                voxelize_mesh_to_voxel_data(&deformed, geometry),
                Err(crate::convert::MeshVoxelizationError::SelfIntersection { .. })
            ));
            eprintln!("liver large adaptive drag: rejected self-intersection");
            continue;
        }
        validate_mesh_for_voxelization(&deformed).expect("deformed liver must remain closed");
        eprintln!("liver deformed validation: {:?}", started.elapsed());
        let started = std::time::Instant::now();
        let resampled = voxelize_mesh_to_voxel_data(&deformed, geometry).unwrap();
        eprintln!("liver deformed resample: {:?}", started.elapsed());
        let [width, height, _] = geometry.dimensions;
        let mut changed = 0;
        for (index, (&before, &after)) in baseline
            .raw_data
            .iter()
            .zip(&resampled.raw_data)
            .enumerate()
        {
            if before == after {
                continue;
            }
            changed += 1;
            let index = index as u32;
            let ijk = [
                index % width,
                (index / width) % height,
                index / (width * height),
            ];
            let world = Vec3::from_array(voxel_index_to_world_mm(ijk.map(|v| v as f32), geometry));
            assert!(
                world.distance(Vec3::from_array(anchor))
                    <= radius.max(1.5 * Vec3::from_array(delta).length())
                        + Vec3::from_array(geometry.spacing()).length()
                        + Vec3::from_array(delta).length(),
                "voxel changed outside brush neighbourhood: {ijk:?}"
            );
        }
        assert!(
            changed > 0,
            "fixture must exercise actual occupancy changes"
        );
        eprintln!("liver changed voxels: {changed}");
    }
}

#[test]
fn test_projected_mesh_drag_creates_preview_without_mutating_authority() {
    let (mut world, mut session, roi_entity) = spawn_mesh_edit_world();
    let viewport_entity = session.input.active_viewport.unwrap();
    let viewport = world.get::<&Viewport>(viewport_entity).unwrap();
    let viewport_state = world.get::<&ViewportState>(viewport_entity).unwrap();
    let projection = build_display_projection_context(&world, &session, &viewport, &viewport_state)
        .expect("display projection");
    // The default view looks along +Y, so this point is inside the
    // front-facing tetrahedron triangle rather than an occluded face.
    let surface_point = [4.25, 4.75, 4.0];
    let click_uv =
        project_world_mm_to_viewport_uv_3d(surface_point, projection).expect("projected anchor");
    drop(viewport_state);
    drop(viewport);

    let selection = select_mesh_vertex(&mut world, &mut session, click_uv)
        .expect("mesh selection")
        .expect("selected surface");
    assert_eq!(selection.roi_entity, roi_entity);
    assert!(
        Vec3::from_array(selection.anchor_world_mm).distance(Vec3::from_array(surface_point))
            < 1e-5
    );
    let authoritative_before = world
        .get::<&Roi>(roi_entity)
        .unwrap()
        .mesh_data()
        .unwrap()
        .clone();
    session.input.drag_start_pos = click_uv;

    let revision = update_selected_mesh_deform_preview(
        &mut world,
        &session,
        [click_uv[0] + 0.05, click_uv[1]],
    )
    .expect("mesh preview");

    assert_eq!(revision, 1);
    let authoritative_after = world
        .get::<&Roi>(roi_entity)
        .unwrap()
        .mesh_data()
        .unwrap()
        .clone();
    assert_eq!(authoritative_after, authoritative_before);
    let roi = world.get::<&Roi>(roi_entity).unwrap();
    let preview = roi.mesh_edit_preview().expect("mesh preview data");
    assert_ne!(
        preview.mesh_data.vertices[selection.vertex_index].world_mm,
        authoritative_before.vertices[selection.vertex_index].world_mm
    );
    let expected_preview = preview.mesh_data.clone();
    drop(roi);
    // Mouse event frequency must not compound the total drag displacement.
    for _ in 0..5 {
        update_selected_mesh_deform_preview(
            &mut world,
            &session,
            [click_uv[0] + 0.05, click_uv[1]],
        )
        .unwrap();
    }
    assert_eq!(
        world
            .get::<&Roi>(roi_entity)
            .unwrap()
            .mesh_edit_preview()
            .unwrap()
            .mesh_data,
        expected_preview,
    );
}
