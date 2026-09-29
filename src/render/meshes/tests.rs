use super::*;
use crate::components::{
    ContourData, MainVolumeTag, MeshCache, MeshFace, MeshVertex, RoiId, Transform, VolumeData,
    VoxelData, VoxelGeometry, WindowSettings,
};
use crate::convert::PlaneFamily;
use crate::render::geometry::world_to_ndc;
use crate::render::geometry::ViewProjection;
use glam::Vec3;

fn spawn_world_base() -> (World, AppEntities) {
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
    let cursor = world.spawn((Transform {
        position: [0.5, 0.5, 0.5],
    },));
    let ws = world.spawn((WindowSettings {
        width: 800,
        height: 600,
        viewport_rect: [0.0, 0.0, 800.0, 600.0],
    },));
    let editor = world.spawn((crate::components::EditorState::default(),));
    let input = world.spawn((crate::components::InputState::default(),));
    let gui_state = world.spawn((crate::components::GuiState {
        status_message: None,
    },));
    let volume_windowing = world.spawn((crate::components::VolumeWindowing::default(),));
    let annotations = world.spawn((crate::components::AnnotationState::default(),));
    let overlay = world.spawn((crate::overlay::OverlayManager::default(),));
    let protocol = world.spawn((crate::components::ProtocolState::default(),));
    (
        world,
        AppEntities {
            input,
            editor,
            gui_state,
            volume_windowing,
            annotations,
            overlay,
            protocol,
            cursor,
            window_settings: ws,
        },
    )
}

#[test]
fn test_prepare_mesh_render_data_empty_without_main_volume() {
    let world = World::new();
    let entities = AppEntities {
        input: hecs::Entity::DANGLING,
        editor: hecs::Entity::DANGLING,
        gui_state: hecs::Entity::DANGLING,
        volume_windowing: hecs::Entity::DANGLING,
        annotations: hecs::Entity::DANGLING,
        overlay: hecs::Entity::DANGLING,
        protocol: hecs::Entity::DANGLING,
        cursor: hecs::Entity::DANGLING,
        window_settings: hecs::Entity::DANGLING,
    };
    let data = prepare_mesh_render_data(&world, &entities);
    assert!(data.chunks.is_empty());
}

#[test]
fn test_prepare_mesh_render_data_skips_non_3d_viewports() {
    let (mut world, entities) = spawn_world_base();
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
    let data = prepare_mesh_render_data(&world, &entities);
    assert!(data.chunks.is_empty());
}

#[test]
fn test_prepare_mesh_render_data_uses_main_volume_geometry_for_projection() {
    let (mut world, entities) = spawn_world_base();
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
    let data = prepare_mesh_render_data(&world, &entities);
    assert_eq!(data.chunks.len(), 1);
    assert_eq!(data.chunks[0].vertices.len(), 3);

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
    assert_eq!(data.chunks[0].vertices[0].position_ndc, composed_ndc);

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
    assert_ne!(data.chunks[0].vertices[0].position_ndc, raw_ndc);
}

#[test]
fn test_prepare_mesh_render_data_emits_wgpu_handle_for_selected_mesh_vertex() {
    let (mut world, entities) = spawn_world_base();
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
        let mut editor = world
            .get::<&mut crate::components::EditorState>(entities.editor)
            .unwrap();
        editor.active_roi = Some(roi_entity);
        editor.active_tool = crate::components::EditorTool::MeshDeform;
        editor.mesh_selection = Some(crate::components::MeshSelection {
            roi_entity,
            vertex_index: 0,
            triangle_vertex_indices: [0, 1, 2],
            anchor_world_mm: [4.0, 4.0, 4.0],
        });
    }

    let data = prepare_mesh_render_data(&world, &entities);
    let handle = data
        .chunks
        .iter()
        .find(|chunk| {
            chunk.key.roi_entity == roi_entity
                && chunk.key.viewport_entity == viewport_entity
                && chunk.key.part == MeshRenderPartKey::Handle
        })
        .expect("selected vertex handle");
    assert_eq!(handle.vertices.len(), 6);
    assert!(handle
        .vertices
        .iter()
        .all(|vertex| vertex.color == [1.0, 0.85, 0.1, 1.0]));
}

#[test]
fn test_prepare_mesh_render_data_skips_invalid_face_indices() {
    let (mut world, entities) = spawn_world_base();
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
    let data = prepare_mesh_render_data(&world, &entities);
    assert!(data.chunks.is_empty());
}

#[test]
fn test_prepare_mesh_render_data_preserves_unchanged_chunk_identity_and_vertices() {
    let (mut world, entities) = spawn_world_base();
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
            active_plane_family: PlaneFamily::Axial,
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
    roi.dirty_state.voxel_cache_dirty = false;
    roi.dirty_state.mesh_cache_dirty = false;
    roi.dirty_state.generations.voxel = roi.dirty_state.generations.authoritative;
    roi.dirty_state.generations.mesh = roi.dirty_state.generations.authoritative;
    let roi_entity = world.spawn((roi,));

    let before = prepare_mesh_render_data(&world, &entities);
    let before_by_part = before
        .chunks
        .iter()
        .map(|chunk| (chunk.key.part, chunk.vertices.clone()))
        .collect::<HashMap<_, _>>();
    assert_eq!(before_by_part.len(), 2);

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
    let after = prepare_mesh_render_data(&world, &entities);
    let after_by_part = after
        .chunks
        .iter()
        .map(|chunk| (chunk.key.part, chunk.vertices.clone()))
        .collect::<HashMap<_, _>>();

    let first = MeshRenderPartKey::Chunk(MeshChunkKey { index: [0, 0, 0] });
    let second = MeshRenderPartKey::Chunk(MeshChunkKey { index: [1, 0, 0] });
    assert_ne!(before_by_part[&first], after_by_part[&first]);
    assert_eq!(before_by_part[&second], after_by_part[&second]);
}
