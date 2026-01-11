// src/systems/input.rs
use crate::components::*;
use crate::systems::picking::get_voxel_at_mouse;
use glam::{Quat, Vec3};
use hecs::World;
use winit::event::{ElementState, MouseButton};
use winit::keyboard::ModifiersState;

/// Update keyboard modifier state (Ctrl/Shift/Alt) in the ECS.
pub fn sys_update_modifiers(world: &mut World, entities: &AppEntities, mods: ModifiersState) {
    if let Ok(mut input) = world.get::<&mut InputState>(entities.input) {
        input.modifiers = mods;
    }
}

/// Update mouse position and calculate which viewport quadrant the mouse is in.
pub fn sys_update_mouse(world: &mut World, entities: &AppEntities, x: f64, y: f64) {
    let mut viewport_rect = [0.0, 0.0, 1.0, 1.0];
    if let Ok(win) = world.get::<&WindowSettings>(entities.window_settings) {
        viewport_rect = win.viewport_rect;
    }

    let vp_x = viewport_rect[0] as f64;
    let vp_y = viewport_rect[1] as f64;
    let vp_w = viewport_rect[2] as f64;
    let vp_h = viewport_rect[3] as f64;

    if x < vp_x || x > (vp_x + vp_w) || y < vp_y || y > (vp_y + vp_h) {
        return;
    }

    let rel_x = x - vp_x;
    let rel_y = y - vp_y;

    let col = if rel_x > vp_w / 2.0 { 1 } else { 0 };
    let row = if rel_y > vp_h / 2.0 { 1 } else { 0 };

    let viewport_idx = match (row, col) {
        (0, 0) => 0,
        (0, 1) => 1,
        (1, 0) => 2,
        (1, 1) => 3,
        _ => 0,
    };

    let local_x = if col == 1 {
        (rel_x - vp_w / 2.0) / (vp_w / 2.0)
    } else {
        rel_x / (vp_w / 2.0)
    };
    let local_y = if row == 1 {
        (rel_y - vp_h / 2.0) / (vp_h / 2.0)
    } else {
        rel_y / (vp_h / 2.0)
    };

    if let Ok(mut input) = world.get::<&mut InputState>(entities.input) {
        input.last_mouse_pos = [x, y];
        input.mouse_uv = [local_x as f32, local_y as f32];
        input.active_viewport = viewport_idx;
    }
}

/// Handle mouse button events for clicking, dragging, and picking.
pub fn sys_handle_mouse_button(
    world: &mut World,
    entities: &AppEntities,
    button: MouseButton,
    state: ElementState,
) {
    let mut click_pos = [0.0, 0.0];
    let mut viewport = 0;
    let mut alt_pressed = false;

    let mut is_editor_tool = false;
    if let Ok(editor) = world.get::<&EditorState>(entities.editor) {
        if editor.active_tool != EditorTool::Navigation {
            is_editor_tool = true;
        }
    }

    if let Ok(mut input) = world.get::<&mut InputState>(entities.input) {
        click_pos = input.mouse_uv;
        viewport = input.active_viewport;
        alt_pressed = input.modifiers.alt_key();

        if state == ElementState::Pressed {
            input.is_dragging = true;
            input.drag_start_pos = input.mouse_uv;

            let ctrl_pressed = input.modifiers.control_key();
            let super_pressed = input.modifiers.super_key();

            if button == MouseButton::Middle
                || (button == MouseButton::Left && (ctrl_pressed || super_pressed))
            {
                input.is_panning = true;
                if let Ok(view) = world.get::<&ViewState>(entities.view) {
                    input.drag_start_pan = view.pan[viewport as usize];
                }
            }

            if button == MouseButton::Right || (button == MouseButton::Left && alt_pressed) {
                input.is_rotating = true;
                input.rotation_start_pos = input.mouse_uv;
                if let Ok(view) = world.get::<&ViewState>(entities.view) {
                    input.rotation_start_val = view.rotation[viewport as usize];
                }
            }
        } else if state == ElementState::Released {
            input.is_dragging = false;
            input.is_panning = false;
            input.is_rotating = false;
            if let Ok(mut editor) = world.get::<&mut EditorState>(entities.editor) {
                editor.last_paint_voxel = None;
            }
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

    if !is_editor_tool
        && button == MouseButton::Left
        && !alt_pressed
        && !ctrl_pressed
        && !super_pressed
        && state == ElementState::Pressed
    {
        if let Some(target_pos) = get_voxel_at_mouse(world, entities, viewport, click_pos) {
            if let Ok(mut t) = world.get::<&mut Transform>(entities.cursor) {
                t.position = target_pos;
            }
        }
    }
}

/// Handle scroll input for zooming or slice scrolling.
pub fn sys_handle_input_scroll(world: &mut World, entities: &AppEntities, delta: f32) {
    let mut mouse_uv = [0.5, 0.5];
    let mut mode = 0;
    let mut is_zoom = false;
    let mut viewport_rect = [0.0, 0.0, 100.0, 100.0];

    if let Ok(input) = world.get::<&InputState>(entities.input) {
        mode = input.active_viewport;
        mouse_uv = input.mouse_uv;
        is_zoom = input.modifiers.control_key();
    }
    if let Ok(win) = world.get::<&WindowSettings>(entities.window_settings) {
        viewport_rect = win.viewport_rect;
    }
    let mut vol_aspects = [1.0, 1.0, 1.0];
    let mut dims = [1u32, 1, 1];
    // Main volume is harder to singleton-ify because it's data dependent,
    // but the system already queries for it. Let's find it.
    for (_, vol) in world.query::<&VolumeData>().iter() {
        vol_aspects = vol.aspect_ratios();
        dims = vol.dimensions;
    }

    if is_zoom {
        if let Ok(mut view) = world.get::<&mut ViewState>(entities.view) {
            let idx = mode as usize;
            let screen_aspect = if viewport_rect[3] > 0.0 {
                viewport_rect[2] / viewport_rect[3]
            } else {
                1.0
            };
            let k = if idx == 0 {
                1.0
            } else {
                let slice_aspect = match idx {
                    1 => vol_aspects[0] / vol_aspects[1],
                    2 => vol_aspects[0] / vol_aspects[2],
                    3 => vol_aspects[1] / vol_aspects[2],
                    _ => 1.0,
                };
                screen_aspect / slice_aspect
            };

            let mx_centered = (mouse_uv[0] - 0.5) * k;
            let my_centered = mouse_uv[1] - 0.5;

            let sensitivity = 0.1;
            let scale_factor = 1.0 + (delta * sensitivity);
            let old_zoom = view.zoom[idx];
            let new_zoom = (old_zoom * scale_factor).clamp(0.5, 100.0);

            view.pan[idx][0] += mx_centered * (1.0 / old_zoom - 1.0 / new_zoom);
            view.pan[idx][1] += my_centered * (1.0 / old_zoom - 1.0 / new_zoom);
            view.zoom[idx] = new_zoom;
            view.pivot[idx] = [0.5, 0.5];
        }
    } else {
        if let Ok(mut transform) = world.get::<&mut Transform>(entities.cursor) {
            let (axis, dim) = match mode {
                1 => (2, dims[2]),
                2 => (1, dims[1]),
                3 => (0, dims[0]),
                _ => return,
            };
            if dim == 0 {
                return;
            }
            let current_uv = transform.position[axis];
            let current_voxel = (current_uv * dim as f32).floor() as i32;
            let step = if delta > 0.0 { 1 } else { -1 };
            let new_voxel = (current_voxel + step).clamp(0, dim as i32 - 1);
            transform.position[axis] = (new_voxel as f32 + 0.5) / dim as f32;
        }
    }
}

/// Handle mouse drag motion for panning and rotating.
pub fn sys_handle_mouse_drag(world: &mut World, entities: &AppEntities) {
    let mut viewport = 0;
    let mut is_dragging = false;
    let mut is_rotating = false;
    let mut is_panning = false;
    let mut viewport_rect = [0.0, 0.0, 100.0, 100.0];

    let mut is_editor_tool = false;
    if let Ok(editor) = world.get::<&EditorState>(entities.editor) {
        if editor.active_tool != EditorTool::Navigation {
            is_editor_tool = true;
        }
    }

    if let Ok(input) = world.get::<&InputState>(entities.input) {
        viewport = input.active_viewport;
        is_dragging = input.is_dragging;
        is_rotating = input.is_rotating;
        is_panning = input.is_panning;
    }

    if !is_dragging && !is_rotating && !is_panning {
        return;
    }

    if let Ok(win) = world.get::<&WindowSettings>(entities.window_settings) {
        viewport_rect = win.viewport_rect;
    }

    let mut vol_aspects = [1.0, 1.0, 1.0];
    for (_, vol) in world.query::<&VolumeData>().iter() {
        vol_aspects = vol.aspect_ratios();
    }

    let mut crosshair_update = None;

    if let Ok(mut view) = world.get::<&mut ViewState>(entities.view) {
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
            if is_dragging && !is_editor_tool && !is_panning && !is_rotating {
                crosshair_update = Some((viewport, input.mouse_uv));
            }
        }

        if let Some((start_pan, start_pos, current_pos)) = drag_info {
            let idx = viewport as usize;
            let zoom = view.zoom[idx];
            let mut k = 1.0;
            if idx > 0 {
                let screen_aspect = if viewport_rect[3] > 0.0 {
                    viewport_rect[2] / viewport_rect[3]
                } else {
                    1.0
                };
                let slice_aspect = match idx {
                    1 => vol_aspects[0] / vol_aspects[1],
                    2 => vol_aspects[0] / vol_aspects[2],
                    3 => vol_aspects[1] / vol_aspects[2],
                    _ => 1.0,
                };
                k = screen_aspect / slice_aspect;
            }
            view.pan[idx][0] = start_pan[0] + ((start_pos[0] - current_pos[0]) * k) / zoom;
            view.pan[idx][1] = start_pan[1] + (start_pos[1] - current_pos[1]) / zoom;
        }

        if let Some((start_quat, start_pos, current_pos)) = rotate_info {
            let sensitivity = 3.0;
            let mut has_shift = false;
            if let Ok(input) = world.get::<&InputState>(entities.input) {
                has_shift = input.modifiers.shift_key();
            }
            let delta_x = (current_pos[0] - start_pos[0]) * sensitivity;
            let delta_y = (current_pos[1] - start_pos[1]) * sensitivity;
            let start_q = Quat::from_array(start_quat);

            let new_quat = if has_shift {
                let roll_quat = Quat::from_axis_angle(Vec3::NEG_Z, delta_x);
                (roll_quat * start_q).normalize()
            } else {
                let yaw_quat = Quat::from_axis_angle(Vec3::Y, delta_x);
                let pitch_quat = Quat::from_axis_angle(Vec3::X, -delta_y);
                (yaw_quat * pitch_quat * start_q).normalize()
            };
            view.rotation[viewport as usize] = new_quat.to_array();
        }
    }

    // Now update crosshair outside ViewState borrow
    if let Some((vp, uv)) = crosshair_update {
        if let Some(target_pos) = get_voxel_at_mouse(world, entities, vp, uv) {
            if let Ok(mut t) = world.get::<&mut Transform>(entities.cursor) {
                t.position = target_pos;
            }
        }
    }
}
