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

fn set_status_message(session: &mut Session, message: String) {
    session.gui.status_message = Some(message);
}

/// Update keyboard modifier state (Ctrl/Shift/Alt) in the ECS.
pub fn sys_update_modifiers(session: &mut Session, mods: ModifiersState) {
    session.input.modifiers = mods;
}

pub fn sys_update_mouse(world: &mut World, session: &mut Session, x: f64, y: f64) {
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

    {
        let input = &mut session.input;
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
    session: &mut Session,
    button: MouseButton,
    state: ElementState,
) {
    crate::systems::clear_contour_draft_if_inactive(&mut session.editor);
    crate::systems::clear_contour_selection_if_inactive(world, &mut session.editor);

    let active_vp = session.input.active_viewport;
    let alt_pressed = session.input.modifiers.alt_key();
    let mut finalize_contour_move = false;
    let mut finalize_mesh_move = false;

    {
        let input = &mut session.input;

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
        let _ = crate::systems::finalize_selected_point_move(world, session);
    }
    if finalize_mesh_move {
        match crate::app::roi::commit_mesh_edit_preview(world, &session.editor) {
            Ok(()) => set_status_message(
                session,
                "Committed mesh deformation; build voxels explicitly when needed.".to_string(),
            ),
            Err(crate::app::roi::MeshMutationError::InvalidMesh(error)) => set_status_message(
                session,
                format!("Deformation rejected; previous mesh kept. Try a shorter drag or larger brush. {error:?}."),
            ),
            Err(error) => set_status_message(
                session,
                format!("Mesh deformation commit failed: {error:?}."),
            ),
        }
    }

    let ctrl_pressed = session.input.modifiers.control_key();
    let super_pressed = session.input.modifiers.super_key();

    if button == MouseButton::Left
        && !alt_pressed
        && !ctrl_pressed
        && !super_pressed
        && state == ElementState::Pressed
    {
        let active_tool = {
            let editor = &session.editor;
            editor.active_tool
        };
        if active_tool == EditorTool::ContourDraw {
            let click_pos = {
                let input = &session.input;
                input.mouse_uv
            };
            match crate::systems::handle_contour_draw_click(world, session, click_pos) {
                Ok(crate::systems::ContourDrawClickOutcome::PointAdded) => {}
                Ok(crate::systems::ContourDrawClickOutcome::LoopCommitted) => {
                    set_status_message(session, "Contour loop committed.".to_string());
                }
                Err(crate::systems::ContourDrawClickError::LoopNeedsThreePoints) => {
                    set_status_message(
                        session,
                        "Need at least 3 points to close a contour loop.".to_string(),
                    );
                }
                Err(crate::systems::ContourDrawClickError::MissingActiveRoi) => {
                    set_status_message(session, "Select a contour ROI before drawing.".to_string());
                }
                Err(crate::systems::ContourDrawClickError::ActiveRoiNotContour) => {
                    set_status_message(session, "Active ROI is not contour-primary.".to_string());
                }
                Err(crate::systems::ContourDrawClickError::Mapping(error)) => {
                    set_status_message(session, format!("Contour draw blocked: {error:?}"));
                }
                Err(crate::systems::ContourDrawClickError::SwitchPending) => {
                    set_status_message(
                        session,
                        "Preparing the ROI for contour editing...".to_string(),
                    );
                }
                Err(crate::systems::ContourDrawClickError::Switch(error)) => {
                    set_status_message(session, error.message());
                }
                Err(_) => {}
            }
            return;
        }
        if active_tool == EditorTool::ContourSelect {
            let click_pos = {
                let input = &session.input;
                input.mouse_uv
            };
            match crate::systems::handle_contour_select_click(world, session, click_pos) {
                Ok(Some(selection)) => {
                    if selection.point_index.is_some() {
                        set_status_message(session, "Selected contour point.".to_string());
                    } else {
                        set_status_message(session, "Selected contour loop.".to_string());
                    }
                }
                Ok(None) => {}
                Err(crate::systems::ContourSelectClickError::MissingActiveRoi) => {
                    set_status_message(
                        session,
                        "Select a contour ROI before selecting contours.".to_string(),
                    );
                }
                Err(crate::systems::ContourSelectClickError::ActiveRoiNotContour) => {
                    set_status_message(session, "Active ROI is not contour-primary.".to_string());
                }
                Err(crate::systems::ContourSelectClickError::Mapping(error)) => {
                    set_status_message(session, format!("Contour selection blocked: {error:?}"));
                }
                Err(crate::systems::ContourSelectClickError::SwitchPending) => {
                    set_status_message(
                        session,
                        "Preparing the ROI for contour editing...".to_string(),
                    );
                }
                Err(crate::systems::ContourSelectClickError::Switch(error)) => {
                    set_status_message(session, error.message());
                }
                Err(_) => {}
            }
            return;
        }
        if active_tool == EditorTool::MeshDeform {
            let click_pos = {
                let input = &session.input;
                input.mouse_uv
            };
            match crate::systems::select_mesh_vertex(world, session, click_pos) {
                Ok(Some(_)) => set_status_message(session, "Selected mesh surface.".to_string()),
                Ok(None) => {}
                Err(crate::systems::MeshEditInteractionError::SwitchPending) => {
                    set_status_message(
                        session,
                        "Preparing the ROI for mesh editing...".to_string(),
                    );
                }
                Err(crate::systems::MeshEditInteractionError::Switch(error)) => {
                    set_status_message(session, error.message());
                }
                Err(error) => {
                    set_status_message(session, format!("Mesh selection blocked: {error:?}."))
                }
            }
            return;
        }

        if let Some(avp) = active_vp {
            let click_pos = session.input.mouse_uv;
            if let Some(target_pos) = get_voxel_at_mouse(world, session, avp, click_pos) {
                {
                    let t = &mut session.cursor;
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
pub fn sys_handle_input_scroll(world: &mut World, session: &mut Session, delta: f32) -> bool {
    if session.input.egui_wants_input {
        return false;
    }
    let active_vp = session.input.active_viewport;
    let mouse_uv = session.input.mouse_uv;
    let is_zoom = session.input.modifiers.control_key();

    let avp = match active_vp {
        Some(e) => e,
        None => return false,
    };

    let mut dims = [1u32, 1, 1];
    for (_, vol) in world.query::<&VolumeData>().iter() {
        dims = vol.dimensions;
    }
    let geometry = crate::app::roi_runtime::main_volume_geometry(world);
    let cursor_uv = session.cursor.position;

    let mut changed_3d_zoom = false;
    if is_zoom {
        if let (Ok(vp), Ok(mut vs)) = (
            world.get::<&Viewport>(avp),
            world.get::<&mut ViewportState>(avp),
        ) {
            let k = viewport_aspect_factor(vp.mode, vp.rect, vs.user_rotation, cursor_uv, geometry);

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
    } else {
        let transform = &mut session.cursor;
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

            {
                let input = &mut session.input;
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

/// `k` of the shared viewport mapping: the screen's aspect over the displayed plane's aspect, the
/// factor that turns a horizontal screen offset into a volume offset. 1 for the 3D view.
fn viewport_aspect_factor(
    mode: ViewMode,
    rect: [f32; 4],
    user_rotation: [f32; 4],
    cursor_uv: [f32; 3],
    geometry: Option<crate::model::VoxelGeometry>,
) -> f32 {
    if mode == ViewMode::ThreeD {
        return 1.0;
    }
    let screen_aspect = if rect[3] > 0.0 {
        rect[2] / rect[3]
    } else {
        1.0
    };
    let slice_aspect = geometry
        .and_then(|geometry| {
            let plane = crate::render::roi_views::displayed_plane_for_viewport(
                mode,
                cursor_uv,
                user_rotation,
                geometry,
            )?;
            crate::convert::plane_display_aspect(plane, geometry)
        })
        .unwrap_or(1.0);
    screen_aspect / slice_aspect
}

/// Handle mouse drag motion for panning and rotating.
pub fn sys_handle_mouse_drag(world: &mut World, session: &mut Session) {
    let input = &session.input;
    if input.egui_wants_input && !input.is_dragging && !input.is_rotating && !input.is_panning {
        return;
    }
    let active_vp = input.active_viewport;
    let is_dragging = input.is_dragging;
    let is_rotating = input.is_rotating;
    let is_panning = input.is_panning;

    if (!is_dragging && !is_rotating && !is_panning) || active_vp.is_none() {
        return;
    }
    let Some(avp) = active_vp else {
        return;
    };

    let geometry = crate::app::roi_runtime::main_volume_geometry(world);
    let cursor_uv = session.cursor.position;

    let mut crosshair_update = None;
    let mut contour_move_update = None;
    let mut mesh_move_update = None;

    if let (Ok(vp), Ok(mut vs)) = (
        world.get::<&Viewport>(avp),
        world.get::<&mut ViewportState>(avp),
    ) {
        let mut drag_info = None;
        let mut rotate_info = None;

        {
            let input = &session.input;
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
            let active_tool = {
                let editor = &session.editor;
                editor.active_tool
            };
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
            let k = viewport_aspect_factor(vp.mode, vp.rect, vs.user_rotation, cursor_uv, geometry);
            vs.pan[0] = start_pan[0] + ((start_pos[0] - current_pos[0]) * k) / zoom;
            vs.pan[1] = start_pan[1] + (start_pos[1] - current_pos[1]) / zoom;
        }

        if let Some((start_quat, start_pos, current_pos)) = rotate_info {
            let sensitivity = 3.0;
            let has_shift = session.input.modifiers.shift_key();
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
        if let Some(target_pos) = get_voxel_at_mouse(world, session, avp, uv) {
            {
                let t = &mut session.cursor;
                t.position = target_pos;
            }
        }
    }

    if let Some(uv) = contour_move_update {
        if crate::systems::move_selected_point_preview(world, session, uv).is_ok() {
            {
                let input = &mut session.input;
                input.contour_move_pending_commit = true;
            }
        }
    }
    if let Some(uv) = mesh_move_update {
        if crate::systems::update_selected_mesh_deform_preview(world, session, uv).is_ok() {
            {
                let input = &mut session.input;
                input.mesh_move_pending_commit = true;
            }
        }
    }
}

#[cfg(test)]
mod tests;
