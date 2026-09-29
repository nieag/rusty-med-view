use crate::app::roi::VoxelGeometry;
use crate::convert::coord_mapping::{volume_uv_to_voxel_index, voxel_index_to_volume_uv};
use glam::{DMat3, DMat4, DVec3, DVec4, Mat3, Quat, Vec3};
use thiserror::Error;

const GEOMETRY_EPSILON: f64 = 1.0e-12;

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
    pub const fn dimensions(self) -> [u32; 3] {
        self.dimensions
    }

    pub const fn ijk_to_world_bits(self) -> [u64; 16] {
        self.ijk_to_world_bits
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum RoiGeometryError {
    #[error("ROI geometry dimensions must be non-zero")]
    EmptyDimensions,
    #[error("ROI IJK-to-world affine contains non-finite values")]
    NonFiniteAffine,
    #[error("ROI IJK-to-world affine must have final row [0, 0, 0, 1]")]
    NonAffineTransform,
    #[error("ROI IJK-to-world affine is singular")]
    SingularAffine,
    #[error("ROI IJK-to-world affine contains a zero-length grid axis")]
    ZeroLengthAxis,
    #[error("legacy ROI geometry contains non-finite values")]
    NonFiniteLegacyParts,
    #[error("legacy ROI geometry has non-positive spacing")]
    InvalidLegacySpacing,
    #[error("legacy ROI geometry has an invalid orientation quaternion")]
    InvalidLegacyOrientation,
}

/// Immutable validated reference grid for an ROI.
///
/// Integer IJK coordinates identify voxel centres. World coordinates are millimetres. The
/// inverse and identity are derived once so callers cannot silently reinterpret a grid.
#[derive(Debug, Clone, PartialEq)]
pub struct RoiGeometry {
    dimensions: [u32; 3],
    ijk_to_world: DMat4,
    world_to_ijk: DMat4,
    identity: GeometryIdentity,
}

impl RoiGeometry {
    pub fn new(dimensions: [u32; 3], ijk_to_world: DMat4) -> Result<Self, RoiGeometryError> {
        if dimensions.contains(&0) {
            return Err(RoiGeometryError::EmptyDimensions);
        }
        if !ijk_to_world.is_finite() {
            return Err(RoiGeometryError::NonFiniteAffine);
        }

        let final_row = DVec4::new(
            ijk_to_world.x_axis.w,
            ijk_to_world.y_axis.w,
            ijk_to_world.z_axis.w,
            ijk_to_world.w_axis.w,
        );
        if !approximately_equal_dvec4(final_row, DVec4::W) {
            return Err(RoiGeometryError::NonAffineTransform);
        }

        let linear = DMat3::from_cols(
            ijk_to_world.x_axis.truncate(),
            ijk_to_world.y_axis.truncate(),
            ijk_to_world.z_axis.truncate(),
        );
        if linear.determinant().abs() <= GEOMETRY_EPSILON {
            return Err(RoiGeometryError::SingularAffine);
        }
        if [linear.x_axis, linear.y_axis, linear.z_axis]
            .into_iter()
            .any(|axis| axis.length() <= GEOMETRY_EPSILON)
        {
            return Err(RoiGeometryError::ZeroLengthAxis);
        }

        let identity = GeometryIdentity {
            dimensions,
            ijk_to_world_bits: ijk_to_world
                .to_cols_array()
                .map(|value| if value == 0.0 { 0.0 } else { value })
                .map(f64::to_bits),
        };
        Ok(Self {
            dimensions,
            world_to_ijk: ijk_to_world.inverse(),
            ijk_to_world,
            identity,
        })
    }

    /// Converts the legacy decomposed geometry while it is being migrated out of the runtime.
    pub fn from_legacy_parts(
        dimensions: [u32; 3],
        spacing: [f32; 3],
        origin: [f32; 3],
        orientation: [f32; 4],
    ) -> Result<Self, RoiGeometryError> {
        if !spacing
            .into_iter()
            .chain(origin)
            .chain(orientation)
            .all(f32::is_finite)
        {
            return Err(RoiGeometryError::NonFiniteLegacyParts);
        }
        if spacing.into_iter().any(|value| value <= 0.0) {
            return Err(RoiGeometryError::InvalidLegacySpacing);
        }
        let orientation = Quat::from_array(orientation);
        let length_squared = orientation.length_squared();
        if length_squared <= 1.0e-12 || !length_squared.is_finite() {
            return Err(RoiGeometryError::InvalidLegacyOrientation);
        }
        let rotation = Mat3::from_quat(orientation.normalize());
        let axes = [
            DVec3::from(rotation.x_axis) * f64::from(spacing[0]),
            DVec3::from(rotation.y_axis) * f64::from(spacing[1]),
            DVec3::from(rotation.z_axis) * f64::from(spacing[2]),
        ];
        Self::new(
            dimensions,
            DMat4::from_cols(
                axes[0].extend(0.0),
                axes[1].extend(0.0),
                axes[2].extend(0.0),
                DVec3::new(
                    f64::from(origin[0]),
                    f64::from(origin[1]),
                    f64::from(origin[2]),
                )
                .extend(1.0),
            ),
        )
    }

    pub const fn dimensions(&self) -> [u32; 3] {
        self.dimensions
    }

    pub const fn ijk_to_world_affine(&self) -> DMat4 {
        self.ijk_to_world
    }

    pub const fn world_to_ijk_affine(&self) -> DMat4 {
        self.world_to_ijk
    }

    pub const fn identity(&self) -> GeometryIdentity {
        self.identity
    }

    pub fn ijk_to_world_mm(&self, ijk: [f64; 3]) -> [f64; 3] {
        self.ijk_to_world
            .transform_point3(DVec3::from_array(ijk))
            .to_array()
    }

    pub fn world_mm_to_ijk(&self, world: [f64; 3]) -> [f64; 3] {
        self.world_to_ijk
            .transform_point3(DVec3::from_array(world))
            .to_array()
    }

    pub fn contains_voxel_center(&self, ijk: [f64; 3]) -> bool {
        let point = DVec3::from_array(ijk);
        let upper = DVec3::new(
            f64::from(self.dimensions[0]) - 0.5,
            f64::from(self.dimensions[1]) - 0.5,
            f64::from(self.dimensions[2]) - 0.5,
        );
        point.cmpge(DVec3::splat(-0.5)).all() && point.cmplt(upper).all()
    }
}

fn approximately_equal_dvec4(left: DVec4, right: DVec4) -> bool {
    (left - right)
        .abs()
        .cmple(DVec4::splat(GEOMETRY_EPSILON))
        .all()
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportMapping {
    pub zoom: f32,
    pub pan: [f32; 2],
    pub pivot: [f32; 2],
    pub screen_aspect: f32,
}

fn normalized_orientation(orientation: [f32; 4]) -> Quat {
    let quat = Quat::from_array(orientation);
    let length_sq = quat.length_squared();
    if length_sq.is_finite() && length_sq > 1e-12 {
        quat.normalize()
    } else {
        Quat::IDENTITY
    }
}

fn validated_orientation(orientation: [f32; 4]) -> Option<Quat> {
    let quat = Quat::from_array(orientation);
    let length_sq = quat.length_squared();
    (length_sq.is_finite() && length_sq > 1e-12).then(|| quat.normalize())
}

fn world_direction_from_index_direction(
    index_direction: Vec3,
    geometry: VoxelGeometry,
) -> Option<Vec3> {
    if index_direction.length_squared() <= 1e-12 {
        return None;
    }

    let spacing = Vec3::new(
        geometry.spacing[0],
        geometry.spacing[1],
        geometry.spacing[2],
    );
    let scaled = index_direction * spacing;
    let world = validated_orientation(geometry.orientation)? * scaled;
    if world.length_squared() <= 1e-12 {
        return None;
    }

    Some(world.normalize())
}

fn index_direction_from_world_direction(
    world_direction: Vec3,
    geometry: VoxelGeometry,
) -> Option<Vec3> {
    if world_direction.length_squared() <= 1e-12 {
        return None;
    }

    let local = validated_orientation(geometry.orientation)?.inverse() * world_direction;
    let mut components = [0.0; 3];
    for axis in 0..3 {
        let spacing = geometry.spacing[axis];
        if spacing.abs() <= 1e-6 {
            return None;
        }
        components[axis] = local[axis] / spacing;
    }

    let dir = Vec3::from_array(components);
    if dir.length_squared() <= 1e-12 {
        None
    } else {
        Some(dir.normalize())
    }
}

fn orthonormalize_plane_axes(u_axis: Vec3, v_axis: Vec3) -> Option<(Vec3, Vec3, Vec3)> {
    if u_axis.length_squared() <= 1e-12 || v_axis.length_squared() <= 1e-12 {
        return None;
    }

    let u = u_axis.normalize();
    let v_orthogonal = v_axis - u * v_axis.dot(u);
    if v_orthogonal.length_squared() <= 1e-12 {
        return None;
    }
    let v = v_orthogonal.normalize();
    let normal = u.cross(v);
    if normal.length_squared() <= 1e-12 {
        return None;
    }
    Some((u, v, normal.normalize()))
}

pub fn orthogonal_plane_from_volume_uv(
    family: PlaneFamily,
    cursor_uv: [f32; 3],
    geometry: VoxelGeometry,
) -> Option<PlaneDefinition> {
    let (u_index_dir, v_index_dir) = match family {
        PlaneFamily::Axial => (Vec3::new(-1.0, 0.0, 0.0), Vec3::new(0.0, -1.0, 0.0)),
        PlaneFamily::Coronal => (Vec3::new(-1.0, 0.0, 0.0), Vec3::new(0.0, 0.0, -1.0)),
        PlaneFamily::Sagittal => (Vec3::new(0.0, -1.0, 0.0), Vec3::new(0.0, 0.0, -1.0)),
        PlaneFamily::Oblique => return None,
    };

    let u_world = world_direction_from_index_direction(u_index_dir, geometry)?;
    let v_world = world_direction_from_index_direction(v_index_dir, geometry)?;
    let (u_axis, v_axis, _) = orthonormalize_plane_axes(u_world, v_world)?;

    PlaneDefinition::new(
        family,
        volume_uv_to_world_mm(cursor_uv, geometry),
        u_axis.to_array(),
        v_axis.to_array(),
    )
}

pub fn oblique_plane_from_view_rotation(
    cursor_uv: [f32; 3],
    rotation: [f32; 4],
    geometry: VoxelGeometry,
) -> Option<PlaneDefinition> {
    let view_rotation = normalized_orientation(rotation);
    let u_index_dir = (view_rotation * Vec3::X).normalize_or_zero();
    let v_index_dir = (view_rotation * Vec3::Y).normalize_or_zero();
    if u_index_dir.length_squared() <= 1e-12 || v_index_dir.length_squared() <= 1e-12 {
        return None;
    }

    let u_world = world_direction_from_index_direction(u_index_dir, geometry)?;
    let v_world = world_direction_from_index_direction(v_index_dir, geometry)?;
    let u_axis = u_world.normalize();
    let v_axis = v_world.normalize();
    let normal_vec = u_axis.cross(v_axis);
    if normal_vec.length_squared() <= 1e-12 {
        return None;
    }
    PlaneDefinition::new(
        PlaneFamily::Oblique,
        volume_uv_to_world_mm(cursor_uv, geometry),
        u_axis.to_array(),
        v_axis.to_array(),
    )
}

pub fn plane_local_mm_to_world_mm(local: [f32; 2], plane: PlaneDefinition) -> [f32; 3] {
    let origin = Vec3::from_array(plane.origin_mm);
    let u_axis = Vec3::from_array(plane.u_axis_mm);
    let v_axis = Vec3::from_array(plane.v_axis_mm);
    (origin + u_axis * local[0] + v_axis * local[1]).to_array()
}

pub fn world_mm_to_plane_local_mm(world: [f32; 3], plane: PlaneDefinition) -> [f32; 2] {
    let delta = Vec3::from_array(world) - Vec3::from_array(plane.origin_mm);
    let u_axis = Vec3::from_array(plane.u_axis_mm);
    let v_axis = Vec3::from_array(plane.v_axis_mm);
    [delta.dot(u_axis), delta.dot(v_axis)]
}

pub fn reproject_plane_local_mm(
    local: [f32; 2],
    source_plane: PlaneDefinition,
    target_plane: PlaneDefinition,
) -> [f32; 2] {
    let world = plane_local_mm_to_world_mm(local, source_plane);
    world_mm_to_plane_local_mm(world, target_plane)
}

fn orthogonal_family(plane: PlaneDefinition) -> Option<PlaneFamily> {
    match plane.family {
        PlaneFamily::Axial | PlaneFamily::Coronal | PlaneFamily::Sagittal => Some(plane.family),
        PlaneFamily::Oblique => None,
    }
}

fn family_depth_axis(family: PlaneFamily) -> usize {
    match family {
        PlaneFamily::Axial => 2,
        PlaneFamily::Coronal => 1,
        PlaneFamily::Sagittal => 0,
        PlaneFamily::Oblique => unreachable!("oblique family has no fixed depth axis"),
    }
}

fn plane_slice_aspect(family: PlaneFamily, geometry: VoxelGeometry) -> Option<f32> {
    let extents = [
        geometry.dimensions[0] as f32 * geometry.spacing[0],
        geometry.dimensions[1] as f32 * geometry.spacing[1],
        geometry.dimensions[2] as f32 * geometry.spacing[2],
    ];

    let (u, v) = match family {
        PlaneFamily::Axial => (extents[0], extents[1]),
        PlaneFamily::Coronal => (extents[0], extents[2]),
        PlaneFamily::Sagittal => (extents[1], extents[2]),
        PlaneFamily::Oblique => return None,
    };

    if !u.is_finite() || !v.is_finite() || v.abs() <= 1e-6 {
        None
    } else {
        Some(u / v)
    }
}

fn normalized_volume_extents(geometry: VoxelGeometry) -> [f32; 3] {
    let extents = [
        geometry.dimensions[0] as f32 * geometry.spacing[0],
        geometry.dimensions[1] as f32 * geometry.spacing[1],
        geometry.dimensions[2] as f32 * geometry.spacing[2],
    ];
    let max_extent = extents[0].max(extents[1]).max(extents[2]).max(1e-6);
    [
        extents[0] / max_extent,
        extents[1] / max_extent,
        extents[2] / max_extent,
    ]
}

fn oblique_uv_basis_and_lengths(
    plane: PlaneDefinition,
    geometry: VoxelGeometry,
) -> Option<(Vec3, Vec3, f32, f32)> {
    let u_dir = index_direction_from_world_direction(Vec3::from_array(plane.u_axis_mm), geometry)?;
    let v_dir = index_direction_from_world_direction(Vec3::from_array(plane.v_axis_mm), geometry)?;

    let extents = normalized_volume_extents(geometry);
    let lu = Vec3::new(
        u_dir.x * extents[0],
        u_dir.y * extents[1],
        u_dir.z * extents[2],
    )
    .length()
    .max(1e-3);
    let lv = Vec3::new(
        v_dir.x * extents[0],
        v_dir.y * extents[1],
        v_dir.z * extents[2],
    )
    .length()
    .max(1e-3);
    Some((u_dir, v_dir, lu, lv))
}

pub fn oblique_volume_uv_basis_and_lengths(
    plane: PlaneDefinition,
    geometry: VoxelGeometry,
) -> Option<([f32; 3], [f32; 3], f32, f32)> {
    if plane.family != PlaneFamily::Oblique {
        return None;
    }
    let (u_dir, v_dir, u_length, v_length) = oblique_uv_basis_and_lengths(plane, geometry)?;
    Some((u_dir.to_array(), v_dir.to_array(), u_length, v_length))
}

fn screen_uv_to_volume_uv_for_family(
    family: PlaneFamily,
    screen_uv: [f32; 2],
    depth: f32,
) -> [f32; 3] {
    match family {
        PlaneFamily::Axial => [1.0 - screen_uv[0], 1.0 - screen_uv[1], depth],
        PlaneFamily::Coronal => [1.0 - screen_uv[0], depth, 1.0 - screen_uv[1]],
        PlaneFamily::Sagittal => [depth, 1.0 - screen_uv[0], 1.0 - screen_uv[1]],
        PlaneFamily::Oblique => unreachable!("oblique family requires different mapping"),
    }
}

fn volume_uv_to_screen_uv_for_family(family: PlaneFamily, volume_uv: [f32; 3]) -> [f32; 2] {
    match family {
        PlaneFamily::Axial => [1.0 - volume_uv[0], 1.0 - volume_uv[1]],
        PlaneFamily::Coronal => [1.0 - volume_uv[0], 1.0 - volume_uv[2]],
        PlaneFamily::Sagittal => [1.0 - volume_uv[1], 1.0 - volume_uv[2]],
        PlaneFamily::Oblique => unreachable!("oblique family requires different mapping"),
    }
}

pub fn viewport_uv_to_volume_uv(
    viewport_uv: [f32; 2],
    plane: PlaneDefinition,
    geometry: VoxelGeometry,
    mapping: ViewportMapping,
) -> Option<[f32; 3]> {
    let zoom = mapping.zoom.max(1e-6);
    match plane.family {
        PlaneFamily::Axial | PlaneFamily::Coronal | PlaneFamily::Sagittal => {
            let family = orthogonal_family(plane)?;
            let slice_aspect = plane_slice_aspect(family, geometry)?;
            let k = mapping.screen_aspect / slice_aspect;

            let screen_uv = [
                ((viewport_uv[0] - mapping.pivot[0]) * k) / zoom
                    + mapping.pivot[0]
                    + mapping.pan[0],
                (viewport_uv[1] - mapping.pivot[1]) / zoom + mapping.pivot[1] + mapping.pan[1],
            ];

            let depth_axis = family_depth_axis(family);
            let depth = world_mm_to_volume_uv(plane.origin_mm, geometry)[depth_axis];
            Some(screen_uv_to_volume_uv_for_family(family, screen_uv, depth))
        }
        PlaneFamily::Oblique => {
            let (u_dir, v_dir, lu, lv) = oblique_uv_basis_and_lengths(plane, geometry)?;
            let slice_aspect = lu / lv;
            let k = mapping.screen_aspect / slice_aspect;
            let screen_uv = [
                ((viewport_uv[0] - mapping.pivot[0]) * k) / zoom
                    + mapping.pivot[0]
                    + mapping.pan[0],
                (viewport_uv[1] - mapping.pivot[1]) / zoom + mapping.pivot[1] + mapping.pan[1],
            ];

            let du = (0.5 - screen_uv[0]) * lu;
            let dv = (0.5 - screen_uv[1]) * lv;
            let origin_uv = Vec3::from_array(world_mm_to_volume_uv(plane.origin_mm, geometry));
            Some((origin_uv + u_dir * du + v_dir * dv).to_array())
        }
    }
}

pub fn volume_uv_to_viewport_uv(
    volume_uv: [f32; 3],
    plane: PlaneDefinition,
    geometry: VoxelGeometry,
    mapping: ViewportMapping,
) -> Option<[f32; 2]> {
    match plane.family {
        PlaneFamily::Axial | PlaneFamily::Coronal | PlaneFamily::Sagittal => {
            let family = orthogonal_family(plane)?;
            let slice_aspect = plane_slice_aspect(family, geometry)?;
            let k = mapping.screen_aspect / slice_aspect;
            let screen_uv = volume_uv_to_screen_uv_for_family(family, volume_uv);

            Some([
                ((screen_uv[0] - mapping.pivot[0] - mapping.pan[0]) * mapping.zoom / k)
                    + mapping.pivot[0],
                ((screen_uv[1] - mapping.pivot[1] - mapping.pan[1]) * mapping.zoom)
                    + mapping.pivot[1],
            ])
        }
        PlaneFamily::Oblique => {
            let (u_dir, v_dir, lu, lv) = oblique_uv_basis_and_lengths(plane, geometry)?;
            let slice_aspect = lu / lv;
            let k = mapping.screen_aspect / slice_aspect;
            let origin_uv = Vec3::from_array(world_mm_to_volume_uv(plane.origin_mm, geometry));
            let delta = Vec3::from_array(volume_uv) - origin_uv;
            let du = delta.dot(u_dir);
            let dv = delta.dot(v_dir);
            let screen_uv = [0.5 - du / lu, 0.5 - dv / lv];

            Some([
                ((screen_uv[0] - mapping.pivot[0] - mapping.pan[0]) * mapping.zoom / k)
                    + mapping.pivot[0],
                ((screen_uv[1] - mapping.pivot[1] - mapping.pan[1]) * mapping.zoom)
                    + mapping.pivot[1],
            ])
        }
    }
}

pub fn viewport_uv_to_plane_local_mm(
    viewport_uv: [f32; 2],
    plane: PlaneDefinition,
    geometry: VoxelGeometry,
    mapping: ViewportMapping,
) -> Option<[f32; 2]> {
    let volume_uv = viewport_uv_to_volume_uv(viewport_uv, plane, geometry, mapping)?;
    let world_mm = volume_uv_to_world_mm(volume_uv, geometry);
    Some(world_mm_to_plane_local_mm(world_mm, plane))
}

pub fn plane_local_mm_to_viewport_uv(
    local_mm: [f32; 2],
    plane: PlaneDefinition,
    geometry: VoxelGeometry,
    mapping: ViewportMapping,
) -> Option<[f32; 2]> {
    let world_mm = plane_local_mm_to_world_mm(local_mm, plane);
    let volume_uv = world_mm_to_volume_uv(world_mm, geometry);
    volume_uv_to_viewport_uv(volume_uv, plane, geometry, mapping)
}

pub fn voxel_index_to_world_mm(index: [f32; 3], geometry: VoxelGeometry) -> [f32; 3] {
    let orientation = normalized_orientation(geometry.orientation);
    let scaled = Vec3::new(
        index[0] * geometry.spacing[0],
        index[1] * geometry.spacing[1],
        index[2] * geometry.spacing[2],
    );
    let translated = orientation * scaled
        + Vec3::new(geometry.origin[0], geometry.origin[1], geometry.origin[2]);
    translated.to_array()
}

pub fn world_mm_to_voxel_index(world: [f32; 3], geometry: VoxelGeometry) -> [f32; 3] {
    let orientation = normalized_orientation(geometry.orientation);
    let world_vec = Vec3::new(world[0], world[1], world[2]);
    let origin_vec = Vec3::new(geometry.origin[0], geometry.origin[1], geometry.origin[2]);
    let local = orientation.inverse() * (world_vec - origin_vec);

    let mut index = [0.0; 3];
    for axis in 0..3 {
        let spacing = geometry.spacing[axis];
        if spacing.abs() > 1e-6 {
            index[axis] = local[axis] / spacing;
        }
    }
    index
}

pub fn volume_uv_to_world_mm(uv: [f32; 3], geometry: VoxelGeometry) -> [f32; 3] {
    let index = volume_uv_to_voxel_index(uv, geometry.dimensions);
    voxel_index_to_world_mm(index, geometry)
}

pub fn world_mm_to_volume_uv(world: [f32; 3], geometry: VoxelGeometry) -> [f32; 3] {
    let index = world_mm_to_voxel_index(world, geometry);
    voxel_index_to_volume_uv(index, geometry.dimensions)
}

pub fn index_space_affine_from_src_to_dst(
    src: VoxelGeometry,
    dst: VoxelGeometry,
) -> Option<[[f32; 4]; 3]> {
    if src.spacing.into_iter().any(|spacing| spacing.abs() <= 1e-6)
        || dst.spacing.into_iter().any(|spacing| spacing.abs() <= 1e-6)
    {
        return None;
    }

    let src_rot = validated_orientation(src.orientation)?;
    let dst_inv_rot = validated_orientation(dst.orientation)?.inverse();

    let src_scale = Mat3::from_diagonal(Vec3::from_array(src.spacing));
    let dst_inv_scale = Mat3::from_diagonal(Vec3::new(
        1.0 / dst.spacing[0],
        1.0 / dst.spacing[1],
        1.0 / dst.spacing[2],
    ));

    let linear = dst_inv_scale * Mat3::from_quat(dst_inv_rot * src_rot) * src_scale;
    let translation = dst_inv_scale
        * (dst_inv_rot * (Vec3::from_array(src.origin) - Vec3::from_array(dst.origin)));

    let cols = linear.to_cols_array_2d();
    Some([
        [cols[0][0], cols[1][0], cols[2][0], translation[0]],
        [cols[0][1], cols[1][1], cols[2][1], translation[1]],
        [cols[0][2], cols[1][2], cols[2][2], translation[2]],
    ])
}

pub fn transform_index_with_affine(index: [f32; 3], affine: [[f32; 4]; 3]) -> [f32; 3] {
    [
        affine[0][0] * index[0] + affine[0][1] * index[1] + affine[0][2] * index[2] + affine[0][3],
        affine[1][0] * index[0] + affine[1][1] * index[1] + affine[1][2] * index[2] + affine[1][3],
        affine[2][0] * index[0] + affine[2][1] * index[1] + affine[2][2] * index[2] + affine[2][3],
    ]
}

#[cfg(test)]
mod tests;
