// src/systems/render_prep.rs
use crate::components::*;
use hecs::World;

/// Prepare a `Uniforms` struct for the given viewport mode.
/// Raymarch steps of the 3D view at rest.
pub const FULL_RAY_STEPS: u32 = 128;

pub fn sys_prepare_render_data(
    world: &mut World,
    session: &Session,
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
    let position = session.cursor.position;
    let cursor_pos = [position[0], position[1], position[2], 0.0];

    // 4. Get Mouse UV
    let mouse_uv = session.input.mouse_uv;

    // 5. Get Volume Info and Compose Rotation
    let mut volume_dims = [0u32; 4];
    let mut volume_spacing = [0.0f32; 4];
    let mut volume_intensity_range = [-1000.0, 1000.0];
    let mut data_orientation = [0.0f32; 4];
    let mut main_geometry = None;
    let mut main_query = world.query::<&VolumeData>().with::<&MainVolumeTag>();
    for (_, vol) in main_query.iter() {
        volume_dims = [vol.dimensions[0], vol.dimensions[1], vol.dimensions[2], 0];
        let spacing = vol.spacing();
        volume_spacing = [spacing[0], spacing[1], spacing[2], 0.0];
        volume_intensity_range = vol.intensity_range;
        data_orientation = vol.orientation();
        main_geometry = vol.geometry;
    }

    let composed_rotation = if view_mode == 0 {
        // 3D
        crate::util::orientation::compose_view_rotation(data_orientation, user_rotation)
    } else {
        user_rotation
    };
    let mut oblique_origin_uv = [0.0; 4];
    let mut oblique_u_dir_length = [0.0; 4];
    let mut oblique_v_dir_length = [0.0; 4];
    if view_mode == ViewMode::Oblique as u32 {
        if let Some(geometry) = main_geometry {
            if let Some(plane) = crate::convert::oblique_plane_from_view_rotation(
                [cursor_pos[0], cursor_pos[1], cursor_pos[2]],
                user_rotation,
                geometry,
            ) {
                if let Some((u_dir, v_dir, u_length, v_length)) =
                    crate::convert::oblique_volume_uv_basis_and_lengths(plane, geometry)
                {
                    let origin_uv =
                        crate::convert::world_mm_to_volume_uv(plane.origin_mm, geometry);
                    oblique_origin_uv = [origin_uv[0], origin_uv[1], origin_uv[2], 0.0];
                    oblique_u_dir_length = [u_dir[0], u_dir[1], u_dir[2], u_length];
                    oblique_v_dir_length = [v_dir[0], v_dir[1], v_dir[2], v_length];
                }
            }
        }
    }

    // 6. Get Overlay Info
    let mut overlay_flags = 0u32;
    let mut voxel_overlays = [VoxelOverlayUniform::default(); MAX_VOXEL_OVERLAY_SLOTS];
    let active_roi = {
        let editor = &session.editor;
        editor.active_roi
    };
    for (layer_count, overlay) in
        crate::app::roi_runtime::renderable_voxel_overlay_rois(world, active_roi)
            .into_iter()
            .enumerate()
    {
        if layer_count >= MAX_VOXEL_OVERLAY_SLOTS {
            break;
        }
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
        overlay_flags |= 1 << layer_count;
        voxel_overlays[layer_count] = VoxelOverlayUniform {
            dimensions: [
                roi_geometry.dimensions[0],
                roi_geometry.dimensions[1],
                roi_geometry.dimensions[2],
                0,
            ],
            opacity: [overlay.opacity, 0.0, 0.0, 0.0],
            main_to_roi_row0: main_to_roi[0],
            main_to_roi_row1: main_to_roi[1],
            main_to_roi_row2: main_to_roi[2],
        };
    }

    // 7. Get Windowing Info (HU-based)
    let mut window_params = [
        40.0,
        400.0,
        volume_intensity_range[0],
        volume_intensity_range[1],
    ];
    {
        let windowing = &session.windowing;
        window_params[0] = windowing.center;
        window_params[1] = windowing.width;
    }

    Uniforms {
        cursor_pos,
        volume_dims,
        volume_spacing,
        voxel_overlays,
        window_params,
        resolution,
        mouse_uv,
        pan,
        zoom_pivot,
        rotation: composed_rotation,
        oblique_origin_uv,
        oblique_u_dir_length,
        oblique_v_dir_length,
        overlay_mouse_uv: mouse_uv,
        overlay_primitive_count: 0,
        overlay_dragging_idx: u32::MAX,
        zoom: zoom_val,
        view_mode,
        overlay_flags,
        ray_steps: FULL_RAY_STEPS,
    }
}

/// Mirrors the annotations into the overlay's marker primitives.
pub fn sys_sync_annotations_to_overlay(session: &mut Session) {
    let overlay = &mut session.overlay;
    overlay.annotations.clear();
    for annotation in &session.annotations.annotations {
        overlay.add_annotation(annotation.world_pos);
    }
    overlay.rebuild_primitives();
}

pub fn get_overlay_render_data(session: &Session) -> (Vec<u8>, u32, u32, [f32; 2]) {
    let overlay = &session.overlay;
    (
        bytemuck::cast_slice(&overlay.primitives).to_vec(),
        overlay.primitives.len() as u32,
        overlay.dragging_idx.map(|i| i as u32).unwrap_or(u32::MAX),
        overlay.mouse_screen_uv,
    )
}
