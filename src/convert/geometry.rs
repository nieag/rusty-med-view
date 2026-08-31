use crate::app::roi::VoxelGeometry;
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
    let world = normalized_orientation(geometry.orientation) * scaled;
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

    let local = normalized_orientation(geometry.orientation).inverse() * world_direction;
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
    let (u_axis, v_axis, normal) = orthonormalize_plane_axes(u_world, v_world)?;

    Some(PlaneDefinition {
        family,
        origin_mm: volume_uv_to_world_mm(cursor_uv, geometry),
        u_axis_mm: u_axis.to_array(),
        v_axis_mm: v_axis.to_array(),
        normal_mm: normal.to_array(),
    })
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
    let normal = normal_vec.normalize();

    Some(PlaneDefinition {
        family: PlaneFamily::Oblique,
        origin_mm: volume_uv_to_world_mm(cursor_uv, geometry),
        u_axis_mm: u_axis.to_array(),
        v_axis_mm: v_axis.to_array(),
        normal_mm: normal.to_array(),
    })
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

pub fn volume_uv_to_voxel_index(uv: [f32; 3], dimensions: [u32; 3]) -> [f32; 3] {
    let mut index = [0.0; 3];
    for axis in 0..3 {
        if dimensions[axis] > 1 {
            index[axis] = uv[axis] * (dimensions[axis] - 1) as f32;
        }
    }
    index
}

pub fn voxel_index_to_volume_uv(index: [f32; 3], dimensions: [u32; 3]) -> [f32; 3] {
    let mut uv = [0.0; 3];
    for axis in 0..3 {
        if dimensions[axis] > 1 {
            uv[axis] = index[axis] / (dimensions[axis] - 1) as f32;
        }
    }
    uv
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

    let src_rot = normalized_orientation(src.orientation);
    let dst_inv_rot = normalized_orientation(dst.orientation).inverse();

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

pub fn sample_index_from_volume_uv(uv: [f32; 3], dimensions: [u32; 3]) -> [u32; 3] {
    let mut sample = [0; 3];
    for axis in 0..3 {
        let dim = dimensions[axis];
        if dim == 0 {
            continue;
        }
        let scaled = (uv[axis] * dim as f32).floor();
        sample[axis] = scaled.clamp(0.0, (dim - 1) as f32) as u32;
    }
    sample
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(lhs: [f32; 3], rhs: [f32; 3], epsilon: f32) -> bool {
        lhs.into_iter()
            .zip(rhs)
            .all(|(left, right)| (left - right).abs() <= epsilon)
    }

    fn approx_eq_scalar(lhs: f32, rhs: f32, epsilon: f32) -> bool {
        (lhs - rhs).abs() <= epsilon
    }

    fn approx_eq2(lhs: [f32; 2], rhs: [f32; 2], epsilon: f32) -> bool {
        lhs.into_iter()
            .zip(rhs)
            .all(|(left, right)| (left - right).abs() <= epsilon)
    }

    fn identity_geometry() -> VoxelGeometry {
        VoxelGeometry {
            dimensions: [10, 10, 10],
            spacing: [1.0, 1.0, 1.0],
            origin: [0.0, 0.0, 0.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        }
    }

    fn default_mapping() -> ViewportMapping {
        ViewportMapping {
            zoom: 1.4,
            pan: [0.03, -0.04],
            pivot: [0.5, 0.5],
            screen_aspect: 16.0 / 10.0,
        }
    }

    #[test]
    fn test_volume_uv_voxel_index_roundtrip_with_standard_dimensions() {
        let dimensions = [17, 9, 5];
        let uv = [0.3, 0.8, 0.5];
        let index = volume_uv_to_voxel_index(uv, dimensions);
        let uv_roundtrip = voxel_index_to_volume_uv(index, dimensions);
        assert!(approx_eq(uv, uv_roundtrip, 1e-6));
    }

    #[test]
    fn test_volume_uv_voxel_index_roundtrip_handles_zero_and_one_dimensions() {
        let dimensions = [0, 1, 8];
        let uv = [0.7, 0.2, 0.5];
        let index = volume_uv_to_voxel_index(uv, dimensions);
        assert_eq!(index[0], 0.0);
        assert_eq!(index[1], 0.0);
        assert!((index[2] - 3.5).abs() < 1e-6);

        let uv_roundtrip = voxel_index_to_volume_uv(index, dimensions);
        assert_eq!(uv_roundtrip[0], 0.0);
        assert_eq!(uv_roundtrip[1], 0.0);
        assert!((uv_roundtrip[2] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_voxel_world_roundtrip_with_origin_spacing_and_rotation() {
        let orientation = Quat::from_euler(glam::EulerRot::XYZ, 0.4, -0.25, 0.7).to_array();
        let geometry = VoxelGeometry {
            dimensions: [32, 24, 16],
            spacing: [0.5, 0.8, 2.0],
            origin: [12.0, -4.0, 3.0],
            orientation,
        };
        let index = [6.5, 3.25, 2.0];
        let world = voxel_index_to_world_mm(index, geometry);
        let index_roundtrip = world_mm_to_voxel_index(world, geometry);
        assert!(approx_eq(index, index_roundtrip, 1e-5));
    }

    #[test]
    fn test_identity_geometry_maps_origin_and_index_as_expected() {
        let geometry = VoxelGeometry {
            dimensions: [8, 8, 8],
            spacing: [0.5, 2.0, 1.0],
            origin: [10.0, -1.0, 3.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };

        let world_at_zero = voxel_index_to_world_mm([0.0, 0.0, 0.0], geometry);
        assert_eq!(world_at_zero, [10.0, -1.0, 3.0]);

        let world = voxel_index_to_world_mm([2.0, 3.0, 4.0], geometry);
        assert_eq!(world, [11.0, 5.0, 7.0]);

        let roundtrip = world_mm_to_voxel_index(world, geometry);
        assert!(approx_eq(roundtrip, [2.0, 3.0, 4.0], 1e-6));
    }

    #[test]
    fn test_sample_index_from_volume_uv_preserves_floor_and_clamp_behavior() {
        let sample = sample_index_from_volume_uv([1.2, -0.1, 0.999], [4, 4, 4]);
        assert_eq!(sample, [3, 0, 3]);

        let sample_with_zero_dim = sample_index_from_volume_uv([0.5, 0.5, 0.5], [0, 1, 2]);
        assert_eq!(sample_with_zero_dim, [0, 0, 1]);
    }

    #[test]
    fn test_index_space_affine_identity_geometry_maps_index_to_itself() {
        let geometry = identity_geometry();
        let affine = index_space_affine_from_src_to_dst(geometry, geometry).unwrap();
        let index = [3.25, 7.5, 1.0];
        let mapped = transform_index_with_affine(index, affine);
        assert!(approx_eq(mapped, index, 1e-6));
    }

    #[test]
    fn test_index_space_affine_matches_world_mapping_for_shifted_geometry() {
        let src = VoxelGeometry {
            dimensions: [64, 48, 24],
            spacing: [0.7, 1.1, 2.0],
            origin: [10.0, -4.0, 2.0],
            orientation: Quat::from_euler(glam::EulerRot::XYZ, 0.2, -0.3, 0.1).to_array(),
        };
        let dst = VoxelGeometry {
            dimensions: [64, 48, 24],
            spacing: [0.9, 0.8, 1.5],
            origin: [4.0, -8.0, 5.0],
            orientation: Quat::from_euler(glam::EulerRot::XYZ, -0.25, 0.1, 0.5).to_array(),
        };

        let affine = index_space_affine_from_src_to_dst(src, dst).unwrap();
        let src_index = [11.25, 9.5, 3.0];
        let mapped = transform_index_with_affine(src_index, affine);

        let world = voxel_index_to_world_mm(src_index, src);
        let expected = world_mm_to_voxel_index(world, dst);
        assert!(approx_eq(mapped, expected, 1e-5));
    }

    #[test]
    fn test_orthogonal_plane_axes_match_radiological_mapping() {
        let geometry = identity_geometry();
        let cursor_uv = [0.25, 0.5, 0.75];

        let axial =
            orthogonal_plane_from_volume_uv(PlaneFamily::Axial, cursor_uv, geometry).unwrap();
        assert_eq!(axial.origin_mm, [2.25, 4.5, 6.75]);
        assert!(approx_eq(axial.u_axis_mm, [-1.0, 0.0, 0.0], 1e-6));
        assert!(approx_eq(axial.v_axis_mm, [0.0, -1.0, 0.0], 1e-6));
        assert!(approx_eq(axial.normal_mm, [0.0, 0.0, 1.0], 1e-6));

        let coronal =
            orthogonal_plane_from_volume_uv(PlaneFamily::Coronal, cursor_uv, geometry).unwrap();
        assert!(approx_eq(coronal.u_axis_mm, [-1.0, 0.0, 0.0], 1e-6));
        assert!(approx_eq(coronal.v_axis_mm, [0.0, 0.0, -1.0], 1e-6));
        assert!(approx_eq(coronal.normal_mm, [0.0, -1.0, 0.0], 1e-6));

        let sagittal =
            orthogonal_plane_from_volume_uv(PlaneFamily::Sagittal, cursor_uv, geometry).unwrap();
        assert!(approx_eq(sagittal.u_axis_mm, [0.0, -1.0, 0.0], 1e-6));
        assert!(approx_eq(sagittal.v_axis_mm, [0.0, 0.0, -1.0], 1e-6));
        assert!(approx_eq(sagittal.normal_mm, [1.0, 0.0, 0.0], 1e-6));
    }

    #[test]
    fn test_oblique_plane_axes_are_normalized_and_orthogonal() {
        let geometry = VoxelGeometry {
            dimensions: [24, 20, 16],
            spacing: [0.7, 1.3, 2.1],
            origin: [2.0, -3.0, 5.0],
            orientation: Quat::from_euler(glam::EulerRot::XYZ, 0.1, -0.2, 0.3).to_array(),
        };

        let plane = oblique_plane_from_view_rotation(
            [0.4, 0.35, 0.6],
            Quat::from_euler(glam::EulerRot::XYZ, 0.35, 0.1, -0.2).to_array(),
            geometry,
        )
        .unwrap();

        let u = Vec3::from_array(plane.u_axis_mm);
        let v = Vec3::from_array(plane.v_axis_mm);
        let n = Vec3::from_array(plane.normal_mm);
        assert!(approx_eq_scalar(u.length(), 1.0, 1e-5));
        assert!(approx_eq_scalar(v.length(), 1.0, 1e-5));
        assert!(approx_eq_scalar(n.length(), 1.0, 1e-5));
        assert!(approx_eq_scalar(u.dot(n), 0.0, 1e-5));
        assert!(approx_eq_scalar(v.dot(n), 0.0, 1e-5));
    }

    #[test]
    fn test_plane_local_world_roundtrip_for_non_origin_point() {
        let geometry = identity_geometry();
        let plane =
            orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 0.2], geometry).unwrap();
        let local = [12.5, -3.25];
        let world = plane_local_mm_to_world_mm(local, plane);
        let local_roundtrip = world_mm_to_plane_local_mm(world, plane);
        assert!(approx_eq_scalar(local_roundtrip[0], local[0], 1e-6));
        assert!(approx_eq_scalar(local_roundtrip[1], local[1], 1e-6));
    }

    #[test]
    fn test_reproject_plane_local_mm_preserves_world_position_across_coplanar_frames() {
        let geometry = identity_geometry();
        let source =
            orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 0.2], geometry).unwrap();
        let source_u = Vec3::from_array(source.u_axis_mm);
        let source_v = Vec3::from_array(source.v_axis_mm);
        let mut target = source;
        target.origin_mm =
            (Vec3::from_array(source.origin_mm) + source_u * 12.0 + source_v * 7.0).to_array();
        target.u_axis_mm = source_v.to_array();
        target.v_axis_mm = (-source_u).to_array();
        let source_local = [3.0, -4.0];

        let target_local = reproject_plane_local_mm(source_local, source, target);

        let source_world = plane_local_mm_to_world_mm(source_local, source);
        let target_world = plane_local_mm_to_world_mm(target_local, target);
        assert!(approx_eq(source_world, target_world, 1e-5));
    }

    fn legacy_viewport_uv_to_volume_uv(
        family: PlaneFamily,
        viewport_uv: [f32; 2],
        depth: f32,
        geometry: VoxelGeometry,
        mapping: ViewportMapping,
    ) -> [f32; 3] {
        let slice_aspect = plane_slice_aspect(family, geometry).unwrap();
        let k = mapping.screen_aspect / slice_aspect;
        let screen_uv = [
            ((viewport_uv[0] - mapping.pivot[0]) * k) / mapping.zoom
                + mapping.pivot[0]
                + mapping.pan[0],
            (viewport_uv[1] - mapping.pivot[1]) / mapping.zoom + mapping.pivot[1] + mapping.pan[1],
        ];
        screen_uv_to_volume_uv_for_family(family, screen_uv, depth)
    }

    fn legacy_oblique_viewport_uv_to_volume_uv(
        viewport_uv: [f32; 2],
        zoom: f32,
        pan: [f32; 2],
        screen_aspect: f32,
        vol_aspects: [f32; 3],
        cursor_uv: [f32; 3],
        rotation: [f32; 4],
    ) -> [f32; 3] {
        let view_rotation = normalized_orientation(rotation);
        let u_axis = (view_rotation * Vec3::X).normalize_or_zero();
        let v_axis = (view_rotation * Vec3::Y).normalize_or_zero();

        let lu = Vec3::new(
            u_axis.x * vol_aspects[0],
            u_axis.y * vol_aspects[1],
            u_axis.z * vol_aspects[2],
        )
        .length()
        .max(1e-3);
        let lv = Vec3::new(
            v_axis.x * vol_aspects[0],
            v_axis.y * vol_aspects[1],
            v_axis.z * vol_aspects[2],
        )
        .length()
        .max(1e-3);

        let k = screen_aspect / (lu / lv);
        let screen_uv = [
            ((viewport_uv[0] - 0.5) * k) / zoom + 0.5 + pan[0],
            (viewport_uv[1] - 0.5) / zoom + 0.5 + pan[1],
        ];
        let du = (0.5 - screen_uv[0]) * lu;
        let dv = (0.5 - screen_uv[1]) * lv;

        [
            cursor_uv[0] + u_axis.x * du + v_axis.x * dv,
            cursor_uv[1] + u_axis.y * du + v_axis.y * dv,
            cursor_uv[2] + u_axis.z * du + v_axis.z * dv,
        ]
    }

    #[test]
    fn test_viewport_uv_to_volume_uv_matches_legacy_axial_behavior() {
        let geometry = VoxelGeometry {
            dimensions: [120, 80, 40],
            spacing: [0.5, 1.0, 2.0],
            origin: [0.0, 0.0, 0.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let plane =
            orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.25, 0.75, 0.3], geometry)
                .unwrap();
        let mapping = ViewportMapping {
            zoom: 1.7,
            pan: [0.05, -0.12],
            pivot: [0.5, 0.5],
            screen_aspect: 16.0 / 9.0,
        };
        let viewport_uv = [0.2, 0.8];
        let volume_uv = viewport_uv_to_volume_uv(viewport_uv, plane, geometry, mapping).unwrap();
        let legacy = legacy_viewport_uv_to_volume_uv(
            PlaneFamily::Axial,
            viewport_uv,
            0.3,
            geometry,
            mapping,
        );
        assert!(approx_eq(volume_uv, legacy, 1e-6));
    }

    #[test]
    fn test_viewport_uv_to_volume_uv_matches_legacy_coronal_behavior() {
        let geometry = VoxelGeometry {
            dimensions: [80, 64, 96],
            spacing: [0.8, 0.8, 1.5],
            origin: [1.0, -2.0, 3.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let plane =
            orthogonal_plane_from_volume_uv(PlaneFamily::Coronal, [0.6, 0.4, 0.2], geometry)
                .unwrap();
        let mapping = ViewportMapping {
            zoom: 1.2,
            pan: [-0.08, 0.03],
            pivot: [0.5, 0.5],
            screen_aspect: 4.0 / 3.0,
        };
        let viewport_uv = [0.1, 0.35];
        let volume_uv = viewport_uv_to_volume_uv(viewport_uv, plane, geometry, mapping).unwrap();
        let legacy = legacy_viewport_uv_to_volume_uv(
            PlaneFamily::Coronal,
            viewport_uv,
            0.4,
            geometry,
            mapping,
        );
        assert!(approx_eq(volume_uv, legacy, 1e-6));
    }

    #[test]
    fn test_viewport_uv_to_volume_uv_matches_legacy_sagittal_behavior() {
        let geometry = VoxelGeometry {
            dimensions: [70, 120, 90],
            spacing: [1.0, 0.6, 1.4],
            origin: [5.0, 2.0, -1.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let plane =
            orthogonal_plane_from_volume_uv(PlaneFamily::Sagittal, [0.55, 0.1, 0.9], geometry)
                .unwrap();
        let mapping = ViewportMapping {
            zoom: 2.0,
            pan: [0.02, 0.11],
            pivot: [0.5, 0.5],
            screen_aspect: 1.0,
        };
        let viewport_uv = [0.85, 0.15];
        let volume_uv = viewport_uv_to_volume_uv(viewport_uv, plane, geometry, mapping).unwrap();
        let legacy = legacy_viewport_uv_to_volume_uv(
            PlaneFamily::Sagittal,
            viewport_uv,
            0.55,
            geometry,
            mapping,
        );
        assert!(approx_eq(volume_uv, legacy, 1e-6));
    }

    #[test]
    fn test_volume_viewport_roundtrip_for_orthogonal_planes() {
        let geometry = identity_geometry();
        let mapping = ViewportMapping {
            zoom: 1.35,
            pan: [0.03, -0.07],
            pivot: [0.5, 0.5],
            screen_aspect: 16.0 / 10.0,
        };

        for (family, volume_uv) in [
            (PlaneFamily::Axial, [0.3, 0.4, 0.7]),
            (PlaneFamily::Coronal, [0.2, 0.6, 0.8]),
            (PlaneFamily::Sagittal, [0.1, 0.9, 0.5]),
        ] {
            let plane = orthogonal_plane_from_volume_uv(family, volume_uv, geometry).unwrap();
            let viewport_uv =
                volume_uv_to_viewport_uv(volume_uv, plane, geometry, mapping).unwrap();
            let roundtrip =
                viewport_uv_to_volume_uv(viewport_uv, plane, geometry, mapping).unwrap();
            assert!(approx_eq(roundtrip, volume_uv, 1e-6));
        }
    }

    #[test]
    fn test_viewport_plane_local_roundtrip_for_orthogonal_planes() {
        let geometry = identity_geometry();
        let mapping = default_mapping();

        for (family, cursor_uv, click_uv) in [
            (PlaneFamily::Axial, [0.35, 0.45, 0.25], [0.2, 0.8]),
            (PlaneFamily::Coronal, [0.7, 0.25, 0.6], [0.65, 0.15]),
            (PlaneFamily::Sagittal, [0.15, 0.8, 0.55], [0.4, 0.3]),
        ] {
            let plane = orthogonal_plane_from_volume_uv(family, cursor_uv, geometry).unwrap();
            let local = viewport_uv_to_plane_local_mm(click_uv, plane, geometry, mapping).unwrap();
            let click_roundtrip =
                plane_local_mm_to_viewport_uv(local, plane, geometry, mapping).unwrap();
            assert!(approx_eq2(click_roundtrip, click_uv, 1e-5));
        }
    }

    #[test]
    fn test_egui_top_left_y_down_convention_is_preserved() {
        let geometry = identity_geometry();
        let plane =
            orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 0.5], geometry).unwrap();
        let mapping = ViewportMapping {
            zoom: 1.0,
            pan: [0.0, 0.0],
            pivot: [0.5, 0.5],
            screen_aspect: 1.0,
        };

        let top_left = viewport_uv_to_volume_uv([0.0, 0.0], plane, geometry, mapping).unwrap();
        assert!(approx_eq(top_left, [1.0, 1.0, 0.5], 1e-6));

        let bottom_right = viewport_uv_to_volume_uv([1.0, 1.0], plane, geometry, mapping).unwrap();
        assert!(approx_eq(bottom_right, [0.0, 0.0, 0.5], 1e-6));

        let projected_back = volume_uv_to_viewport_uv(top_left, plane, geometry, mapping).unwrap();
        assert!(approx_eq2(projected_back, [0.0, 0.0], 1e-6));
    }

    #[test]
    fn test_oblique_viewport_mapping_matches_legacy_identity_rotation() {
        let geometry = VoxelGeometry {
            dimensions: [96, 80, 64],
            spacing: [1.0, 0.8, 1.2],
            origin: [0.0, 0.0, 0.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let cursor_uv = [0.4, 0.55, 0.6];
        let rotation = [0.0, 0.0, 0.0, 1.0];
        let plane = oblique_plane_from_view_rotation(cursor_uv, rotation, geometry).unwrap();
        let mapping = ViewportMapping {
            zoom: 1.3,
            pan: [0.04, -0.02],
            pivot: [0.5, 0.5],
            screen_aspect: 16.0 / 9.0,
        };
        let viewport_uv = [0.12, 0.77];

        let new_pos = viewport_uv_to_volume_uv(viewport_uv, plane, geometry, mapping).unwrap();
        let legacy_pos = legacy_oblique_viewport_uv_to_volume_uv(
            viewport_uv,
            mapping.zoom,
            mapping.pan,
            mapping.screen_aspect,
            normalized_volume_extents(geometry),
            cursor_uv,
            rotation,
        );
        assert!(approx_eq(new_pos, legacy_pos, 1e-5));
    }

    #[test]
    fn test_oblique_viewport_mapping_matches_legacy_non_identity_rotation() {
        let geometry = VoxelGeometry {
            dimensions: [120, 96, 84],
            spacing: [0.7, 1.0, 1.4],
            origin: [0.0, 0.0, 0.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let cursor_uv = [0.5, 0.5, 0.5];
        let rotation = Quat::from_euler(glam::EulerRot::XYZ, 0.35, -0.2, 0.45).to_array();
        let plane = oblique_plane_from_view_rotation(cursor_uv, rotation, geometry).unwrap();
        let mapping = ViewportMapping {
            zoom: 0.85,
            pan: [-0.05, 0.07],
            pivot: [0.5, 0.5],
            screen_aspect: 1.0,
        };
        let viewport_uv = [0.9, 0.2];

        let new_pos = viewport_uv_to_volume_uv(viewport_uv, plane, geometry, mapping).unwrap();
        let legacy_pos = legacy_oblique_viewport_uv_to_volume_uv(
            viewport_uv,
            mapping.zoom,
            mapping.pan,
            mapping.screen_aspect,
            normalized_volume_extents(geometry),
            cursor_uv,
            rotation,
        );
        assert!(approx_eq(new_pos, legacy_pos, 1e-5));
    }

    #[test]
    fn test_shared_oblique_uniform_basis_matches_viewport_mapping_for_oriented_geometry() {
        let geometry = VoxelGeometry {
            dimensions: [120, 96, 84],
            spacing: [0.7, 1.0, 1.4],
            origin: [12.0, -7.0, 3.0],
            orientation: Quat::from_euler(glam::EulerRot::XYZ, 0.2, -0.3, 0.1).to_array(),
        };
        let cursor_uv = [0.45, 0.55, 0.4];
        let plane = oblique_plane_from_view_rotation(
            cursor_uv,
            Quat::from_euler(glam::EulerRot::XYZ, 0.35, -0.2, 0.45).to_array(),
            geometry,
        )
        .unwrap();
        let mapping = ViewportMapping {
            zoom: 1.2,
            pan: [0.03, -0.04],
            pivot: [0.5, 0.5],
            screen_aspect: 16.0 / 10.0,
        };
        let viewport_uv = [0.2, 0.75];
        let expected = viewport_uv_to_volume_uv(viewport_uv, plane, geometry, mapping).unwrap();
        let (u_dir, v_dir, u_length, v_length) =
            oblique_volume_uv_basis_and_lengths(plane, geometry).unwrap();
        let slice_aspect = u_length / v_length;
        let k = mapping.screen_aspect / slice_aspect;
        let screen_uv = [
            ((viewport_uv[0] - mapping.pivot[0]) * k) / mapping.zoom
                + mapping.pivot[0]
                + mapping.pan[0],
            (viewport_uv[1] - mapping.pivot[1]) / mapping.zoom + mapping.pivot[1] + mapping.pan[1],
        ];
        let origin_uv = world_mm_to_volume_uv(plane.origin_mm, geometry);
        let shader_equivalent = (Vec3::from_array(origin_uv)
            + Vec3::from_array(u_dir) * ((0.5 - screen_uv[0]) * u_length)
            + Vec3::from_array(v_dir) * ((0.5 - screen_uv[1]) * v_length))
            .to_array();

        assert!(approx_eq(shader_equivalent, expected, 1e-5));
    }

    #[test]
    fn test_roi_geometry_roundtrips_rotated_anisotropic_affine() {
        let affine = DMat4::from_cols(
            DVec4::new(0.0, 2.0, 0.0, 0.0),
            DVec4::new(-3.0, 0.0, 0.0, 0.0),
            DVec4::new(0.0, 0.0, 4.0, 0.0),
            DVec4::new(10.0, -5.0, 2.5, 1.0),
        );
        let geometry = RoiGeometry::new([9, 8, 7], affine).unwrap();
        let ijk = [2.25, 3.5, 1.75];

        let world = geometry.ijk_to_world_mm(ijk);
        let roundtrip = geometry.world_mm_to_ijk(world);

        for axis in 0..3 {
            assert!((roundtrip[axis] - ijk[axis]).abs() < 1.0e-12);
        }
        assert!(geometry.contains_voxel_center([0.0, 0.0, 0.0]));
        assert!(geometry.contains_voxel_center([8.499, 7.499, 6.499]));
        assert!(!geometry.contains_voxel_center([8.5, 7.0, 6.0]));
    }

    #[test]
    fn test_roi_geometry_preserves_reflection_in_identity() {
        let reflected = RoiGeometry::new(
            [4, 5, 6],
            DMat4::from_cols(
                DVec4::new(-1.0, 0.0, 0.0, 0.0),
                DVec4::new(0.0, 2.0, 0.0, 0.0),
                DVec4::new(0.0, 0.0, 3.0, 0.0),
                DVec4::new(11.0, 12.0, 13.0, 1.0),
            ),
        )
        .unwrap();
        let unreflected = RoiGeometry::new(
            [4, 5, 6],
            DMat4::from_cols(
                DVec4::new(1.0, 0.0, 0.0, 0.0),
                DVec4::new(0.0, 2.0, 0.0, 0.0),
                DVec4::new(0.0, 0.0, 3.0, 0.0),
                DVec4::new(11.0, 12.0, 13.0, 1.0),
            ),
        )
        .unwrap();

        assert_eq!(
            reflected.ijk_to_world_mm([1.0, 0.0, 0.0]),
            [10.0, 12.0, 13.0]
        );
        assert_ne!(reflected.identity(), unreflected.identity());
    }

    #[test]
    fn test_roi_geometry_rejects_invalid_affines() {
        assert_eq!(
            RoiGeometry::new([0, 2, 3], DMat4::IDENTITY),
            Err(RoiGeometryError::EmptyDimensions)
        );
        assert_eq!(
            RoiGeometry::new(
                [2, 2, 2],
                DMat4::from_cols(DVec4::X, DVec4::Y, DVec4::Z, DVec4::new(0.0, 0.0, 0.0, 2.0),),
            ),
            Err(RoiGeometryError::NonAffineTransform)
        );
        assert_eq!(
            RoiGeometry::new(
                [2, 2, 2],
                DMat4::from_cols(DVec4::ZERO, DVec4::Y, DVec4::Z, DVec4::W),
            ),
            Err(RoiGeometryError::SingularAffine)
        );
    }

    #[test]
    fn test_roi_geometry_legacy_conversion_matches_existing_voxel_world_mapping() {
        let legacy = VoxelGeometry {
            dimensions: [8, 7, 6],
            spacing: [0.5, 1.25, 2.0],
            origin: [4.0, -3.0, 8.0],
            orientation: Quat::from_euler(glam::EulerRot::XYZ, 0.1, -0.3, 0.25).to_array(),
        };
        let geometry = RoiGeometry::from_legacy_parts(
            legacy.dimensions,
            legacy.spacing,
            legacy.origin,
            legacy.orientation,
        )
        .unwrap();
        let ijk = [3.25, 1.5, 5.0];
        let legacy_world = voxel_index_to_world_mm(ijk.map(|value| value as f32), legacy);
        let affine_world = geometry.ijk_to_world_mm(ijk);

        for axis in 0..3 {
            assert!((affine_world[axis] - f64::from(legacy_world[axis])).abs() < 1.0e-6);
        }
    }
}
