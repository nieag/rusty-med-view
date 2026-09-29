use super::*;
use glam::Vec3;

#[test]
fn test_axial_conversions() {
    let plane = SlicePlane::Axial;
    // Top-left (0,0) -> Radiological Right (x=1.0)
    assert_eq!(plane.screen_uv_to_volume([0.0, 0.0], 0.5), [1.0, 1.0, 0.5]);
    // Bottom-right (1,1) -> Radiological Left (x=0.0)
    assert_eq!(plane.screen_uv_to_volume([1.0, 1.0], 0.5), [0.0, 0.0, 0.5]);
}

#[test]
fn test_coronal_conversions() {
    let plane = SlicePlane::Coronal;
    // Top-left (0,0) -> Radiological Right (x=1.0)
    assert_eq!(plane.screen_uv_to_volume([0.0, 0.0], 0.5), [1.0, 0.5, 1.0]);
    // Bottom-right (1,1) -> Radiological Left (x=0.0)
    assert_eq!(plane.screen_uv_to_volume([1.0, 1.0], 0.5), [0.0, 0.5, 0.0]);
}

#[test]
fn test_sagittal_conversions() {
    let plane = SlicePlane::Sagittal;
    // Top-left (0,0) -> Anterior (y=1.0), Superior (z=1.0)
    assert_eq!(plane.screen_uv_to_volume([0.0, 0.0], 0.5), [0.5, 1.0, 1.0]);
    // Bottom-right (1,1) -> Posterior (y=0.0), Inferior (z=0.0)
    assert_eq!(plane.screen_uv_to_volume([1.0, 1.0], 0.5), [0.5, 0.0, 0.0]);
}

#[test]
fn test_round_trip() {
    let planes = [SlicePlane::Axial, SlicePlane::Coronal, SlicePlane::Sagittal];
    for plane in planes {
        let uv = [0.3, 0.7];
        let depth = 0.4;
        let vol = plane.screen_uv_to_volume(uv, depth);
        let recovered_uv = plane.volume_to_screen_uv(vol);
        assert!((uv[0] - recovered_uv[0]).abs() < 1e-6);
        assert!((uv[1] - recovered_uv[1]).abs() < 1e-6);
    }
}

#[test]
fn test_slice_plane_plane_family_adapters() {
    assert_eq!(
        SlicePlane::from_plane_family(SlicePlane::Axial.to_plane_family()),
        Some(SlicePlane::Axial)
    );
    assert_eq!(
        SlicePlane::from_plane_family(SlicePlane::Coronal.to_plane_family()),
        Some(SlicePlane::Coronal)
    );
    assert_eq!(
        SlicePlane::from_plane_family(SlicePlane::Sagittal.to_plane_family()),
        Some(SlicePlane::Sagittal)
    );
    assert_eq!(
        SlicePlane::from_plane_family(crate::convert::PlaneFamily::Oblique),
        None
    );
}

#[test]
fn test_quat_identity() {
    let q = [0.0, 0.0, 0.0, 1.0];
    let m = quat_to_mat3(q);
    assert_eq!(m, [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
}

#[test]
fn test_screen_to_ray_matches_shader() {
    let rotation = [0.0, 0.0, 0.0, 1.0]; // Identity
    let (cam, dir) = screen_to_ray_3d([0.5, 0.5], rotation, 1.0, [0.0, 0.0], 1.0);

    // Center screen should give forward ray (0,0,1)
    assert!((dir[0]).abs() < 1e-5);
    assert!((dir[1]).abs() < 1e-5);
    assert!((dir[2] - 1.0).abs() < 1e-5);

    // Camera should be at (0,0,-3.5)
    assert!((cam[0]).abs() < 1e-5);
    assert!((cam[1]).abs() < 1e-5);
    assert!((cam[2] + 3.5).abs() < 1e-5);
}

#[test]
fn test_volume_to_screen_roundtrip() {
    let rotation = [0.0, 0.0, 0.0, 1.0];
    let vol_pos = [0.4, 0.6, 0.5]; // Some point in the volume
    let aspects = [1.0, 1.0, 1.0];

    let screen = volume_to_screen_3d(vol_pos, rotation, aspects, 1.0, [0.0, 0.0], 1.0);
    assert!(screen.is_some());

    let (cam, dir) = screen_to_ray_3d(screen.unwrap(), rotation, 1.0, [0.0, 0.0], 1.0);

    // Ray p = cam + t*dir. For vol_pos [0.4, 0.6, 0.5], object pos is (0.4-0.5, 0.6-0.5, 0.5-0.5) = (-0.1, 0.1, 0.0)
    // Since cam is at (0,0,-3.5) and point has Z=0, t should be 3.5 / dir[2]
    let t = 3.5 / dir[2];
    let p = [
        cam[0] + t * dir[0],
        cam[1] + t * dir[1],
        cam[2] + t * dir[2],
    ];

    assert!((p[0] - (-0.1)).abs() < 1e-4);
    assert!((p[1] - 0.1).abs() < 1e-4);
    assert!((p[2] - 0.0).abs() < 1e-4);
}

#[test]
fn test_orthographic_projection_does_not_taper_with_depth() {
    let rotation = [0.0, 0.0, 0.0, 1.0];
    let near = volume_to_screen_3d(
        [0.2, 0.5, 0.2],
        rotation,
        [1.0, 1.0, 1.0],
        1.0,
        [0.0, 0.0],
        1.0,
    )
    .expect("near point is visible");
    let far = volume_to_screen_3d(
        [0.2, 0.5, 0.8],
        rotation,
        [1.0, 1.0, 1.0],
        1.0,
        [0.0, 0.0],
        1.0,
    )
    .expect("far point is visible");

    assert!((near[0] - far[0]).abs() < 1e-6);
    assert!((near[1] - far[1]).abs() < 1e-6);
}

#[test]
fn test_compose_rotation() {
    let data = [0.0, 0.0, 0.0, 1.0];
    let user = [0.0, 0.0, 0.0, 1.0];
    let result = compose_view_rotation(data, user);

    // Should equal BASE_ROTATION
    assert!((result[0] - BASE_ROTATION[0]).abs() < 1e-5);
    assert!((result[1] - BASE_ROTATION[1]).abs() < 1e-5);
    assert!((result[2] - BASE_ROTATION[2]).abs() < 1e-5);
    assert!((result[3] - BASE_ROTATION[3]).abs() < 1e-5);
}

#[test]
fn test_default_3d_projects_superior_up() {
    let rotation = compose_view_rotation([0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, 1.0]);
    let superior = project_axis_3d([0.0, 0.0, 1.0], rotation, [1.0, 1.0, 1.0], 1.0);
    let inferior = project_axis_3d([0.0, 0.0, -1.0], rotation, [1.0, 1.0, 1.0], 1.0);

    assert!(
        superior[1] < 0.0,
        "superior must project screen-up by default"
    );
    assert!(
        inferior[1] > 0.0,
        "inferior must project screen-down by default"
    );
}

#[test]
fn test_project_axis() {
    let rotation = [0.0, 0.0, 0.0, 1.0];
    let aspects = [1.0, 1.0, 1.0];

    // X Axis (Right)
    let proj_x = project_axis_3d([1.0, 0.0, 0.0], rotation, aspects, 1.0);
    // Should be negative X on screen (Radiological)
    assert!(proj_x[0] < -0.5); // Large negative value
    assert!(proj_x[1].abs() < 1e-5);

    // Y Axis (Anterior)
    let proj_y = project_axis_3d([0.0, 1.0, 0.0], rotation, aspects, 1.0);
    // At identity BASE_ROTATION (90 deg around X), Y is rotated to Z?
    // Let's check: BASE rotates Object Y (Anterior) to World -Z (Inferior)?
    // Wait, sin(45) = 0.707. BASE = [0.707, 0, 0, 0.707].
    // Quat * [0, 1, 0] * Conj(Quat)
    // Y becomes Z globally.
    assert!((proj_y[0]).abs() < 1e-5);
    assert!((proj_y[1] - (-1.0)).abs() < 1e-5); // Y points UP (negative screen Y)

    // Z Axis (Superior)
    let proj_z = project_axis_3d([0.0, 0.0, 1.0], rotation, aspects, 1.0);
    // Z becomes -Y? (Inferior)
    assert!((proj_z[0]).abs() < 1e-5);
    assert!((proj_z[1] - 0.0).abs() < 1e-5); // Z points straight (no Y component in screen space at this specific BASE_ROTATION)
}

/// Replicates the shader logic from `fs_main` in pure Rust for parity checking.
fn shader_logic_emulation(
    view_mode: u32,
    uv: [f32; 2],
    zoom: f32,
    pan: [f32; 2],
    resolution: [f32; 2],
    vol_aspects: [f32; 3],
    cursor: [f32; 3],
) -> [f32; 3] {
    let pivot = [0.5, 0.5];
    let screen_aspect = resolution[0] / resolution[1];

    // 1. Calculate aspect correction K
    let slice_aspect = match view_mode {
        1 => vol_aspects[0] / vol_aspects[1],
        2 => vol_aspects[0] / vol_aspects[2],
        3 => vol_aspects[1] / vol_aspects[2],
        _ => 1.0,
    };
    let k = screen_aspect / slice_aspect;

    // 2. Map Screen UV to "Corrected" centered coord
    let centered_uv = [(uv[0] - pivot[0]) * k, uv[1] - pivot[1]];

    // 3. Apply Zoom and Pan
    let zoomed_uv = [
        centered_uv[0] / zoom + pivot[0] + pan[0],
        centered_uv[1] / zoom + pivot[1] + pan[1],
    ];

    // 4. Sample projection logic (MUST MATCH SHADER EXACTLY)
    match view_mode {
        0 => {
            // 3D orthographic projection
            // Represents the orthographic crosshair calculation in mode 0.

            let q = [0.0, 0.0, 0.0, 1.0]; // Identity rotation for test
            let rot_mat = quat_to_mat3(q);

            // For parity test, we assume standard Aspect Ratio Vol [1,1,1]
            let cursor_obj = [cursor[0] - 0.5, cursor[1] - 0.5, cursor[2] - 0.5];
            let cursor_world = rotate_vec3(rot_mat, cursor_obj);
            let screen_u = -cursor_world[0] / (ORTHOGRAPHIC_VIEW_SCALE * screen_aspect);
            let screen_v = -cursor_world[1] / ORTHOGRAPHIC_VIEW_SCALE;
            let p_uv = [screen_u + 0.5, screen_v + 0.5];
            let crosshair_pos = [
                (p_uv[0] - pan[0] - pivot[0]) * zoom + pivot[0],
                (p_uv[1] - pan[1] - pivot[1]) * zoom + pivot[1],
            ];
            [crosshair_pos[0], crosshair_pos[1], 0.0]
        }
        1 => [1.0 - zoomed_uv[0], 1.0 - zoomed_uv[1], cursor[2]], // Axial
        2 => [1.0 - zoomed_uv[0], cursor[1], 1.0 - zoomed_uv[1]], // Coronal
        3 => [cursor[0], 1.0 - zoomed_uv[0], 1.0 - zoomed_uv[1]], // Sagittal
        _ => [0.0, 0.0, 0.0],
    }
}

#[test]
fn test_shader_parity_axial() {
    let uv = [0.2, 0.3];
    let zoom = 2.0;
    let pan = [0.1, -0.1];
    let res = [800.0, 600.0];
    let aspects = [1.0, 0.8, 1.2];
    let cursor = [0.5, 0.5, 0.5];

    let shader_result = shader_logic_emulation(1, uv, zoom, pan, res, aspects, cursor);

    // Calculate Clean API result
    let plane = SlicePlane::Axial;
    let slice_aspect = plane.slice_aspect(aspects);
    let k = res[0] / res[1] / slice_aspect;
    let volume_uv = [
        ((uv[0] - 0.5) * k) / zoom + 0.5 + pan[0],
        (uv[1] - 0.5) / zoom + 0.5 + pan[1],
    ];
    let api_result = plane.screen_uv_to_volume(volume_uv, cursor[2]);

    for i in 0..3 {
        assert!((shader_result[i] - api_result[i]).abs() < 1e-6);
    }
}

#[test]
fn test_shader_parity_coronal() {
    let uv = [0.6, 0.2];
    let zoom = 1.5;
    let pan = [-0.2, 0.2];
    let res = [1024.0, 768.0];
    let aspects = [1.0, 1.0, 1.0];
    let cursor = [0.4, 0.4, 0.4];

    let shader_result = shader_logic_emulation(2, uv, zoom, pan, res, aspects, cursor);

    let plane = SlicePlane::Coronal;
    let slice_aspect = plane.slice_aspect(aspects);
    let k = res[0] / res[1] / slice_aspect;
    let volume_uv = [
        ((uv[0] - 0.5) * k) / zoom + 0.5 + pan[0],
        (uv[1] - 0.5) / zoom + 0.5 + pan[1],
    ];
    let api_result = plane.screen_uv_to_volume(volume_uv, cursor[1]);

    for i in 0..3 {
        assert!((shader_result[i] - api_result[i]).abs() < 1e-6);
    }
}

#[test]
fn test_shader_parity_sagittal() {
    let uv = [0.1, 0.9];
    let zoom = 0.8;
    let pan = [0.0, 0.0];
    let res = [600.0, 600.0];
    let aspects = [1.0, 1.1, 0.9];
    let cursor = [0.1, 0.2, 0.3];

    let shader_result = shader_logic_emulation(3, uv, zoom, pan, res, aspects, cursor);

    let plane = SlicePlane::Sagittal;
    let slice_aspect = plane.slice_aspect(aspects);
    let k = res[0] / res[1] / slice_aspect;
    let volume_uv = [
        ((uv[0] - 0.5) * k) / zoom + 0.5 + pan[0],
        (uv[1] - 0.5) / zoom + 0.5 + pan[1],
    ];
    let api_result = plane.screen_uv_to_volume(volume_uv, cursor[0]);

    for i in 0..3 {
        assert!((shader_result[i] - api_result[i]).abs() < 1e-6);
    }
}

#[test]
fn test_picking_3d_parity() {
    // Test that our picking ray math (picking.rs logic) matches
    // the shader's crosshair projection logic.
    let zoom = 1.0;
    let pan = [0.0, 0.0];
    let res = [800.0, 800.0]; // Square to simplify
    let aspects = [1.0, 1.0, 1.0];
    let cursor = [1.0, 0.5, 0.5]; // Patient Right (R)

    // Find Screen UV where this cursor should be projected in 3D
    let shader_uv = shader_logic_emulation(0, [0.5, 0.5], zoom, pan, res, aspects, cursor);
    // radiological should be Left (u < 0.5)
    assert!(shader_uv[0] < 0.5);

    // Now if we pick at that UV in picking logic, we should get cursor back
    let mouse_uv = [shader_uv[0], shader_uv[1]];

    // Manual picking logic (replicated from picking.rs)
    let uv = [mouse_uv[0] - 0.5, mouse_uv[1] - 0.5];
    let screen_pos = [-uv[0] * 1.0, -uv[1]]; // THE RADIOLOGICAL + VERTICAL FIX

    let ray_origin_world = Vec3::from([
        screen_pos[0] * ORTHOGRAPHIC_VIEW_SCALE,
        screen_pos[1] * ORTHOGRAPHIC_VIEW_SCALE,
        -ORTHOGRAPHIC_VIEW_SCALE,
    ]);
    assert!((ray_origin_world.x - 0.5).abs() < 1e-5);

    let cursor_up = [0.5, 0.8, 0.5]; // Anterior (Up in identity)
    let shader_uv_up = shader_logic_emulation(0, [0.5, 0.5], zoom, pan, res, aspects, cursor_up);
    // Anterior (+Y) should be at Top (v < 0.5)
    assert!(shader_uv_up[1] < 0.5);

    let uv_up = [shader_uv_up[0] - 0.5, shader_uv_up[1] - 0.5];
    let screen_pos_up = [-uv_up[0], -uv_up[1]];
    let ray_up_origin = Vec3::from([
        screen_pos_up[0] * ORTHOGRAPHIC_VIEW_SCALE,
        screen_pos_up[1] * ORTHOGRAPHIC_VIEW_SCALE,
        -ORTHOGRAPHIC_VIEW_SCALE,
    ]);
    assert!((ray_up_origin.y - 0.3).abs() < 1e-5);
}
