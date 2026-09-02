// src/systems/input.rs
use crate::components::*;
use crate::convert::{slice_center_uv, slice_index_from_cursor_uv};
use crate::systems::picking::get_voxel_at_mouse;
use glam::{Quat, Vec3};
use hecs::World;
use winit::event::{ElementState, MouseButton};
use winit::keyboard::ModifiersState;

const CONTOUR_DRAG_THRESHOLD_PX: f32 = 2.0;

fn drag_exceeds_threshold_px(
    start_uv: [f32; 2],
    current_uv: [f32; 2],
    viewport_size_px: [f32; 2],
) -> bool {
    let delta_x = (current_uv[0] - start_uv[0]) * viewport_size_px[0];
    let delta_y = (current_uv[1] - start_uv[1]) * viewport_size_px[1];
    delta_x * delta_x + delta_y * delta_y >= CONTOUR_DRAG_THRESHOLD_PX * CONTOUR_DRAG_THRESHOLD_PX
}

fn set_status_message(world: &mut World, entities: &AppEntities, message: String) {
    if let Ok(mut gui_state) = world.get::<&mut GuiState>(entities.gui_state) {
        gui_state.status_message = Some(message);
    }
}

/// Update keyboard modifier state (Ctrl/Shift/Alt) in the ECS.
pub fn sys_update_modifiers(world: &mut World, entities: &AppEntities, mods: ModifiersState) {
    if let Ok(mut input) = world.get::<&mut InputState>(entities.input) {
        input.modifiers = mods;
    }
}

pub fn sys_update_mouse(world: &mut World, entities: &AppEntities, x: f64, y: f64) {
    let mut found_viewport = None;
    let mut local_uv = [0.0, 0.0];

    // Query all viewports to see which one contains the mouse
    for (e, vp) in world.query::<&Viewport>().iter() {
        let vx = vp.rect[0] as f64;
        let vy = vp.rect[1] as f64;
        let vw = vp.rect[2] as f64;
        let vh = vp.rect[3] as f64;

        if x >= vx && x <= (vx + vw) && y >= vy && y <= (vy + vh) {
            found_viewport = Some(e);
            let u = (x - vx) / vw;
            let v = (y - vy) / vh;
            local_uv = [u as f32, v as f32];
            break;
        }
    }

    if let Ok(mut input) = world.get::<&mut InputState>(entities.input) {
        input.last_mouse_pos = [x, y];
        if let Some(e) = found_viewport {
            input.active_viewport = Some(e);
            input.mouse_uv = local_uv;
        }
    }
}

/// Handle mouse button events for clicking, dragging, and picking.
pub fn sys_handle_mouse_button(
    world: &mut World,
    entities: &AppEntities,
    button: MouseButton,
    state: ElementState,
) {
    crate::systems::clear_contour_draft_if_inactive(world, entities.editor);
    crate::systems::clear_contour_selection_if_inactive(world, entities.editor);

    let mut active_vp = None;
    let mut alt_pressed = false;
    let mut finalize_contour_move = false;
    let mut finalize_mesh_move = false;

    if let Ok(mut input) = world.get::<&mut InputState>(entities.input) {
        active_vp = input.active_viewport;
        alt_pressed = input.modifiers.alt_key();

        if input.egui_wants_input && state == ElementState::Pressed {
            return;
        }

        if state == ElementState::Pressed {
            input.is_dragging = true;
            input.drag_start_pos = input.mouse_uv;

            let ctrl_pressed = input.modifiers.control_key();
            let super_pressed = input.modifiers.super_key();

            if let Some(avp) = active_vp {
                if button == MouseButton::Middle
                    || (button == MouseButton::Left && (ctrl_pressed || super_pressed))
                {
                    input.is_panning = true;
                    if let Ok(vs) = world.get::<&ViewportState>(avp) {
                        input.drag_start_pan = vs.pan;
                    }
                }

                if button == MouseButton::Right || (button == MouseButton::Left && alt_pressed) {
                    input.is_rotating = true;
                    input.rotation_start_pos = input.mouse_uv;
                    if let Ok(vs) = world.get::<&ViewportState>(avp) {
                        input.rotation_start_val = vs.user_rotation;
                    }
                }
            }
        } else if state == ElementState::Released {
            input.is_dragging = false;
            input.is_panning = false;
            input.is_rotating = false;
            if input.contour_move_pending_commit {
                finalize_contour_move = true;
                input.contour_move_pending_commit = false;
            }
            if input.mesh_move_pending_commit {
                finalize_mesh_move = true;
                input.mesh_move_pending_commit = false;
            }
        }
    }

    if finalize_contour_move {
        let _ = crate::systems::finalize_selected_point_move(world, entities);
    }
    if finalize_mesh_move {
        match crate::app::roi::commit_mesh_edit_preview(world, entities.editor) {
            Ok(()) => set_status_message(
                world,
                entities,
                "Committed mesh deformation; build voxels explicitly when needed.".to_string(),
            ),
            Err(error) => set_status_message(
                world,
                entities,
                format!("Mesh deformation commit failed: {error:?}."),
            ),
        }
    }

    let ctrl_pressed = if let Ok(input) = world.get::<&InputState>(entities.input) {
        input.modifiers.control_key()
    } else {
        false
    };
    let super_pressed = if let Ok(input) = world.get::<&InputState>(entities.input) {
        input.modifiers.super_key()
    } else {
        false
    };

    if button == MouseButton::Left
        && !alt_pressed
        && !ctrl_pressed
        && !super_pressed
        && state == ElementState::Pressed
    {
        let active_tool = world
            .get::<&EditorState>(entities.editor)
            .map(|editor| editor.active_tool)
            .unwrap_or(EditorTool::Navigation);
        if active_tool == EditorTool::ContourDraw {
            let click_pos = world
                .get::<&InputState>(entities.input)
                .map(|input| input.mouse_uv)
                .unwrap_or([0.5, 0.5]);
            match crate::systems::handle_contour_draw_click(world, entities, click_pos) {
                Ok(crate::systems::ContourDrawClickOutcome::PointAdded) => {}
                Ok(crate::systems::ContourDrawClickOutcome::LoopCommitted) => {
                    set_status_message(world, entities, "Contour loop committed.".to_string());
                }
                Err(crate::systems::ContourDrawClickError::LoopNeedsThreePoints) => {
                    set_status_message(
                        world,
                        entities,
                        "Need at least 3 points to close a contour loop.".to_string(),
                    );
                }
                Err(crate::systems::ContourDrawClickError::MissingActiveRoi) => {
                    set_status_message(
                        world,
                        entities,
                        "Select a contour ROI before drawing.".to_string(),
                    );
                }
                Err(crate::systems::ContourDrawClickError::ActiveRoiNotContour) => {
                    set_status_message(
                        world,
                        entities,
                        "Active ROI is not contour-primary.".to_string(),
                    );
                }
                Err(crate::systems::ContourDrawClickError::Mapping(error)) => {
                    set_status_message(world, entities, format!("Contour draw blocked: {error:?}"));
                }
                Err(_) => {}
            }
            return;
        }
        if active_tool == EditorTool::ContourSelect {
            let click_pos = world
                .get::<&InputState>(entities.input)
                .map(|input| input.mouse_uv)
                .unwrap_or([0.5, 0.5]);
            match crate::systems::handle_contour_select_click(world, entities, click_pos) {
                Ok(Some(selection)) => {
                    if selection.point_index.is_some() {
                        set_status_message(world, entities, "Selected contour point.".to_string());
                    } else {
                        set_status_message(world, entities, "Selected contour loop.".to_string());
                    }
                }
                Ok(None) => {}
                Err(crate::systems::ContourSelectClickError::MissingActiveRoi) => {
                    set_status_message(
                        world,
                        entities,
                        "Select a contour ROI before selecting contours.".to_string(),
                    );
                }
                Err(crate::systems::ContourSelectClickError::ActiveRoiNotContour) => {
                    set_status_message(
                        world,
                        entities,
                        "Active ROI is not contour-primary.".to_string(),
                    );
                }
                Err(crate::systems::ContourSelectClickError::Mapping(error)) => {
                    set_status_message(
                        world,
                        entities,
                        format!("Contour selection blocked: {error:?}"),
                    );
                }
                Err(_) => {}
            }
            return;
        }
        if active_tool == EditorTool::MeshDeform {
            let click_pos = world
                .get::<&InputState>(entities.input)
                .map(|input| input.mouse_uv)
                .unwrap_or([0.5, 0.5]);
            match crate::systems::select_mesh_vertex(world, entities, click_pos) {
                Ok(Some(_)) => {
                    set_status_message(world, entities, "Selected mesh surface.".to_string())
                }
                Ok(None) => {}
                Err(error) => set_status_message(
                    world,
                    entities,
                    format!("Mesh selection blocked: {error:?}."),
                ),
            }
            return;
        }

        if let Some(avp) = active_vp {
            let mut click_pos = [0.0, 0.0];
            if let Ok(input) = world.get::<&InputState>(entities.input) {
                click_pos = input.mouse_uv;
            }
            if let Some(target_pos) = get_voxel_at_mouse(world, entities, avp, click_pos) {
                if let Ok(mut t) = world.get::<&mut Transform>(entities.cursor) {
                    t.position = target_pos;
                }
            }
        }
    }
}

/// Handle scroll input for zooming or slice scrolling.
///
/// Returns whether this changed a 3D zoom. Callers can coalesce the expensive
/// 3D redraw path while preserving immediate redraws for the slice views.
pub fn sys_handle_input_scroll(world: &mut World, entities: &AppEntities, delta: f32) -> bool {
    let mut active_vp = None;
    let mut mouse_uv = [0.5, 0.5];
    let mut is_zoom = false;

    if let Ok(input) = world.get::<&InputState>(entities.input) {
        if input.egui_wants_input {
            return false;
        }
        active_vp = input.active_viewport;
        mouse_uv = input.mouse_uv;
        is_zoom = input.modifiers.control_key();
    }

    let avp = match active_vp {
        Some(e) => e,
        None => return false,
    };

    let mut vol_aspects = [1.0, 1.0, 1.0];
    let mut dims = [1u32, 1, 1];
    for (_, vol) in world.query::<&VolumeData>().iter() {
        vol_aspects = vol.aspect_ratios();
        dims = vol.dimensions;
    }

    let mut changed_3d_zoom = false;
    if is_zoom {
        if let (Ok(vp), Ok(mut vs)) = (
            world.get::<&Viewport>(avp),
            world.get::<&mut ViewportState>(avp),
        ) {
            let screen_aspect = if vp.rect[3] > 0.0 {
                vp.rect[2] / vp.rect[3]
            } else {
                1.0
            };
            let k = match vp.mode {
                ViewMode::ThreeD => 1.0,
                ViewMode::Axial => screen_aspect / (vol_aspects[0] / vol_aspects[1]),
                ViewMode::Coronal => screen_aspect / (vol_aspects[0] / vol_aspects[2]),
                ViewMode::Sagittal => screen_aspect / (vol_aspects[1] / vol_aspects[2]),
                ViewMode::Oblique => screen_aspect,
            };

            let mx_centered = (mouse_uv[0] - 0.5) * k;
            let my_centered = mouse_uv[1] - 0.5;

            let sensitivity = 0.1;
            let scale_factor = 1.0 + (delta * sensitivity);
            let old_zoom = vs.zoom;
            let new_zoom = (old_zoom * scale_factor).clamp(0.5, 100.0);

            vs.pan[0] += mx_centered * (1.0 / old_zoom - 1.0 / new_zoom);
            vs.pan[1] += my_centered * (1.0 / old_zoom - 1.0 / new_zoom);
            vs.zoom = new_zoom;
            vs.pivot = [0.5, 0.5];
            changed_3d_zoom = vp.mode == ViewMode::ThreeD;
        }
    } else if let Ok(mut transform) = world.get::<&mut Transform>(entities.cursor) {
        if let Ok(vp) = world.get::<&Viewport>(avp) {
            let (axis, dim) = match vp.mode {
                ViewMode::Axial => (2, dims[2]),
                ViewMode::Coronal => (1, dims[1]),
                ViewMode::Sagittal => (0, dims[0]),
                ViewMode::Oblique => (2, dims[2]),
                _ => return false,
            };
            if dim == 0 {
                return false;
            }

            if let Ok(mut input) = world.get::<&mut InputState>(entities.input) {
                let abs_delta = delta.abs();
                let factor = if abs_delta <= 1.0 {
                    1.0
                } else {
                    1.0 + (abs_delta - 1.0) * 0.5
                };
                let accelerated_delta = delta * factor;

                // For now, we'll use a simplified accumulator or just apply it.
                // The old code used scroll_accumulator[vp_idx], but now we have multiple viewports.
                // We'll use index 0 for now or add it to ViewportState?
                // Let's just use index 0 to avoid adding to ViewportState yet.
                input.scroll_accumulator[0] += accelerated_delta;

                let mut steps_to_move = 0;
                if input.scroll_accumulator[0].abs() >= 1.0 {
                    steps_to_move = input.scroll_accumulator[0].trunc() as i32;
                    input.scroll_accumulator[0] -= steps_to_move as f32;
                }

                if steps_to_move != 0 {
                    let current_uv = transform.position[axis];
                    let current_voxel = slice_index_from_cursor_uv(current_uv, dim);
                    let new_voxel = (current_voxel + steps_to_move).clamp(0, dim as i32 - 1);
                    transform.position[axis] = slice_center_uv(new_voxel, dim);
                }
            }
        }
    }

    changed_3d_zoom
}

/// Handle mouse drag motion for panning and rotating.
pub fn sys_handle_mouse_drag(world: &mut World, entities: &AppEntities) {
    let mut active_vp = None;
    let mut is_dragging = false;
    let mut is_rotating = false;
    let mut is_panning = false;

    if let Ok(input) = world.get::<&InputState>(entities.input) {
        if input.egui_wants_input && !input.is_dragging && !input.is_rotating && !input.is_panning {
            return;
        }
        active_vp = input.active_viewport;
        is_dragging = input.is_dragging;
        is_rotating = input.is_rotating;
        is_panning = input.is_panning;
    }

    if (!is_dragging && !is_rotating && !is_panning) || active_vp.is_none() {
        return;
    }
    let Some(avp) = active_vp else {
        return;
    };

    let mut vol_aspects = [1.0, 1.0, 1.0];
    for (_, vol) in world.query::<&VolumeData>().iter() {
        vol_aspects = vol.aspect_ratios();
    }

    let mut crosshair_update = None;
    let mut contour_move_update = None;
    let mut mesh_move_update = None;

    if let (Ok(vp), Ok(mut vs)) = (
        world.get::<&Viewport>(avp),
        world.get::<&mut ViewportState>(avp),
    ) {
        let mut drag_info = None;
        let mut rotate_info = None;

        if let Ok(input) = world.get::<&InputState>(entities.input) {
            if is_panning {
                drag_info = Some((input.drag_start_pan, input.drag_start_pos, input.mouse_uv));
            }
            if is_rotating {
                rotate_info = Some((
                    input.rotation_start_val,
                    input.rotation_start_pos,
                    input.mouse_uv,
                ));
            }
            // Crosshair update during drag - prepare info
            let active_tool = world
                .get::<&EditorState>(entities.editor)
                .map(|editor| editor.active_tool)
                .unwrap_or(EditorTool::Navigation);
            if is_dragging && !is_panning && !is_rotating {
                if active_tool == EditorTool::Navigation {
                    crosshair_update = Some((avp, input.mouse_uv));
                } else if active_tool == EditorTool::ContourSelect
                    && drag_exceeds_threshold_px(
                        input.drag_start_pos,
                        input.mouse_uv,
                        [vp.rect[2], vp.rect[3]],
                    )
                {
                    contour_move_update = Some(input.mouse_uv);
                } else if active_tool == EditorTool::MeshDeform
                    && drag_exceeds_threshold_px(
                        input.drag_start_pos,
                        input.mouse_uv,
                        [vp.rect[2], vp.rect[3]],
                    )
                {
                    mesh_move_update = Some(input.mouse_uv);
                }
            }
        }

        if let Some((start_pan, start_pos, current_pos)) = drag_info {
            let zoom = vs.zoom;
            let mut k = 1.0;
            if vp.mode != ViewMode::ThreeD {
                let screen_aspect = if vp.rect[3] > 0.0 {
                    vp.rect[2] / vp.rect[3]
                } else {
                    1.0
                };
                let slice_aspect = match vp.mode {
                    ViewMode::Axial => vol_aspects[0] / vol_aspects[1],
                    ViewMode::Coronal => vol_aspects[0] / vol_aspects[2],
                    ViewMode::Sagittal => vol_aspects[1] / vol_aspects[2],
                    ViewMode::Oblique => 1.0,
                    _ => 1.0,
                };
                k = screen_aspect / slice_aspect;
            }
            vs.pan[0] = start_pan[0] + ((start_pos[0] - current_pos[0]) * k) / zoom;
            vs.pan[1] = start_pan[1] + (start_pos[1] - current_pos[1]) / zoom;
        }

        if let Some((start_quat, start_pos, current_pos)) = rotate_info {
            let sensitivity = 3.0;
            let mut has_shift = false;
            if let Ok(input) = world.get::<&InputState>(entities.input) {
                has_shift = input.modifiers.shift_key();
            }
            let delta_x = (current_pos[0] - start_pos[0]) * sensitivity;
            let delta_y = (start_pos[1] - current_pos[1]) * sensitivity;
            let start_q = Quat::from_array(start_quat);

            let new_quat = if has_shift {
                let roll_quat = Quat::from_axis_angle(Vec3::NEG_Z, delta_x);
                (roll_quat * start_q).normalize()
            } else {
                let yaw_quat = Quat::from_axis_angle(Vec3::Y, -delta_x);
                let pitch_quat = Quat::from_axis_angle(Vec3::X, delta_y);
                (yaw_quat * pitch_quat * start_q).normalize()
            };
            vs.user_rotation = new_quat.to_array();
        }
    }

    // Now update crosshair outside ViewPortState borrow
    if let Some((avp, uv)) = crosshair_update {
        if let Some(target_pos) = get_voxel_at_mouse(world, entities, avp, uv) {
            if let Ok(mut t) = world.get::<&mut Transform>(entities.cursor) {
                t.position = target_pos;
            }
        }
    }

    if let Some(uv) = contour_move_update {
        if crate::systems::move_selected_point_preview(world, entities, uv).is_ok() {
            if let Ok(mut input) = world.get::<&mut InputState>(entities.input) {
                input.contour_move_pending_commit = true;
            }
        }
    }
    if let Some(uv) = mesh_move_update {
        if crate::systems::update_selected_mesh_deform_preview(world, entities, uv).is_ok() {
            if let Ok(mut input) = world.get::<&mut InputState>(entities.input) {
                input.mesh_move_pending_commit = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hecs::Entity;

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
        let input = world.spawn((InputState {
            active_viewport: Some(viewport),
            mouse_uv: [0.65, 0.4],
            is_dragging: true,
            is_rotating: true,
            rotation_start_pos: [0.5, 0.5],
            rotation_start_val: [0.0, 0.0, 0.0, 1.0],
            ..InputState::default()
        },));
        let editor = world.spawn((EditorState::default(),));
        let entities = AppEntities {
            input,
            editor,
            gui_state: Entity::DANGLING,
            volume_windowing: Entity::DANGLING,
            annotations: Entity::DANGLING,
            overlay: Entity::DANGLING,
            protocol: Entity::DANGLING,
            cursor: Entity::DANGLING,
            window_settings: Entity::DANGLING,
        };

        sys_handle_mouse_drag(&mut world, &entities);

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
        let input = world.spawn((InputState {
            active_viewport: Some(viewport),
            modifiers: ModifiersState::CONTROL,
            ..InputState::default()
        },));
        let editor = world.spawn((EditorState::default(),));
        let entities = AppEntities {
            input,
            editor,
            gui_state: Entity::DANGLING,
            volume_windowing: Entity::DANGLING,
            annotations: Entity::DANGLING,
            overlay: Entity::DANGLING,
            protocol: Entity::DANGLING,
            cursor: Entity::DANGLING,
            window_settings: Entity::DANGLING,
        };

        assert!(sys_handle_input_scroll(&mut world, &entities, 1.0));
    }
}
