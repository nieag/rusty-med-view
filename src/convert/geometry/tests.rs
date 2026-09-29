use super::*;
use crate::app::roi::VoxelGeometryError;
use glam::{DMat4, DVec4};

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
    VoxelGeometry::new(
        [10, 10, 10],
        [1.0, 1.0, 1.0],
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap()
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
    // An empty axis has no cells; a one-voxel axis is a real cell spanning index [-0.5, 0.5].
    assert_eq!(index[0], 0.0);
    assert!((index[1] - -0.3).abs() < 1e-6);
    assert!((index[2] - 3.5).abs() < 1e-6);

    let uv_roundtrip = voxel_index_to_volume_uv(index, dimensions);
    assert_eq!(uv_roundtrip[0], 0.0);
    assert!((uv_roundtrip[1] - 0.2).abs() < 1e-6);
    assert!((uv_roundtrip[2] - 0.5).abs() < 1e-6);
}

#[test]
fn test_voxel_world_roundtrip_with_origin_spacing_and_rotation() {
    let orientation = Quat::from_euler(glam::EulerRot::XYZ, 0.4, -0.25, 0.7).to_array();
    let geometry = VoxelGeometry::new(
        [32, 24, 16],
        [0.5, 0.8, 2.0],
        [12.0, -4.0, 3.0],
        orientation,
    )
    .unwrap();
    let index = [6.5, 3.25, 2.0];
    let world = voxel_index_to_world_mm(index, geometry);
    let index_roundtrip = world_mm_to_voxel_index(world, geometry);
    assert!(approx_eq(index, index_roundtrip, 1e-5));
}

#[test]
fn test_identity_geometry_maps_origin_and_index_as_expected() {
    let geometry = VoxelGeometry::new(
        [8, 8, 8],
        [0.5, 2.0, 1.0],
        [10.0, -1.0, 3.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap();

    let world_at_zero = voxel_index_to_world_mm([0.0, 0.0, 0.0], geometry);
    assert_eq!(world_at_zero, [10.0, -1.0, 3.0]);

    let world = voxel_index_to_world_mm([2.0, 3.0, 4.0], geometry);
    assert_eq!(world, [11.0, 5.0, 7.0]);

    let roundtrip = world_mm_to_voxel_index(world, geometry);
    assert!(approx_eq(roundtrip, [2.0, 3.0, 4.0], 1e-6));
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
    let src = VoxelGeometry::new(
        [64, 48, 24],
        [0.7, 1.1, 2.0],
        [10.0, -4.0, 2.0],
        Quat::from_euler(glam::EulerRot::XYZ, 0.2, -0.3, 0.1).to_array(),
    )
    .unwrap();
    let dst = VoxelGeometry::new(
        [64, 48, 24],
        [0.9, 0.8, 1.5],
        [4.0, -8.0, 5.0],
        Quat::from_euler(glam::EulerRot::XYZ, -0.25, 0.1, 0.5).to_array(),
    )
    .unwrap();

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

    let axial = orthogonal_plane_from_volume_uv(PlaneFamily::Axial, cursor_uv, geometry).unwrap();
    // Cell-centred: index = uv * dim - 0.5 on a 10-voxel grid.
    assert_eq!(axial.origin_mm, [2.0, 4.5, 7.0]);
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
    let geometry = VoxelGeometry::new(
        [24, 20, 16],
        [0.7, 1.3, 2.1],
        [2.0, -3.0, 5.0],
        Quat::from_euler(glam::EulerRot::XYZ, 0.1, -0.2, 0.3).to_array(),
    )
    .unwrap();

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
    let geometry = VoxelGeometry::new(
        [120, 80, 40],
        [0.5, 1.0, 2.0],
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap();
    let plane =
        orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.25, 0.75, 0.3], geometry).unwrap();
    let mapping = ViewportMapping {
        zoom: 1.7,
        pan: [0.05, -0.12],
        pivot: [0.5, 0.5],
        screen_aspect: 16.0 / 9.0,
    };
    let viewport_uv = [0.2, 0.8];
    let volume_uv = viewport_uv_to_volume_uv(viewport_uv, plane, geometry, mapping).unwrap();
    let legacy =
        legacy_viewport_uv_to_volume_uv(PlaneFamily::Axial, viewport_uv, 0.3, geometry, mapping);
    assert!(approx_eq(volume_uv, legacy, 1e-6));
}

#[test]
fn test_viewport_uv_to_volume_uv_matches_legacy_coronal_behavior() {
    let geometry = VoxelGeometry::new(
        [80, 64, 96],
        [0.8, 0.8, 1.5],
        [1.0, -2.0, 3.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap();
    let plane =
        orthogonal_plane_from_volume_uv(PlaneFamily::Coronal, [0.6, 0.4, 0.2], geometry).unwrap();
    let mapping = ViewportMapping {
        zoom: 1.2,
        pan: [-0.08, 0.03],
        pivot: [0.5, 0.5],
        screen_aspect: 4.0 / 3.0,
    };
    let viewport_uv = [0.1, 0.35];
    let volume_uv = viewport_uv_to_volume_uv(viewport_uv, plane, geometry, mapping).unwrap();
    let legacy =
        legacy_viewport_uv_to_volume_uv(PlaneFamily::Coronal, viewport_uv, 0.4, geometry, mapping);
    assert!(approx_eq(volume_uv, legacy, 1e-6));
}

#[test]
fn test_viewport_uv_to_volume_uv_matches_legacy_sagittal_behavior() {
    let geometry = VoxelGeometry::new(
        [70, 120, 90],
        [1.0, 0.6, 1.4],
        [5.0, 2.0, -1.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap();
    let plane =
        orthogonal_plane_from_volume_uv(PlaneFamily::Sagittal, [0.55, 0.1, 0.9], geometry).unwrap();
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
        let viewport_uv = volume_uv_to_viewport_uv(volume_uv, plane, geometry, mapping).unwrap();
        let roundtrip = viewport_uv_to_volume_uv(viewport_uv, plane, geometry, mapping).unwrap();
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
    let geometry = VoxelGeometry::new(
        [96, 80, 64],
        [1.0, 0.8, 1.2],
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap();
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
    let geometry = VoxelGeometry::new(
        [120, 96, 84],
        [0.7, 1.0, 1.4],
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap();
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
    let geometry = VoxelGeometry::new(
        [120, 96, 84],
        [0.7, 1.0, 1.4],
        [12.0, -7.0, 3.0],
        Quat::from_euler(glam::EulerRot::XYZ, 0.2, -0.3, 0.1).to_array(),
    )
    .unwrap();
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
    let geometry = VoxelGeometry::from_affine([9, 8, 7], affine).unwrap();
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
    let reflected = VoxelGeometry::from_affine(
        [4, 5, 6],
        DMat4::from_cols(
            DVec4::new(-1.0, 0.0, 0.0, 0.0),
            DVec4::new(0.0, 2.0, 0.0, 0.0),
            DVec4::new(0.0, 0.0, 3.0, 0.0),
            DVec4::new(11.0, 12.0, 13.0, 1.0),
        ),
    )
    .unwrap();
    let unreflected = VoxelGeometry::from_affine(
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
        VoxelGeometry::from_affine([0, 2, 3], DMat4::IDENTITY),
        Err(VoxelGeometryError::EmptyDimensions)
    );
    assert_eq!(
        VoxelGeometry::from_affine(
            [2, 2, 2],
            DMat4::from_cols(DVec4::X, DVec4::Y, DVec4::Z, DVec4::new(0.0, 0.0, 0.0, 2.0),),
        ),
        Err(VoxelGeometryError::NonAffineTransform)
    );
    assert_eq!(
        VoxelGeometry::from_affine(
            [2, 2, 2],
            DMat4::from_cols(DVec4::ZERO, DVec4::Y, DVec4::Z, DVec4::W),
        ),
        Err(VoxelGeometryError::SingularAffine)
    );
}

#[test]
fn test_roi_geometry_legacy_conversion_matches_existing_voxel_world_mapping() {
    let legacy = VoxelGeometry::new(
        [8, 7, 6],
        [0.5, 1.25, 2.0],
        [4.0, -3.0, 8.0],
        Quat::from_euler(glam::EulerRot::XYZ, 0.1, -0.3, 0.25).to_array(),
    )
    .unwrap();
    let geometry = VoxelGeometry::new(
        legacy.dimensions,
        legacy.spacing(),
        legacy.origin(),
        legacy.orientation(),
    )
    .unwrap();
    let ijk = [3.25, 1.5, 5.0];
    let legacy_world = voxel_index_to_world_mm(ijk.map(|value| value as f32), legacy);
    let affine_world = geometry.ijk_to_world_mm(ijk);

    for axis in 0..3 {
        assert!((affine_world[axis] - f64::from(legacy_world[axis])).abs() < 1.0e-6);
    }
}

#[test]
fn test_plane_definition_constructor_derives_normal_and_rejects_degenerate_axes() {
    let plane = PlaneDefinition::new(
        PlaneFamily::Axial,
        [0.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [0.0, 3.0, 0.0],
    )
    .unwrap();
    assert_eq!(plane.normal_mm, [0.0, 0.0, 1.0]);
    assert!(PlaneDefinition::new(
        PlaneFamily::Axial,
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
    )
    .is_none());
}

// Target for the Phase 1 geometry rework. The oblique mapping treats a unit index-space direction
// as a uv-space direction, so on anisotropic or non-cubic volumes a rotated oblique reslice is
// sheared in patient millimetres and its plane frame (contour local coordinates) is skewed.
// Remove `ignore` when the mapping derives its basis from an orthonormal millimetre frame.
#[test]
#[ignore = "known defect: oblique reslice is skewed in mm on anisotropic volumes (Phase 1)"]
fn test_oblique_reslice_is_planar_and_orthogonal_in_millimetres() {
    let geometry = VoxelGeometry::new(
        [120, 96, 84],
        [0.7, 1.0, 1.4],
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap();
    let rotation = Quat::from_euler(glam::EulerRot::XYZ, 0.35, -0.2, 0.45).to_array();
    let plane = oblique_plane_from_view_rotation([0.5; 3], rotation, geometry).unwrap();
    let mapping = ViewportMapping {
        zoom: 0.85,
        pan: [-0.05, 0.07],
        pivot: [0.5, 0.5],
        screen_aspect: 1.0,
    };
    let world_at = |viewport_uv: [f32; 2]| {
        let uv = viewport_uv_to_volume_uv(viewport_uv, plane, geometry, mapping).unwrap();
        Vec3::from_array(volume_uv_to_world_mm(uv, geometry))
    };

    let origin = world_at([0.5, 0.5]);
    let horizontal = world_at([0.6, 0.5]) - origin;
    let vertical = world_at([0.5, 0.6]) - origin;
    let normal = Vec3::from_array(plane.normal_mm);

    assert!(
        horizontal.dot(normal).abs() < 1e-3,
        "horizontal leaves plane"
    );
    assert!(vertical.dot(normal).abs() < 1e-3, "vertical leaves plane");
    let cosine = horizontal.normalize().dot(vertical.normalize());
    assert!(
        cosine.abs() < 1e-4,
        "screen axes skewed in mm: cos = {cosine}"
    );
}

fn axial_plane_at(z_mm: f32) -> PlaneDefinition {
    PlaneDefinition {
        family: PlaneFamily::Axial,
        origin_mm: [0.0, 0.0, z_mm],
        u_axis_mm: [1.0, 0.0, 0.0],
        v_axis_mm: [0.0, 1.0, 0.0],
        normal_mm: [0.0, 0.0, 1.0],
    }
}

fn grid_with_spacing(spacing: [f32; 3]) -> VoxelGeometry {
    VoxelGeometry::new([64, 64, 64], spacing, [0.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap()
}

#[test]
fn test_same_slice_follows_the_voxel_layer_on_a_one_millimetre_grid() {
    let geometry = grid_with_spacing([1.0; 3]);
    let displayed = axial_plane_at(10.0);

    assert!(planes_are_same_slice(
        displayed,
        axial_plane_at(10.4),
        geometry
    ));
    assert!(!planes_are_same_slice(
        displayed,
        axial_plane_at(10.8),
        geometry
    ));
}

#[test]
fn test_adjacent_layers_stay_distinct_on_a_sub_half_millimetre_grid() {
    // A fixed 0.5 mm tolerance merged neighbouring slices of a 0.3 mm grid into one contour.
    let geometry = grid_with_spacing([0.3, 0.3, 0.3]);

    assert!(!planes_are_same_slice(
        axial_plane_at(3.0),
        axial_plane_at(3.3),
        geometry
    ));
    assert!(planes_are_same_slice(
        axial_plane_at(3.0),
        axial_plane_at(3.05),
        geometry
    ));
}

#[test]
fn test_thick_slices_are_not_split_by_a_millimetre_offset() {
    // Layers 5 mm apart: a sub-millimetre cursor offset stays in the same layer.
    let geometry = grid_with_spacing([1.0, 1.0, 5.0]);

    assert!(planes_are_same_slice(
        axial_plane_at(10.0),
        axial_plane_at(11.9),
        geometry
    ));
    assert!(!planes_are_same_slice(
        axial_plane_at(10.0),
        axial_plane_at(12.6),
        geometry
    ));
}

#[test]
fn test_same_slice_requires_matching_family_and_parallel_normals() {
    let geometry = grid_with_spacing([1.0; 3]);
    let axial = axial_plane_at(10.0);
    let other_family = PlaneDefinition {
        family: PlaneFamily::Coronal,
        ..axial
    };
    let tilted = PlaneDefinition {
        normal_mm: [0.0, 0.2, 0.98],
        ..axial
    };

    assert!(!planes_are_same_slice(axial, other_family, geometry));
    assert!(!planes_are_same_slice(axial, tilted, geometry));
}

#[test]
fn test_oblique_slices_match_within_half_the_smallest_spacing() {
    let geometry = grid_with_spacing([0.4, 1.0, 2.0]);
    let oblique = PlaneDefinition {
        family: PlaneFamily::Oblique,
        ..axial_plane_at(10.0)
    };
    let near = PlaneDefinition {
        origin_mm: [0.0, 0.0, 10.15],
        ..oblique
    };
    let far = PlaneDefinition {
        origin_mm: [0.0, 0.0, 10.3],
        ..oblique
    };

    assert!(planes_are_same_slice(oblique, near, geometry));
    assert!(!planes_are_same_slice(oblique, far, geometry));
}
