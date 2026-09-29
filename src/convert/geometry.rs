use crate::app::roi::VoxelGeometry;
use crate::convert::coord_mapping::{volume_uv_to_voxel_index, voxel_index_to_volume_uv};
use glam::{DVec3, Quat, Vec3};

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
    let world = geometry
        .ijk_to_world_affine()
        .transform_vector3(DVec3::from(index_direction));
    if world.length_squared() <= 1e-12 {
        return None;
    }
    Some(world.normalize().as_vec3())
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

/// Oblique reslice plane through the cursor, rotated by `rotation` in the volume's physical
/// (millimetre) frame, the same frame the 3D view rotates. The plane axes are orthonormal in
/// world space by construction, so plane-local coordinates are true millimetres.
pub fn oblique_plane_from_view_rotation(
    cursor_uv: [f32; 3],
    rotation: [f32; 4],
    geometry: VoxelGeometry,
) -> Option<PlaneDefinition> {
    let view_rotation = normalized_orientation(rotation);
    let frame = geometry.axis_frame();
    let u_axis = (frame * DVec3::from(view_rotation * Vec3::X)).try_normalize()?;
    let v_axis = (frame * DVec3::from(view_rotation * Vec3::Y)).try_normalize()?;
    PlaneDefinition::new(
        PlaneFamily::Oblique,
        volume_uv_to_world_mm(cursor_uv, geometry),
        u_axis.as_vec3().to_array(),
        v_axis.as_vec3().to_array(),
    )
}

/// Voxel-grid depth axis of an orthogonal plane family; `None` for oblique planes.
pub(crate) fn orthogonal_depth_axis(family: PlaneFamily) -> Option<usize> {
    match family {
        PlaneFamily::Axial => Some(2),
        PlaneFamily::Coronal => Some(1),
        PlaneFamily::Sagittal => Some(0),
        PlaneFamily::Oblique => None,
    }
}

/// Nearest voxel layer along `axis`, or `None` when the plane is not finite. The layer may lie
/// outside the volume; callers must treat those slices as empty rather than clamp them.
pub(crate) fn nearest_depth_layer(
    origin_mm: [f32; 3],
    axis: usize,
    geometry: VoxelGeometry,
) -> Option<i64> {
    let depth = world_mm_to_voxel_index(origin_mm, geometry)[axis];
    depth.is_finite().then(|| depth.round() as i64)
}

/// Minimum |cos| between plane normals for two planes to be treated as parallel.
pub(crate) const PLANE_NORMAL_ALIGNMENT_COS: f32 = 0.999;

/// Whether two planes denote the same contour slice on this grid.
///
/// Different families never match and normals must be parallel. Orthogonal planes match when
/// they select the same voxel layer, so the rule follows the voxel spacing (a fixed millimetre
/// tolerance merged adjacent slices on sub-half-millimetre grids). Oblique planes have no layers
/// and match within half the smallest voxel spacing.
pub fn planes_are_same_slice(
    first: PlaneDefinition,
    second: PlaneDefinition,
    geometry: VoxelGeometry,
) -> bool {
    if first.family != second.family {
        return false;
    }
    let (Some(first_normal), Some(second_normal)) = (
        Vec3::from_array(first.normal_mm).try_normalize(),
        Vec3::from_array(second.normal_mm).try_normalize(),
    ) else {
        return false;
    };
    if first_normal.dot(second_normal).abs() < PLANE_NORMAL_ALIGNMENT_COS {
        return false;
    }
    if let Some(axis) = orthogonal_depth_axis(first.family) {
        return match (
            nearest_depth_layer(first.origin_mm, axis, geometry),
            nearest_depth_layer(second.origin_mm, axis, geometry),
        ) {
            (Some(first_layer), Some(second_layer)) => first_layer == second_layer,
            _ => false,
        };
    }
    let spacing = geometry.spacing();
    let tolerance_mm = 0.5 * spacing[0].min(spacing[1]).min(spacing[2]);
    let offset = Vec3::from_array(second.origin_mm) - Vec3::from_array(first.origin_mm);
    offset.dot(first_normal).abs() <= tolerance_mm
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
        geometry.dimensions[0] as f32 * geometry.spacing()[0],
        geometry.dimensions[1] as f32 * geometry.spacing()[1],
        geometry.dimensions[2] as f32 * geometry.spacing()[2],
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

/// Volume-UV displacement per millimetre along the plane's `u` and `v` axes, and the view window
/// (millimetres) along each axis: the extent of the volume's projection onto that axis, so the
/// whole volume is visible at zoom 1 and the window has the plane's true physical aspect.
fn oblique_uv_basis_and_lengths(
    plane: PlaneDefinition,
    geometry: VoxelGeometry,
) -> Option<(Vec3, Vec3, f32, f32)> {
    let dimensions = DVec3::new(
        f64::from(geometry.dimensions[0]),
        f64::from(geometry.dimensions[1]),
        f64::from(geometry.dimensions[2]),
    );
    let world_to_ijk = geometry.world_to_ijk_affine();
    let ijk_to_world = geometry.ijk_to_world_affine();
    let axis = |direction: [f32; 3]| {
        let direction = DVec3::from(Vec3::from_array(direction));
        direction.is_finite().then_some(direction)
    };
    let (u_axis, v_axis) = (axis(plane.u_axis_mm)?, axis(plane.v_axis_mm)?);

    let uv_per_mm = |direction: DVec3| world_to_ijk.transform_vector3(direction) / dimensions;
    let edges = [
        ijk_to_world.x_axis.truncate() * dimensions.x,
        ijk_to_world.y_axis.truncate() * dimensions.y,
        ijk_to_world.z_axis.truncate() * dimensions.z,
    ];
    let window = |direction: DVec3| {
        edges
            .iter()
            .map(|edge| direction.dot(*edge).abs())
            .sum::<f64>()
            .max(1e-3) as f32
    };
    Some((
        uv_per_mm(u_axis).as_vec3(),
        uv_per_mm(v_axis).as_vec3(),
        window(u_axis),
        window(v_axis),
    ))
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
            let (_, _, lu, lv) = oblique_uv_basis_and_lengths(plane, geometry)?;
            let slice_aspect = lu / lv;
            let k = mapping.screen_aspect / slice_aspect;
            // The plane axes are orthonormal in millimetres, so plane-local coordinates are a
            // plain projection of the world position.
            let world_mm = volume_uv_to_world_mm(volume_uv, geometry);
            let local_mm = world_mm_to_plane_local_mm(world_mm, plane);
            let screen_uv = [0.5 - local_mm[0] / lu, 0.5 - local_mm[1] / lv];

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
    geometry
        .ijk_to_world_affine()
        .transform_point3(DVec3::new(
            f64::from(index[0]),
            f64::from(index[1]),
            f64::from(index[2]),
        ))
        .as_vec3()
        .to_array()
}

pub fn world_mm_to_voxel_index(world: [f32; 3], geometry: VoxelGeometry) -> [f32; 3] {
    geometry
        .world_to_ijk_affine()
        .transform_point3(DVec3::new(
            f64::from(world[0]),
            f64::from(world[1]),
            f64::from(world[2]),
        ))
        .as_vec3()
        .to_array()
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
    // Exact for any affine, including reflections and shear.
    let affine = dst.world_to_ijk_affine() * src.ijk_to_world_affine();
    let cols = affine.to_cols_array_2d();
    Some(std::array::from_fn(|row| {
        std::array::from_fn(|col| cols[col][row] as f32)
    }))
}

#[cfg(test)]
pub fn transform_index_with_affine(index: [f32; 3], affine: [[f32; 4]; 3]) -> [f32; 3] {
    [
        affine[0][0] * index[0] + affine[0][1] * index[1] + affine[0][2] * index[2] + affine[0][3],
        affine[1][0] * index[0] + affine[1][1] * index[1] + affine[1][2] * index[2] + affine[1][3],
        affine[2][0] * index[0] + affine[2][1] * index[1] + affine[2][2] * index[2] + affine[2][3],
    ]
}

#[cfg(test)]
mod tests;
