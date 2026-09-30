use super::*;
use crate::components::{
    ContourData, MainVolumeTag, MeshCache, MeshFace, MeshVertex, RoiId, VolumeData, VoxelData,
    VoxelGeometry,
};
use crate::model::OrthogonalFamily;
use crate::render::geometry::world_to_ndc;
use crate::render::geometry::ViewProjection;
use glam::Vec3;

fn spawn_world_base() -> (World, Session) {
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
            intensities: vec![],
            intensity_range: [0.0, 1.0],
        },
        MainVolumeTag,
    ));
    (world, Session::new(800, 600))
}

#[test]
fn test_prepare_mesh_render_data_empty_without_main_volume() {
    let world = World::new();
    let session = Session::new(800, 600);
    let data = prepare_mesh_render_data(&world, &session, &HashMap::new());
    assert!(data.parts.is_empty());
    assert!(data.draws.is_empty());
}

#[test]
fn test_prepare_mesh_render_data_skips_non_3d_viewports() {
    let (mut world, session) = spawn_world_base();
    world.spawn((
        Viewport {
            mode: ViewMode::Axial,
            rect: [0.0, 0.0, 400.0, 300.0],
            uniform_index: 0,
        },
        ViewportState::default(),
    ));
    world.spawn((Roi::new_mesh(
        RoiId(1),
        "Mesh".to_string(),
        MeshData {
            vertices: vec![
                MeshVertex {
                    world_mm: [1.0, 1.0, 1.0],
                },
                MeshVertex {
                    world_mm: [2.0, 1.0, 1.0],
                },
                MeshVertex {
                    world_mm: [1.0, 2.0, 1.0],
                },
            ],
            faces: vec![MeshFace {
                vertex_indices: [0, 1, 2],
            }],
        },
    ),));
    let data = prepare_mesh_render_data(&world, &session, &HashMap::new());
    assert!(data.parts.is_empty());
    assert!(data.draws.is_empty());
}

#[test]
fn test_prepare_mesh_render_data_uses_main_volume_geometry_for_projection() {
    let (mut world, session) = spawn_world_base();
    {
        let mut query = world.query::<&mut VolumeData>().with::<&MainVolumeTag>();
        let (_, vol) = query.iter().next().expect("main volume");
        let base = vol.geometry.expect("main volume geometry");
        vol.geometry = Some(
            VoxelGeometry::new(
                vol.dimensions,
                base.spacing(),
                base.origin(),
                glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2).to_array(),
            )
            .unwrap(),
        );
    }
    let user_rotation = glam::Quat::from_rotation_x(std::f32::consts::FRAC_PI_4).to_array();
    world.spawn((
        Viewport {
            mode: ViewMode::ThreeD,
            rect: [100.0, 50.0, 400.0, 300.0],
            uniform_index: 0,
        },
        ViewportState {
            user_rotation,
            ..ViewportState::default()
        },
    ));
    let world_vertex = [2.0, 2.0, 2.0];
    world.spawn((Roi::new_mesh(
        RoiId(2),
        "Mesh".to_string(),
        MeshData {
            vertices: vec![
                MeshVertex {
                    world_mm: world_vertex,
                },
                MeshVertex {
                    world_mm: [3.0, 2.0, 2.0],
                },
                MeshVertex {
                    world_mm: [2.0, 3.0, 2.0],
                },
            ],
            faces: vec![MeshFace {
                vertex_indices: [0, 1, 2],
            }],
        },
    ),));
    let data = prepare_mesh_render_data(&world, &session, &HashMap::new());
    assert_eq!(data.draws.len(), 1);
    assert_eq!(data.parts.len(), 1);
    let geometry = data.parts[0]
        .geometry
        .as_ref()
        .expect("new part carries geometry");
    assert_eq!(geometry.positions[0], world_vertex);
    assert_eq!(geometry.indices, vec![0, 1, 2]);
    let ndc_of = |uniform: &MeshDrawUniform, p: [f32; 3]| {
        [
            uniform.row0[0] * p[0]
                + uniform.row0[1] * p[1]
                + uniform.row0[2] * p[2]
                + uniform.row0[3],
            uniform.row1[0] * p[0]
                + uniform.row1[1] * p[1]
                + uniform.row1[2] * p[2]
                + uniform.row1[3],
        ]
    };
    let drawn_ndc = ndc_of(&data.draws[0].uniform, world_vertex);

    let main_geometry = crate::app::roi_runtime::main_volume_geometry(&world).unwrap();
    let viewport_rect = {
        let mut q = world.query::<&Viewport>();
        q.iter().next().unwrap().1.rect
    };
    let viewport_state = {
        let mut q = world.query::<&ViewportState>();
        *q.iter().next().unwrap().1
    };
    let (main_orientation, main_aspect_ratios) = {
        let mut q = world.query::<&VolumeData>().with::<&MainVolumeTag>();
        let vol = q.iter().next().unwrap().1;
        (vol.orientation(), vol.aspect_ratios())
    };
    let uv = crate::convert::world_mm_to_volume_uv(world_vertex, main_geometry);
    let screen_aspect = viewport_rect[2] / viewport_rect[3];

    let composed_projection = ViewProjection {
        zoom: viewport_state.zoom,
        pan: viewport_state.pan,
        pivot: viewport_state.pivot,
        rotation: crate::util::orientation::compose_view_rotation(
            main_orientation,
            viewport_state.user_rotation,
        ),
        aspect_ratios: main_aspect_ratios,
        geometry: main_geometry,
        cursor_pos: [0.5, 0.5, 0.5],
    };
    let composed_uv = world_to_ndc(
        Vec3::from_array(uv),
        ViewMode::ThreeD,
        &composed_projection,
        screen_aspect,
    )
    .unwrap();
    let composed_ndc = viewport_uv_to_full_ndc(composed_uv, viewport_rect, [800.0, 600.0]).unwrap();
    assert!(
        (drawn_ndc[0] - composed_ndc[0]).abs() < 1e-4
            && (drawn_ndc[1] - composed_ndc[1]).abs() < 1e-4
    );

    let raw_projection = ViewProjection {
        rotation: viewport_state.user_rotation,
        ..composed_projection
    };
    let raw_uv = world_to_ndc(
        Vec3::from_array(uv),
        ViewMode::ThreeD,
        &raw_projection,
        screen_aspect,
    )
    .unwrap();
    let raw_ndc = viewport_uv_to_full_ndc(raw_uv, viewport_rect, [800.0, 600.0]).unwrap();
    assert!((drawn_ndc[0] - raw_ndc[0]).abs() > 1e-3 || (drawn_ndc[1] - raw_ndc[1]).abs() > 1e-3);
}

#[test]
fn test_prepare_mesh_render_data_emits_wgpu_handle_for_selected_mesh_vertex() {
    let (mut world, mut session) = spawn_world_base();
    let viewport_entity = world.spawn((
        Viewport {
            mode: ViewMode::ThreeD,
            rect: [0.0, 0.0, 400.0, 300.0],
            uniform_index: 0,
        },
        ViewportState::default(),
    ));
    let roi_entity = world.spawn((Roi::new_mesh(
        RoiId(22),
        "Selected mesh".to_string(),
        MeshData {
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
            ],
            faces: vec![MeshFace {
                vertex_indices: [0, 1, 2],
            }],
        },
    ),));
    {
        let editor = &mut session.editor;
        editor.active_roi = Some(roi_entity);
        editor.active_tool = crate::components::EditorTool::MeshDeform;
        editor.mesh_selection = Some(crate::components::MeshSelection {
            roi_entity,
            vertex_index: 0,
            triangle_vertex_indices: [0, 1, 2],
            anchor_world_mm: [4.0, 4.0, 4.0],
        });
    }

    let data = prepare_mesh_render_data(&world, &session, &HashMap::new());
    let key = MeshPartKey {
        roi_entity,
        part: MeshRenderPartKey::Handle(viewport_entity),
    };
    let handle = data
        .parts
        .iter()
        .find(|part| part.key == key)
        .expect("selected vertex handle");
    let geometry = handle.geometry.as_ref().expect("handle geometry");
    assert_eq!(geometry.positions.len(), 4);
    assert_eq!(geometry.indices.len(), 6);
    let draw = data.draws.iter().find(|draw| draw.key == key).unwrap();
    assert_eq!(draw.uniform.color, HANDLE_COLOR);
}

#[test]
fn test_prepare_mesh_render_data_skips_invalid_face_indices() {
    let (mut world, session) = spawn_world_base();
    world.spawn((
        Viewport {
            mode: ViewMode::ThreeD,
            rect: [0.0, 0.0, 400.0, 300.0],
            uniform_index: 0,
        },
        ViewportState::default(),
    ));
    world.spawn((Roi::new_mesh(
        RoiId(3),
        "Mesh".to_string(),
        MeshData {
            vertices: vec![MeshVertex {
                world_mm: [1.0, 1.0, 1.0],
            }],
            faces: vec![MeshFace {
                vertex_indices: [0, 1, 2],
            }],
        },
    ),));
    let data = prepare_mesh_render_data(&world, &session, &HashMap::new());
    assert!(data.parts.is_empty());
    assert!(data.draws.is_empty());
}

#[test]
fn test_prepare_mesh_render_data_preserves_unchanged_chunk_identity_and_vertices() {
    let (mut world, session) = spawn_world_base();
    world.spawn((
        Viewport {
            mode: ViewMode::ThreeD,
            rect: [0.0, 0.0, 400.0, 300.0],
            uniform_index: 0,
        },
        ViewportState::default(),
    ));
    let geometry =
        VoxelGeometry::new([32, 2, 1], [1.0; 3], [0.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap();
    let mut voxel = VoxelData {
        geometry,
        raw_data: vec![0; 64],
    };
    voxel.raw_data[1] = 1;
    voxel.raw_data[20] = 1;
    let chunked = crate::convert::extract_chunked_mesh_from_voxel_data(&voxel, 16).unwrap();
    let mut roi = Roi::new_contour(
        RoiId(4),
        "Contour".to_string(),
        ContourData {
            active_plane_family: OrthogonalFamily::Axial,
            slices: Vec::new(),
        },
    );
    roi.session_caches.voxel = Some(crate::components::VoxelCache {
        data: voxel.clone(),
        gpu_resources: None,
    });
    roi.session_caches.mesh = Some(MeshCache {
        data: chunked.merged_mesh(),
        chunks: Some(chunked),
    });
    roi.dirty_state.voxel.dirty = false;
    roi.dirty_state.mesh.dirty = false;
    roi.dirty_state.voxel.built_from = roi.dirty_state.authoritative;
    roi.dirty_state.mesh.built_from = roi.dirty_state.authoritative;
    let roi_entity = world.spawn((roi,));

    let before = prepare_mesh_render_data(&world, &session, &HashMap::new());
    assert_eq!(before.parts.len(), 2);
    assert!(before.parts.iter().all(|part| part.geometry.is_some()));
    let known = before
        .parts
        .iter()
        .map(|part| (part.key, part.fingerprint))
        .collect::<HashMap<_, _>>();

    voxel.raw_data[2] = 1;
    let rebuilt = crate::convert::extract_chunked_mesh_from_voxel_data(&voxel, 16).unwrap();
    {
        let mut roi = world.get::<&mut Roi>(roi_entity).unwrap();
        roi.session_caches.voxel.as_mut().unwrap().data = voxel;
        roi.session_caches.mesh = Some(MeshCache {
            data: rebuilt.merged_mesh(),
            chunks: Some(rebuilt),
        });
    }
    let after = prepare_mesh_render_data(&world, &session, &known);

    let part_for = |chunk: MeshChunkKey| {
        after
            .parts
            .iter()
            .find(|part| part.key.part == MeshRenderPartKey::Chunk(chunk))
            .expect("chunk part")
    };
    let changed = part_for(MeshChunkKey { index: [0, 0, 0] });
    let unchanged = part_for(MeshChunkKey { index: [1, 0, 0] });
    assert!(changed.geometry.is_some(), "an edited chunk is rebuilt");
    assert!(
        unchanged.geometry.is_none(),
        "an untouched chunk is reused from the GPU"
    );
    assert_eq!(after.draws.len(), 2);
}

#[test]
fn test_camera_motion_changes_only_the_draw_uniform() {
    let (mut world, session) = spawn_world_base();
    let viewport = world.spawn((
        Viewport {
            mode: ViewMode::ThreeD,
            rect: [0.0, 0.0, 400.0, 300.0],
            uniform_index: 0,
        },
        ViewportState::default(),
    ));
    world.spawn((Roi::new_mesh(
        RoiId(5),
        "Mesh".to_string(),
        MeshData {
            vertices: vec![
                MeshVertex {
                    world_mm: [1.0, 1.0, 1.0],
                },
                MeshVertex {
                    world_mm: [4.0, 1.0, 1.0],
                },
                MeshVertex {
                    world_mm: [1.0, 4.0, 1.0],
                },
            ],
            faces: vec![MeshFace {
                vertex_indices: [0, 1, 2],
            }],
        },
    ),));
    let first = prepare_mesh_render_data(&world, &session, &HashMap::new());
    let known = first
        .parts
        .iter()
        .map(|part| (part.key, part.fingerprint))
        .collect::<HashMap<_, _>>();

    {
        let mut state = world.get::<&mut ViewportState>(viewport).unwrap();
        state.zoom = 2.5;
        state.pan = [0.1, -0.05];
        state.user_rotation = glam::Quat::from_rotation_y(0.7).to_array();
    }
    let second = prepare_mesh_render_data(&world, &session, &known);

    assert!(second.parts.iter().all(|part| part.geometry.is_none()));
    assert_ne!(first.draws[0].uniform.row0, second.draws[0].uniform.row0);
}
