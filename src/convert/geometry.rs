use crate::components::VoxelGeometry;
use glam::{Quat, Vec3};

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
}
