//! Voxel grids and slice planes: the geometry every other layer agrees on.

use glam::{DMat3, DMat4, DQuat, DVec3, DVec4, Quat, Vec3};
use thiserror::Error;

/// Stable identity for a validated voxel grid and its coordinate convention.
///
/// This intentionally stores affine bits rather than a hash so cache acceptance can remain
/// deterministic and collision-free without introducing a hashing policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GeometryIdentity {
    dimensions: [u32; 3],
    ijk_to_world_bits: [u64; 16],
}

impl GeometryIdentity {
    pub(crate) const fn new(dimensions: [u32; 3], ijk_to_world_bits: [u64; 16]) -> Self {
        Self {
            dimensions,
            ijk_to_world_bits,
        }
    }

    pub const fn dimensions(self) -> [u32; 3] {
        self.dimensions
    }

    pub const fn ijk_to_world_bits(self) -> [u64; 16] {
        self.ijk_to_world_bits
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlaneFamily {
    Axial,
    Coronal,
    Sagittal,
    Oblique,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaneDefinition {
    pub family: PlaneFamily,
    pub origin_mm: [f32; 3],
    pub u_axis_mm: [f32; 3],
    pub v_axis_mm: [f32; 3],
    pub normal_mm: [f32; 3],
}

impl PlaneDefinition {
    /// Builds a finite, non-degenerate plane frame and derives its normal from the two axes.
    pub fn new(
        family: PlaneFamily,
        origin_mm: [f32; 3],
        u_axis_mm: [f32; 3],
        v_axis_mm: [f32; 3],
    ) -> Option<Self> {
        let origin = Vec3::from_array(origin_mm);
        if !origin.is_finite() {
            return None;
        }
        let u_axis = Vec3::from_array(u_axis_mm);
        let v_axis = Vec3::from_array(v_axis_mm);
        if !u_axis.is_finite() || !v_axis.is_finite() {
            return None;
        }
        let normal = u_axis.cross(v_axis);
        if normal.length_squared() <= 1e-12 {
            return None;
        }
        Some(Self {
            family,
            origin_mm,
            u_axis_mm,
            v_axis_mm,
            normal_mm: normal.normalize().to_array(),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct VoxelData {
    pub geometry: VoxelGeometry,
    pub raw_data: Vec<u8>,
}

const GEOMETRY_EPSILON: f64 = 1.0e-12;

/// Validated voxel grid: dimensions plus an IJK-to-world affine.
///
/// Integer IJK coordinates identify voxel centres and world coordinates are millimetres. The
/// affine is the single source of truth, so reflections (for example an LAS-stored volume) and
/// shear are preserved. `spacing`, `origin`, and `orientation` are derived views for consumers
/// that still think in decomposed terms; the mapping helpers use the affine directly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoxelGeometry {
    pub(crate) dimensions: [u32; 3],
    ijk_to_world: DMat4,
    world_to_ijk: DMat4,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum VoxelGeometryError {
    #[error("geometry dimensions must be non-zero")]
    EmptyDimensions,
    #[error("geometry spacing must be finite and positive")]
    InvalidSpacing,
    #[error("geometry origin must be finite")]
    NonFiniteOrigin,
    #[error("geometry orientation must be a finite, non-zero quaternion")]
    InvalidOrientation,
    #[error("IJK-to-world affine contains non-finite values")]
    NonFiniteAffine,
    #[error("IJK-to-world affine must have final row [0, 0, 0, 1]")]
    NonAffineTransform,
    #[error("IJK-to-world affine is singular")]
    SingularAffine,
    #[error("IJK-to-world affine contains a zero-length grid axis")]
    ZeroLengthAxis,
}

impl VoxelGeometry {
    /// Builds a geometry from decomposed parts: `translate(origin) * rotate(orientation) *
    /// scale(spacing)`. Prefer [`Self::from_affine`] when a full transform is available.
    pub fn new(
        dimensions: [u32; 3],
        spacing: [f32; 3],
        origin: [f32; 3],
        orientation: [f32; 4],
    ) -> Result<Self, VoxelGeometryError> {
        if dimensions.contains(&0) {
            return Err(VoxelGeometryError::EmptyDimensions);
        }
        if spacing
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
        {
            return Err(VoxelGeometryError::InvalidSpacing);
        }
        if origin.iter().any(|value| !value.is_finite()) {
            return Err(VoxelGeometryError::NonFiniteOrigin);
        }
        let orientation_length_sq = orientation
            .into_iter()
            .map(|value| value * value)
            .sum::<f32>();
        if !orientation_length_sq.is_finite() || orientation_length_sq <= 1e-12 {
            return Err(VoxelGeometryError::InvalidOrientation);
        }
        let rotation = Quat::from_array(orientation).normalize();
        let affine = DMat4::from_scale_rotation_translation(
            DVec3::new(
                f64::from(spacing[0]),
                f64::from(spacing[1]),
                f64::from(spacing[2]),
            ),
            DQuat::from_xyzw(
                f64::from(rotation.x),
                f64::from(rotation.y),
                f64::from(rotation.z),
                f64::from(rotation.w),
            ),
            DVec3::new(
                f64::from(origin[0]),
                f64::from(origin[1]),
                f64::from(origin[2]),
            ),
        );
        Self::from_affine(dimensions, affine)
    }

    /// Builds a geometry from a full IJK-to-world affine (column-major, millimetres).
    pub fn from_affine(
        dimensions: [u32; 3],
        ijk_to_world: DMat4,
    ) -> Result<Self, VoxelGeometryError> {
        if dimensions.contains(&0) {
            return Err(VoxelGeometryError::EmptyDimensions);
        }
        if !ijk_to_world.is_finite() {
            return Err(VoxelGeometryError::NonFiniteAffine);
        }
        let final_row = DVec4::new(
            ijk_to_world.x_axis.w,
            ijk_to_world.y_axis.w,
            ijk_to_world.z_axis.w,
            ijk_to_world.w_axis.w,
        );
        if (final_row - DVec4::W)
            .abs()
            .cmpgt(DVec4::splat(GEOMETRY_EPSILON))
            .any()
        {
            return Err(VoxelGeometryError::NonAffineTransform);
        }
        let linear = linear_part(&ijk_to_world);
        if linear.determinant().abs() <= GEOMETRY_EPSILON {
            return Err(VoxelGeometryError::SingularAffine);
        }
        if [linear.x_axis, linear.y_axis, linear.z_axis]
            .into_iter()
            .any(|axis| axis.length() <= GEOMETRY_EPSILON)
        {
            return Err(VoxelGeometryError::ZeroLengthAxis);
        }
        Ok(Self {
            dimensions,
            ijk_to_world,
            world_to_ijk: ijk_to_world.inverse(),
        })
    }

    pub fn dimensions(self) -> [u32; 3] {
        self.dimensions
    }

    pub fn ijk_to_world_affine(self) -> DMat4 {
        self.ijk_to_world
    }

    pub fn world_to_ijk_affine(self) -> DMat4 {
        self.world_to_ijk
    }

    /// Stable identity of this grid and its coordinate convention, for cache acceptance.
    pub fn identity(self) -> GeometryIdentity {
        GeometryIdentity::new(
            self.dimensions,
            self.ijk_to_world
                .to_cols_array()
                .map(|value| if value == 0.0 { 0.0 } else { value })
                .map(f64::to_bits),
        )
    }

    pub fn ijk_to_world_mm(self, ijk: [f64; 3]) -> [f64; 3] {
        self.ijk_to_world
            .transform_point3(DVec3::from_array(ijk))
            .to_array()
    }

    #[cfg(test)]
    pub fn world_mm_to_ijk(self, world: [f64; 3]) -> [f64; 3] {
        self.world_to_ijk
            .transform_point3(DVec3::from_array(world))
            .to_array()
    }

    /// Whether a continuous IJK point lies inside the grid's voxel cells.
    #[cfg(test)]
    pub fn contains_voxel_center(self, ijk: [f64; 3]) -> bool {
        let point = DVec3::from_array(ijk);
        let upper = DVec3::new(
            f64::from(self.dimensions[0]) - 0.5,
            f64::from(self.dimensions[1]) - 0.5,
            f64::from(self.dimensions[2]) - 0.5,
        );
        point.cmpge(DVec3::splat(-0.5)).all() && point.cmplt(upper).all()
    }

    /// Millimetre length of one voxel step along each grid axis (always positive).
    pub fn spacing(self) -> [f32; 3] {
        let linear = linear_part(&self.ijk_to_world);
        [
            linear.x_axis.length() as f32,
            linear.y_axis.length() as f32,
            linear.z_axis.length() as f32,
        ]
    }

    /// World position of voxel `(0, 0, 0)`'s centre.
    pub fn origin(self) -> [f32; 3] {
        let translation = self.ijk_to_world.w_axis;
        [
            translation.x as f32,
            translation.y as f32,
            translation.z as f32,
        ]
    }

    /// Orthonormal frame of the grid axes in world space: the columns are the unit `i`, `j`, and
    /// `k` directions with shear removed. A reflection is kept, so the determinant is -1 for an
    /// LAS-like grid. Rotating in this frame rotates in the volume's physical (millimetre) frame.
    pub fn axis_frame(self) -> DMat3 {
        let linear = linear_part(&self.ijk_to_world);
        let unit = |axis: DVec3, fallback: DVec3| axis.try_normalize().unwrap_or(fallback);
        let x = unit(linear.x_axis, DVec3::X);
        let y_raw = unit(linear.y_axis, DVec3::Y);
        let z_raw = unit(linear.z_axis, DVec3::Z);
        let handedness = DMat3::from_cols(x, y_raw, z_raw).determinant().signum();
        let y = (y_raw - x * y_raw.dot(x))
            .try_normalize()
            .unwrap_or_else(|| x.any_orthonormal_vector());
        let z = x.cross(y) * if handedness < 0.0 { -1.0 } else { 1.0 };
        DMat3::from_cols(x, y, z)
    }

    /// Proper-rotation view of the grid axes as a quaternion `[x, y, z, w]` with `w >= 0`.
    ///
    /// A reflection is folded into the x axis and shear is removed, so this is only a
    /// display-orientation approximation for consumers that cannot use the affine. Registration
    /// and measurement must use [`Self::ijk_to_world_affine`].
    pub fn orientation(self) -> [f32; 4] {
        let mut frame = self.axis_frame();
        if frame.determinant() < 0.0 {
            frame.x_axis = -frame.x_axis;
        }
        let mut rotation = DQuat::from_mat3(&frame).normalize();
        if rotation.w < 0.0 {
            rotation = -rotation;
        }
        [
            rotation.x as f32,
            rotation.y as f32,
            rotation.z as f32,
            rotation.w as f32,
        ]
    }
}

fn linear_part(affine: &DMat4) -> DMat3 {
    DMat3::from_cols(
        affine.x_axis.truncate(),
        affine.y_axis.truncate(),
        affine.z_axis.truncate(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_voxel_geometry_constructor_rejects_invalid_orientation() {
        assert_eq!(
            VoxelGeometry::new([1, 1, 1], [1.0; 3], [0.0; 3], [0.0; 4]),
            Err(VoxelGeometryError::InvalidOrientation)
        );
    }

    #[test]
    fn test_geometry_rejects_singular_grids_instead_of_panicking_later() {
        // Spacing 1e-5 mm passes per-axis checks but its affine is numerically singular; an ROI
        // built on it used to abort the app at construction.
        assert_eq!(
            VoxelGeometry::new([2, 2, 2], [1.0e-5; 3], [0.0; 3], [0.0, 0.0, 0.0, 1.0]),
            Err(VoxelGeometryError::SingularAffine)
        );
        assert_eq!(
            VoxelGeometry::new([0, 2, 2], [1.0; 3], [0.0; 3], [0.0, 0.0, 0.0, 1.0]),
            Err(VoxelGeometryError::EmptyDimensions)
        );
    }

    #[test]
    fn test_affine_geometry_preserves_reflection_and_orientation_view_is_proper() {
        let affine = DMat4::from_cols(
            DVec4::new(-2.0, 0.0, 0.0, 0.0),
            DVec4::new(0.0, 2.0, 0.0, 0.0),
            DVec4::new(0.0, 0.0, 3.0, 0.0),
            DVec4::new(100.0, 0.0, 0.0, 1.0),
        );
        let geometry = VoxelGeometry::from_affine([10, 4, 4], affine).unwrap();

        assert_eq!(geometry.ijk_to_world_mm([9.0, 0.0, 0.0]), [82.0, 0.0, 0.0]);
        assert_eq!(geometry.spacing(), [2.0, 2.0, 3.0]);
        let quat = Quat::from_array(geometry.orientation());
        assert!(
            (quat.length() - 1.0).abs() < 1e-6,
            "orientation is a unit quaternion"
        );
        assert!(quat.w >= 0.0);
        assert_eq!(geometry.world_mm_to_ijk([82.0, 0.0, 0.0]), [9.0, 0.0, 0.0]);
    }

    #[test]
    fn test_decomposed_constructor_round_trips_spacing_origin_and_orientation() {
        let orientation = Quat::from_euler(glam::EulerRot::XYZ, 0.4, -0.25, 0.7).to_array();
        let geometry =
            VoxelGeometry::new([4, 5, 6], [0.5, 0.8, 2.0], [12.0, -4.0, 3.0], orientation).unwrap();

        let spacing = geometry.spacing();
        for (actual, expected) in spacing.iter().zip([0.5, 0.8, 2.0]) {
            assert!((actual - expected).abs() < 1e-6);
        }
        assert_eq!(geometry.origin(), [12.0, -4.0, 3.0]);
        let recovered = Quat::from_array(geometry.orientation());
        assert!(recovered.dot(Quat::from_array(orientation)).abs() > 0.99999);
    }
}
