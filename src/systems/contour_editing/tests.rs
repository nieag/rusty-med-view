use super::*;
use crate::components::{
    ContourData, InputState, RoiAuthoritativeData, RoiCacheKind, RoiJobKind, ViewportState,
    VoxelCache, WindowSettings,
};

fn test_geometry() -> VoxelGeometry {
    VoxelGeometry::new(
        [64, 48, 32],
        [1.0, 1.0, 1.0],
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap()
}

fn spawn_test_entities(
    world: &mut World,
    mode: ViewMode,
    user_rotation: [f32; 4],
    active_viewport: Option<hecs::Entity>,
) -> AppEntities {
    let cursor = world.spawn((Transform {
        position: [0.4, 0.55, 0.2],
    },));
    let viewport = world.spawn((
        Viewport {
            mode,
            rect: [0.0, 0.0, 800.0, 600.0],
            uniform_index: 0,
        },
        ViewportState {
            zoom: 1.1,
            pan: [0.02, -0.01],
            pivot: [0.5, 0.5],
            user_rotation,
        },
    ));
    let input = world.spawn((InputState {
        active_viewport: active_viewport.or(Some(viewport)),
        ..InputState::default()
    },));
    world.spawn((
        crate::components::VolumeData {
            dimensions: test_geometry().dimensions,
            geometry: Some(
                VoxelGeometry::new(
                    test_geometry().dimensions,
                    test_geometry().spacing(),
                    test_geometry().origin(),
                    test_geometry().orientation(),
                )
                .unwrap(),
            ),
            intensities: vec![],
            intensity_range: [0.0, 1.0],
        },
        MainVolumeTag,
    ));

    let editor = world.spawn((crate::components::EditorState::default(),));
    let gui_state = world.spawn((crate::components::GuiState {
        status_message: None,
    },));
    let volume_windowing = world.spawn((crate::components::VolumeWindowing::default(),));
    let annotations = world.spawn((crate::components::AnnotationState::default(),));
    let overlay = world.spawn((crate::overlay::OverlayManager::default(),));
    let protocol = world.spawn((crate::components::ProtocolState::default(),));
    let window_settings = world.spawn((WindowSettings {
        width: 800,
        height: 600,
        viewport_rect: [0.0, 0.0, 800.0, 600.0],
    },));

    AppEntities {
        input,
        editor,
        gui_state,
        volume_windowing,
        annotations,
        overlay,
        protocol,
        cursor,
        window_settings,
    }
}

fn spawn_test_contour_roi(world: &mut World, family: PlaneFamily) -> hecs::Entity {
    world.spawn((Roi::new_contour(
        crate::components::RoiId(100),
        "Contour".to_string(),
        ContourData {
            active_plane_family: family,
            slices: Vec::new(),
        },
    ),))
}

fn spawn_test_contour_roi_with_loop(world: &mut World, family: PlaneFamily) -> hecs::Entity {
    let geometry = VoxelGeometry::new(
        [64, 48, 32],
        [1.0, 1.0, 1.0],
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap();
    let plane = orthogonal_plane_from_volume_uv(family, [0.4, 0.55, 0.2], geometry).unwrap();
    let entity = world.spawn((Roi::new_contour_with_geometry(
        crate::components::RoiId(101),
        "ContourWithLoop".to_string(),
        VoxelGeometry::new(
            geometry.dimensions,
            geometry.spacing(),
            geometry.origin(),
            geometry.orientation(),
        )
        .unwrap(),
        ContourData {
            active_plane_family: family,
            slices: vec![ContourSlice {
                plane,
                loops: vec![ContourLoop {
                    points: vec![
                        ContourPoint {
                            local_mm: [-5.0, -5.0],
                        },
                        ContourPoint {
                            local_mm: [5.0, -5.0],
                        },
                        ContourPoint {
                            local_mm: [5.0, 5.0],
                        },
                        ContourPoint {
                            local_mm: [-5.0, 5.0],
                        },
                    ],
                    is_closed: true,
                }],
            }],
        },
    ),));
    world.get::<&mut Roi>(entity).unwrap().session_caches.voxel = Some(VoxelCache {
        data: VoxelData {
            geometry,
            raw_data: vec![0; 64 * 48 * 32],
        },
        gpu_resources: None,
    });
    entity
}

fn reframe_first_contour_slice(world: &mut World, roi_entity: hecs::Entity) {
    let mut roi = world.get::<&mut Roi>(roi_entity).unwrap();
    let RoiAuthoritativeData::Contour(contour) = &mut roi.authoritative_data else {
        panic!("expected contour authority");
    };
    let slice = &mut contour.slices[0];
    let old_plane = slice.plane;
    let old_u = glam::Vec3::from_array(old_plane.u_axis_mm);
    let old_v = glam::Vec3::from_array(old_plane.v_axis_mm);
    let old_origin = glam::Vec3::from_array(old_plane.origin_mm);
    let mut new_plane = old_plane;
    new_plane.origin_mm = (old_origin + old_u * 12.0 + old_v * 7.0).to_array();
    new_plane.u_axis_mm = old_v.to_array();
    new_plane.v_axis_mm = (-old_u).to_array();
    for contour_loop in &mut slice.loops {
        for point in &mut contour_loop.points {
            let world = plane_local_mm_to_world_mm(point.local_mm, old_plane);
            point.local_mm = crate::convert::world_mm_to_plane_local_mm(world, new_plane);
        }
    }
    slice.plane = new_plane;
}

#[test]
fn test_contour_edit_rejects_viewport_family_mismatch() {
    let mut world = World::new();
    let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
    let contour = ContourData {
        active_plane_family: PlaneFamily::Coronal,
        slices: Vec::new(),
    };

    let result = resolve_active_contour_edit_viewport(&world, &entities, &contour);
    assert_eq!(
        result,
        Err(ContourEditMappingError::PlaneFamilyMismatch {
            contour_family: PlaneFamily::Coronal,
            viewport_family: PlaneFamily::Axial,
        })
    );
}

#[test]
fn test_contour_edit_rejects_three_d_viewport() {
    let mut world = World::new();
    let entities = spawn_test_entities(&mut world, ViewMode::ThreeD, [0.0, 0.0, 0.0, 1.0], None);
    let contour = ContourData {
        active_plane_family: PlaneFamily::Axial,
        slices: Vec::new(),
    };

    let result = resolve_active_contour_edit_viewport(&world, &entities, &contour);
    assert_eq!(
        result,
        Err(ContourEditMappingError::UnsupportedViewportMode)
    );
}

#[test]
fn test_contour_edit_rejects_invalid_main_volume_geometry() {
    let mut world = World::new();
    let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
    for (_, volume) in world
        .query_mut::<&mut crate::components::VolumeData>()
        .with::<&MainVolumeTag>()
    {
        volume.geometry = None;
    }
    let contour = ContourData {
        active_plane_family: PlaneFamily::Axial,
        slices: Vec::new(),
    };

    assert_eq!(
        resolve_active_contour_edit_viewport(&world, &entities, &contour),
        Err(ContourEditMappingError::InvalidMainVolumeGeometry)
    );
}

#[test]
fn test_contour_edit_oblique_path_uses_plane_definition_roundtrip() {
    let mut world = World::new();
    let oblique_rotation = glam::Quat::from_euler(glam::EulerRot::XYZ, 0.3, -0.2, 0.15).to_array();
    let entities = spawn_test_entities(&mut world, ViewMode::Oblique, oblique_rotation, None);
    let contour = ContourData {
        active_plane_family: PlaneFamily::Oblique,
        slices: Vec::new(),
    };

    let viewport =
        resolve_active_contour_edit_viewport(&world, &entities, &contour).expect("viewport");
    assert_eq!(viewport.plane.family, PlaneFamily::Oblique);

    let click_a = [0.32, 0.67];
    let click_b = [0.61, 0.28];
    let local_a = viewport_uv_to_contour_plane_local_mm(click_a, viewport).expect("local mm a");
    let local_b = viewport_uv_to_contour_plane_local_mm(click_b, viewport).expect("local mm b");
    let viewport_a =
        contour_plane_local_mm_to_viewport_uv(local_a, viewport).expect("viewport uv a");
    let viewport_b =
        contour_plane_local_mm_to_viewport_uv(local_b, viewport).expect("viewport uv b");

    assert!(local_a[0].is_finite() && local_a[1].is_finite());
    assert!(local_b[0].is_finite() && local_b[1].is_finite());
    assert_ne!(local_a, local_b);
    assert!(viewport_a[0].is_finite() && viewport_a[1].is_finite());
    assert!(viewport_b[0].is_finite() && viewport_b[1].is_finite());
}

#[test]
fn test_contour_draw_click_appends_draft_without_authoritative_mutation() {
    let mut world = World::new();
    let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
    let roi_entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial);
    {
        let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
        editor.active_roi = Some(roi_entity);
        editor.active_tool = EditorTool::ContourDraw;
    }

    let result = handle_contour_draw_click(&mut world, &entities, [0.4, 0.5]);
    assert_eq!(result, Ok(ContourDrawClickOutcome::PointAdded));

    let editor = world.get::<&EditorState>(entities.editor).unwrap();
    let draft = editor.contour_draft.as_ref().expect("draft");
    assert_eq!(draft.points.len(), 1);
    let roi = world.get::<&Roi>(roi_entity).unwrap();
    assert!(roi.contour_data().unwrap().slices.is_empty());
}

#[test]
fn test_contour_draw_loop_closure_rejects_fewer_than_three_points() {
    let mut world = World::new();
    let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
    let roi_entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial);
    {
        let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
        editor.active_roi = Some(roi_entity);
        editor.active_tool = EditorTool::ContourDraw;
    }

    assert_eq!(
        handle_contour_draw_click(&mut world, &entities, [0.45, 0.45]),
        Ok(ContourDrawClickOutcome::PointAdded)
    );
    assert_eq!(
        handle_contour_draw_click(&mut world, &entities, [0.55, 0.45]),
        Ok(ContourDrawClickOutcome::PointAdded)
    );
    let close_result = handle_contour_draw_click(&mut world, &entities, [0.45, 0.45]);
    assert_eq!(
        close_result,
        Err(ContourDrawClickError::LoopNeedsThreePoints)
    );
}

#[test]
fn test_contour_draw_loop_commit_adds_slice_loop_and_queues_voxel_rebuild() {
    let mut world = World::new();
    let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
    let roi_entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial);
    {
        let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
        editor.active_roi = Some(roi_entity);
        editor.active_tool = EditorTool::ContourDraw;
    }

    assert_eq!(
        handle_contour_draw_click(&mut world, &entities, [0.40, 0.40]),
        Ok(ContourDrawClickOutcome::PointAdded)
    );
    assert_eq!(
        handle_contour_draw_click(&mut world, &entities, [0.58, 0.42]),
        Ok(ContourDrawClickOutcome::PointAdded)
    );
    assert_eq!(
        handle_contour_draw_click(&mut world, &entities, [0.52, 0.62]),
        Ok(ContourDrawClickOutcome::PointAdded)
    );
    let close_result = handle_contour_draw_click(&mut world, &entities, [0.40, 0.40]);
    assert_eq!(close_result, Ok(ContourDrawClickOutcome::LoopCommitted));

    let roi = world.get::<&Roi>(roi_entity).unwrap();
    let contour = roi.contour_data().unwrap();
    assert_eq!(contour.slices.len(), 1);
    assert_eq!(contour.slices[0].loops.len(), 1);
    assert!(contour.slices[0].loops[0].is_closed);
    assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildVoxelCache));
    assert!(roi.job_state.pending.iter().any(|request| {
        request.kind == RoiJobKind::RebuildVoxelCache
            && request.dirty_region
                == crate::components::RoiDirtyRegion::ContourSlice(
                    crate::components::ContourSliceKey::from_plane(contour.slices[0].plane),
                )
    }));
    assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
    assert!(world
        .get::<&EditorState>(entities.editor)
        .unwrap()
        .contour_draft
        .is_none());
}

#[test]
fn test_add_loop_path_reentering_existing_contour_commits_union_and_fills_voxels() {
    let mut world = World::new();
    let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
    let roi_entity = spawn_test_contour_roi_with_loop(&mut world, PlaneFamily::Axial);
    let initial_contour = world
        .get::<&Roi>(roi_entity)
        .unwrap()
        .contour_data()
        .unwrap()
        .clone();
    let initial_voxel =
        crate::convert::rasterize_contours_to_voxel_data(&initial_contour, test_geometry())
            .unwrap();
    {
        let mut roi = world.get::<&mut Roi>(roi_entity).unwrap();
        roi.session_caches.voxel = Some(VoxelCache {
            data: initial_voxel,
            gpu_resources: None,
        });
        roi.dirty_state.voxel.dirty = false;
        roi.dirty_state.voxel.built_from = roi.dirty_state.authoritative;
    }
    {
        let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
        editor.active_roi = Some(roi_entity);
        editor.active_tool = EditorTool::ContourDraw;
    }
    let viewport = resolve_active_contour_edit_viewport(&world, &entities, &initial_contour)
        .expect("edit viewport");
    let click_uvs = [[4.0, -2.0], [10.0, -5.0], [10.0, 5.0], [4.0, 2.0]]
        .map(|local_mm| contour_plane_local_mm_to_viewport_uv(local_mm, viewport).unwrap());

    for click_uv in &click_uvs[..3] {
        assert_eq!(
            handle_contour_draw_click(&mut world, &entities, *click_uv),
            Ok(ContourDrawClickOutcome::PointAdded)
        );
    }
    assert_eq!(
        handle_contour_draw_click(&mut world, &entities, click_uvs[3]),
        Ok(ContourDrawClickOutcome::LoopCommitted)
    );

    let merged_contour = world
        .get::<&Roi>(roi_entity)
        .unwrap()
        .contour_data()
        .unwrap()
        .clone();
    assert_eq!(merged_contour.slices[0].loops.len(), 1);
    assert!(contour_slice_contains_point(
        &merged_contour.slices[0],
        [0.0, 0.0]
    ));
    assert!(contour_slice_contains_point(
        &merged_contour.slices[0],
        [8.0, 0.0]
    ));

    roi_runtime::process_contour_voxel_rebuild_jobs(&mut world);

    let plane = merged_contour.slices[0].plane;
    let added_world = plane_local_mm_to_world_mm([8.0, 0.0], plane);
    let added_index = crate::convert::world_mm_to_voxel_index(added_world, test_geometry())
        .map(|value| value.round() as u32);
    let dimensions = test_geometry().dimensions;
    let linear = ((added_index[2] * dimensions[1] + added_index[1]) * dimensions[0]
        + added_index[0]) as usize;
    let roi = world.get::<&Roi>(roi_entity).unwrap();
    assert_eq!(roi.voxel_cache().unwrap().data.raw_data[linear], 1);
    assert!(roi.is_cache_current(RoiCacheKind::Voxel));
}

#[test]
fn test_add_loop_reprojects_display_points_into_existing_coplanar_slice_frame() {
    let mut world = World::new();
    let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
    let roi_entity = spawn_test_contour_roi_with_loop(&mut world, PlaneFamily::Axial);
    reframe_first_contour_slice(&mut world, roi_entity);
    let contour = world
        .get::<&Roi>(roi_entity)
        .unwrap()
        .contour_data()
        .unwrap()
        .clone();
    let viewport = resolve_active_contour_edit_viewport(&world, &entities, &contour).unwrap();
    {
        let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
        editor.active_roi = Some(roi_entity);
        editor.active_tool = EditorTool::ContourDraw;
    }
    let click_uvs = [[4.0, -2.0], [10.0, -5.0], [10.0, 5.0], [4.0, 2.0]]
        .map(|local_mm| contour_plane_local_mm_to_viewport_uv(local_mm, viewport).unwrap());

    for click_uv in &click_uvs[..3] {
        assert_eq!(
            handle_contour_draw_click(&mut world, &entities, *click_uv),
            Ok(ContourDrawClickOutcome::PointAdded)
        );
    }
    assert_eq!(
        handle_contour_draw_click(&mut world, &entities, click_uvs[3]),
        Ok(ContourDrawClickOutcome::LoopCommitted)
    );

    let roi = world.get::<&Roi>(roi_entity).unwrap();
    let merged_slice = &roi.contour_data().unwrap().slices[0];
    let added_world = plane_local_mm_to_world_mm([8.0, 0.0], viewport.plane);
    let added_in_slice =
        crate::convert::world_mm_to_plane_local_mm(added_world, merged_slice.plane);
    assert!(contour_slice_contains_point(merged_slice, added_in_slice));
    assert!(roi.job_state.pending.iter().any(|request| {
        request.kind == RoiJobKind::RebuildVoxelCache
            && request.dirty_region
                == crate::components::RoiDirtyRegion::ContourSlice(
                    crate::components::ContourSliceKey::from_plane(merged_slice.plane),
                )
    }));
}

#[test]
fn test_clear_contour_draft_when_tool_not_draw() {
    let mut world = World::new();
    let editor = world.spawn((EditorState {
        active_roi: Some(hecs::Entity::DANGLING),
        active_tool: EditorTool::Navigation,
        contour_draft: Some(ContourDraft {
            roi_entity: hecs::Entity::DANGLING,
            plane: PlaneDefinition {
                family: PlaneFamily::Axial,
                origin_mm: [0.0, 0.0, 0.0],
                u_axis_mm: [1.0, 0.0, 0.0],
                v_axis_mm: [0.0, 1.0, 0.0],
                normal_mm: [0.0, 0.0, 1.0],
            },
            points: vec![],
        }),
        contour_selection: None,
        roi_edit_preview: None,
        ..EditorState::default()
    },));

    clear_contour_draft_if_inactive(&mut world, editor);
    assert!(world
        .get::<&EditorState>(editor)
        .unwrap()
        .contour_draft
        .is_none());
}

#[test]
fn test_nearest_point_hit_selects_closest_point() {
    let candidates = vec![
        (0usize, 0usize, 0usize, [0.4, 0.4]),
        (0usize, 0usize, 1usize, [0.42, 0.4]),
        (0usize, 1usize, 0usize, [0.8, 0.8]),
    ];
    let result = nearest_point_hit(&candidates, [0.421, 0.401], [800.0, 600.0], 10.0);
    assert_eq!(result, Some((0, 0, 1)));
}

#[test]
fn test_nearest_point_hit_respects_threshold() {
    let candidates = vec![(0usize, 0usize, 0usize, [0.4, 0.4])];
    let result = nearest_point_hit(&candidates, [0.6, 0.6], [800.0, 600.0], 5.0);
    assert_eq!(result, None);
}

#[test]
fn test_contour_selection_rejects_non_contour_active_roi() {
    let mut world = World::new();
    let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
    let voxel_roi = world.spawn((Roi::new_voxel_with_cache(
        crate::components::RoiId(1),
        "Voxel".to_string(),
        VoxelGeometry::new(
            [8, 8, 8],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap(),
        vec![0; 512],
        None,
    ),));
    {
        let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
        editor.active_roi = Some(voxel_roi);
        editor.active_tool = EditorTool::ContourSelect;
    }

    let result = handle_contour_select_click(&mut world, &entities, [0.5, 0.5]);
    assert_eq!(result, Err(ContourSelectClickError::ActiveRoiNotContour));
}

#[test]
fn test_contour_selection_rejects_mismatched_plane_family() {
    let mut world = World::new();
    let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
    let roi_entity = spawn_test_contour_roi(&mut world, PlaneFamily::Coronal);
    {
        let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
        editor.active_roi = Some(roi_entity);
        editor.active_tool = EditorTool::ContourSelect;
    }

    let result = handle_contour_select_click(&mut world, &entities, [0.5, 0.5]);
    assert_eq!(
        result,
        Err(ContourSelectClickError::Mapping(
            ContourEditMappingError::PlaneFamilyMismatch {
                contour_family: PlaneFamily::Coronal,
                viewport_family: PlaneFamily::Axial,
            }
        ))
    );
}

#[test]
fn test_move_selected_point_updates_only_selected_point() {
    let mut world = World::new();
    let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
    let roi_entity = spawn_test_contour_roi_with_loop(&mut world, PlaneFamily::Axial);
    let before_points = world
        .get::<&Roi>(roi_entity)
        .unwrap()
        .contour_data()
        .unwrap()
        .slices[0]
        .loops[0]
        .points
        .clone();
    {
        let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
        editor.active_roi = Some(roi_entity);
        editor.active_tool = EditorTool::ContourSelect;
        editor.contour_selection = Some(ContourSelection {
            roi_entity,
            slice_index: 0,
            loop_index: 0,
            point_index: Some(1),
        });
    }

    move_selected_point(&mut world, &entities, [0.6, 0.55]).unwrap();

    let roi = world.get::<&Roi>(roi_entity).unwrap();
    let after_points = &roi.contour_data().unwrap().slices[0].loops[0].points;
    assert_ne!(after_points[1].local_mm, before_points[1].local_mm);
    assert_eq!(after_points[0].local_mm, before_points[0].local_mm);
    assert_eq!(after_points[2].local_mm, before_points[2].local_mm);
    assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildVoxelCache));
}

#[test]
fn test_move_selected_point_reprojects_display_point_into_slice_frame() {
    let mut world = World::new();
    let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
    let roi_entity = spawn_test_contour_roi_with_loop(&mut world, PlaneFamily::Axial);
    reframe_first_contour_slice(&mut world, roi_entity);
    let contour = world
        .get::<&Roi>(roi_entity)
        .unwrap()
        .contour_data()
        .unwrap()
        .clone();
    let viewport = resolve_active_contour_edit_viewport(&world, &entities, &contour).unwrap();
    let target_local = [2.0, 3.0];
    let target_uv = contour_plane_local_mm_to_viewport_uv(target_local, viewport).unwrap();
    {
        let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
        editor.active_roi = Some(roi_entity);
        editor.active_tool = EditorTool::ContourSelect;
        editor.contour_selection = Some(ContourSelection {
            roi_entity,
            slice_index: 0,
            loop_index: 0,
            point_index: Some(1),
        });
    }

    move_selected_point(&mut world, &entities, target_uv).unwrap();

    let roi = world.get::<&Roi>(roi_entity).unwrap();
    let slice = &roi.contour_data().unwrap().slices[0];
    let actual_world = plane_local_mm_to_world_mm(slice.loops[0].points[1].local_mm, slice.plane);
    let expected_world = plane_local_mm_to_world_mm(target_local, viewport.plane);
    for axis in 0..3 {
        assert!((actual_world[axis] - expected_world[axis]).abs() < 1e-4);
    }
    assert!(roi.job_state.pending.iter().any(|request| {
        request.kind == RoiJobKind::RebuildVoxelCache
            && request.dirty_region
                == crate::components::RoiDirtyRegion::ContourSlice(
                    crate::components::ContourSliceKey::from_plane(slice.plane),
                )
    }));
}

#[test]
fn test_move_selected_point_preview_defers_authoritative_commit_until_finalize() {
    let mut world = World::new();
    let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
    let roi_entity = spawn_test_contour_roi_with_loop(&mut world, PlaneFamily::Axial);
    let before_points = world
        .get::<&Roi>(roi_entity)
        .unwrap()
        .contour_data()
        .unwrap()
        .slices[0]
        .loops[0]
        .points
        .clone();
    {
        let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
        editor.active_roi = Some(roi_entity);
        editor.active_tool = EditorTool::ContourSelect;
        editor.contour_selection = Some(ContourSelection {
            roi_entity,
            slice_index: 0,
            loop_index: 0,
            point_index: Some(1),
        });
    }

    move_selected_point_preview(&mut world, &entities, [0.9, 0.55]).unwrap();

    {
        let roi = world.get::<&Roi>(roi_entity).unwrap();
        let authoritative_points = &roi.contour_data().unwrap().slices[0].loops[0].points;
        assert_eq!(authoritative_points, &before_points);
        assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildVoxelCache));
        assert!(roi.preview_state.active);
        assert_eq!(roi.preview_state.revision, 1);
    }
    {
        let editor = world.get::<&EditorState>(entities.editor).unwrap();
        assert!(editor.contour_move_preview().is_some());
        assert!(editor.roi_undo_stack.is_empty());
    }

    roi_runtime::process_contour_voxel_rebuild_jobs(&mut world);
    {
        let roi = world.get::<&Roi>(roi_entity).unwrap();
        assert!(roi.session_caches.preview_voxel.is_some());
        assert!(roi.session_caches.preview_mesh.is_none());
        assert_eq!(roi.running_job_kind(), Some(RoiJobKind::RebuildVoxelCache));
    }
    move_selected_point_preview(&mut world, &entities, [0.6, 0.55]).unwrap();
    for _ in 0..64 {
        roi_runtime::process_contour_voxel_rebuild_jobs(&mut world);
        if world.get::<&Roi>(roi_entity).is_ok_and(|roi| {
            roi.session_caches
                .preview_mesh
                .as_ref()
                .is_some_and(|cache| cache.preview_revision == 2)
        }) {
            break;
        }
    }
    let geometry = roi_runtime::main_volume_geometry(&world).unwrap();
    let cross_plane =
        orthogonal_plane_from_volume_uv(PlaneFamily::Coronal, [0.5, 0.5, 0.5], geometry).unwrap();
    let cross_key = crate::components::ContourViewKey::from_plane(cross_plane);
    let cross_status = roi_runtime::ensure_contour_view_cache(&mut world, roi_entity, &cross_key);
    assert_eq!(
        cross_status.state,
        roi_runtime::RepresentationRequestState::Preview
    );
    {
        let roi = world.get::<&Roi>(roi_entity).unwrap();
        assert!(roi.session_caches.preview_voxel.is_some());
        assert!(roi
            .session_caches
            .preview_mesh
            .as_ref()
            .is_some_and(|cache| cache.chunks.is_some()));
        let preview_voxel = &roi.session_caches.preview_voxel.as_ref().unwrap().data;
        let clean_chunks = crate::convert::extract_chunked_mesh_from_voxel_data(
            preview_voxel,
            crate::convert::DEFAULT_MESH_CHUNK_SIZE,
        )
        .unwrap();
        let preview_chunks = roi
            .session_caches
            .preview_mesh
            .as_ref()
            .unwrap()
            .chunks
            .as_ref()
            .unwrap();
        assert_eq!(
            preview_chunks
                .chunks
                .iter()
                .map(|chunk| chunk.key)
                .collect::<Vec<_>>(),
            clean_chunks
                .chunks
                .iter()
                .map(|chunk| chunk.key)
                .collect::<Vec<_>>()
        );
        for (preview, clean) in preview_chunks.chunks.iter().zip(&clean_chunks.chunks) {
            assert_eq!(preview.key, clean.key);
            assert_eq!(
                preview.data.faces.len(),
                clean.data.faces.len(),
                "{:?}",
                preview.key
            );
            assert_eq!(
                preview.data.vertices.len(),
                clean.data.vertices.len(),
                "{:?}",
                preview.key
            );
        }
        assert_eq!(
            roi.session_caches.preview_mesh.as_ref().unwrap().data,
            clean_chunks.merged_mesh()
        );
        assert!(matches!(
            roi.contour_view_cache(&cross_key).map(|cache| &cache.state),
            Some(crate::components::CacheViewState::Preview { revision: 2 })
        ));
        assert_eq!(roi.running_job_kind(), None);
        assert_eq!(roi.queued_job_kind(), None);
    }

    finalize_selected_point_move(&mut world, &entities).unwrap();

    let roi = world.get::<&Roi>(roi_entity).unwrap();
    let after_points = &roi.contour_data().unwrap().slices[0].loops[0].points;
    assert_ne!(after_points[1].local_mm, before_points[1].local_mm);
    assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildVoxelCache));
    assert!(!roi.preview_state.active);
    {
        let editor = world.get::<&EditorState>(entities.editor).unwrap();
        assert!(editor.contour_move_preview().is_none());
        assert_eq!(editor.roi_undo_stack.len(), 1);
    }
    drop(roi);

    roi_runtime::process_contour_voxel_rebuild_jobs(&mut world);

    let roi = world.get::<&Roi>(roi_entity).unwrap();
    assert!(roi.is_cache_current(RoiCacheKind::Voxel));
    assert_eq!(roi.running_job_kind(), None);
    assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildMeshCache));
}

#[test]
fn test_insert_point_adds_at_expected_loop_position() {
    let mut world = World::new();
    let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
    let roi_entity = spawn_test_contour_roi_with_loop(&mut world, PlaneFamily::Axial);
    {
        let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
        editor.active_roi = Some(roi_entity);
        editor.active_tool = EditorTool::ContourSelect;
        editor.contour_selection = Some(ContourSelection {
            roi_entity,
            slice_index: 0,
            loop_index: 0,
            point_index: Some(1),
        });
    }

    insert_point_into_selected_loop(&mut world, &entities, [0.55, 0.45]).unwrap();

    let roi = world.get::<&Roi>(roi_entity).unwrap();
    let points = &roi.contour_data().unwrap().slices[0].loops[0].points;
    assert_eq!(points.len(), 5);
    assert_eq!(
        world
            .get::<&EditorState>(entities.editor)
            .unwrap()
            .contour_selection,
        Some(ContourSelection {
            roi_entity,
            slice_index: 0,
            loop_index: 0,
            point_index: Some(2),
        })
    );
    assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildVoxelCache));
}

#[test]
fn test_insert_point_reprojects_display_point_into_slice_frame() {
    let mut world = World::new();
    let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
    let roi_entity = spawn_test_contour_roi_with_loop(&mut world, PlaneFamily::Axial);
    reframe_first_contour_slice(&mut world, roi_entity);
    let contour = world
        .get::<&Roi>(roi_entity)
        .unwrap()
        .contour_data()
        .unwrap()
        .clone();
    let viewport = resolve_active_contour_edit_viewport(&world, &entities, &contour).unwrap();
    let target_local = [2.0, 3.0];
    let target_uv = contour_plane_local_mm_to_viewport_uv(target_local, viewport).unwrap();
    {
        let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
        editor.active_roi = Some(roi_entity);
        editor.active_tool = EditorTool::ContourSelect;
        editor.contour_selection = Some(ContourSelection {
            roi_entity,
            slice_index: 0,
            loop_index: 0,
            point_index: Some(1),
        });
    }

    insert_point_into_selected_loop(&mut world, &entities, target_uv).unwrap();

    let roi = world.get::<&Roi>(roi_entity).unwrap();
    let slice = &roi.contour_data().unwrap().slices[0];
    let actual_world = plane_local_mm_to_world_mm(slice.loops[0].points[2].local_mm, slice.plane);
    let expected_world = plane_local_mm_to_world_mm(target_local, viewport.plane);
    for axis in 0..3 {
        assert!((actual_world[axis] - expected_world[axis]).abs() < 1e-4);
    }
}

#[test]
fn test_delete_selected_point_removes_expected_point() {
    let mut world = World::new();
    let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
    let roi_entity = spawn_test_contour_roi_with_loop(&mut world, PlaneFamily::Axial);
    {
        let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
        editor.active_roi = Some(roi_entity);
        editor.active_tool = EditorTool::ContourSelect;
        editor.contour_selection = Some(ContourSelection {
            roi_entity,
            slice_index: 0,
            loop_index: 0,
            point_index: Some(1),
        });
    }

    delete_selected_contour_element(&mut world, &entities).unwrap();

    let roi = world.get::<&Roi>(roi_entity).unwrap();
    let points = &roi.contour_data().unwrap().slices[0].loops[0].points;
    assert_eq!(points.len(), 3);
    assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildVoxelCache));
}

#[test]
fn test_delete_below_valid_size_removes_loop_and_clears_selection() {
    let mut world = World::new();
    let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
    let geometry = VoxelGeometry::new(
        [64, 48, 32],
        [1.0, 1.0, 1.0],
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap();
    let plane =
        orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.4, 0.55, 0.2], geometry).unwrap();
    let roi_entity = world.spawn((Roi::new_contour(
        crate::components::RoiId(102),
        "TinyLoop".to_string(),
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
                            local_mm: [2.0, 0.0],
                        },
                        ContourPoint {
                            local_mm: [1.0, 2.0],
                        },
                    ],
                    is_closed: true,
                }],
            }],
        },
    ),));
    {
        let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
        editor.active_roi = Some(roi_entity);
        editor.active_tool = EditorTool::ContourSelect;
        editor.contour_selection = Some(ContourSelection {
            roi_entity,
            slice_index: 0,
            loop_index: 0,
            point_index: Some(1),
        });
    }

    delete_selected_contour_element(&mut world, &entities).unwrap();

    let roi = world.get::<&Roi>(roi_entity).unwrap();
    assert!(roi.contour_data().unwrap().slices.is_empty());
    assert!(world
        .get::<&EditorState>(entities.editor)
        .unwrap()
        .contour_selection
        .is_none());
    assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildVoxelCache));
}
