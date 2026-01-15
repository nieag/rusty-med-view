// src/orientation.rs
//! Centralized module for volume orientation and coordinate systems.
//!
//! Handles mapping between:
//! - Volume UV (0..1, +Y=Anterior, +Z=Superior)
//! - Screen UV (0..1, Top-Left=0,0, +V=Down)
//! - World Space (Anatomical labels)

/// Anatomical slice planes for 2D viewports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlicePlane {
    /// XY plane, viewing from Superior (top-down)
    Axial = 1,
    /// XZ plane, viewing from Anterior (front)
    Coronal = 2,
    /// YZ plane, viewing from Right (side)
    Sagittal = 3,
}

impl SlicePlane {
    /// Create from ViewMode
    pub fn from_mode(mode: crate::components::ViewMode) -> Option<Self> {
        match mode {
            crate::components::ViewMode::Axial => Some(SlicePlane::Axial),
            crate::components::ViewMode::Coronal => Some(SlicePlane::Coronal),
            crate::components::ViewMode::Sagittal => Some(SlicePlane::Sagittal),
            _ => None,
        }
    }

    /// Create from viewport index (1=Axial, 2=Coronal, 3=Sagittal) [DEPRECATED]
    pub fn from_viewport(idx: u32) -> Option<Self> {
        match idx {
            1 => Some(SlicePlane::Axial),
            2 => Some(SlicePlane::Coronal),
            3 => Some(SlicePlane::Sagittal),
            _ => None,
        }
    }

    /// Get the volume axis index for the depth dimension (0=X, 1=Y, 2=Z)
    pub fn depth_axis(&self) -> usize {
        match self {
            SlicePlane::Axial => 2,    // Z
            SlicePlane::Coronal => 1,  // Y
            SlicePlane::Sagittal => 0, // X
        }
    }

    /// Get slice aspect ratio from volume aspects (dimensions * spacing)
    pub fn slice_aspect(&self, vol_aspects: [f32; 3]) -> f32 {
        match self {
            SlicePlane::Axial => vol_aspects[0] / vol_aspects[1],
            SlicePlane::Coronal => vol_aspects[0] / vol_aspects[2],
            SlicePlane::Sagittal => vol_aspects[1] / vol_aspects[2],
        }
    }

    /// Convert screen UV (0..1, top-left origin) to volume UV (0..1)
    pub fn screen_uv_to_volume(&self, uv: [f32; 2], cursor_depth: f32) -> [f32; 3] {
        match self {
            SlicePlane::Axial => {
                // RADIOLOGICAL: Patient Right (x=1) on Screen Left (u=0)
                [1.0 - uv[0], 1.0 - uv[1], cursor_depth]
            }
            SlicePlane::Coronal => {
                // RADIOLOGICAL: Patient Right (x=1) on Screen Left (u=0)
                [1.0 - uv[0], cursor_depth, 1.0 - uv[1]]
            }
            SlicePlane::Sagittal => {
                // Anterior is LEFT (u=0), Superior is UP (v=0)
                [cursor_depth, 1.0 - uv[0], 1.0 - uv[1]]
            }
        }
    }

    /// Convert volume UV (0..1) to screen UV (0..1)
    pub fn volume_to_screen_uv(&self, pos: [f32; 3]) -> [f32; 2] {
        match self {
            SlicePlane::Axial => [1.0 - pos[0], 1.0 - pos[1]],
            SlicePlane::Coronal => [1.0 - pos[0], 1.0 - pos[2]],
            SlicePlane::Sagittal => [1.0 - pos[1], 1.0 - pos[2]],
        }
    }
}

/// Base rotation to make Superior UP in default 3D view
pub const BASE_ROTATION: [f32; 4] = [0.7071068, 0.0, 0.0, 0.7071068]; // 90° rotation around X

/// Compose final view rotation from data orientation and user rotation.
/// Formula: final = user_rotation * BASE_ROTATION * data_orientation
pub fn compose_view_rotation(data_orientation: [f32; 4], user_rotation: [f32; 4]) -> [f32; 4] {
    let data = glam::Quat::from_array(data_orientation);
    let base = glam::Quat::from_array(BASE_ROTATION);
    let user = glam::Quat::from_array(user_rotation);
    (user * base * data).normalize().to_array()
}

/// Convert quaternion [x, y, z, w] to 3x3 rotation matrix (column-major)
pub fn quat_to_mat3(q: [f32; 4]) -> [[f32; 3]; 3] {
    let x2 = q[0] + q[0];
    let y2 = q[1] + q[1];
    let z2 = q[2] + q[2];
    let xx = q[0] * x2;
    let xy = q[0] * y2;
    let xz = q[0] * z2;
    let yy = q[1] * y2;
    let yz = q[1] * z2;
    let zz = q[2] * z2;
    let wx = q[3] * x2;
    let wy = q[3] * y2;
    let wz = q[3] * z2;

    [
        [1.0 - (yy + zz), xy + wz, xz - wy],
        [xy - wz, 1.0 - (xx + zz), yz + wx],
        [xz + wy, yz - wx, 1.0 - (xx + yy)],
    ]
}

/// Transpose a 3x3 matrix (for inverse of rotation)
pub fn transpose_mat3(m: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    [
        [m[0][0], m[1][0], m[2][0]],
        [m[0][1], m[1][1], m[2][1]],
        [m[0][2], m[1][2], m[2][2]],
    ]
}

/// Apply a 3x3 rotation matrix to a 3D vector
pub fn rotate_vec3(m: [[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[1][0] * v[1] + m[2][0] * v[2],
        m[0][1] * v[0] + m[1][1] * v[1] + m[2][1] * v[2],
        m[0][2] * v[0] + m[1][2] * v[1] + m[2][2] * v[2],
    ]
}

/// Normalize a 3D vector
pub fn normalize_vec3(v: [f32; 3]) -> [f32; 3] {
    let len_sq = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if len_sq > 1e-8 {
        let len = len_sq.sqrt();
        [v[0] / len, v[1] / len, v[2] / len]
    } else {
        [0.0, 0.0, 0.0]
    }
}

/// Screen UV → Ray for 3D picking (in object space)
/// Returns (origin, direction) in object space.
pub fn screen_to_ray_3d(
    screen_uv: [f32; 2],
    rotation: [f32; 4],
    zoom: f32,
    pan: [f32; 2],
    screen_aspect: f32,
) -> ([f32; 3], [f32; 3]) {
    let pivot = [0.5, 0.5];
    let zoomed_uv = [
        (screen_uv[0] - pivot[0]) / zoom + pivot[0] + pan[0],
        (screen_uv[1] - pivot[1]) / zoom + pivot[1] + pan[1],
    ];

    // RADIOLOGICAL: Flip X (Patient Right on Screen Left). Flip Y (Vertical screen orientation).
    let uv = [zoomed_uv[0] - 0.5, zoomed_uv[1] - 0.5];
    let screen_pos = [-uv[0] * screen_aspect, -uv[1]];

    let cam_pos_world = [0.0, 0.0, -3.5];
    let forward = [0.0, 0.0, 1.0];
    let right = [1.0, 0.0, 0.0];
    let up = [0.0, 1.0, 0.0];

    let raw_dir = [
        forward[0] + right[0] * screen_pos[0] + up[0] * screen_pos[1],
        forward[1] + right[1] * screen_pos[0] + up[1] * screen_pos[1],
        forward[2] + right[2] * screen_pos[0] + up[2] * screen_pos[1],
    ];
    let ray_dir_world = normalize_vec3(raw_dir);

    let rot_mat = quat_to_mat3(rotation);
    let inv_rot_mat = transpose_mat3(rot_mat);

    let cam_pos_obj = rotate_vec3(inv_rot_mat, cam_pos_world);
    let ray_dir_obj = normalize_vec3(rotate_vec3(inv_rot_mat, ray_dir_world));

    (cam_pos_obj, ray_dir_obj)
}

/// Volume UV → Screen UV for 3D projection
pub fn volume_to_screen_3d(
    volume_pos: [f32; 3],
    rotation: [f32; 4],
    vol_aspects: [f32; 3],
    zoom: f32,
    pan: [f32; 2],
    screen_aspect: f32,
) -> Option<[f32; 2]> {
    let pivot = [0.5, 0.5];
    let cursor_obj = [
        (volume_pos[0] - 0.5) * vol_aspects[0],
        (volume_pos[1] - 0.5) * vol_aspects[1],
        (volume_pos[2] - 0.5) * vol_aspects[2],
    ];

    let rot_mat = quat_to_mat3(rotation);
    let cursor_world = rotate_vec3(rot_mat, cursor_obj);

    let cam_pos = [0.0, 0.0, -3.5];
    let forward = [0.0, 0.0, 1.0];
    let right = [1.0, 0.0, 0.0];
    let up = [0.0, 1.0, 0.0];

    let to_cursor = [
        cursor_world[0] - cam_pos[0],
        cursor_world[1] - cam_pos[1],
        cursor_world[2] - cam_pos[2],
    ];

    let dist_z = to_cursor[0] * forward[0] + to_cursor[1] * forward[1] + to_cursor[2] * forward[2];
    if dist_z <= 0.01 {
        return None;
    }

    let dist_x = to_cursor[0] * right[0] + to_cursor[1] * right[1] + to_cursor[2] * right[2];
    let dist_y = to_cursor[0] * up[0] + to_cursor[1] * up[1] + to_cursor[2] * up[2];

    // RADIOLOGICAL: Flip X and Y projection
    let screen_u = -(dist_x / dist_z) / screen_aspect;
    let screen_v = -(dist_y / dist_z);

    let p_uv = [screen_u + 0.5, screen_v + 0.5];

    let final_u = (p_uv[0] - pan[0] - pivot[0]) * zoom + pivot[0];
    let final_v = (p_uv[1] - pan[1] - pivot[1]) * zoom + pivot[1];

    Some([final_u, final_v])
}

/// Project axis direction to screen (for gizmo/crosshairs)
pub fn project_axis_3d(
    axis: [f32; 3],
    rotation: [f32; 4],
    vol_aspects: [f32; 3],
    screen_aspect: f32,
) -> [f32; 2] {
    let rot_mat = quat_to_mat3(rotation);
    let world_axis = [
        rot_mat[0][0] * axis[0] * vol_aspects[0]
            + rot_mat[1][0] * axis[1] * vol_aspects[1]
            + rot_mat[2][0] * axis[2] * vol_aspects[2],
        rot_mat[0][1] * axis[0] * vol_aspects[0]
            + rot_mat[1][1] * axis[1] * vol_aspects[1]
            + rot_mat[2][1] * axis[2] * vol_aspects[2],
        rot_mat[0][2] * axis[0] * vol_aspects[0]
            + rot_mat[1][2] * axis[1] * vol_aspects[1]
            + rot_mat[2][2] * axis[2] * vol_aspects[2],
    ];

    // RADIOLOGICAL: Flip X and Y projection
    [-world_axis[0] / screen_aspect, -world_axis[1]]
}

#[cfg(test)]
mod tests {
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
                // 3D Ray Projection (Perspective)
                // Represents the "crosshair_screen_pos" calculation in mode 0
                let cam_pos = [0.0, 0.0, -3.5];
                let forward = [0.0, 0.0, 1.0];
                let right = [1.0, 0.0, 0.0];
                let up = [0.0, 1.0, 0.0];

                let q = [0.0, 0.0, 0.0, 1.0]; // Identity rotation for test
                let rot_mat = quat_to_mat3(q);

                // For parity test, we assume standard Aspect Ratio Vol [1,1,1]
                let cursor_obj = [cursor[0] - 0.5, cursor[1] - 0.5, cursor[2] - 0.5];
                let cursor_world = rotate_vec3(rot_mat, cursor_obj);
                let to_cursor = [
                    cursor_world[0] - cam_pos[0],
                    cursor_world[1] - cam_pos[1],
                    cursor_world[2] - cam_pos[2],
                ];
                let dist_z = to_cursor[0] * forward[0]
                    + to_cursor[1] * forward[1]
                    + to_cursor[2] * forward[2];
                let dist_x =
                    to_cursor[0] * right[0] + to_cursor[1] * right[1] + to_cursor[2] * right[2];
                let dist_y = to_cursor[0] * up[0] + to_cursor[1] * up[1] + to_cursor[2] * up[2];

                let screen_u = -(dist_x / dist_z) / screen_aspect;
                let screen_v = -(dist_y / dist_z); // THE VERTICAL FIX
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

        let forward = Vec3::from([0.0, 0.0, 1.0]);
        let right = Vec3::from([1.0, 0.0, 0.0]);
        let up = Vec3::from([0.0, 1.0, 0.0]);
        let ray_dir_world = (forward + right * screen_pos[0] + up * screen_pos[1]).normalize();

        // Ray should point towards Patient Right (+X)
        assert!(ray_dir_world.x > 0.0);
        // Ray should point towards World Y- (Down) because cursor is at Y=0.5 (center)
        // Wait, if cursor is at center (0.5), ray should be straight forward.
        // Let's test a non-center cursor.

        let cursor_up = [0.5, 0.8, 0.5]; // Anterior (Up in identity)
        let shader_uv_up =
            shader_logic_emulation(0, [0.5, 0.5], zoom, pan, res, aspects, cursor_up);
        // Anterior (+Y) should be at Top (v < 0.5)
        assert!(shader_uv_up[1] < 0.5);

        let uv_up = [shader_uv_up[0] - 0.5, shader_uv_up[1] - 0.5];
        let screen_pos_up = [-uv_up[0], -uv_up[1]];
        let ray_up = (forward + right * screen_pos_up[0] + up * screen_pos_up[1]).normalize();
        // ray_up.y should be positive (points Up)
        assert!(ray_up.y > 0.0);
    }
}
