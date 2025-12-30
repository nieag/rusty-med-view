use crate::components::*;
use hecs::World; // Import our structs
use winit::keyboard::ModifiersState;

// NEW: Helper to update modifier state (Ctrl/Shift/Alt)
pub fn sys_update_modifiers(world: &mut World, mods: ModifiersState) {
    for (_, input) in world.query::<&mut InputState>().iter() {
        input.modifiers = mods;
    }
}
pub fn sys_prepare_render_data(world: &mut World, view_mode: u32) -> Uniforms {
    // 1. Get Window Settings
    let mut resolution = [800.0, 600.0];
    for (_, win) in world.query::<&WindowSettings>().iter() {
        resolution = [win.width as f32 / 2.0, win.height as f32 / 2.0];
    }

    // 2. Get Camera Time
    let mut time_val = 0.0;
    for (_, rig) in world.query::<&CameraRig>().iter() {
        time_val = rig.start_time.elapsed().as_secs_f32() * rig.speed;
    }

    // 3. Get Cursor
    let mut cursor_pos = [0.0; 4];
    for (_, (t, _tag)) in world.query::<(&Transform, &CursorTag)>().iter() {
        cursor_pos[0] = t.position[0];
        cursor_pos[1] = t.position[1];
        cursor_pos[2] = t.position[2];
    }

    let mut zoom_val = 1.0;
    let mut pan = [0.0, 0.0];
    let mut zoom_pivot = [0.5, 0.5];
    let mut rotation = [0.0, 0.0, 0.0];
    for (_, view) in world.query::<&ViewState>().iter() {
        zoom_val = view.zoom[view_mode as usize];
        pan = view.pan[view_mode as usize];
        zoom_pivot = view.pivot[view_mode as usize];
        rotation = view.rotation[view_mode as usize];
    }

    // 4. Get Mouse UV
    let mut mouse_uv = [0.5, 0.5];
    for (_, inp) in world.query::<&InputState>().iter() {
        mouse_uv = inp.mouse_uv;
    }

    // 5. Get Volume Info
    let mut volume_dims = [0u32; 4];
    let mut volume_spacing = [0.0f32; 4];
    for (_, vol) in world.query::<&VolumeData>().iter() {
        volume_dims = [vol.dimensions[0], vol.dimensions[1], vol.dimensions[2], 0];
        volume_spacing = [vol.spacing[0], vol.spacing[1], vol.spacing[2], 0.0];
    }

    // 6. Get Overlay Info
    let mut overlay_flags = 0u32;
    let mut overlay_opacities = [0.0f32; 4];
    let mut layer_count = 0;

    for (_, (seg, settings)) in world.query::<(&Segmentation, &LayerSettings)>().iter() {
        if !seg.is_visible {
            continue;
        }
        if layer_count >= 4 {
            break;
        }

        overlay_flags |= 1 << layer_count;
        overlay_opacities[layer_count] = settings.opacity;
        layer_count += 1;
    }

    Uniforms {
        cursor_pos,
        volume_dims,
        volume_spacing,
        overlay_opacities,
        resolution,
        mouse_uv,
        pan,
        zoom_pivot,
        rotation: [rotation[0], rotation[1], rotation[2], 0.0],
        zoom: zoom_val,
        time: time_val,
        view_mode,
        overlay_flags,
    }
}

pub fn sys_handle_input_scroll(world: &mut World, delta: f32) {
    let mut mouse_uv = [0.5, 0.5];
    let mut mode = 0;
    let mut is_zoom = false;

    // Collect input state first to avoid borrow-check issues
    for (_, input) in world.query::<&InputState>().iter() {
        mode = input.active_viewport;
        mouse_uv = input.mouse_uv;
        is_zoom = input.modifiers.control_key();
    }

    if is_zoom {
        for (_, view) in world.query::<&mut ViewState>().iter() {
            let idx = mode as usize;
            let sensitivity = 0.1;
            let scale_factor = 1.0 + (delta * sensitivity);

            let old_zoom = view.zoom[idx];
            let new_zoom = (old_zoom * scale_factor).clamp(0.5, 50.0);

            let old_pivot = view.pivot[idx];
            let new_pivot = mouse_uv;

            // Adjust pan to transition to the new pivot position
            // Formula: Pan' = Pan + (P' - P) * (1/Zoom - 1)
            view.pan[idx][0] += (new_pivot[0] - old_pivot[0]) * (1.0 / old_zoom - 1.0);
            view.pan[idx][1] += (new_pivot[1] - old_pivot[1]) * (1.0 / old_zoom - 1.0);

            view.pivot[idx] = new_pivot;
            view.zoom[idx] = new_zoom;
        }
    } else {
        // --- SLICE MODE (Existing Logic) ---
        for (_, (transform, _tag)) in world.query::<(&mut Transform, &CursorTag)>().iter() {
            let speed = 0.05;
            let change = delta * speed;
            if mode == 1 {
                transform.position[2] = (transform.position[2] + change).clamp(0.0, 1.0);
            } else if mode == 2 {
                transform.position[1] = (transform.position[1] + change).clamp(0.0, 1.0);
            } else if mode == 3 {
                transform.position[0] = (transform.position[0] + change).clamp(0.0, 1.0);
            }
        }
    }
}

// Helper to update mouse position and calculate active viewport
pub fn sys_update_mouse(world: &mut World, x: f64, y: f64) {
    // We need window dimensions to calculate quadrants
    let mut width = 1.0;
    let mut height = 1.0;

    for (_, win) in world.query::<&WindowSettings>().iter() {
        width = win.width as f64;
        height = win.height as f64;
    }

    // Determine Quadrant (Winit 0,0 is Top-Left)
    // TL: 3D (0)   | TR: XY (1)
    // -------------+-----------
    // BL: XZ (2)   | BR: YZ (3)

    let col = if x > width / 2.0 { 1 } else { 0 };
    let row = if y > height / 2.0 { 1 } else { 0 };

    let viewport_idx = match (row, col) {
        (0, 0) => 0, // Top-Left
        (0, 1) => 1, // Top-Right
        (1, 0) => 2, // Bottom-Left
        (1, 1) => 3, // Bottom-Right
        _ => 0,
    };

    // Map back to local 0..1 UV coordinates within the quadrant
    let local_x = if col == 1 {
        (x - width / 2.0) / (width / 2.0)
    } else {
        x / (width / 2.0)
    };

    let local_y = if row == 1 {
        (y - height / 2.0) / (height / 2.0)
    } else {
        y / (height / 2.0)
    };

    // Update the component
    for (_, input) in world.query::<&mut InputState>().iter() {
        input.last_mouse_pos = [x, y];
        input.mouse_uv = [local_x as f32, local_y as f32];
        input.active_viewport = viewport_idx;
    }
}

// Ray-Box Intersection for 3D picking
fn intersect_aabb(origin: [f32; 3], dir: [f32; 3], min: [f32; 3], max: [f32; 3]) -> Option<f32> {
    let mut t_min = f32::NEG_INFINITY;
    let mut t_max = f32::INFINITY;

    for i in 0..3 {
        if dir[i].abs() < f32::EPSILON {
            if origin[i] < min[i] || origin[i] > max[i] {
                return None;
            }
        } else {
            let t1 = (min[i] - origin[i]) / dir[i];
            let t2 = (max[i] - origin[i]) / dir[i];
            let (tm1, tm2) = if t1 < t2 { (t1, t2) } else { (t2, t1) };
            t_min = t_min.max(tm1);
            t_max = t_max.min(tm2);
        }
    }

    if t_min <= t_max && t_max >= 0.0 {
        Some(t_min.max(0.0))
    } else {
        None
    }
}

use winit::event::{ElementState, MouseButton};

pub fn sys_handle_mouse_button(world: &mut World, button: MouseButton, state: ElementState) {
    let mut click_pos = [0.0, 0.0];
    let mut viewport = 0;
    let mut alt_pressed = false;

    for (_, input) in world.query::<&mut InputState>().iter() {
        viewport = input.active_viewport;
        click_pos = input.mouse_uv;
        alt_pressed = input.modifiers.alt_key();

        // --- DRAG DETECTION ---
        // Start dragging on Middle Click or Alt + Left Click
        let drag_trigger =
            button == MouseButton::Middle || (button == MouseButton::Left && alt_pressed);

        // Start rotating on Right Click
        let rotate_trigger = button == MouseButton::Right;

        if drag_trigger && state == ElementState::Pressed {
            input.is_dragging = true;
            input.drag_start_pos = input.mouse_uv;
            // Capture current pan
            for (_, view) in world.query::<&ViewState>().iter() {
                input.drag_start_pan = view.pan[viewport as usize];
            }
        } else if rotate_trigger && state == ElementState::Pressed {
            input.is_rotating = true;
            input.rotation_start_pos = input.mouse_uv;
            // Capture current rotation
            for (_, view) in world.query::<&ViewState>().iter() {
                input.rotation_start_val = view.rotation[viewport as usize];
            }
        } else if state == ElementState::Released {
            input.is_dragging = false;
            input.is_rotating = false;
        }
    }

    // --- CROSSHAIR CLICK (Left click without Alt) ---
    if button == MouseButton::Left && !alt_pressed && state == ElementState::Pressed {
        if viewport > 0 {
            // 2D View Logic (Existing)
            let mut zoom = 1.0;
            let mut pan = [0.0, 0.0];
            let mut pivot = [0.5, 0.5];
            for (_, view) in world.query::<&ViewState>().iter() {
                zoom = view.zoom[viewport as usize];
                pan = view.pan[viewport as usize];
                pivot = view.pivot[viewport as usize];
            }

            let volume_uv = [
                (click_pos[0] - pivot[0]) / zoom + pivot[0] + pan[0],
                (click_pos[1] - pivot[1]) / zoom + pivot[1] + pan[1],
            ];

            for (_, (t, _tag)) in world.query::<(&mut Transform, &CursorTag)>().iter() {
                if viewport == 1 {
                    // XY
                    t.position[0] = volume_uv[0].clamp(0.0, 1.0);
                    t.position[1] = volume_uv[1].clamp(0.0, 1.0);
                } else if viewport == 2 {
                    // XZ
                    t.position[0] = volume_uv[0].clamp(0.0, 1.0);
                    t.position[2] = volume_uv[1].clamp(0.0, 1.0);
                } else if viewport == 3 {
                    // YZ
                    t.position[1] = volume_uv[0].clamp(0.0, 1.0);
                    t.position[2] = volume_uv[1].clamp(0.0, 1.0);
                }
            }
        } else {
            // --- 3D VIEW PICKING ---
            let mut zoom = 1.0;
            let mut pan = [0.0, 0.0];
            let mut pivot = [0.5, 0.5];
            for (_, view) in world.query::<&ViewState>().iter() {
                zoom = view.zoom[0];
                pan = view.pan[0];
                pivot = view.pivot[0];
            }

            let mut aspect = 1.0;
            for (_, win) in world.query::<&WindowSettings>().iter() {
                aspect = (win.width as f32) / (win.height as f32);
            }

            let mut vol_info = None;
            for (_, vol) in world.query::<&VolumeData>().iter() {
                vol_info = Some((vol.dimensions, vol.aspect_ratios()));
            }

            if let Some(([width, height, depth], aspect_ratios)) = vol_info {
                let half_ar = [
                    aspect_ratios[0] * 0.5,
                    aspect_ratios[1] * 0.5,
                    aspect_ratios[2] * 0.5,
                ];

                // Ray construction with zoom/pan
                let zoomed_uv = [
                    (click_pos[0] - pivot[0]) / zoom + pivot[0] + pan[0],
                    (click_pos[1] - pivot[1]) / zoom + pivot[1] + pan[1],
                ];

                let uv = [zoomed_uv[0] - 0.5, zoomed_uv[1] - 0.5];
                let screen_pos = [uv[0] * aspect, uv[1]];

                let radius = 3.5;
                let eye = [0.0, 0.0, -radius];
                let forward = [0.0, 0.0, 1.0];
                let right = [1.0, 0.0, 0.0];
                let up = [0.0, 1.0, 0.0];

                let raw_dir = [
                    forward[0] + right[0] * screen_pos[0] + up[0] * screen_pos[1],
                    forward[1] + right[1] * screen_pos[0] + up[1] * screen_pos[1],
                    forward[2] + right[2] * screen_pos[0] + up[2] * screen_pos[1],
                ];
                let mag =
                    (raw_dir[0] * raw_dir[0] + raw_dir[1] * raw_dir[1] + raw_dir[2] * raw_dir[2])
                        .sqrt();
                let ray_dir = [raw_dir[0] / mag, raw_dir[1] / mag, raw_dir[2] / mag];

                let min_bound = [-half_ar[0], -half_ar[1], -half_ar[2]];
                let max_bound = [half_ar[0], half_ar[1], half_ar[2]];

                if let Some(t_entry) = intersect_aabb(eye, ray_dir, min_bound, max_bound) {
                    let mut t_exit = f32::INFINITY;
                    for i in 0..3 {
                        if ray_dir[i].abs() > f32::EPSILON {
                            let t1 = (min_bound[i] - eye[i]) / ray_dir[i];
                            let t2 = (max_bound[i] - eye[i]) / ray_dir[i];
                            t_exit = t_exit.min(t1.max(t2));
                        }
                    }

                    let mut best_t = t_entry;
                    let mut max_density = 0u8;

                    for (_, vol) in world.query::<&VolumeData>().iter() {
                        let steps = 128;
                        let step_size = (t_exit - t_entry) / steps as f32;

                        for i in 0..steps {
                            let t = t_entry + step_size * i as f32;
                            let p = [
                                eye[0] + ray_dir[0] * t,
                                eye[1] + ray_dir[1] * t,
                                eye[2] + ray_dir[2] * t,
                            ];

                            // Map P back to 0..1 range within the aspect-ratio-scaled box
                            let uvw = [
                                (p[0] / aspect_ratios[0]) + 0.5,
                                (p[1] / aspect_ratios[1]) + 0.5,
                                (p[2] / aspect_ratios[2]) + 0.5,
                            ];

                            let ix = (uvw[0] * width as f32) as i32;
                            let iy = (uvw[1] * height as f32) as i32;
                            let iz = (uvw[2] * depth as f32) as i32;

                            if ix >= 0
                                && ix < width as i32
                                && iy >= 0
                                && iy < height as i32
                                && iz >= 0
                                && iz < depth as i32
                            {
                                let idx = (iz as u32 * height * width
                                    + iy as u32 * width
                                    + ix as u32) as usize;
                                let intensity = vol.intensities.get(idx).copied().unwrap_or(0.0);
                                let d = (intensity * 255.0) as u8;
                                if d > max_density {
                                    max_density = d;
                                    best_t = t;
                                }
                            }
                        }
                    }

                    let final_t = if max_density > 20 { best_t } else { t_entry };
                    let hit_point = [
                        eye[0] + ray_dir[0] * final_t,
                        eye[1] + ray_dir[1] * final_t,
                        eye[2] + ray_dir[2] * final_t,
                    ];

                    for (_, (t, _tag)) in world.query::<(&mut Transform, &CursorTag)>().iter() {
                        t.position[0] = ((hit_point[0] / aspect_ratios[0]) + 0.5).clamp(0.0, 1.0);
                        t.position[1] = ((hit_point[1] / aspect_ratios[1]) + 0.5).clamp(0.0, 1.0);
                        t.position[2] = ((hit_point[2] / aspect_ratios[2]) + 0.5).clamp(0.0, 1.0);
                    }
                }
            }
        }
    }
}

pub fn sys_handle_mouse_drag(world: &mut World) {
    let mut viewport = 0;
    let mut is_dragging = false;
    let mut is_rotating = false;

    for (_, input) in world.query::<&InputState>().iter() {
        viewport = input.active_viewport;
        is_dragging = input.is_dragging;
        is_rotating = input.is_rotating;
    }

    if !is_dragging && !is_rotating {
        return;
    }

    for (_, view) in world.query::<&mut ViewState>().iter() {
        let mut drag_info = None;
        let mut rotate_info = None;

        for (_, input) in world.query::<&InputState>().iter() {
            if is_dragging {
                drag_info = Some((input.drag_start_pan, input.drag_start_pos, input.mouse_uv));
            }
            if is_rotating {
                rotate_info = Some((
                    input.rotation_start_val,
                    input.rotation_start_pos,
                    input.mouse_uv,
                ));
            }
        }

        if let Some((start_pan, start_pos, current_pos)) = drag_info {
            let zoom = view.zoom[viewport as usize];
            view.pan[viewport as usize] = [
                start_pan[0] + (start_pos[0] - current_pos[0]) / zoom,
                start_pan[1] + (start_pos[1] - current_pos[1]) / zoom,
            ];
        }

        if let Some((start_rot, start_pos, current_pos)) = rotate_info {
            let sensitivity = 3.0; // Radians per screen unit
            let mut current_rotation = start_rot;

            // Check for Shift modifier
            let mut has_shift = false;
            for (_, input) in world.query::<&InputState>().iter() {
                has_shift = input.modifiers.shift_key();
            }

            if has_shift {
                // Planar rotation: Roll
                current_rotation[2] = start_rot[2] + (current_pos[0] - start_pos[0]) * sensitivity;
            } else {
                // Orbital rotation: Yaw and Pitch
                current_rotation[0] = start_rot[0] + (current_pos[0] - start_pos[0]) * sensitivity;
                current_rotation[1] =
                    (start_rot[1] + (current_pos[1] - start_pos[1]) * sensitivity).clamp(-1.5, 1.5);
                // Clamp pitch to avoid gimbal lock at poles
            }
            view.rotation[viewport as usize] = current_rotation;
        }
    }
}
