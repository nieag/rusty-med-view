//! Core ECS systems for input handling, rendering data preparation, and interaction logic.
//!
//! This module contains the "systems" that operate on ECS components:
//! - Input handling (mouse, scroll, keyboard modifiers)
//! - Render data preparation (building `Uniforms` for the shader)
//! - 3D picking via CPU-side raymarching

use crate::components::*;
use glam::{Mat3, Quat, Vec3};
use hecs::World;
use winit::keyboard::ModifiersState;

/// Update keyboard modifier state (Ctrl/Shift/Alt) in the ECS.
pub fn sys_update_modifiers(world: &mut World, mods: ModifiersState) {
    for (_, input) in world.query::<&mut InputState>().iter() {
        input.modifiers = mods;
    }
}

/// Get the HU intensity value at the current mouse position.
/// Returns None if mouse is outside volume bounds or no volume is loaded.
pub fn get_hu_at_mouse(world: &World) -> Option<f32> {
    // Get current input state
    let mut viewport = 0u32;
    let mut mouse_uv = [0.5, 0.5];
    for (_, input) in world.query::<&InputState>().iter() {
        viewport = input.active_viewport;
        mouse_uv = input.mouse_uv;
    }

    // Get voxel position at mouse
    let voxel_pos = get_voxel_at_mouse(world, viewport, mouse_uv)?;

    // Sample from volume data
    for (_, vol) in world.query::<&VolumeData>().iter() {
        let [w, h, d] = vol.dimensions;
        let x = ((voxel_pos[0] * w as f32) as u32).min(w - 1);
        let y = ((voxel_pos[1] * h as f32) as u32).min(h - 1);
        let z = ((voxel_pos[2] * d as f32) as u32).min(d - 1);

        let idx = (z * h * w + y * w + x) as usize;
        if idx < vol.intensities.len() {
            return Some(vol.intensities[idx]);
        }
    }
    None
}

/// Prepare a `Uniforms` struct for the given viewport mode.
///
/// Gathers data from all relevant ECS components (window settings, camera,
/// cursor, view state, volume data, overlays) and packs them into a
/// shader-compatible uniform struct.
///
/// # Arguments
/// * `view_mode` - 0=3D, 1=Axial, 2=Coronal, 3=Sagittal
pub fn sys_prepare_render_data(world: &mut World, view_mode: u32) -> Uniforms {
    // 1. Get Window Settings and Viewport Rect
    let mut resolution = [800.0, 600.0];
    for (_, win) in world.query::<&WindowSettings>().iter() {
        // Use the actual render viewport dimensions for the shader
        resolution = [win.viewport_rect[2] / 2.0, win.viewport_rect[3] / 2.0];
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
    let mut rotation = [0.0, 0.0, 0.0, 1.0]; // quaternion [x, y, z, w]
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

    // 7. Get Windowing Info (HU-based)
    let mut window_params = [40.0, 400.0, -1000.0, 1000.0]; // default: soft tissue, typical HU range
                                                            // Get intensity range from volume data
    for (_, vol) in world.query::<&VolumeData>().iter() {
        window_params[2] = vol.intensity_range[0];
        window_params[3] = vol.intensity_range[1];
    }
    // Get window center/width from windowing component
    for (_, windowing) in world.query::<&VolumeWindowing>().iter() {
        window_params[0] = windowing.center;
        window_params[1] = windowing.width;
    }

    // Get brush preview settings
    // [brush_size_voxels, active, active_viewport, reserved]
    let mut brush_preview = [0.0f32; 4];
    let mut brush_active_viewport = 0u32;
    for (_, editor) in world.query::<&EditorState>().iter() {
        match editor.active_tool {
            EditorTool::Brush | EditorTool::Eraser => {
                brush_preview[0] = editor.brush_size; // size in voxels
                brush_preview[1] = 1.0; // active
            }
            _ => {}
        }
    }
    // Set active viewport (only show in 2D views)
    for (_, input) in world.query::<&InputState>().iter() {
        if input.active_viewport > 0 && brush_preview[1] > 0.0 {
            brush_preview[2] = input.active_viewport as f32;
            brush_active_viewport = input.active_viewport;
        }
    }

    // Compute brush center voxel using same function as paint
    let brush_center_voxel = if brush_preview[1] > 0.0 && brush_active_viewport > 0 {
        match get_voxel_at_mouse(world, brush_active_viewport, mouse_uv) {
            Some([x, y, z]) => [x, y, z, 1.0], // valid
            None => [0.0, 0.0, 0.0, 0.0],      // invalid
        }
    } else {
        [0.0, 0.0, 0.0, 0.0] // not active
    };

    Uniforms {
        cursor_pos,
        volume_dims,
        volume_spacing,
        overlay_opacities,
        window_params,
        resolution,
        mouse_uv,
        pan,
        zoom_pivot,
        rotation,                       // quaternion [x, y, z, w]
        overlay_mouse_uv: mouse_uv,     // Use current mouse position
        overlay_primitive_count: 0,     // Will be updated by render_frame after sync
        overlay_dragging_idx: u32::MAX, // MAX = no drag
        brush_preview,
        brush_center_voxel,
        zoom: zoom_val,
        time: time_val,
        view_mode,
        overlay_flags,
    }
}

/// Sync annotations to overlay primitives for GPU rendering.
///
/// This should be called each frame before rendering to update the overlay
/// primitive buffer with current annotation positions.
pub fn sys_sync_annotations_to_overlay(world: &mut World) {
    // Collect annotations first to avoid borrow conflicts
    let mut annotation_positions: Vec<Vec3> = Vec::new();

    for (_, ann_state) in world.query::<&AnnotationState>().iter() {
        for ann in &ann_state.annotations {
            annotation_positions.push(ann.world_pos);
        }
    }

    // Update overlay state with annotation primitives
    for (_, overlay) in world.query_mut::<&mut OverlayState>() {
        overlay.primitives.clear();

        for pos in &annotation_positions {
            // Create a circle primitive for each annotation
            // Visible in all viewports (mask = 15 = 1|2|4|8)
            overlay.primitives.push(OverlayPrimitive::circle(
                *pos,
                0.5,                  // radius in world units
                [1.0, 0.3, 0.3, 1.0], // Red-ish color
                15,                   // All viewports
            ));
        }
    }
}

/// Get the overlay state data for use by the render frame.
/// Returns (primitives as bytes, count, dragging_idx, mouse_uv)
pub fn get_overlay_render_data(world: &World) -> (Vec<u8>, u32, u32, [f32; 2]) {
    let mut primitives_bytes = Vec::new();
    let mut count = 0u32;
    let mut dragging_idx = u32::MAX;
    let mut mouse_uv = [0.5f32, 0.5];

    for (_, overlay) in world.query::<&OverlayState>().iter() {
        count = overlay.primitives.len() as u32;
        dragging_idx = overlay.dragging_idx.map(|i| i as u32).unwrap_or(u32::MAX);
        mouse_uv = overlay.mouse_screen_uv;

        // Serialize primitives to bytes
        primitives_bytes = bytemuck::cast_slice(&overlay.primitives).to_vec();
    }

    (primitives_bytes, count, dragging_idx, mouse_uv)
}

/// Handle scroll input for zooming or slice scrolling.
///
/// - **With Ctrl**: Zooms in/out centered on mouse position
/// - **Without Ctrl**: Scrolls the active slice (axial/coronal/sagittal)
pub fn sys_handle_input_scroll(world: &mut World, delta: f32) {
    let mut mouse_uv = [0.5, 0.5];
    let mut mode = 0;
    let mut is_zoom = false;
    let mut viewport_rect = [0.0, 0.0, 100.0, 100.0];

    // Collect input state first
    for (_, input) in world.query::<&InputState>().iter() {
        mode = input.active_viewport;
        mouse_uv = input.mouse_uv;
        is_zoom = input.modifiers.control_key();
    }
    // Collect window settings for aspect ratio
    for (_, win) in world.query::<&WindowSettings>().iter() {
        viewport_rect = win.viewport_rect;
    }
    // Collect Volume Aspect Ratios
    let mut vol_aspects = [1.0, 1.0, 1.0];
    for (_, vol) in world.query::<&VolumeData>().iter() {
        vol_aspects = vol.aspect_ratios();
    }

    if is_zoom {
        for (_, view) in world.query::<&mut ViewState>().iter() {
            let idx = mode as usize;

            // 2D View Aspect Correction Calculation
            let screen_w = viewport_rect[2];
            let screen_h = viewport_rect[3];
            let screen_aspect = if screen_h > 0.0 {
                screen_w / screen_h
            } else {
                1.0
            };

            // Correction Factor K
            // For 2D views, K = ScreenAspect / SliceAspect.
            // For 3D view (idx 0), we want K = 1.0 because aspect correction is handled
            // in the shader specifically (scaling screen position, not UVs).
            let k = if idx == 0 {
                1.0
            } else {
                // Determine Physical Aspect Ratio of the slice
                let slice_aspect = match idx {
                    1 => vol_aspects[0] / vol_aspects[1],
                    2 => vol_aspects[0] / vol_aspects[2],
                    3 => vol_aspects[1] / vol_aspects[2],
                    _ => 1.0,
                };
                screen_aspect / slice_aspect
            };

            // Compute Center-Referenced Mouse Position in "Corrected UV Space"
            // The conceptual "physical world" coordinate of the mouse relative to center
            let mx_centered = (mouse_uv[0] - 0.5) * k;
            let my_centered = mouse_uv[1] - 0.5;

            // Apply Zoom
            let sensitivity = 0.1;
            let scale_factor = 1.0 + (delta * sensitivity); // Zoom In > 1
            let old_zoom = view.zoom[idx];
            let new_zoom = (old_zoom * scale_factor).clamp(0.5, 100.0);

            // Update Pan to stabilize Mouse Position
            // Formula: Pan_new = Pan_old + Mouse_World * (1/Zoom_Old - 1/Zoom_New)
            // Where Mouse_World is the offset from center.
            view.pan[idx][0] += mx_centered * (1.0 / old_zoom - 1.0 / new_zoom);
            view.pan[idx][1] += my_centered * (1.0 / old_zoom - 1.0 / new_zoom);

            view.zoom[idx] = new_zoom;
            // Reset pivot to center (we don't use it anymore for storage)
            view.pivot[idx] = [0.5, 0.5];
        }
    } else {
        // --- SLICE MODE: Step by discrete voxels ---
        // Get volume dimensions to calculate voxel step size
        let mut dims = [1u32, 1, 1];
        for (_, vol) in world.query::<&VolumeData>().iter() {
            dims = vol.dimensions;
        }

        for (_, (transform, _tag)) in world.query::<(&mut Transform, &CursorTag)>().iter() {
            // Determine which axis to scroll based on viewport
            let (axis, dim) = match mode {
                1 => (2, dims[2]), // Axial: scroll Z
                2 => (1, dims[1]), // Coronal: scroll Y
                3 => (0, dims[0]), // Sagittal: scroll X
                _ => continue,
            };

            // Convert current UV position to voxel index
            let current_uv = transform.position[axis];
            let current_voxel = (current_uv * dim as f32).floor() as i32;

            // Step by one voxel in scroll direction
            let step = if delta > 0.0 { 1 } else { -1 };
            let new_voxel = (current_voxel + step).clamp(0, dim as i32 - 1);

            // Convert back to UV, snapped to voxel center
            // Voxel center = (voxel_index + 0.5) / dimension
            let new_uv = (new_voxel as f32 + 0.5) / dim as f32;
            transform.position[axis] = new_uv;
        }
    }
}

/// Update mouse position and calculate which viewport quadrant the mouse is in.
///
/// Converts screen coordinates to local UV coordinates within the active viewport.
pub fn sys_update_mouse(world: &mut World, x: f64, y: f64) {
    // We need window dimensions and viewport rect to calculate quadrants
    let mut viewport_rect = [0.0, 0.0, 1.0, 1.0];

    for (_, win) in world.query::<&WindowSettings>().iter() {
        viewport_rect = win.viewport_rect;
    }

    let vp_x = viewport_rect[0] as f64;
    let vp_y = viewport_rect[1] as f64;
    let vp_w = viewport_rect[2] as f64;
    let vp_h = viewport_rect[3] as f64;

    // Check if mouse is inside the render viewport
    if x < vp_x || x > (vp_x + vp_w) || y < vp_y || y > (vp_y + vp_h) {
        // Option: we could set active_viewport to something indicating "UI" or keep last valid
        // For now, let's just not update the mouse_uv so we don't jump wildly
        return;
    }

    // Relative mouse position within the viewport rect
    let rel_x = x - vp_x;
    let rel_y = y - vp_y;

    // Determine Quadrant based on relative position
    let col = if rel_x > vp_w / 2.0 { 1 } else { 0 };
    let row = if rel_y > vp_h / 2.0 { 1 } else { 0 };

    let viewport_idx = match (row, col) {
        (0, 0) => 0, // Top-Left
        (0, 1) => 1, // Top-Right
        (1, 0) => 2, // Bottom-Left
        (1, 1) => 3, // Bottom-Right
        _ => 0,
    };

    // Map back to local 0..1 UV coordinates within the quadrant
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

// --- Quaternion & Vector Math Helpers ---

// --- REPLACED WITH GLAM ---

use winit::event::{ElementState, MouseButton};

/// Calculate volume coordinate (0.0..1.0) under the mouse cursor.
/// Works for both 2D slices and 3D raymarching.
pub fn get_voxel_at_mouse(world: &World, viewport: u32, mouse_uv: [f32; 2]) -> Option<[f32; 3]> {
    // Shared view state
    let mut zoom = 1.0;
    let mut pan = [0.0, 0.0];
    let mut rotation = [0.0, 0.0, 0.0, 1.0];
    let mut viewport_rect = [0.0, 0.0, 100.0, 100.0];

    for (_, view) in world.query::<&ViewState>().iter() {
        let idx = viewport as usize;
        zoom = view.zoom[idx];
        pan = view.pan[idx];
        rotation = view.rotation[idx];
    }

    // Get window settings
    for (_, win) in world.query::<&WindowSettings>().iter() {
        viewport_rect = win.viewport_rect;
    }

    // Get volume info
    let mut vol_aspects = [1.0, 1.0, 1.0];
    let mut vol_dims = None;
    for (_, vol) in world.query::<&VolumeData>().iter() {
        vol_aspects = vol.aspect_ratios();
        vol_dims = Some(vol.dimensions);
    }

    // Current cursor position (for determining the slice in 2D)
    let mut cursor_pos = [0.0; 3];
    for (_, (t, _tag)) in world.query::<(&Transform, &CursorTag)>().iter() {
        cursor_pos = t.position;
    }

    if viewport > 0 {
        // --- 2D Slices ---

        // Calculate Correction Factor K
        let screen_w = viewport_rect[2];
        let screen_h = viewport_rect[3];
        let screen_aspect = if screen_h > 0.0 {
            screen_w / screen_h
        } else {
            1.0
        };

        let slice_aspect = match viewport {
            1 => vol_aspects[0] / vol_aspects[1],
            2 => vol_aspects[0] / vol_aspects[2],
            3 => vol_aspects[1] / vol_aspects[2],
            _ => 1.0,
        };
        let k = screen_aspect / slice_aspect;

        // Apply Aspect Corrected, Center-Based mapping
        // VolUV.x = ((MouseUV.x - 0.5) * K / Zoom) + 0.5 + Pan.x
        // VolUV.y = ((MouseUV.y - 0.5) / Zoom) + 0.5 + Pan.y
        let volume_uv = [
            ((mouse_uv[0] - 0.5) * k) / zoom + 0.5 + pan[0],
            (mouse_uv[1] - 0.5) / zoom + 0.5 + pan[1],
        ];

        let mut pos = [0.0; 3];
        match viewport {
            1 => {
                // Axial (xy)
                pos = [volume_uv[0], volume_uv[1], cursor_pos[2]];
            }
            2 => {
                // Coronal (xz)
                pos = [volume_uv[0], cursor_pos[1], volume_uv[1]];
            }
            3 => {
                // Sagittal (yz)
                pos = [cursor_pos[0], volume_uv[0], volume_uv[1]];
            }
            _ => return None,
        }
        // Clamp to volume bounds
        if pos[0] >= 0.0
            && pos[0] <= 1.0
            && pos[1] >= 0.0
            && pos[1] <= 1.0
            && pos[2] >= 0.0
            && pos[2] <= 1.0
        {
            return Some(pos);
        }
        return None;
    } else {
        // --- 3D View (Volume-Based Rotation) ---
        // Camera is FIXED, volume rotates - same as shader
        let mut aspect = 1.0;
        if viewport_rect[3] > 0.0 {
            aspect = viewport_rect[2] / viewport_rect[3];
        }

        if let Some([width, height, depth]) = vol_dims {
            let aspect_ratios = vol_aspects;
            let half_ar = [
                aspect_ratios[0] * 0.5,
                aspect_ratios[1] * 0.5,
                aspect_ratios[2] * 0.5,
            ];

            // Ray construction in world space
            // 3D View uses simple aspect ratio correction for screen->world mapping
            // But currently it's "zoomed_uv" based.
            // Let's adapt it to CENTER based too for consistency?
            // "zoomed_uv = (mouse_uv - pivot) / zoom + pivot + pan"
            // If Pivot is 0.5, then:
            // "zoomed_uv = (mouse_uv - 0.5) / zoom + 0.5 + pan"
            // This is equivalent to center-based without extra K (since aspect handled later in ray dir).

            let pivot = [0.5, 0.5]; // Assuming we reset pivot for all viewports, or at least consistency
            let zoomed_uv = [
                (mouse_uv[0] - pivot[0]) / zoom + pivot[0] + pan[0],
                (mouse_uv[1] - pivot[1]) / zoom + pivot[1] + pan[1],
            ];
            let uv = [zoomed_uv[0] - 0.5, zoomed_uv[1] - 0.5];
            let screen_pos = [uv[0] * aspect, uv[1]];

            // Fixed camera in world space
            let radius = 3.5;
            let cam_pos_world = [0.0, 0.0, -radius];
            let forward = [0.0, 0.0, 1.0];
            let right = [1.0, 0.0, 0.0];
            let up = [0.0, 1.0, 0.0];

            // Ray direction in world space
            let raw_dir = [
                forward[0] + right[0] * screen_pos[0] + up[0] * screen_pos[1],
                forward[1] + right[1] * screen_pos[0] + up[1] * screen_pos[1],
                forward[2] + right[2] * screen_pos[0] + up[2] * screen_pos[1],
            ];
            let ray_dir_world = Vec3::from(raw_dir).normalize();

            // Transform ray into object space using inverse of volume rotation
            let rot_quat = Quat::from_array(rotation);
            let rot_mat = Mat3::from_quat(rot_quat);
            // World -> Object = Inverse Rotation
            let inv_rot_mat = rot_mat.inverse();

            let cam_pos_obj = inv_rot_mat * Vec3::from(cam_pos_world);
            let ray_dir_obj = (inv_rot_mat * ray_dir_world).normalize();

            // AABB Intersection in object space
            let min_bound = Vec3::from([-half_ar[0], -half_ar[1], -half_ar[2]]);
            let max_bound = Vec3::from([half_ar[0], half_ar[1], half_ar[2]]);

            if let Some(t_entry) = intersect_aabb(
                cam_pos_obj.into(),
                ray_dir_obj.into(),
                min_bound.into(),
                max_bound.into(),
            ) {
                // Raymarching to find surface
                let mut t_exit = f32::INFINITY;
                for i in 0..3 {
                    if ray_dir_obj[i].abs() > f32::EPSILON {
                        let t1 = (min_bound[i] - cam_pos_obj[i]) / ray_dir_obj[i];
                        let t2 = (max_bound[i] - cam_pos_obj[i]) / ray_dir_obj[i];
                        t_exit = t_exit.min(t1.max(t2));
                    }
                }

                let mut best_t = t_entry;
                let mut max_density = 0u8;

                // Raymarch loop in object space
                for (_, vol) in world.query::<&VolumeData>().iter() {
                    let steps = 128;
                    let step_size = (t_exit - t_entry) / steps as f32;

                    for i in 0..steps {
                        let t = t_entry + step_size * i as f32;
                        let p = [
                            cam_pos_obj[0] + ray_dir_obj[0] * t,
                            cam_pos_obj[1] + ray_dir_obj[1] * t,
                            cam_pos_obj[2] + ray_dir_obj[2] * t,
                        ];

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
                            let idx = (iz as u32 * height * width + iy as u32 * width + ix as u32)
                                as usize;
                            let intensity = vol.intensities.get(idx).copied().unwrap_or(0.0);
                            let d = (intensity * 255.0) as u8;
                            if d > max_density {
                                max_density = d;
                                best_t = t;
                            }
                        }
                    }
                }

                // If we hit something strictly dense enough, return that point
                // Otherwise return entry point
                let final_t = if max_density > 20 { best_t } else { t_entry };
                let hit_point = [
                    cam_pos_obj[0] + ray_dir_obj[0] * final_t,
                    cam_pos_obj[1] + ray_dir_obj[1] * final_t,
                    cam_pos_obj[2] + ray_dir_obj[2] * final_t,
                ];

                return Some([
                    ((hit_point[0] / aspect_ratios[0]) + 0.5).clamp(0.0, 1.0),
                    ((hit_point[1] / aspect_ratios[1]) + 0.5).clamp(0.0, 1.0),
                    ((hit_point[2] / aspect_ratios[2]) + 0.5).clamp(0.0, 1.0),
                ]);
            }
        }
        None
    }
}

/// Handle mouse button events for clicking, dragging, and picking.
///
/// # Behaviors:
/// - **Left click**: Set crosshair position OR Paint
/// - **Middle click / Alt+Left**: Start panning drag
/// - **Right click**: Start rotation drag
/// - **Release**: End any drag operation
pub fn sys_handle_mouse_button(world: &mut World, button: MouseButton, state: ElementState) {
    let mut click_pos = [0.0, 0.0];
    let mut viewport = 0;
    let mut alt_pressed = false;

    // --- Editor Mode Logic ---
    let mut is_editor_tool = false;
    for (_, editor) in world.query::<&EditorState>().iter() {
        if editor.active_tool != EditorTool::Navigation {
            is_editor_tool = true;
        }
    }

    for (_, input) in world.query::<&mut InputState>().iter() {
        viewport = input.active_viewport;
        click_pos = input.mouse_uv;
        alt_pressed = input.modifiers.alt_key();

        let is_left_click = button == MouseButton::Left && !alt_pressed;

        // If in paint mode, Left Click starts PAINTING, not moving cursor directly (handled in sys_paint)
        // Only Navigation tool allows 'setting cursor' via click
        // Pan/Rotate logic remains unless overridden

        // --- DRAG DETECTION ---
        // Start panning: Middle Click OR (Alt + Left)
        // If Paint Tool active -> Middle Click ONLY (Alt+Left reserved? Maybe just Middle)
        let drag_trigger = if is_editor_tool {
            button == MouseButton::Middle
        } else {
            button == MouseButton::Middle || (button == MouseButton::Left && alt_pressed)
        };

        // Start rotating: Right Click
        let rotate_trigger = button == MouseButton::Right;

        // Paint Trigger: Left Click (no Alt) AND Editor Tool is Brush/Eraser
        let paint_trigger = is_editor_tool && is_left_click;

        if paint_trigger && state == ElementState::Pressed {
            // "Dragging" effectively means "Painting" here
            input.is_dragging = true;
            // Don't set drag_start_pan/pos for painting, handled in sys_paint
        } else if drag_trigger && state == ElementState::Pressed {
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

            // Clear stroke interpolation state on release
            for (_, editor) in world.query::<&mut EditorState>().iter() {
                editor.last_paint_voxel = None;
            }
        }
    }

    // --- CROSSHAIR CLICK (Navigation Tool Only) ---
    if !is_editor_tool
        && button == MouseButton::Left
        && !alt_pressed
        && state == ElementState::Pressed
    {
        if let Some(target_pos) = get_voxel_at_mouse(world, viewport, click_pos) {
            for (_, (t, _tag)) in world.query::<(&mut Transform, &CursorTag)>().iter() {
                t.position = target_pos;
            }
        }
    }
}

/// Handle mouse drag motion for panning and rotating.
pub fn sys_handle_mouse_drag(world: &mut World) {
    let mut viewport = 0;
    let mut is_dragging = false;
    let mut is_rotating = false;
    let mut viewport_rect = [0.0, 0.0, 100.0, 100.0];

    // Check if we are painting
    let mut is_editor_tool = false;
    for (_, editor) in world.query::<&EditorState>().iter() {
        if editor.active_tool != EditorTool::Navigation {
            is_editor_tool = true;
        }
    }

    for (_, input) in world.query::<&InputState>().iter() {
        viewport = input.active_viewport;
        is_dragging = input.is_dragging;
        is_rotating = input.is_rotating;
    }

    // Collect window settings
    for (_, win) in world.query::<&WindowSettings>().iter() {
        viewport_rect = win.viewport_rect;
    }

    // Collect Volume Aspect Ratios
    let mut vol_aspects = [1.0, 1.0, 1.0];
    for (_, vol) in world.query::<&VolumeData>().iter() {
        vol_aspects = vol.aspect_ratios();
    }

    if !is_dragging && !is_rotating {
        return;
    }

    for (_, view) in world.query::<&mut ViewState>().iter() {
        let mut drag_info = None;
        let mut rotate_info = None;

        for (_, input) in world.query::<&InputState>().iter() {
            if is_dragging && !is_editor_tool {
                // Normal Pan
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
            let idx = viewport as usize;

            // Calculate K (Aspect Correction)
            // Reuse logic from scroll (could extract to helper, but robust enough to copy for now)
            let mut k = 1.0;
            if idx > 0 {
                let screen_w = viewport_rect[2];
                let screen_h = viewport_rect[3];
                let screen_aspect = if screen_h > 0.0 {
                    screen_w / screen_h
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

            // Apply Pan with Correction
            // Delta X in UV space * K / Zoom
            view.pan[idx][0] = start_pan[0] + ((start_pos[0] - current_pos[0]) * k) / zoom;
            view.pan[idx][1] = start_pan[1] + (start_pos[1] - current_pos[1]) / zoom;
        }

        if let Some((start_quat, start_pos, current_pos)) = rotate_info {
            let sensitivity = 3.0; // Radians per screen unit

            // Check for Shift modifier
            let mut has_shift = false;
            for (_, input) in world.query::<&InputState>().iter() {
                has_shift = input.modifiers.shift_key();
            }

            // Calculate deltas
            let delta_x = (current_pos[0] - start_pos[0]) * sensitivity;
            let delta_y = (current_pos[1] - start_pos[1]) * sensitivity;

            let start_q = Quat::from_array(start_quat);

            let new_quat = if has_shift {
                // Planar rotation: Roll around Z-axis (forward)
                let roll_quat = Quat::from_axis_angle(Vec3::NEG_Z, delta_x);
                (roll_quat * start_q).normalize()
            } else {
                // Orbital rotation: Yaw around Y (world up), Pitch around X (world right)
                let yaw_quat = Quat::from_axis_angle(Vec3::Y, delta_x);
                let pitch_quat = Quat::from_axis_angle(Vec3::X, -delta_y);
                (yaw_quat * pitch_quat * start_q).normalize()
            };

            view.rotation[viewport as usize] = new_quat.to_array();
        }
    }
}

/// Paint System: Updates labelmap based on mouse stroke
pub fn sys_paint(world: &mut World, queue: &wgpu::Queue) {
    let mut interactions = Vec::new();

    // 1. Collect Input and Editor State
    for (_, input) in world.query::<&InputState>().iter() {
        if !input.is_dragging {
            continue;
        }
        for (_, editor) in world.query::<&mut EditorState>().iter() {
            if editor.active_tool == EditorTool::Navigation {
                continue;
            }

            // We are dragging in paint mode
            interactions.push((
                input.active_viewport,
                input.mouse_uv,
                editor.active_tool,
                editor.brush_size,
                editor.active_label_index,
                editor.active_layer,
                editor.last_paint_voxel,
            ));
        }
    }

    if interactions.is_empty() {
        return;
    }

    // 2. Perform Painting
    for (viewport, mouse_uv, tool, brush_size, label_idx, layer_entity, last_voxel_opt) in
        interactions
    {
        // Get target layer
        let layer_ent = if let Some(e) = layer_entity {
            e
        } else {
            // No active layer selected
            continue;
        };

        // Calculate current voxel target
        let cur_target_opt = get_voxel_at_mouse(world, viewport, mouse_uv);

        if let Some(pos) = cur_target_opt {
            if let Ok(mut query) =
                world.query_one::<(&mut LabelmapData, &Representation)>(layer_ent)
            {
                if let Some((label_data, repr)) = query.get() {
                    let texture = if let Representation::Voxel(res) = repr {
                        &res.texture
                    } else {
                        continue;
                    };

                    let dim = label_data.dimensions;
                    let width = dim[0];
                    let height = dim[1];
                    let depth = dim[2];

                    // Coordinate conversion to integer voxel indices
                    let cx = (pos[0] * width as f32) as i32;
                    let cy = (pos[1] * height as f32) as i32;
                    let cz = (pos[2] * depth as f32) as i32;
                    let current_voxel = [cx, cy, cz];

                    // Skip painting in 3D viewport (viewport 0) - only allow 2D slice painting
                    if viewport == 0 {
                        continue;
                    }

                    // Define points to paint (interpolation in 2D plane only)
                    let mut points_to_paint = Vec::new();

                    if let Some(last_voxel) = last_voxel_opt {
                        let lx = last_voxel[0] as i32;
                        let ly = last_voxel[1] as i32;
                        let lz = last_voxel[2] as i32;

                        // 2D interpolation based on viewport
                        // For Axial (1): interpolate in xy, z is fixed
                        // For Coronal (2): interpolate in xz, y is fixed
                        // For Sagittal (3): interpolate in yz, x is fixed
                        let dist = match viewport {
                            1 => (((cx - lx).pow(2) + (cy - ly).pow(2)) as f32).sqrt(), // Axial: xy
                            2 => (((cx - lx).pow(2) + (cz - lz).pow(2)) as f32).sqrt(), // Coronal: xz
                            3 => (((cy - ly).pow(2) + (cz - lz).pow(2)) as f32).sqrt(), // Sagittal: yz
                            _ => 0.0,
                        };
                        let steps = dist.ceil() as i32 + 1;

                        for i in 0..=steps {
                            let t = i as f32 / steps as f32;
                            // Interpolate only in the 2D plane, keep slice coord fixed
                            let (ix, iy, iz) = match viewport {
                                1 => {
                                    // Axial: interpolate x,y; z stays at current slice
                                    (
                                        (lx as f32 + (cx - lx) as f32 * t) as i32,
                                        (ly as f32 + (cy - ly) as f32 * t) as i32,
                                        cz,
                                    )
                                }
                                2 => {
                                    // Coronal: interpolate x,z; y stays at current slice
                                    (
                                        (lx as f32 + (cx - lx) as f32 * t) as i32,
                                        cy,
                                        (lz as f32 + (cz - lz) as f32 * t) as i32,
                                    )
                                }
                                3 => {
                                    // Sagittal: interpolate y,z; x stays at current slice
                                    (
                                        cx,
                                        (ly as f32 + (cy - ly) as f32 * t) as i32,
                                        (lz as f32 + (cz - lz) as f32 * t) as i32,
                                    )
                                }
                                _ => (cx, cy, cz),
                            };
                            points_to_paint.push([ix, iy, iz]);
                        }
                    } else {
                        points_to_paint.push(current_voxel);
                    }

                    // Apply Paint to CPU Buffer
                    let val_to_write = if tool == EditorTool::Eraser {
                        0
                    } else {
                        label_idx
                    };

                    let r = brush_size as i32;
                    let r2 = (brush_size * brush_size) as i32;

                    let mut min_bound = [width as i32, height as i32, depth as i32];
                    let mut max_bound = [-1, -1, -1];
                    let mut modified = false;

                    for pt in points_to_paint {
                        let px = pt[0];
                        let py = pt[1];
                        let pz = pt[2];

                        // 2D brush: only iterate in the 2D plane for this viewport
                        match viewport {
                            1 => {
                                // Axial (xy plane): fixed z
                                let start_x = (px - r).max(0);
                                let end_x = (px + r).min(width as i32 - 1);
                                let start_y = (py - r).max(0);
                                let end_y = (py + r).min(height as i32 - 1);
                                let z = pz;

                                if z >= 0 && z < depth as i32 {
                                    for y in start_y..=end_y {
                                        for x in start_x..=end_x {
                                            let dx = x - px;
                                            let dy = y - py;
                                            if dx * dx + dy * dy <= r2 {
                                                let idx = (z as u32 * width * height
                                                    + y as u32 * width
                                                    + x as u32)
                                                    as usize;
                                                if label_data.raw_data[idx] != val_to_write {
                                                    label_data.raw_data[idx] = val_to_write;
                                                    modified = true;
                                                    min_bound[0] = min_bound[0].min(x);
                                                    min_bound[1] = min_bound[1].min(y);
                                                    min_bound[2] = min_bound[2].min(z);
                                                    max_bound[0] = max_bound[0].max(x);
                                                    max_bound[1] = max_bound[1].max(y);
                                                    max_bound[2] = max_bound[2].max(z);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            2 => {
                                // Coronal (xz plane): fixed y
                                let start_x = (px - r).max(0);
                                let end_x = (px + r).min(width as i32 - 1);
                                let start_z = (pz - r).max(0);
                                let end_z = (pz + r).min(depth as i32 - 1);
                                let y = py;

                                if y >= 0 && y < height as i32 {
                                    for z in start_z..=end_z {
                                        for x in start_x..=end_x {
                                            let dx = x - px;
                                            let dz = z - pz;
                                            if dx * dx + dz * dz <= r2 {
                                                let idx = (z as u32 * width * height
                                                    + y as u32 * width
                                                    + x as u32)
                                                    as usize;
                                                if label_data.raw_data[idx] != val_to_write {
                                                    label_data.raw_data[idx] = val_to_write;
                                                    modified = true;
                                                    min_bound[0] = min_bound[0].min(x);
                                                    min_bound[1] = min_bound[1].min(y);
                                                    min_bound[2] = min_bound[2].min(z);
                                                    max_bound[0] = max_bound[0].max(x);
                                                    max_bound[1] = max_bound[1].max(y);
                                                    max_bound[2] = max_bound[2].max(z);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            3 => {
                                // Sagittal (yz plane): fixed x
                                let start_y = (py - r).max(0);
                                let end_y = (py + r).min(height as i32 - 1);
                                let start_z = (pz - r).max(0);
                                let end_z = (pz + r).min(depth as i32 - 1);
                                let x = px;

                                if x >= 0 && x < width as i32 {
                                    for z in start_z..=end_z {
                                        for y in start_y..=end_y {
                                            let dy = y - py;
                                            let dz = z - pz;
                                            if dy * dy + dz * dz <= r2 {
                                                let idx = (z as u32 * width * height
                                                    + y as u32 * width
                                                    + x as u32)
                                                    as usize;
                                                if label_data.raw_data[idx] != val_to_write {
                                                    label_data.raw_data[idx] = val_to_write;
                                                    modified = true;
                                                    min_bound[0] = min_bound[0].min(x);
                                                    min_bound[1] = min_bound[1].min(y);
                                                    min_bound[2] = min_bound[2].min(z);
                                                    max_bound[0] = max_bound[0].max(x);
                                                    max_bound[1] = max_bound[1].max(y);
                                                    max_bound[2] = max_bound[2].max(z);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }

                    // GPU Upload (Partial)
                    if modified {
                        let w = (max_bound[0] - min_bound[0] + 1) as u32;
                        let h = (max_bound[1] - min_bound[1] + 1) as u32;
                        let d = (max_bound[2] - min_bound[2] + 1) as u32;

                        // Extract sub-volume
                        let mut sub_data = Vec::with_capacity((w * h * d) as usize);
                        for z in min_bound[2]..=max_bound[2] {
                            for y in min_bound[1]..=max_bound[1] {
                                let start = (z as u32 * width * height
                                    + y as u32 * width
                                    + min_bound[0] as u32)
                                    as usize;
                                let end = start + w as usize;
                                sub_data.extend_from_slice(&label_data.raw_data[start..end]);
                            }
                        }

                        queue.write_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: &texture,
                                mip_level: 0,
                                origin: wgpu::Origin3d {
                                    x: min_bound[0] as u32,
                                    y: min_bound[1] as u32,
                                    z: min_bound[2] as u32,
                                },
                                aspect: wgpu::TextureAspect::All,
                            },
                            &sub_data,
                            wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(w),
                                rows_per_image: Some(h),
                            },
                            wgpu::Extent3d {
                                width: w,
                                height: h,
                                depth_or_array_layers: d,
                            },
                        );
                    }
                }
            } // Update EditorState last_voxel for next frame
              // We need to re-query active EditorState again to write back, or use RefCell, or component update
              // HECS query iter gives us a reference. We need mutable access which we have in the loop.
              // But we need to update the Component in the World.
              // We are inside a loop over the query.
        }

        // Updating last_voxel
        // Since we collected 'interactions' (copies of data) to avoid borrow issues,
        // we now need to write back the new position.
        if let Some(_pos) = cur_target_opt {
            // Dummy, we just need to cast pos to int
            // Actually we need dimensions to cast to int correctly.
            // But we did it above inside the block.
            // Simpler: Just update it.
            // We'll update it for ALL editors (usually just one).
            for (_, editor) in world.query::<&mut EditorState>().iter() {
                // We need to fetch dimensions again or guess?
                // Let's assume default rounding since we don't have dims here easily without re-query.
                // Actually this is just for next frame interpolation which is approximate.
                // But wait, interpolation needs VOXEL coords, not UV.
                // We CANNOT convert to voxel coords without determining which volume/layer we edited.
                // We did that inside the block.

                // To fix this cleanly: We should do the paint logic *inside* the system query loop if possible,
                // OR store the computed voxel coord in `interactions` too? No, can't compute it before loop.

                // Re-query layers here?
                // Let's just grab the Main Volume dimensions as a fallback if Layer is missing?
                // Or just do a separate pass to update state.
                // Best: Pass mutable `editor` to the logic if we didn't decouple it.
                // Decoupling caused this.
                // Let's just perform the update inside the "interactions" loop if we can?
                // No, "interactions" is a Vec of copies.

                // Let's re-query EditorState and update.
                // We need to know the dimensions to convert cur_target_opt (0..1) to Voxel.
                // We can get dimensions from VolumeData (Main) as a proxy if we assume they match.
                // Or query the layer again.
                if let Some(target) = cur_target_opt {
                    // Attempt to solve dimensions
                    let mut dims = [100, 100, 100]; // Fallback
                    for (_, vol) in world.query::<&VolumeData>().iter() {
                        dims = vol.dimensions; // Use main volume dims
                    }
                    // Correct: Uses active layer dims priority?
                    // The paint logic used layer dims.
                    // If layer dims != main dims, this will be slightly off, but acceptable for next frame start point?
                    // Ideally we use exact.

                    let vx = (target[0] * dims[0] as f32) as u32;
                    let vy = (target[1] * dims[1] as f32) as u32;
                    let vz = (target[2] * dims[2] as f32) as u32;
                    editor.last_paint_voxel = Some([vx, vy, vz]);
                }
            }
        }
    }
}

// --- Windowing Helper ---

/// Apply windowing transform to a normalized intensity value.
///
/// Mirrors the shader windowing logic for CPU-side testing.
/// Maps intensities from [center - width/2, center + width/2] to [0, 1].
///
/// # Arguments
/// * `value` - Input intensity (normalized 0-1)
/// * `center` - Window center (L)
/// * `width` - Window width (W)
///
/// # Returns
/// Windowed intensity clamped to [0, 1]
pub fn apply_window(value: f32, center: f32, width: f32) -> f32 {
    if width <= 0.0 {
        return 0.5; // Guard against invalid width
    }
    let low = center - width * 0.5;
    ((value - low) / width).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_window_center() {
        // Value at center should map to 0.5
        assert!((apply_window(0.5, 0.5, 1.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_apply_window_edges() {
        // Value at low edge → 0, high edge → 1
        assert!((apply_window(0.0, 0.5, 1.0) - 0.0).abs() < 1e-6);
        assert!((apply_window(1.0, 0.5, 1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_apply_window_clamp() {
        // Values outside range should clamp
        assert_eq!(apply_window(-0.5, 0.5, 1.0), 0.0);
        assert_eq!(apply_window(1.5, 0.5, 1.0), 1.0);
    }

    #[test]
    fn test_apply_window_narrow() {
        // Narrow window (high contrast)
        assert!((apply_window(0.5, 0.5, 0.1) - 0.5).abs() < 1e-6);
        assert_eq!(apply_window(0.4, 0.5, 0.1), 0.0); // Outside narrow window
    }

    #[test]
    fn test_apply_window_zero_width() {
        // Zero width should return 0.5 (safe fallback)
        assert_eq!(apply_window(0.5, 0.5, 0.0), 0.5);
    }
}
