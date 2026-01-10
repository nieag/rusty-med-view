// src/systems/render_prep.rs
use crate::components::*;
use crate::overlay::{OverlayManager, OverlayPrimitive};
use crate::systems::picking::get_voxel_at_mouse;
use glam::Vec3;
use hecs::World;

/// Prepare a `Uniforms` struct for the given viewport mode.
pub fn sys_prepare_render_data(world: &mut World, view_mode: u32) -> Uniforms {
    // 1. Get Window Settings and Viewport Rect
    let mut resolution = [800.0, 600.0];
    for (_, win) in world.query::<&WindowSettings>().iter() {
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
    let mut window_params = [40.0, 400.0, -1000.0, 1000.0];
    for (_, vol) in world.query::<&VolumeData>().iter() {
        window_params[2] = vol.intensity_range[0];
        window_params[3] = vol.intensity_range[1];
    }
    for (_, windowing) in world.query::<&VolumeWindowing>().iter() {
        window_params[0] = windowing.center;
        window_params[1] = windowing.width;
    }

    // 8. Get brush preview settings
    let mut brush_preview = [0.0f32; 4];
    let mut brush_active_viewport = 0u32;
    for (_, editor) in world.query::<&EditorState>().iter() {
        match editor.active_tool {
            EditorTool::Brush | EditorTool::Eraser => {
                brush_preview[0] = editor.brush_size;
                brush_preview[1] = 1.0;
            }
            _ => {}
        }
    }
    for (_, input) in world.query::<&InputState>().iter() {
        if input.active_viewport > 0 && brush_preview[1] > 0.0 {
            brush_preview[2] = input.active_viewport as f32;
            brush_active_viewport = input.active_viewport;
        }
    }

    let brush_center_voxel = if brush_preview[1] > 0.0 && brush_active_viewport > 0 {
        match get_voxel_at_mouse(world, brush_active_viewport, mouse_uv) {
            Some([x, y, z]) => [x, y, z, 1.0],
            None => [0.0, 0.0, 0.0, 0.0],
        }
    } else {
        [0.0, 0.0, 0.0, 0.0]
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
        rotation,
        overlay_mouse_uv: mouse_uv,
        overlay_primitive_count: 0,
        overlay_dragging_idx: u32::MAX,
        brush_preview,
        brush_center_voxel,
        zoom: zoom_val,
        time: time_val,
        view_mode,
        overlay_flags,
    }
}

pub fn sys_sync_annotations_to_overlay(world: &mut World) {
    let mut annotation_positions: Vec<Vec3> = Vec::new();
    for (_, ann_state) in world.query::<&AnnotationState>().iter() {
        for ann in &ann_state.annotations {
            annotation_positions.push(ann.world_pos);
        }
    }

    for (_, overlay) in world.query_mut::<&mut OverlayManager>() {
        overlay.annotations.clear();
        for (i, pos) in annotation_positions.iter().enumerate() {
            overlay.add_annotation(*pos, format!("A{}", i), i);
        }
        overlay.rebuild_primitives();
    }
}

pub fn get_overlay_render_data(world: &World) -> (Vec<u8>, u32, u32, [f32; 2]) {
    let mut primitives_bytes = Vec::new();
    let mut count = 0u32;
    let mut dragging_idx = u32::MAX;
    let mut mouse_uv = [0.5f32, 0.5];

    for (_, overlay) in world.query::<&OverlayManager>().iter() {
        count = overlay.primitives.len() as u32;
        dragging_idx = overlay.dragging_idx.map(|i| i as u32).unwrap_or(u32::MAX);
        mouse_uv = overlay.mouse_screen_uv;
        primitives_bytes = bytemuck::cast_slice(&overlay.primitives).to_vec();
    }

    (primitives_bytes, count, dragging_idx, mouse_uv)
}
