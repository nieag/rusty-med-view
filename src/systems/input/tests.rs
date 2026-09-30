use super::*;

#[test]
fn test_contour_selection_click_does_not_cross_drag_threshold() {
    assert!(!drag_exceeds_threshold_px(
        [0.5, 0.5],
        [0.501, 0.501],
        [500.0, 500.0],
    ));
}

#[test]
fn test_contour_point_motion_crosses_drag_threshold() {
    assert!(drag_exceeds_threshold_px(
        [0.5, 0.5],
        [0.51, 0.5],
        [500.0, 500.0],
    ));
}

#[test]
fn test_mouse_drag_rotates_oblique_viewport() {
    let mut world = World::new();
    let viewport = world.spawn((
        Viewport {
            mode: ViewMode::Oblique,
            rect: [0.0, 0.0, 500.0, 500.0],
            uniform_index: 0,
        },
        ViewportState::default(),
    ));
    let mut session = Session::new(800, 600);
    session.input = InputState {
        active_viewport: Some(viewport),
        mouse_uv: [0.65, 0.4],
        is_dragging: true,
        is_rotating: true,
        rotation_start_pos: [0.5, 0.5],
        rotation_start_val: [0.0, 0.0, 0.0, 1.0],
        ..InputState::default()
    };

    sys_handle_mouse_drag(&mut world, &mut session);

    let rotation = world.get::<&ViewportState>(viewport).unwrap().user_rotation;
    assert_ne!(rotation, [0.0, 0.0, 0.0, 1.0]);
    assert!((Quat::from_array(rotation).length() - 1.0).abs() < 1e-6);
}

#[test]
fn test_scroll_reports_3d_zoom_for_redraw_coalescing() {
    let mut world = World::new();
    let viewport = world.spawn((
        Viewport {
            mode: ViewMode::ThreeD,
            rect: [0.0, 0.0, 500.0, 500.0],
            uniform_index: 0,
        },
        ViewportState::default(),
    ));
    let mut session = Session::new(800, 600);
    session.input = InputState {
        active_viewport: Some(viewport),
        modifiers: ModifiersState::CONTROL,
        ..InputState::default()
    };

    assert!(sys_handle_input_scroll(&mut world, &mut session, 1.0));
}
