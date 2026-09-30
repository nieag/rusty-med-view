use super::*;
use crate::convert::{
    plane_local_mm_to_world_mm, rasterize_contours_to_voxel_data, world_mm_to_voxel_index,
};
use crate::model::VoxelGeometry;
use glam::Quat;

fn identity_geometry(dimensions: [u32; 3]) -> VoxelGeometry {
    VoxelGeometry::new(
        dimensions,
        [1.0, 1.0, 1.0],
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap()
}

fn voxel_data_with_fill(
    dimensions: [u32; 3],
    filled_indices: impl IntoIterator<Item = [u32; 3]>,
) -> VoxelData {
    let mut raw = vec![0_u8; (dimensions[0] * dimensions[1] * dimensions[2]) as usize];
    for [x, y, z] in filled_indices {
        let idx = voxel_index([x, y, z], dimensions);
        raw[idx] = 1;
    }
    VoxelData {
        geometry: identity_geometry(dimensions),
        raw_data: raw,
    }
}

fn voxel_data_with_geometry_fill(
    geometry: VoxelGeometry,
    filled_indices: impl IntoIterator<Item = [u32; 3]>,
) -> VoxelData {
    let mut raw = vec![
        0_u8;
        (geometry.dimensions[0] * geometry.dimensions[1] * geometry.dimensions[2])
            as usize
    ];
    for [x, y, z] in filled_indices {
        let idx = voxel_index([x, y, z], geometry.dimensions);
        raw[idx] = 1;
    }
    VoxelData {
        geometry,
        raw_data: raw,
    }
}

#[test]
fn test_extract_empty_voxel_data_returns_empty_contours() {
    let voxel = voxel_data_with_fill([4, 4, 2], []);
    let contour = extract_contours_from_voxel_data(&voxel, PlaneFamily::Axial)
        .expect("axial extraction should succeed");

    assert_eq!(contour.active_plane_family, PlaneFamily::Axial);
    assert!(contour.slices.is_empty());
}

#[test]
fn test_extract_single_component_on_one_axial_slice_returns_one_closed_loop() {
    let voxel = voxel_data_with_fill([5, 5, 2], [[2, 2, 1]]);
    let contour = extract_contours_from_voxel_data(&voxel, PlaneFamily::Axial)
        .expect("axial extraction should succeed");

    assert_eq!(contour.slices.len(), 1);
    assert_eq!(contour.slices[0].loops.len(), 1);
    let loop0 = &contour.slices[0].loops[0];
    assert!(loop0.is_closed);
    assert!(loop0.points.len() >= 4);
}

#[test]
fn test_extract_multiple_disconnected_components_returns_multiple_loops() {
    let voxel = voxel_data_with_fill([6, 6, 1], [[1, 1, 0], [4, 4, 0]]);
    let contour = extract_contours_from_voxel_data(&voxel, PlaneFamily::Axial)
        .expect("axial extraction should succeed");

    assert_eq!(contour.slices.len(), 1);
    assert_eq!(contour.slices[0].loops.len(), 2);
    assert!(contour.slices[0]
        .loops
        .iter()
        .all(|loop_data| loop_data.is_closed));
}

#[test]
fn test_extract_component_touching_left_top_border_still_emits_loop() {
    let voxel = voxel_data_with_fill([5, 5, 1], [[0, 0, 0], [1, 0, 0], [0, 1, 0]]);
    let contour = extract_contours_from_voxel_data(&voxel, PlaneFamily::Axial)
        .expect("axial extraction should succeed");

    assert_eq!(contour.slices.len(), 1);
    assert_eq!(contour.slices[0].loops.len(), 1);
    assert!(contour.slices[0].loops[0].is_closed);
    assert!(contour.slices[0].loops[0].points.len() >= 4);
}

#[test]
fn test_extract_sets_active_plane_family_to_requested_family() {
    let voxel = voxel_data_with_fill([3, 3, 1], [[1, 1, 0]]);
    let contour = extract_contours_from_voxel_data(&voxel, PlaneFamily::Axial)
        .expect("axial extraction should succeed");
    assert_eq!(contour.active_plane_family, PlaneFamily::Axial);
}

#[test]
fn test_extract_rejects_oblique_family_in_v1() {
    let voxel = voxel_data_with_fill([3, 3, 1], [[1, 1, 0]]);
    let result = extract_contours_from_voxel_data(&voxel, PlaneFamily::Oblique);
    assert_eq!(
        result,
        Err(VoxelContourExtractionError::UnsupportedPlaneFamily {
            family: PlaneFamily::Oblique
        })
    );
}

fn assert_loop_points_on_expected_slice(contour: &ContourData, family: PlaneFamily, depth: f32) {
    assert_eq!(contour.slices.len(), 1);
    assert_eq!(contour.slices[0].loops.len(), 1);
    let plane = contour.slices[0].plane;
    for point in &contour.slices[0].loops[0].points {
        let world = plane_local_mm_to_world_mm(point.local_mm, plane);
        let index = world_mm_to_voxel_index(world, identity_geometry([5, 5, 5]));
        let actual = match family {
            PlaneFamily::Axial => index[2],
            PlaneFamily::Coronal => index[1],
            PlaneFamily::Sagittal => index[0],
            PlaneFamily::Oblique => unreachable!(),
        };
        assert!((actual - depth).abs() < 1e-3);
    }
}

#[test]
fn test_axial_extraction_geometry_preserves_slice_depth() {
    let voxel = voxel_data_with_fill([5, 5, 5], [[2, 2, 3]]);
    let contour = extract_contours_from_voxel_data(&voxel, PlaneFamily::Axial).unwrap();
    assert_loop_points_on_expected_slice(&contour, PlaneFamily::Axial, 3.0);
}

#[test]
fn test_coronal_extraction_geometry_preserves_slice_depth() {
    let voxel = voxel_data_with_fill([5, 5, 5], [[2, 3, 2]]);
    let contour = extract_contours_from_voxel_data(&voxel, PlaneFamily::Coronal).unwrap();
    assert_loop_points_on_expected_slice(&contour, PlaneFamily::Coronal, 3.0);
}

#[test]
fn test_sagittal_extraction_geometry_preserves_slice_depth() {
    let voxel = voxel_data_with_fill([5, 5, 5], [[3, 2, 2]]);
    let contour = extract_contours_from_voxel_data(&voxel, PlaneFamily::Sagittal).unwrap();
    assert_loop_points_on_expected_slice(&contour, PlaneFamily::Sagittal, 3.0);
}

#[test]
fn test_extraction_preserves_origin_and_spacing_through_plane_local_points() {
    let geometry = VoxelGeometry::new(
        [4, 4, 3],
        [1.5, 2.0, 2.5],
        [10.0, -5.0, 3.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap();
    let mut raw = vec![
        0_u8;
        (geometry.dimensions[0] * geometry.dimensions[1] * geometry.dimensions[2])
            as usize
    ];
    raw[voxel_index([1, 2, 1], geometry.dimensions)] = 1;
    let voxel = VoxelData {
        geometry,
        raw_data: raw,
    };

    let contour = extract_contours_from_voxel_data(&voxel, PlaneFamily::Axial).unwrap();
    assert_eq!(contour.slices.len(), 1);
    let plane = contour.slices[0].plane;
    let point = contour.slices[0].loops[0].points[0];
    let world = plane_local_mm_to_world_mm(point.local_mm, plane);
    let index = world_mm_to_voxel_index(world, geometry);

    assert!((index[2] - 1.0).abs() < 1e-3);
    assert!((index[0] - index[0].round()).abs() <= 0.5 + 1e-3);
    assert!((index[1] - index[1].round()).abs() <= 0.5 + 1e-3);
}

#[test]
fn test_axial_voxel_contour_voxel_roundtrip_preserves_mask_geometry() {
    let voxel = voxel_data_with_fill(
        [7, 6, 3],
        [[1, 1, 1], [2, 1, 1], [1, 2, 1], [4, 3, 1], [4, 4, 1]],
    );

    let contour = extract_contours_from_voxel_data(&voxel, PlaneFamily::Axial)
        .expect("axial extraction should succeed");
    let rebuilt = rasterize_contours_to_voxel_data(&contour, voxel.geometry)
        .expect("roundtrip rasterization should succeed");

    assert_eq!(rebuilt.geometry, voxel.geometry);
    assert_eq!(rebuilt.raw_data, voxel.raw_data);
}

#[test]
fn test_non_identity_orientation_roundtrip_preserves_mask_geometry() {
    let orientation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2).to_array();
    let geometry =
        VoxelGeometry::new([6, 6, 3], [0.7, 1.3, 2.1], [12.0, -9.5, 4.25], orientation).unwrap();
    let voxel = voxel_data_with_geometry_fill(
        geometry,
        [[1, 1, 1], [2, 1, 1], [1, 2, 1], [3, 4, 1], [4, 4, 1]],
    );

    let contour = extract_contours_from_voxel_data(&voxel, PlaneFamily::Axial)
        .expect("axial extraction should succeed");
    let rebuilt = rasterize_contours_to_voxel_data(&contour, voxel.geometry)
        .expect("roundtrip rasterization should succeed");

    assert_eq!(rebuilt.raw_data, voxel.raw_data);
}

#[test]
fn test_extracted_points_map_back_to_expected_source_slice_and_component_bounds() {
    let voxel = voxel_data_with_fill([6, 6, 4], [[2, 2, 3], [3, 2, 3], [2, 3, 3], [3, 3, 3]]);
    let contour = extract_contours_from_voxel_data(&voxel, PlaneFamily::Axial)
        .expect("axial extraction should succeed");

    assert_eq!(contour.slices.len(), 1);
    assert_eq!(contour.slices[0].loops.len(), 1);
    let plane = contour.slices[0].plane;
    for point in &contour.slices[0].loops[0].points {
        let world = plane_local_mm_to_world_mm(point.local_mm, plane);
        let idx = world_mm_to_voxel_index(world, voxel.geometry);
        assert!((idx[2] - 3.0).abs() < 1e-3);
        assert!((1.5 - 1e-3..=3.5 + 1e-3).contains(&idx[0]));
        assert!((1.5 - 1e-3..=3.5 + 1e-3).contains(&idx[1]));
    }
}

#[test]
fn test_coronal_and_sagittal_voxel_contour_voxel_roundtrip_preserve_mask_geometry() {
    let voxel = voxel_data_with_fill(
        [6, 5, 4],
        [[2, 2, 1], [2, 2, 2], [2, 3, 1], [3, 1, 2], [4, 1, 2]],
    );

    for family in [PlaneFamily::Coronal, PlaneFamily::Sagittal] {
        let contour = extract_contours_from_voxel_data(&voxel, family)
            .expect("orthogonal extraction should succeed");
        let rebuilt = rasterize_contours_to_voxel_data(&contour, voxel.geometry)
            .expect("roundtrip rasterization should succeed");
        assert_eq!(rebuilt.raw_data, voxel.raw_data);
    }
}

#[test]
fn test_extract_preserves_hole_as_inner_loop_and_roundtrip() {
    let dimensions = [5, 5, 1];
    let mut raw_data = vec![0_u8; 25];
    for y in 1..=3 {
        for x in 1..=3 {
            if [x, y] != [2, 2] {
                raw_data[voxel_index([x, y, 0], dimensions)] = 1;
            }
        }
    }
    let voxel_data = VoxelData {
        geometry: identity_geometry(dimensions),
        raw_data: raw_data.clone(),
    };

    let contour = extract_contours_from_voxel_data(&voxel_data, PlaneFamily::Axial).unwrap();

    assert_eq!(contour.slices.len(), 1);
    assert_eq!(contour.slices[0].loops.len(), 2);
    let rebuilt = rasterize_contours_to_voxel_data(&contour, voxel_data.geometry).unwrap();
    assert_eq!(rebuilt.raw_data, raw_data);
}

#[test]
fn test_requested_slice_extraction_uses_requested_display_plane() {
    let dimensions = [4, 4, 4];
    let geometry = VoxelGeometry::new(
        dimensions,
        [1.0, 1.0, 2.0],
        [10.0, 20.0, 30.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap();
    let mut raw_data = vec![0_u8; 64];
    raw_data[voxel_index([1, 1, 2], dimensions)] = 1;
    let voxel_data = VoxelData { geometry, raw_data };
    let display_geometry = VoxelGeometry::new(
        [7, 7, 7],
        [0.5, 0.5, 1.0],
        [10.0, 20.0, 30.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap();
    // Display index 4 sits at z = 34 mm, which is the centre of the source volume's layer 2
    // (30 mm + 2 * 2 mm), the layer holding the filled voxel.
    let display_uv = crate::convert::voxel_index_to_volume_uv([3.0, 3.0, 4.0], [7, 7, 7]);
    let requested_plane =
        orthogonal_plane_from_volume_uv(PlaneFamily::Axial, display_uv, display_geometry).unwrap();

    let contour = extract_contour_slice_from_voxel_data(&voxel_data, requested_plane).unwrap();

    assert_eq!(contour.slices.len(), 1);
    assert_eq!(contour.slices[0].plane, requested_plane);
    assert!(contour.slices[0]
        .loops
        .iter()
        .any(|loop_| !loop_.points.is_empty()));
}

#[test]
fn test_oblique_slice_extraction_emits_closed_plane_local_loops() {
    let dimensions = [7, 7, 7];
    let filled = (2..=4).flat_map(|z| (2..=4).flat_map(move |y| (2..=4).map(move |x| [x, y, z])));
    let voxel = voxel_data_with_fill(dimensions, filled);
    let plane = crate::convert::oblique_plane_from_view_rotation(
        [0.5, 0.5, 0.5],
        Quat::from_rotation_y(0.45).to_array(),
        voxel.geometry,
    )
    .unwrap();

    let contour = extract_contour_slice_from_voxel_data(&voxel, plane).unwrap();

    assert_eq!(contour.active_plane_family, PlaneFamily::Oblique);
    assert_eq!(contour.slices.len(), 1);
    assert!(!contour.slices[0].loops.is_empty());
    assert!(contour.slices[0]
        .loops
        .iter()
        .all(|loop_data| loop_data.is_closed && loop_data.points.len() >= 4));
    let normal = glam::Vec3::from_array(plane.normal_mm).normalize();
    let origin = glam::Vec3::from_array(plane.origin_mm);
    for point in contour.slices[0]
        .loops
        .iter()
        .flat_map(|loop_data| &loop_data.points)
    {
        let world = glam::Vec3::from_array(plane_local_mm_to_world_mm(point.local_mm, plane));
        assert!((world - origin).dot(normal).abs() <= 1e-4);
    }
}

#[test]
fn test_oblique_slice_outside_voxel_bounds_returns_no_slices() {
    let voxel = voxel_data_with_fill([5, 5, 5], [[2, 2, 2]]);
    let mut plane = crate::convert::oblique_plane_from_view_rotation(
        [0.5, 0.5, 0.5],
        Quat::from_rotation_x(0.3).to_array(),
        voxel.geometry,
    )
    .unwrap();
    plane.origin_mm = [100.0, 100.0, 100.0];

    let contour = extract_contour_slice_from_voxel_data(&voxel, plane).unwrap();

    assert!(contour.slices.is_empty());
}
