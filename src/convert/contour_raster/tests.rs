use super::*;
use crate::convert::{orthogonal_plane_from_volume_uv, PlaneFamily};
use crate::model::OrthogonalFamily;
use crate::model::{ContourLoop, ContourPoint, ContourSlice};

fn index(dimensions: [u32; 3], x: u32, y: u32, z: u32) -> usize {
    (z as usize * dimensions[1] as usize + y as usize) * dimensions[0] as usize + x as usize
}

fn identity_geometry(dimensions: [u32; 3]) -> VoxelGeometry {
    VoxelGeometry::new(
        dimensions,
        [1.0, 1.0, 1.0],
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap()
}

fn square_loop(half_extent: f32) -> ContourLoop {
    ContourLoop {
        points: vec![
            ContourPoint {
                local_mm: [-half_extent, -half_extent],
            },
            ContourPoint {
                local_mm: [half_extent, -half_extent],
            },
            ContourPoint {
                local_mm: [half_extent, half_extent],
            },
            ContourPoint {
                local_mm: [-half_extent, half_extent],
            },
        ],
        is_closed: true,
    }
}

#[test]
fn test_rasterize_simple_axial_square_fill() {
    let geometry = identity_geometry([5, 5, 3]);
    let plane = orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 0.5], geometry)
        .expect("axial plane should resolve");
    let contour = ContourData {
        active_plane_family: OrthogonalFamily::Axial,
        slices: vec![ContourSlice {
            plane,
            loops: vec![square_loop(1.4)],
        }],
    };

    let voxel = rasterize_contours_to_voxel_data(&contour, geometry).expect("raster succeeds");
    let occupied = voxel.raw_data.iter().filter(|v| **v != 0).count();

    assert_eq!(occupied, 9);
    assert_eq!(voxel.raw_data[index(geometry.dimensions, 2, 2, 1)], 1);
    assert_eq!(voxel.raw_data[index(geometry.dimensions, 0, 0, 1)], 0);
    assert_eq!(voxel.raw_data[index(geometry.dimensions, 2, 2, 0)], 0);
}

#[test]
fn test_rasterize_empty_contours_returns_zero_filled_voxel_data() {
    let geometry = identity_geometry([4, 3, 2]);
    let contour = ContourData {
        active_plane_family: OrthogonalFamily::Axial,
        slices: Vec::new(),
    };

    let voxel = rasterize_contours_to_voxel_data(&contour, geometry).expect("raster succeeds");

    assert_eq!(voxel.geometry, geometry);
    assert_eq!(voxel.raw_data.len(), 24);
    assert!(voxel.raw_data.iter().all(|v| *v == 0));
}

#[test]
fn test_rasterize_skips_invalid_open_loop() {
    let geometry = identity_geometry([5, 5, 1]);
    let plane = orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 0.0], geometry)
        .expect("axial plane should resolve");
    let contour = ContourData {
        active_plane_family: OrthogonalFamily::Axial,
        slices: vec![ContourSlice {
            plane,
            loops: vec![ContourLoop {
                points: vec![
                    ContourPoint {
                        local_mm: [-2.0, -2.0],
                    },
                    ContourPoint {
                        local_mm: [2.0, -2.0],
                    },
                    ContourPoint {
                        local_mm: [2.0, 2.0],
                    },
                    ContourPoint {
                        local_mm: [-2.0, 2.0],
                    },
                ],
                is_closed: false,
            }],
        }],
    };

    let voxel = rasterize_contours_to_voxel_data(&contour, geometry).expect("raster succeeds");
    assert!(voxel.raw_data.iter().all(|v| *v == 0));
}

#[test]
fn test_rasterize_multiple_loops_use_even_odd_fill() {
    let geometry = identity_geometry([7, 7, 1]);
    // Depth UV 0.0 is the outer face of the volume; slice 0 is centred at UV 0.5 of one voxel.
    let plane = orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 0.5], geometry)
        .expect("axial plane should resolve");
    let contour = ContourData {
        active_plane_family: OrthogonalFamily::Axial,
        slices: vec![ContourSlice {
            plane,
            loops: vec![square_loop(2.4), square_loop(1.4)],
        }],
    };

    let voxel = rasterize_contours_to_voxel_data(&contour, geometry).expect("raster succeeds");
    let occupied = voxel.raw_data.iter().filter(|v| **v != 0).count();

    assert_eq!(occupied, 16);
    assert_eq!(voxel.raw_data[index(geometry.dimensions, 3, 3, 0)], 0);
    assert_eq!(voxel.raw_data[index(geometry.dimensions, 1, 3, 0)], 1);
}

#[test]
fn test_rasterize_preserves_target_geometry_in_result() {
    let geometry = VoxelGeometry::new(
        [3, 4, 2],
        [0.7, 1.1, 2.3],
        [5.0, -2.0, 8.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap();
    let contour = ContourData {
        active_plane_family: OrthogonalFamily::Sagittal,
        slices: Vec::new(),
    };

    let voxel = rasterize_contours_to_voxel_data(&contour, geometry).expect("raster succeeds");
    assert_eq!(voxel.geometry, geometry);
}

#[test]
fn test_preview_slice_raster_preserves_unaffected_voxel_slices() {
    let geometry = identity_geometry([4, 4, 4]);
    let plane =
        orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 1.0 / 3.0], geometry)
            .unwrap();
    let contour = ContourData {
        active_plane_family: OrthogonalFamily::Axial,
        slices: vec![ContourSlice {
            plane,
            loops: vec![square_loop(1.25)],
        }],
    };
    let mut base = VoxelData {
        geometry,
        raw_data: vec![0; 64],
    };
    base.raw_data[index(geometry.dimensions, 0, 0, 2)] = 1;

    let preview = rasterize_contour_preview_slices_to_voxel_data(&contour, &base).unwrap();

    assert_eq!(preview.raw_data[index(geometry.dimensions, 0, 0, 2)], 1);
    assert!(preview.raw_data[16..32].iter().any(|value| *value != 0));
    assert!(preview.raw_data[..16].iter().all(|value| *value == 0));
}

#[test]
fn test_preview_slice_raster_updates_coronal_and_sagittal_depth_axes() {
    let geometry = identity_geometry([5, 6, 7]);
    for (family, depth_axis, depth, cursor_uv) in [
        (OrthogonalFamily::Coronal, 1, 2, [0.5, 2.0 / 5.0, 0.5]),
        (OrthogonalFamily::Sagittal, 0, 3, [3.0 / 4.0, 0.5, 0.5]),
    ] {
        let plane = orthogonal_plane_from_volume_uv(family.into(), cursor_uv, geometry).unwrap();
        let contour = ContourData {
            active_plane_family: family,
            slices: vec![ContourSlice {
                plane,
                loops: vec![square_loop(1.5)],
            }],
        };
        let base = VoxelData {
            geometry,
            raw_data: vec![0; 5 * 6 * 7],
        };

        let preview = rasterize_contour_preview_slices_to_voxel_data(&contour, &base).unwrap();

        let mut occupied = 0;
        for z in 0..7 {
            for y in 0..6 {
                for x in 0..5 {
                    if preview.raw_data[index(geometry.dimensions, x, y, z)] == 0 {
                        continue;
                    }
                    occupied += 1;
                    assert_eq!([x, y, z][depth_axis], depth);
                }
            }
        }
        assert!(
            occupied > 0,
            "{family:?} preview should fill its edited slab"
        );
    }
}

#[test]
fn test_contour_slice_aabb_is_one_max_exclusive_voxel_slab() {
    let geometry = identity_geometry([8, 7, 6]);
    let plane =
        orthogonal_plane_from_volume_uv(PlaneFamily::Coronal, [0.5, 3.0 / 6.0, 0.5], geometry)
            .unwrap();
    let contour = ContourData {
        active_plane_family: OrthogonalFamily::Coronal,
        slices: vec![ContourSlice {
            plane,
            loops: vec![square_loop(1.0)],
        }],
    };

    assert_eq!(
        contour_slices_voxel_aabb(&contour, geometry),
        Some(([0, 3, 0], [8, 4, 6]))
    );
}

#[test]
fn test_contour_geometry_aabb_tightens_in_plane_bounds() {
    let geometry = identity_geometry([8, 8, 4]);
    let plane =
        orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 1.0 / 3.0], geometry)
            .unwrap();
    let contour = ContourData {
        active_plane_family: OrthogonalFamily::Axial,
        slices: vec![ContourSlice {
            plane,
            loops: vec![square_loop(1.0)],
        }],
    };

    assert_eq!(
        contour_geometry_voxel_aabb(&contour, geometry),
        Some(([2, 2, 1], [6, 6, 2]))
    );
}

#[test]
fn test_full_and_slice_local_rasterizers_agree_for_every_plane_depth() {
    use crate::convert::{
        orthogonal_plane_from_volume_uv, rasterize_contour_preview_slices_to_voxel_data,
    };

    for dim_z in [4_u32, 5] {
        let geometry =
            VoxelGeometry::new([8, 8, dim_z], [1.0; 3], [0.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap();
        let mut plane =
            orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 0.5], geometry).unwrap();
        let square = ContourLoop {
            points: [[-2.2, -2.2], [2.2, -2.2], [2.2, 2.2], [-2.2, 2.2]]
                .into_iter()
                .map(|local_mm| ContourPoint { local_mm })
                .collect(),
            is_closed: true,
        };
        let layers = |data: &VoxelData| -> Vec<usize> {
            (0..dim_z as usize)
                .map(|z| {
                    data.raw_data[z * 64..(z + 1) * 64]
                        .iter()
                        .filter(|value| **value != 0)
                        .count()
                })
                .collect()
        };

        // Centered, exactly between two layers (even axes), just off center, and outside.
        let base_z = plane.origin_mm[2];
        for offset in [0.0_f32, 0.3, -0.3, 20.0, -20.0] {
            plane.origin_mm[2] = base_z + offset;
            let contour = ContourData {
                active_plane_family: OrthogonalFamily::Axial,
                slices: vec![ContourSlice {
                    plane,
                    loops: vec![square.clone()],
                }],
            };
            let full = rasterize_contours_to_voxel_data(&contour, geometry).unwrap();
            let base = VoxelData {
                geometry,
                raw_data: vec![0; (8 * 8 * dim_z) as usize],
            };
            let local = rasterize_contour_preview_slices_to_voxel_data(&contour, &base).unwrap();

            assert_eq!(
                layers(&full),
                layers(&local),
                "dim_z {dim_z}, plane offset {offset}"
            );
            let filled = layers(&full).iter().filter(|count| **count > 0).count();
            assert!(filled <= 1, "a slice must fill at most one voxel layer");
        }
    }
}

#[test]
fn test_a_repeated_point_in_a_loop_does_not_fill_the_whole_slice() {
    let geometry = VoxelGeometry::new([5, 5, 3], [1.0; 3], [0.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap();
    let plane =
        orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 0.5], geometry).unwrap();
    // A square around one voxel centre, with its second corner entered twice (a double click).
    let corners = [
        [-0.4, -0.4],
        [0.4, -0.4],
        [0.4, -0.4],
        [0.4, 0.4],
        [-0.4, 0.4],
    ];
    let contour = ContourData {
        active_plane_family: OrthogonalFamily::Axial,
        slices: vec![ContourSlice {
            plane,
            loops: vec![ContourLoop {
                points: corners.map(|local_mm| ContourPoint { local_mm }).to_vec(),
                is_closed: true,
            }],
        }],
    };

    let filled = rasterize_contours_to_voxel_data(&contour, geometry).unwrap();

    assert_eq!(filled.raw_data.iter().filter(|v| **v != 0).count(), 1);
}

#[test]
fn test_a_very_short_edge_does_not_claim_distant_points() {
    let geometry = VoxelGeometry::new([5, 5, 3], [1.0; 3], [0.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap();
    let plane =
        orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 0.5], geometry).unwrap();
    // A square around one voxel centre with a corner entered twice, a hair apart.
    let corners = [
        [-0.4, -0.4],
        [0.4, -0.4],
        [0.4000086, -0.3999962],
        [0.4, 0.4],
        [-0.4, 0.4],
    ];
    let contour = ContourData {
        active_plane_family: OrthogonalFamily::Axial,
        slices: vec![ContourSlice {
            plane,
            loops: vec![ContourLoop {
                points: corners.map(|local_mm| ContourPoint { local_mm }).to_vec(),
                is_closed: true,
            }],
        }],
    };

    let filled = rasterize_contours_to_voxel_data(&contour, geometry).unwrap();

    assert_eq!(filled.raw_data.iter().filter(|v| **v != 0).count(), 1);
}
