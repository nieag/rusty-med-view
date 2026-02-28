// src/systems/input.rs
use crate::components::*;
use crate::convert::slice_index_from_cursor_uv;
use crate::systems::picking::get_voxel_at_mouse;
use crate::systems::segment_system::{
    add_drawing_point, cancel_drawing, finish_drawing, is_drawing, start_drawing, SegmentManager,
};
use crate::util::orientation::SlicePlane;
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
    let mut active_vp = None;
    let mut alt_pressed = false;

    let mut is_editor_tool = false;
    if let Ok(editor) = world.get::<&EditorState>(entities.editor) {
        if editor.active_tool != EditorTool::Navigation {
            is_editor_tool = true;
        }
    }

    if let Ok(mut input) = world.get::<&mut InputState>(entities.input) {
        if input.egui_wants_input {
            return;
        }
        active_vp = input.active_viewport;
        alt_pressed = input.modifiers.alt_key();

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
pub fn sys_handle_input_scroll(world: &mut World, entities: &AppEntities, delta: f32) {
    let mut active_vp = None;
    let mut mouse_uv = [0.5, 0.5];
    let mut is_zoom = false;

    if let Ok(input) = world.get::<&InputState>(entities.input) {
        if input.egui_wants_input {
            return;
        }
        active_vp = input.active_viewport;
        mouse_uv = input.mouse_uv;
        is_zoom = input.modifiers.control_key();
    }

    let avp = match active_vp {
        Some(e) => e,
        None => return,
    };

    let mut vol_aspects = [1.0, 1.0, 1.0];
    let mut dims = [1u32, 1, 1];
    for (_, vol) in world.query::<&VolumeData>().iter() {
        vol_aspects = vol.aspect_ratios();
        dims = vol.dimensions;
    }

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
        }
    } else if let Ok(mut transform) = world.get::<&mut Transform>(entities.cursor) {
        if let Ok(vp) = world.get::<&Viewport>(avp) {
            let (axis, dim) = match vp.mode {
                ViewMode::Axial => (2, dims[2]),
                ViewMode::Coronal => (1, dims[1]),
                ViewMode::Sagittal => (0, dims[0]),
                ViewMode::Oblique => (2, dims[2]),
                _ => return,
            };
            if dim == 0 {
                return;
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
                    transform.position[axis] = (new_voxel as f32 + 0.5) / dim as f32;
                }
            }
        }
    }
}

/// Handle mouse drag motion for panning and rotating.
pub fn sys_handle_mouse_drag(world: &mut World, entities: &AppEntities) {
    let mut active_vp = None;
    let mut is_dragging = false;
    let mut is_rotating = false;
    let mut is_panning = false;

    let mut is_editor_tool = false;
    if let Ok(editor) = world.get::<&EditorState>(entities.editor) {
        if editor.active_tool != EditorTool::Navigation {
            is_editor_tool = true;
        }
    }

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
            if is_dragging && !is_editor_tool && !is_panning && !is_rotating {
                crosshair_update = Some((avp, input.mouse_uv));
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
}

// ============================================================================
// Contour Drawing Input Handlers
// ============================================================================

/// Handle mouse button for contour drawing tool.
/// Call this from sys_handle_mouse_button when ContourDraw tool is active.
pub fn sys_handle_contour_mouse_button(
    world: &mut World,
    entities: &AppEntities,
    button: MouseButton,
    state: ElementState,
) {
    // Check if ContourDraw tool is active
    let is_contour_tool = if let Ok(editor) = world.get::<&EditorState>(entities.editor) {
        editor.active_tool == EditorTool::ContourDraw
    } else {
        false
    };

    if !is_contour_tool {
        return;
    }

    // Get viewport and mouse info
    let (active_vp, mouse_uv, egui_wants) =
        if let Ok(input) = world.get::<&InputState>(entities.input) {
            (
                input.active_viewport,
                input.mouse_uv,
                input.egui_wants_input,
            )
        } else {
            return;
        };

    if egui_wants {
        return;
    }

    let avp = match active_vp {
        Some(e) => e,
        None => return,
    };

    // Only handle left button in 2D views
    if button != MouseButton::Left {
        return;
    }

    // Get viewport mode, volume info, and viewport state for projection (Fix zoom/pan offset)
    let (view_mode, slice_index, compensated_uv) = {
        let (vp, vs) = match (
            world.get::<&Viewport>(avp),
            world.get::<&ViewportState>(avp),
        ) {
            (Ok(vp), Ok(vs)) => (vp, vs),
            _ => return,
        };

        // Only work in 2D views
        if vp.mode == ViewMode::ThreeD {
            return;
        }

        let cursor_pos = if let Ok(t) = world.get::<&Transform>(entities.cursor) {
            t.position
        } else {
            [0.5, 0.5, 0.5]
        };

        let (vol_aspects, dims) = {
            let mut d = [1u32, 1, 1];
            let mut a = [1.0f32, 1.0, 1.0];
            for (_, vol) in world.query::<&VolumeData>().iter() {
                d = vol.dimensions;
                a = vol.aspect_ratios();
            }
            (a, d)
        };

        // Aspect and zoom/pan compensation math (mirrors picking.rs)
        let screen_aspect = if vp.rect[3] > 0.0 {
            vp.rect[2] / vp.rect[3]
        } else {
            1.0
        };
        let slice_plane = match vp.mode {
            ViewMode::Axial => SlicePlane::Axial,
            ViewMode::Coronal => SlicePlane::Coronal,
            ViewMode::Sagittal => SlicePlane::Sagittal,
            ViewMode::Oblique => return,
            ViewMode::ThreeD => return,
        };
        let slice_aspect = slice_plane.slice_aspect(vol_aspects);
        let k = screen_aspect / slice_aspect;

        let comp_uv = [
            ((mouse_uv[0] - 0.5) * k) / vs.zoom + 0.5 + vs.pan[0],
            (mouse_uv[1] - 0.5) / vs.zoom + 0.5 + vs.pan[1],
        ];

        let slice_idx = match vp.mode {
            ViewMode::Axial => slice_index_from_cursor_uv(cursor_pos[2], dims[2]),
            ViewMode::Coronal => slice_index_from_cursor_uv(cursor_pos[1], dims[1]),
            ViewMode::Sagittal => slice_index_from_cursor_uv(cursor_pos[0], dims[0]),
            ViewMode::Oblique => return,
            ViewMode::ThreeD => return,
        };

        (slice_plane, slice_idx, comp_uv)
    };

    if state == ElementState::Pressed {
        // Start drawing
        if let Ok(mut mgr) = world.get::<&mut SegmentManager>(entities.segments) {
            start_drawing(&mut mgr, view_mode, slice_index, compensated_uv);
        }
    } else {
        // ... (rest of the function remains the same)
        // Finish drawing
        let (dims, spacing) = {
            let mut d = [1u32, 1, 1];
            let mut s = [1.0f32, 1.0, 1.0];
            for (_, vol) in world.query::<&VolumeData>().iter() {
                d = vol.dimensions;
                s = vol.spacing;
            }
            (d, s)
        };

        if let Ok(mut mgr) = world.get::<&mut SegmentManager>(entities.segments) {
            finish_drawing(&mut mgr, dims, spacing);
        }
    }
}

/// Handle mouse drag for contour drawing tool.
/// Call this from sys_handle_mouse_drag when ContourDraw tool is active.
pub fn sys_handle_contour_mouse_drag(world: &mut World, entities: &AppEntities) {
    // Check if ContourDraw tool is active and drawing
    let is_contour_tool = if let Ok(editor) = world.get::<&EditorState>(entities.editor) {
        editor.active_tool == EditorTool::ContourDraw
    } else {
        false
    };

    if !is_contour_tool {
        return;
    }

    // Check if we're actively drawing
    let currently_drawing = if let Ok(mgr) = world.get::<&SegmentManager>(entities.segments) {
        is_drawing(&mgr)
    } else {
        false
    };

    if !currently_drawing {
        return;
    }

    // Get current mouse position and viewport state for compensation
    let compensated_uv = if let Ok(input) = world.get::<&InputState>(entities.input) {
        if !input.is_dragging {
            return;
        }

        // Compute compensation if possible
        if let Some(avp) = input.active_viewport {
            if let (Ok(vp), Ok(vs)) = (
                world.get::<&Viewport>(avp),
                world.get::<&ViewportState>(avp),
            ) {
                let vol_aspects = {
                    let mut a = [1.0f32, 1.0, 1.0];
                    for (_, vol) in world.query::<&VolumeData>().iter() {
                        a = vol.aspect_ratios();
                    }
                    a
                };
                let screen_aspect = if vp.rect[3] > 0.0 {
                    vp.rect[2] / vp.rect[3]
                } else {
                    1.0
                };
                let slice_plane = SlicePlane::from_mode(vp.mode);

                if let Some(plane) = slice_plane {
                    let slice_aspect = plane.slice_aspect(vol_aspects);
                    let k = screen_aspect / slice_aspect;
                    [
                        ((input.mouse_uv[0] - 0.5) * k) / vs.zoom + 0.5 + vs.pan[0],
                        (input.mouse_uv[1] - 0.5) / vs.zoom + 0.5 + vs.pan[1],
                    ]
                } else {
                    input.mouse_uv
                }
            } else {
                input.mouse_uv
            }
        } else {
            input.mouse_uv
        }
    } else {
        return;
    };

    // Add point
    if let Ok(mut mgr) = world.get::<&mut SegmentManager>(entities.segments) {
        add_drawing_point(&mut mgr, compensated_uv);
    }
}

/// Cancel current contour drawing (e.g., on Escape key).
pub fn sys_cancel_contour_drawing(world: &mut World, entities: &AppEntities) {
    if let Ok(mut mgr) = world.get::<&mut SegmentManager>(entities.segments) {
        cancel_drawing(&mut mgr);
    }
}
