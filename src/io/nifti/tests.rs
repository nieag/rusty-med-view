use super::*;

fn header(sform_code: i16, qform_code: i16, srow: [[f32; 4]; 3]) -> NiftiHeader {
    NiftiHeader {
        srow_x: srow[0],
        srow_y: srow[1],
        srow_z: srow[2],
        sform_code,
        qform_code,
        pixdim: [1.0, 2.0, 2.0, 3.0, 0.0, 0.0, 0.0, 0.0],
        ..NiftiHeader::default()
    }
}

const NO_SFORM: [[f32; 4]; 3] = [[0.0; 4]; 3];

fn world_of(geometry: VoxelGeometry, ijk: [f64; 3]) -> [f64; 3] {
    geometry.ijk_to_world_mm(ijk)
}

fn approx(actual: [f64; 3], expected: [f64; 3]) {
    for axis in 0..3 {
        assert!(
            (actual[axis] - expected[axis]).abs() < 1e-4,
            "{actual:?} != {expected:?}"
        );
    }
}

/// Minimal single-file NIfTI-1 (uint8, sform only) for end-to-end loader tests.
fn build_nifti(dimensions: [i16; 3], srow: [[f32; 4]; 3]) -> Vec<u8> {
    let voxels = dimensions.iter().map(|d| *d as usize).product::<usize>();
    let mut bytes = vec![0u8; 352 + voxels];
    bytes[0..4].copy_from_slice(&348i32.to_le_bytes());
    let dims = [
        3i16,
        dimensions[0],
        dimensions[1],
        dimensions[2],
        1,
        1,
        1,
        1,
    ];
    for (index, dim) in dims.iter().enumerate() {
        bytes[40 + index * 2..42 + index * 2].copy_from_slice(&dim.to_le_bytes());
    }
    bytes[70..72].copy_from_slice(&2i16.to_le_bytes()); // uint8
    bytes[72..74].copy_from_slice(&8i16.to_le_bytes());
    let pixdim = [1.0f32, 2.0, 2.0, 3.0, 0.0, 0.0, 0.0, 0.0];
    for (index, value) in pixdim.iter().enumerate() {
        bytes[76 + index * 4..80 + index * 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[108..112].copy_from_slice(&352f32.to_le_bytes());
    bytes[112..116].copy_from_slice(&1f32.to_le_bytes());
    bytes[254..256].copy_from_slice(&1i16.to_le_bytes()); // sform_code
    for (row, offset) in [280usize, 296, 312].into_iter().enumerate() {
        for column in 0..4 {
            bytes[offset + column * 4..offset + column * 4 + 4]
                .copy_from_slice(&srow[row][column].to_le_bytes());
        }
    }
    bytes[344..348].copy_from_slice(b"n+1\0");
    bytes
}

#[test]
fn test_gzip_detection() {
    assert!(is_gzipped(&[0x1f, 0x8b, 0x08]));
    assert!(!is_gzipped(&[0x00, 0x00, 0x00]));
    assert!(!is_gzipped(&[0x1f])); // Too short
}

#[test]
fn test_ras_sform_maps_ijk_to_scaled_world_with_translation() {
    let header = header(
        1,
        0,
        [
            [2.0, 0.0, 0.0, 12.0],
            [0.0, 2.0, 0.0, -5.0],
            [0.0, 0.0, 3.0, 33.5],
        ],
    );
    let geometry = geometry_from_header(&header, [10, 4, 4]).unwrap();

    approx(world_of(geometry, [0.0, 0.0, 0.0]), [12.0, -5.0, 33.5]);
    approx(world_of(geometry, [9.0, 3.0, 2.0]), [30.0, 1.0, 39.5]);
}

#[test]
fn test_las_sform_keeps_its_reflection_in_world_placement() {
    // LAS: voxel index i runs toward -x. Voxel 0 sits at x=100, voxel 9 at 100 - 9*2 = 82.
    let header = header(
        1,
        0,
        [
            [-2.0, 0.0, 0.0, 100.0],
            [0.0, 2.0, 0.0, 0.0],
            [0.0, 0.0, 3.0, 0.0],
        ],
    );
    let geometry = geometry_from_header(&header, [10, 4, 4]).unwrap();

    approx(world_of(geometry, [0.0, 0.0, 0.0]), [100.0, 0.0, 0.0]);
    approx(world_of(geometry, [9.0, 0.0, 0.0]), [82.0, 0.0, 0.0]);
    assert_eq!(geometry.spacing(), [2.0, 2.0, 3.0]);
}

#[test]
fn test_sform_shear_is_preserved() {
    let header = header(
        1,
        0,
        [
            [1.0, 0.5, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ],
    );
    let geometry = geometry_from_header(&header, [4, 4, 4]).unwrap();

    approx(world_of(geometry, [0.0, 1.0, 0.0]), [0.5, 1.0, 0.0]);
}

#[test]
fn test_sform_is_ignored_when_its_code_is_zero() {
    let mut header = header(
        0,
        1,
        [
            [9.0, 0.0, 0.0, 99.0],
            [0.0, 9.0, 0.0, 99.0],
            [0.0, 0.0, 9.0, 99.0],
        ],
    );
    header.pixdim[0] = 1.0;
    header.quatern_x = -185.74844;
    header.quatern_y = -178.64844;
    header.quatern_z = -369.0;
    let geometry = geometry_from_header(&header, [4, 4, 4]).unwrap();

    approx(
        world_of(geometry, [0.0, 0.0, 0.0]),
        [-185.74844, -178.64844, -369.0],
    );
    assert_eq!(geometry.spacing(), [2.0, 2.0, 3.0]);
}

#[test]
fn test_degenerate_sform_falls_back_to_qform_translation_and_identity_orientation() {
    let mut header = header(1, 1, NO_SFORM);
    header.pixdim[0] = 1.0;
    header.quatern_x = -185.74844;
    header.quatern_y = -178.64844;
    header.quatern_z = -369.0;
    let geometry = geometry_from_header(&header, [4, 4, 4]).unwrap();

    approx(
        world_of(geometry, [0.0, 0.0, 0.0]),
        [-185.74844, -178.64844, -369.0],
    );
    assert_eq!(geometry.orientation(), [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(geometry.spacing(), [2.0, 2.0, 3.0]);
}

#[test]
fn test_missing_spatial_transform_falls_back_to_pixdim_scale_at_the_origin() {
    let header = header(0, 0, NO_SFORM);
    let geometry = geometry_from_header(&header, [4, 4, 4]).unwrap();

    approx(world_of(geometry, [0.0, 0.0, 0.0]), [0.0, 0.0, 0.0]);
    approx(world_of(geometry, [1.0, 1.0, 1.0]), [2.0, 2.0, 3.0]);
}

#[test]
fn test_unusable_spatial_transform_and_pixdim_is_rejected_not_defaulted() {
    let mut header = header(0, 0, NO_SFORM);
    header.pixdim = [1.0, 0.0, 2.0, 3.0, 0.0, 0.0, 0.0, 0.0];

    assert!(matches!(
        geometry_from_header(&header, [4, 4, 4]),
        Err(LoadError::InvalidGeometry(_))
    ));
}

#[test]
fn test_loading_a_las_volume_and_label_places_voxels_where_the_sform_says() {
    let srow = [
        [-2.0, 0.0, 0.0, 100.0],
        [0.0, 2.0, 0.0, 0.0],
        [0.0, 0.0, 3.0, 0.0],
    ];
    let bytes = build_nifti([10, 4, 4], srow);

    let volume = load_nifti_from_bytes(&bytes).expect("volume loads");
    approx(
        volume.geometry.ijk_to_world_mm([9.0, 0.0, 0.0]),
        [82.0, 0.0, 0.0],
    );

    let label = load_label_from_bytes(&bytes, "las.nii".to_string()).expect("label loads");
    approx(
        label.geometry.ijk_to_world_mm([9.0, 0.0, 0.0]),
        [82.0, 0.0, 0.0],
    );
    assert_eq!(volume.geometry.identity(), label.geometry.identity());
}

#[test]
fn test_loading_a_label_with_a_singular_grid_is_rejected_by_the_loader() {
    // Spacing 1e-5 mm makes the affine numerically singular; it used to reach ROI construction
    // and abort the app.
    let srow = [
        [1.0e-5, 0.0, 0.0, 0.0],
        [0.0, 1.0e-5, 0.0, 0.0],
        [0.0, 0.0, 1.0e-5, 0.0],
    ];
    let bytes = build_nifti([4, 4, 4], srow);

    assert!(matches!(
        load_label_from_bytes(&bytes, "tiny.nii".to_string()),
        Err(LoadError::InvalidGeometry(_))
    ));
    assert!(matches!(
        load_nifti_from_bytes(&bytes),
        Err(LoadError::InvalidGeometry(_))
    ));
}
