use super::*;
use crate::convert::{orthogonal_plane_from_volume_uv, PlaneFamily};

fn approx_eq(a: [f32; 2], b: [f32; 2], epsilon: f32) -> bool {
    (a[0] - b[0]).abs() <= epsilon && (a[1] - b[1]).abs() <= epsilon
}

#[test]
fn test_build_polyline_triangles_produces_expected_geometry_for_horizontal_segment() {
    let points = [[0.0, 0.0], [1.0, 0.0]];
    let color = [0.9, 0.2, 0.2, 0.8];
    let vertices = build_polyline_triangles_ndc(&points, 0.2, color);

    assert_eq!(vertices.len(), 6);
    assert_eq!(vertices[0].position_ndc, [0.0, 0.1]);
    assert_eq!(vertices[1].position_ndc, [0.0, -0.1]);
    assert_eq!(vertices[2].position_ndc, [1.0, 0.1]);
    assert_eq!(vertices[3].position_ndc, [1.0, 0.1]);
    assert_eq!(vertices[4].position_ndc, [0.0, -0.1]);
    assert_eq!(vertices[5].position_ndc, [1.0, -0.1]);
    assert!(vertices.iter().all(|vertex| vertex.color == color));
}

#[test]
fn test_build_point_marker_triangles_produces_quad_geometry() {
    let color = [0.2, 0.8, 0.4, 1.0];
    let vertices = build_point_marker_triangles_ndc([0.0, 0.0], 0.2, color);
    assert_eq!(vertices.len(), 6);
    assert_eq!(vertices[0].position_ndc, [-0.1, -0.1]);
    assert_eq!(vertices[2].position_ndc, [0.1, 0.1]);
    assert!(vertices.iter().all(|vertex| vertex.color == color));
}

#[test]
fn test_projection_helper_local_world_viewport_roundtrip_stays_stable() {
    let geometry = VoxelGeometry::new(
        [16, 16, 16],
        [1.0, 1.0, 1.0],
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap();
    let plane =
        orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 0.5], geometry).unwrap();
    let mapping = ViewportMapping {
        zoom: 1.0,
        pan: [0.0, 0.0],
        pivot: [0.5, 0.5],
        screen_aspect: 1.0,
    };
    let local = [2.0, -1.5];
    let world = plane_local_mm_to_world_mm(local, plane);
    let uv = world_mm_to_volume_uv(world, geometry);
    let viewport_uv = volume_uv_to_viewport_uv(uv, plane, geometry, mapping).unwrap();
    let world_roundtrip = plane_local_mm_to_world_mm(local, plane);
    let uv_roundtrip = world_mm_to_volume_uv(world_roundtrip, geometry);
    let viewport_uv_roundtrip =
        volume_uv_to_viewport_uv(uv_roundtrip, plane, geometry, mapping).unwrap();
    assert!(approx_eq(viewport_uv, viewport_uv_roundtrip, 1e-6));
}

#[test]
fn test_prepare_contour_render_data_is_empty_when_no_visible_contour_roi() {
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

    let data = prepare_contour_render_data(&world, &entities);
    assert!(data.vertices.is_empty());
}

#[test]
fn test_prepare_contour_render_data_emits_vertices_for_matching_slice() {
    let mut world = World::new();
    let cursor = world.spawn((Transform {
        position: [0.5, 0.5, 0.5],
    },));
    let window_settings = world.spawn((WindowSettings {
        width: 800,
        height: 600,
        viewport_rect: [0.0, 0.0, 800.0, 600.0],
    },));
    let editor = world.spawn((EditorState {
        active_roi: None,
        active_tool: EditorTool::Navigation,
        contour_draft: None,
        contour_selection: None,
        roi_edit_preview: None,
        ..EditorState::default()
    },));
    let viewport = world.spawn((
        Viewport {
            mode: ViewMode::Axial,
            rect: [0.0, 0.0, 800.0, 600.0],
            uniform_index: 0,
        },
        ViewportState {
            zoom: 1.0,
            pan: [0.0, 0.0],
            pivot: [0.5, 0.5],
            user_rotation: [0.0, 0.0, 0.0, 1.0],
        },
    ));
    let _ = viewport;

    world.spawn((
        VolumeData {
            dimensions: [32, 32, 32],
            geometry: Some(
                VoxelGeometry::new(
                    [32, 32, 32],
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

    let geometry = VoxelGeometry::new(
        [32, 32, 32],
        [1.0, 1.0, 1.0],
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap();
    let plane =
        orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 0.5], geometry).unwrap();
    let mut roi = Roi::new_contour(
        RoiId(1),
        "Contour".to_string(),
        ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: vec![ContourSlice {
                plane,
                loops: vec![ContourLoop {
                    points: vec![
                        ContourPoint {
                            local_mm: [0.0, 0.0],
                        },
                        ContourPoint {
                            local_mm: [3.0, 0.0],
                        },
                        ContourPoint {
                            local_mm: [3.0, 3.0],
                        },
                    ],
                    is_closed: true,
                }],
            }],
        },
    );
    roi.session_caches.voxel = Some(VoxelCache {
        data: VoxelData {
            geometry: VoxelGeometry::new(
                [16, 16, 16],
                [2.0, 2.0, 2.0],
                [100.0, 100.0, 100.0],
                [0.0, 0.0, 0.0, 1.0],
            )
            .unwrap(),
            raw_data: vec![0; 16 * 16 * 16],
        },
        gpu_resources: None,
    });
    let roi_entity = world.spawn((roi, LayerSettings { opacity: 1.0 }, RoiTag));
    world.get::<&mut EditorState>(editor).unwrap().active_roi = Some(roi_entity);

    let entities = AppEntities {
        input: hecs::Entity::DANGLING,
        editor,
        gui_state: hecs::Entity::DANGLING,
        volume_windowing: hecs::Entity::DANGLING,
        annotations: hecs::Entity::DANGLING,
        overlay: hecs::Entity::DANGLING,
        protocol: hecs::Entity::DANGLING,
        cursor,
        window_settings,
    };

    let data = prepare_contour_render_data(&world, &entities);
    assert!(!data.vertices.is_empty());
    assert_eq!(data.batches.len(), 1);
    assert_eq!(data.batches[0].scissor_rect, [0, 0, 800, 600]);

    let second_contour = world
        .get::<&Roi>(roi_entity)
        .unwrap()
        .contour_data()
        .unwrap()
        .clone();
    let mut second_roi = Roi::new_contour(RoiId(2), "Second contour".to_string(), second_contour);
    second_roi.metadata.color = [0.1, 0.7, 0.2, 0.8];
    world.spawn((second_roi, LayerSettings { opacity: 0.5 }, RoiTag));
    {
        let mut editor_state = world.get::<&mut EditorState>(editor).unwrap();
        editor_state.active_roi = None;
        editor_state.active_tool = EditorTool::ContourSelect;
    }

    let multi_roi_data = prepare_contour_render_data(&world, &entities);
    assert_eq!(multi_roi_data.batches.len(), 1);
    assert!(multi_roi_data
        .vertices
        .iter()
        .any(|vertex| vertex.color == [0.1, 0.7, 0.2, 0.4]));
    let second_roi_vertex_count = multi_roi_data
        .vertices
        .iter()
        .filter(|vertex| vertex.color == [0.1, 0.7, 0.2, 0.4])
        .count();
    assert_eq!(second_roi_vertex_count, 18);

    world
        .get::<&mut Roi>(roi_entity)
        .unwrap()
        .metadata
        .is_visible = false;
    let hidden_data = prepare_contour_render_data(&world, &entities);
    assert!(!hidden_data.vertices.is_empty());
    assert_eq!(hidden_data.batches.len(), 1);
}

#[test]
fn test_prepare_contour_render_data_uses_roi_geometry_when_main_volume_missing() {
    let mut world = World::new();
    let cursor = world.spawn((Transform {
        position: [0.5, 0.5, 0.5],
    },));
    let window_settings = world.spawn((WindowSettings {
        width: 800,
        height: 600,
        viewport_rect: [0.0, 0.0, 800.0, 600.0],
    },));
    let editor = world.spawn((EditorState {
        active_roi: None,
        active_tool: EditorTool::Navigation,
        contour_draft: None,
        contour_selection: None,
        roi_edit_preview: None,
        ..EditorState::default()
    },));
    world.spawn((
        Viewport {
            mode: ViewMode::Axial,
            rect: [0.0, 0.0, 800.0, 600.0],
            uniform_index: 0,
        },
        ViewportState {
            zoom: 1.0,
            pan: [0.0, 0.0],
            pivot: [0.5, 0.5],
            user_rotation: [0.0, 0.0, 0.0, 1.0],
        },
    ));

    let geometry = VoxelGeometry::new(
        [32, 32, 32],
        [1.5, 0.75, 2.0],
        [12.0, -8.0, 4.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap();
    let plane =
        orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 0.5], geometry).unwrap();
    let mut roi = Roi::new_contour(
        RoiId(2),
        "Contour".to_string(),
        ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: vec![ContourSlice {
                plane,
                loops: vec![ContourLoop {
                    points: vec![
                        ContourPoint {
                            local_mm: [0.0, 0.0],
                        },
                        ContourPoint {
                            local_mm: [3.0, 0.0],
                        },
                        ContourPoint {
                            local_mm: [3.0, 3.0],
                        },
                    ],
                    is_closed: true,
                }],
            }],
        },
    );
    roi.session_caches.voxel = Some(VoxelCache {
        data: VoxelData {
            geometry,
            raw_data: vec![0; (32 * 32 * 32) as usize],
        },
        gpu_resources: None,
    });
    let roi_entity = world.spawn((roi, LayerSettings { opacity: 1.0 }, RoiTag));
    world.get::<&mut EditorState>(editor).unwrap().active_roi = Some(roi_entity);

    let entities = AppEntities {
        input: hecs::Entity::DANGLING,
        editor,
        gui_state: hecs::Entity::DANGLING,
        volume_windowing: hecs::Entity::DANGLING,
        annotations: hecs::Entity::DANGLING,
        overlay: hecs::Entity::DANGLING,
        protocol: hecs::Entity::DANGLING,
        cursor,
        window_settings,
    };

    let data = prepare_contour_render_data(&world, &entities);
    assert!(!data.vertices.is_empty());
}

#[test]
fn test_voxel_primary_oblique_view_cache_emits_contour_vertices() {
    let mut world = World::new();
    let geometry = VoxelGeometry::new(
        [8, 8, 8],
        [1.0, 1.0, 1.0],
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap();
    world.spawn((
        VolumeData {
            dimensions: geometry.dimensions,
            geometry: Some(
                VoxelGeometry::new(
                    geometry.dimensions,
                    geometry.spacing(),
                    geometry.origin(),
                    geometry.orientation(),
                )
                .unwrap(),
            ),
            intensities: Vec::new(),
            intensity_range: [0.0, 1.0],
        },
        MainVolumeTag,
    ));
    let cursor = world.spawn((Transform {
        position: [0.5, 0.5, 0.5],
    },));
    let window_settings = world.spawn((WindowSettings {
        width: 800,
        height: 600,
        viewport_rect: [0.0, 0.0, 800.0, 600.0],
    },));
    let editor = world.spawn((EditorState::default(),));
    world.spawn((
        Viewport {
            mode: ViewMode::Oblique,
            rect: [0.0, 0.0, 800.0, 600.0],
            uniform_index: 0,
        },
        ViewportState {
            user_rotation: glam::Quat::from_rotation_y(0.35).to_array(),
            ..ViewportState::default()
        },
    ));
    let mut raw_data = vec![0; 8 * 8 * 8];
    for z in 2..=5 {
        for y in 2..=5 {
            for x in 2..=5 {
                raw_data[(z * 64 + y * 8 + x) as usize] = 1;
            }
        }
    }
    let mut roi = Roi::new_voxel_with_cache(
        RoiId(77),
        "Oblique cube".to_string(),
        geometry,
        raw_data,
        None,
    );
    let axial_plane =
        orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 0.5], geometry).unwrap();
    roi.upsert_contour_view_cache(
        ContourViewKey::from_plane(axial_plane),
        ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: vec![ContourSlice {
                plane: axial_plane,
                loops: Vec::new(),
            }],
        },
        roi.dirty_state.authoritative,
        CacheViewState::Current,
    );
    let roi_entity = world.spawn((roi, LayerSettings { opacity: 0.5 }, RoiTag));
    world.get::<&mut EditorState>(editor).unwrap().active_roi = Some(roi_entity);
    let entities = AppEntities {
        input: hecs::Entity::DANGLING,
        editor,
        gui_state: hecs::Entity::DANGLING,
        volume_windowing: hecs::Entity::DANGLING,
        annotations: hecs::Entity::DANGLING,
        overlay: hecs::Entity::DANGLING,
        protocol: hecs::Entity::DANGLING,
        cursor,
        window_settings,
    };

    crate::app::roi_runtime::sync_roi_contour_view_caches_for_viewports(&mut world);
    let data = prepare_contour_render_data(&world, &entities);

    let roi = world.get::<&Roi>(roi_entity).unwrap();
    assert!(roi
        .contour_cache()
        .is_some_and(|cache| cache
            .views
            .iter()
            .any(|view| view.key.family == PlaneFamily::Oblique && view.data.has_loops())));
    assert!(!data.vertices.is_empty());
    assert_eq!(data.batches.len(), 1);
}
