// src/orientation.rs
//! Centralized module for volume orientation and coordinate systems.
//!
//! Handles mapping between:
//! - Volume UV (0..1, +Y=Anterior, +Z=Superior)
//! - Screen UV (0..1, Top-Left=0,0, +V=Down)
//! - World Space (Anatomical labels)

/// Anatomical slice planes for 2D viewports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    pub fn from_mode(mode: crate::model::ViewMode) -> Option<Self> {
        match mode {
            crate::model::ViewMode::Axial => Some(SlicePlane::Axial),
            crate::model::ViewMode::Coronal => Some(SlicePlane::Coronal),
            crate::model::ViewMode::Sagittal => Some(SlicePlane::Sagittal),
            _ => None,
        }
    }

    /// Convert to the shared plane-family abstraction.
    pub fn to_plane_family(self) -> crate::convert::PlaneFamily {
        match self {
            SlicePlane::Axial => crate::convert::PlaneFamily::Axial,
            SlicePlane::Coronal => crate::convert::PlaneFamily::Coronal,
            SlicePlane::Sagittal => crate::convert::PlaneFamily::Sagittal,
        }
    }

    /// Convert from the shared plane-family abstraction.
    ///
    /// Returns `None` for non-orthogonal families (for now, `Oblique`).
    #[cfg(test)]
    pub fn from_plane_family(family: crate::convert::PlaneFamily) -> Option<Self> {
        match family {
            crate::convert::PlaneFamily::Axial => Some(SlicePlane::Axial),
            crate::convert::PlaneFamily::Coronal => Some(SlicePlane::Coronal),
            crate::convert::PlaneFamily::Sagittal => Some(SlicePlane::Sagittal),
            crate::convert::PlaneFamily::Oblique => None,
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

/// Anatomical letter (`R`, `L`, `A`, `P`, `S`, `I`) of the dominant axis of a world direction, in
/// the NIfTI RAS+ world convention (+x right, +y anterior, +z superior). `None` for a zero vector.
pub fn anatomical_letter(world_direction: [f64; 3]) -> Option<char> {
    let [x, y, z] = world_direction;
    let (ax, ay, az) = (x.abs(), y.abs(), z.abs());
    if !world_direction.iter().all(|value| value.is_finite()) || ax.max(ay).max(az) <= 1e-9 {
        return None;
    }
    Some(if ax >= ay && ax >= az {
        if x >= 0.0 {
            'R'
        } else {
            'L'
        }
    } else if ay >= az {
        if y >= 0.0 {
            'A'
        } else {
            'P'
        }
    } else if z >= 0.0 {
        'S'
    } else {
        'I'
    })
}

/// The letter for the opposite direction.
pub fn opposite_letter(letter: char) -> char {
    match letter {
        'R' => 'L',
        'L' => 'R',
        'A' => 'P',
        'P' => 'A',
        'S' => 'I',
        'I' => 'S',
        other => other,
    }
}

/// Anatomical letters of the positive and negative direction of each voxel index axis, derived
/// from the grid's affine so reflected (LAS/LPS) and permuted volumes are labelled truthfully.
pub fn index_axis_letters(geometry: crate::model::VoxelGeometry) -> [[char; 2]; 3] {
    let affine = geometry.ijk_to_world_affine();
    let columns = [affine.x_axis, affine.y_axis, affine.z_axis];
    columns.map(|column| {
        let positive = anatomical_letter([column.x, column.y, column.z]).unwrap_or('?');
        [positive, opposite_letter(positive)]
    })
}

/// Letters for the four edges of a 2D slice view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgeLetters {
    pub left: char,
    pub right: char,
    pub top: char,
    pub bottom: char,
}

impl SlicePlane {
    /// Voxel index axes shown toward the screen's left and top edges. Both increase toward
    /// the edge (see `screen_uv_to_volume`).
    fn left_top_index_axes(&self) -> (usize, usize) {
        match self {
            SlicePlane::Axial => (0, 1),
            SlicePlane::Coronal => (0, 2),
            SlicePlane::Sagittal => (1, 2),
        }
    }

    /// Anatomical letters of this view's edges for the given grid.
    pub fn edge_letters(&self, geometry: crate::model::VoxelGeometry) -> EdgeLetters {
        let letters = index_axis_letters(geometry);
        let (left_axis, top_axis) = self.left_top_index_axes();
        EdgeLetters {
            left: letters[left_axis][0],
            right: letters[left_axis][1],
            top: letters[top_axis][0],
            bottom: letters[top_axis][1],
        }
    }

    /// Anatomical plane this index-space view actually shows, from the world direction of its
    /// depth axis: `"axial"`, `"coronal"`, or `"sagittal"`.
    pub fn anatomical_plane_name(&self, geometry: crate::model::VoxelGeometry) -> &'static str {
        match index_axis_letters(geometry)[self.depth_axis()][0] {
            'R' | 'L' => "sagittal",
            'A' | 'P' => "coronal",
            _ => "axial",
        }
    }
}

/// Base rotation to make Superior UP in the default 3D view.
///
/// Screen projection flips Y, so this must be -90° around X: object +Z maps to world +Y
/// and therefore to screen-up.
pub const BASE_ROTATION: [f32; 4] = [
    -std::f32::consts::FRAC_1_SQRT_2,
    0.0,
    0.0,
    std::f32::consts::FRAC_1_SQRT_2,
]; // -90° rotation around X

/// Half-height of the default orthographic 3D view in normalized physical-volume units.
/// Must match `ORTHOGRAPHIC_VIEW_SCALE` in `src/shaders/shader.wgsl`.
pub const ORTHOGRAPHIC_VIEW_SCALE: f32 = 3.5;

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

    let cam_pos_world = [
        screen_pos[0] * ORTHOGRAPHIC_VIEW_SCALE,
        screen_pos[1] * ORTHOGRAPHIC_VIEW_SCALE,
        -ORTHOGRAPHIC_VIEW_SCALE,
    ];
    let forward = [0.0, 0.0, 1.0];
    let ray_dir_world = forward;

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

    // Orthographic projection deliberately ignores depth, so rotating a volume does not taper it.
    // RADIOLOGICAL: flip X and Y projection.
    let screen_u = -cursor_world[0] / (ORTHOGRAPHIC_VIEW_SCALE * screen_aspect);
    let screen_v = -cursor_world[1] / ORTHOGRAPHIC_VIEW_SCALE;

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
mod tests;
