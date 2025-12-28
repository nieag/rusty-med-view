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
    for (_, view) in world.query::<&ViewState>().iter() {
        zoom_val = view.zoom[view_mode as usize];
    }

    Uniforms {
        resolution,
        time: time_val,
        view_mode,
        cursor_pos,
        zoom: zoom_val,
        _padding: [0.0; 3],
    }
}

pub fn sys_handle_input_scroll(world: &mut World, delta: f32) {
    // 1. Find out which viewport is active
    let mut mode = 0;
    let mut is_zoom = false;
    for (_, input) in world.query::<&InputState>().iter() {
        mode = input.active_viewport;
    }

    for (_, input) in world.query::<&InputState>().iter() {
        mode = input.active_viewport;
        // .state() is not needed on newer winit, depends on version.
        // For winit 0.29, modifiers is a struct with methods:
        is_zoom = input.modifiers.state().control_key();
    }

    if is_zoom {
        // --- ZOOM MODE ---
        for (_, view) in world.query::<&mut ViewState>().iter() {
            let idx = mode as usize;
            let sensitivity = 0.1;
            let scale_factor = 1.0 + (delta * sensitivity);

            if mode == 0 {
                // 3D View: Zooming IN (Scroll Up) -> Radius gets SMALLER
                view.zoom[idx] = (view.zoom[idx] / scale_factor).clamp(0.1, 20.0);
            } else {
                // 2D View: Zooming IN (Scroll Up) -> Scale gets LARGER
                view.zoom[idx] = (view.zoom[idx] * scale_factor).clamp(0.5, 50.0);
            }
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

    // Update the component
    for (_, input) in world.query::<&mut InputState>().iter() {
        input.last_mouse_pos = [x, y];
        input.active_viewport = viewport_idx;
    }
}
