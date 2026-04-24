use crate::components::VoxelGeometry;
use glam::{Quat, Vec3};

fn normalized_orientation(orientation: [f32; 4]) -> Quat {
    let quat = Quat::from_array(orientation);
    let length_sq = quat.length_squared();
    if length_sq.is_finite() && length_sq > 1e-12 {
        quat.normalize()
    } else {
        Quat::IDENTITY
    }
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
}
