use super::*;
use glam::Vec3;

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

/// Where the shader puts the 3D crosshair (mode 0 of `fs_main`), for an identity rotation and a
/// unit-aspect volume, as a screen UV. The 2D views are compared with the real shader on the GPU
/// instead (`render::pipeline` tests).
fn shader_crosshair_3d(cursor: [f32; 3], zoom: f32, pan: [f32; 2], screen_aspect: f32) -> [f32; 2] {
    let pivot = [0.5, 0.5];
    let cursor_world = [cursor[0] - 0.5, cursor[1] - 0.5, cursor[2] - 0.5];
    let screen_u = -cursor_world[0] / (ORTHOGRAPHIC_VIEW_SCALE * screen_aspect);
    let screen_v = -cursor_world[1] / ORTHOGRAPHIC_VIEW_SCALE;
    let p_uv = [screen_u + 0.5, screen_v + 0.5];
    [
        (p_uv[0] - pan[0] - pivot[0]) * zoom + pivot[0],
        (p_uv[1] - pan[1] - pivot[1]) * zoom + pivot[1],
    ]
}

#[test]
fn test_picking_3d_parity() {
    // Test that our picking ray math (picking.rs logic) matches
    // the shader's crosshair projection logic.
    let zoom = 1.0;
    let pan = [0.0, 0.0];
    let screen_aspect = 1.0; // Square to simplify
    let cursor = [1.0, 0.5, 0.5]; // Patient Right (R)

    // Find Screen UV where this cursor should be projected in 3D
    let shader_uv = shader_crosshair_3d(cursor, zoom, pan, screen_aspect);
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
    let shader_uv_up = shader_crosshair_3d(cursor_up, zoom, pan, screen_aspect);
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

fn geometry_from_columns(
    column_x: [f64; 3],
    column_y: [f64; 3],
    column_z: [f64; 3],
) -> crate::model::VoxelGeometry {
    use glam::{DMat4, DVec4};
    crate::model::VoxelGeometry::from_affine(
        [8, 8, 8],
        DMat4::from_cols(
            DVec4::new(column_x[0], column_x[1], column_x[2], 0.0),
            DVec4::new(column_y[0], column_y[1], column_y[2], 0.0),
            DVec4::new(column_z[0], column_z[1], column_z[2], 0.0),
            DVec4::W,
        ),
    )
    .unwrap()
}

fn letters(left: char, right: char, top: char, bottom: char) -> EdgeLetters {
    EdgeLetters {
        left,
        right,
        top,
        bottom,
    }
}

#[test]
fn test_anatomical_letter_uses_the_dominant_axis() {
    assert_eq!(anatomical_letter([2.0, 0.5, 0.1]), Some('R'));
    assert_eq!(anatomical_letter([-2.0, 0.5, 0.1]), Some('L'));
    assert_eq!(anatomical_letter([0.1, 0.0, -3.0]), Some('I'));
    assert_eq!(anatomical_letter([0.0, -1.0, 0.2]), Some('P'));
    assert_eq!(anatomical_letter([0.0, 0.0, 0.0]), None);
}

#[test]
fn test_ras_stored_volume_keeps_the_radiological_edge_letters() {
    // The letters the viewer used to hard-code, which are only right for RAS storage.
    let ras = geometry_from_columns([2.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 3.0]);

    assert_eq!(
        SlicePlane::Axial.edge_letters(ras),
        letters('R', 'L', 'A', 'P')
    );
    assert_eq!(
        SlicePlane::Coronal.edge_letters(ras),
        letters('R', 'L', 'S', 'I')
    );
    assert_eq!(
        SlicePlane::Sagittal.edge_letters(ras),
        letters('A', 'P', 'S', 'I')
    );
}

#[test]
fn test_las_stored_volume_swaps_left_and_right_labels() {
    // Index i runs toward the patient's left, so the screen-left edge is the patient's left.
    let las = geometry_from_columns([-2.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 3.0]);

    assert_eq!(
        SlicePlane::Axial.edge_letters(las),
        letters('L', 'R', 'A', 'P')
    );
    assert_eq!(
        SlicePlane::Coronal.edge_letters(las),
        letters('L', 'R', 'S', 'I')
    );
    assert_eq!(
        SlicePlane::Sagittal.edge_letters(las),
        letters('A', 'P', 'S', 'I')
    );
}

#[test]
fn test_lps_stored_volume_flips_left_right_and_anterior_posterior() {
    let lps = geometry_from_columns([-2.0, 0.0, 0.0], [0.0, -2.0, 0.0], [0.0, 0.0, 3.0]);

    assert_eq!(
        SlicePlane::Axial.edge_letters(lps),
        letters('L', 'R', 'P', 'A')
    );
    assert_eq!(
        SlicePlane::Sagittal.edge_letters(lps),
        letters('P', 'A', 'S', 'I')
    );
}

#[test]
fn test_permuted_axes_are_labelled_by_their_real_direction() {
    // A sagittal acquisition stored with i -> anterior, j -> superior, k -> right.
    let permuted = geometry_from_columns([0.0, 2.0, 0.0], [0.0, 0.0, 2.0], [3.0, 0.0, 0.0]);

    assert_eq!(
        SlicePlane::Axial.edge_letters(permuted),
        letters('A', 'P', 'S', 'I')
    );
    assert_eq!(
        SlicePlane::Axial.anatomical_plane_name(permuted),
        "sagittal"
    );
    assert_eq!(
        SlicePlane::Sagittal.anatomical_plane_name(permuted),
        "coronal"
    );
}

#[test]
fn test_anatomical_plane_name_matches_the_view_for_standard_storage() {
    let ras = geometry_from_columns([2.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 3.0]);

    assert_eq!(SlicePlane::Axial.anatomical_plane_name(ras), "axial");
    assert_eq!(SlicePlane::Coronal.anatomical_plane_name(ras), "coronal");
    assert_eq!(SlicePlane::Sagittal.anatomical_plane_name(ras), "sagittal");
}

#[test]
fn test_anatomical_letter_rejects_non_finite_directions() {
    assert_eq!(anatomical_letter([f64::NAN, 1.0, 0.0]), None);
    assert_eq!(anatomical_letter([f64::INFINITY, 0.0, 0.0]), None);
}

#[test]
fn test_orthographic_view_scale_matches_the_shader_constant() {
    let shader = include_str!("../../shaders/shader.wgsl");
    let declared = shader
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("const ORTHOGRAPHIC_VIEW_SCALE: f32 = ")
                .and_then(|rest| rest.strip_suffix(';'))
        })
        .expect("the shader declares ORTHOGRAPHIC_VIEW_SCALE");
    assert_eq!(declared.parse::<f32>().unwrap(), ORTHOGRAPHIC_VIEW_SCALE);
}
