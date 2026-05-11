// src/systems/render_prep.rs
use crate::components::*;
use crate::overlay::OverlayManager;
use glam::Vec3;
use hecs::World;

/// Prepare a `Uniforms` struct for the given viewport mode.
pub fn sys_prepare_render_data(
    world: &mut World,
    entities: &AppEntities,
    viewport_entity: hecs::Entity,
) -> Uniforms {
    // 1. Get Viewport and State
    let (vp_rect, view_mode, zoom_val, pan, zoom_pivot, user_rotation) = if let (Ok(vp), Ok(vs)) = (
        world.get::<&Viewport>(viewport_entity),
        world.get::<&ViewportState>(viewport_entity),
    ) {
        (
            vp.rect,
            vp.mode as u32,
            vs.zoom,
            vs.pan,
            vs.pivot,
            vs.user_rotation,
        )
    } else {
        (
            [0.0; 4],
            0,
            1.0,
            [0.0, 0.0],
            [0.5, 0.5],
            [0.0, 0.0, 0.0, 1.0],
        )
    };

    let resolution = [vp_rect[2], vp_rect[3]];

    // 2. Get Cursor
    let mut cursor_pos = [0.0; 4];
    if let Ok(t) = world.get::<&Transform>(entities.cursor) {
        cursor_pos[0] = t.position[0];
        cursor_pos[1] = t.position[1];
        cursor_pos[2] = t.position[2];
    }

    // 4. Get Mouse UV
    let mut mouse_uv = [0.5, 0.5];
    if let Ok(inp) = world.get::<&InputState>(entities.input) {
        mouse_uv = inp.mouse_uv;
    }

    // 5. Get Volume Info and Compose Rotation
    let mut volume_dims = [0u32; 4];
    let mut volume_spacing = [0.0f32; 4];
    let mut volume_intensity_range = [-1000.0, 1000.0];
    let mut data_orientation = [0.0f32; 4];
    let mut main_geometry = None;
    let mut main_query = world.query::<&VolumeData>().with::<&MainVolumeTag>();
    for (_, vol) in main_query.iter() {
        volume_dims = [vol.dimensions[0], vol.dimensions[1], vol.dimensions[2], 0];
        volume_spacing = [vol.spacing[0], vol.spacing[1], vol.spacing[2], 0.0];
        volume_intensity_range = vol.intensity_range;
        data_orientation = vol.orientation;
        main_geometry = Some(VoxelGeometry {
            dimensions: vol.dimensions,
            spacing: vol.spacing,
            origin: vol.origin,
            orientation: vol.orientation,
        });
    }

    let composed_rotation = if view_mode == 0 {
        // 3D
        crate::util::orientation::compose_view_rotation(data_orientation, user_rotation)
    } else {
        user_rotation
    };

    // 6. Get Overlay Info
    let mut overlay_flags = 0u32;
    let mut overlay_opacities = [0.0f32; 4];
    let mut overlay1_dims = [0u32; 4];
    let mut overlay2_dims = [0u32; 4];
    let mut overlay1_main_to_roi_row0 = [0.0, 0.0, 0.0, 0.0];
    let mut overlay1_main_to_roi_row1 = [0.0, 0.0, 0.0, 0.0];
    let mut overlay1_main_to_roi_row2 = [0.0, 0.0, 0.0, 0.0];
    let mut overlay2_main_to_roi_row0 = [0.0, 0.0, 0.0, 0.0];
    let mut overlay2_main_to_roi_row1 = [0.0, 0.0, 0.0, 0.0];
    let mut overlay2_main_to_roi_row2 = [0.0, 0.0, 0.0, 0.0];
    let active_roi = world
        .get::<&EditorState>(entities.editor)
        .ok()
        .and_then(|editor| editor.active_roi);
    for (layer_count, overlay) in
        crate::app::roi_runtime::renderable_voxel_overlay_rois(world, active_roi)
            .into_iter()
            .enumerate()
    {
        overlay_flags |= 1 << layer_count;
        overlay_opacities[layer_count] = overlay.opacity;
        let Some(main_geometry) = main_geometry else {
            continue;
        };
        let Ok(roi) = world.get::<&Roi>(overlay.entity) else {
            continue;
        };
        let Some(voxel_cache) = roi.voxel_cache() else {
            continue;
        };
        let roi_geometry = voxel_cache.data.geometry;
        let Some(main_to_roi) =
            crate::convert::index_space_affine_from_src_to_dst(main_geometry, roi_geometry)
        else {
            continue;
        };
        match layer_count {
            0 => {
                overlay1_dims = [
                    roi_geometry.dimensions[0],
                    roi_geometry.dimensions[1],
                    roi_geometry.dimensions[2],
                    0,
                ];
                overlay1_main_to_roi_row0 = main_to_roi[0];
                overlay1_main_to_roi_row1 = main_to_roi[1];
                overlay1_main_to_roi_row2 = main_to_roi[2];
            }
            1 => {
                overlay2_dims = [
                    roi_geometry.dimensions[0],
                    roi_geometry.dimensions[1],
                    roi_geometry.dimensions[2],
                    0,
                ];
                overlay2_main_to_roi_row0 = main_to_roi[0];
                overlay2_main_to_roi_row1 = main_to_roi[1];
                overlay2_main_to_roi_row2 = main_to_roi[2];
            }
            _ => {}
        }
    }

    // 7. Get Windowing Info (HU-based)
    let mut window_params = [
        40.0,
        400.0,
        volume_intensity_range[0],
        volume_intensity_range[1],
    ];
    if let Ok(windowing) = world.get::<&VolumeWindowing>(entities.volume_windowing) {
        window_params[0] = windowing.center;
        window_params[1] = windowing.width;
    }

    Uniforms {
        cursor_pos,
        volume_dims,
        volume_spacing,
        overlay1_dims,
        overlay2_dims,
        overlay_opacities,
        overlay1_main_to_roi_row0,
        overlay1_main_to_roi_row1,
        overlay1_main_to_roi_row2,
        overlay2_main_to_roi_row0,
        overlay2_main_to_roi_row1,
        overlay2_main_to_roi_row2,
        window_params,
        resolution,
        mouse_uv,
        pan,
        zoom_pivot,
        rotation: composed_rotation,
        overlay_mouse_uv: mouse_uv,
        overlay_primitive_count: 0,
        overlay_dragging_idx: u32::MAX,
        zoom: zoom_val,
        view_mode,
        overlay_flags,
        _padding: 0,
    }
}

pub fn sys_sync_annotations_to_overlay(world: &mut World, entities: &AppEntities) {
    let mut annotation_positions: Vec<Vec3> = Vec::new();
    if let Ok(ann_state) = world.get::<&AnnotationState>(entities.annotations) {
        for ann in &ann_state.annotations {
            annotation_positions.push(ann.world_pos);
        }
    }

    if let Ok(mut overlay) = world.get::<&mut OverlayManager>(entities.overlay) {
        overlay.annotations.clear();
        for pos in &annotation_positions {
            overlay.add_annotation(*pos);
        }
        overlay.rebuild_primitives();
    }
}

pub fn get_overlay_render_data(
    world: &World,
    entities: &AppEntities,
) -> (Vec<u8>, u32, u32, [f32; 2]) {
    let mut primitives_bytes = Vec::new();
    let mut count = 0u32;
    let mut dragging_idx = u32::MAX;
    let mut mouse_uv = [0.5f32, 0.5];

    if let Ok(overlay) = world.get::<&OverlayManager>(entities.overlay) {
        count = overlay.primitives.len() as u32;
        dragging_idx = overlay.dragging_idx.map(|i| i as u32).unwrap_or(u32::MAX);
        mouse_uv = overlay.mouse_screen_uv;
        primitives_bytes = bytemuck::cast_slice(&overlay.primitives).to_vec();
    }

    (primitives_bytes, count, dragging_idx, mouse_uv)
}
