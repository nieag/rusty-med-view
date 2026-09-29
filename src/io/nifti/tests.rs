use super::*;

#[test]
fn test_gzip_detection() {
    assert!(is_gzipped(&[0x1f, 0x8b, 0x08]));
    assert!(!is_gzipped(&[0x00, 0x00, 0x00]));
    assert!(!is_gzipped(&[0x1f])); // Too short
}

#[test]
fn test_normalize_unit_vector() {
    let v = normalize_vec3([1.0, 0.0, 0.0]);
    assert!((v[0] - 1.0).abs() < 1e-6);
    assert!((v[1]).abs() < 1e-6);
    assert!((v[2]).abs() < 1e-6);
}

#[test]
fn test_normalize_arbitrary_vector() {
    let v = normalize_vec3([3.0, 4.0, 0.0]);
    assert!((v[0] - 0.6).abs() < 1e-6);
    assert!((v[1] - 0.8).abs() < 1e-6);
    assert!((v[2]).abs() < 1e-6);
}

#[test]
fn test_normalize_zero_vector_fallback() {
    let v = normalize_vec3([0.0, 0.0, 0.0]);
    assert_eq!(v, [1.0, 0.0, 0.0]); // Fallback to X axis
}

#[test]
fn test_rotation_matrix_identity() {
    let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let q = rotation_matrix_to_quaternion(identity);
    assert!((q[0]).abs() < 1e-6);
    assert!((q[1]).abs() < 1e-6);
    assert!((q[2]).abs() < 1e-6);
    assert!((q[3] - 1.0).abs() < 1e-6);
}

#[test]
fn test_rotation_matrix_round_trip() {
    use glam::{Mat3, Quat};
    let test_quat = Quat::from_euler(glam::EulerRot::XYZ, 0.5, 0.3, 0.7);
    let mat = Mat3::from_quat(test_quat);
    let m = [
        [mat.x_axis.x, mat.y_axis.x, mat.z_axis.x],
        [mat.x_axis.y, mat.y_axis.y, mat.z_axis.y],
        [mat.x_axis.z, mat.y_axis.z, mat.z_axis.z],
    ];
    let result = rotation_matrix_to_quaternion(m);
    let result_quat = Quat::from_array(result);
    // Quaternions may differ by sign, check dot product is ±1
    assert!((test_quat.dot(result_quat)).abs() > 0.9999);
}

#[test]
fn test_calculate_orientation_ras() {
    // RAS orientation (Identity matrix)
    let srow_x = [1.0, 0.0, 0.0, 0.0];
    let srow_y = [0.0, 1.0, 0.0, 0.0];
    let srow_z = [0.0, 0.0, 1.0, 0.0];
    let q = calculate_orientation_from_rows(srow_x, srow_y, srow_z);
    assert_eq!(q, [0.0, 0.0, 0.0, 1.0]);
}

#[test]
fn test_calculate_orientation_las() {
    // LAS orientation (X is flipped)
    let srow_x = [-1.0, 0.0, 0.0, 0.0];
    let srow_y = [0.0, 1.0, 0.0, 0.0];
    let srow_z = [0.0, 0.0, 1.0, 0.0];
    let _q = calculate_orientation_from_rows(srow_x, srow_y, srow_z);

    let result = calculate_orientation_from_rows(srow_x, srow_y, srow_z);
    assert!((result[3]).abs() > 0.0);
}

#[test]
fn test_extract_origin_from_rows_uses_sform_translation() {
    let srow_x = [1.0, 0.0, 0.0, 12.0];
    let srow_y = [0.0, 1.0, 0.0, -5.0];
    let srow_z = [0.0, 0.0, 1.0, 33.5];

    let origin = extract_origin_from_rows(srow_x, srow_y, srow_z);
    assert_eq!(origin, [12.0, -5.0, 33.5]);
}

#[test]
fn test_extract_origin_from_rows_invalid_axes_fallback_to_zero() {
    let srow_x = [0.0, 0.0, 0.0, 12.0];
    let srow_y = [0.0, 0.0, 0.0, -5.0];
    let srow_z = [0.0, 0.0, 0.0, 33.5];

    let origin = extract_origin_from_rows(srow_x, srow_y, srow_z);
    assert_eq!(origin, [0.0, 0.0, 0.0]);
}

#[test]
fn test_extract_origin_from_header_falls_back_to_qform_translation() {
    let mut pixdim = [0.0; 8];
    pixdim[0] = 1.0;
    pixdim[1] = 2.0;
    pixdim[2] = 2.0;
    pixdim[3] = 3.0;
    let header = NiftiHeader {
        srow_x: [0.0, 0.0, 0.0, 0.0],
        srow_y: [0.0, 0.0, 0.0, 0.0],
        srow_z: [0.0, 0.0, 0.0, 0.0],
        sform_code: 0,
        qform_code: 1,
        pixdim,
        quatern_x: -185.74844,
        quatern_y: -178.64844,
        quatern_z: -369.0,
        ..NiftiHeader::default()
    };

    let origin = extract_origin_from_header(&header);
    assert_eq!(origin, [-185.74844, -178.64844, -369.0]);
}

#[test]
fn test_extract_orientation_from_header_falls_back_to_qform_identity() {
    let mut pixdim = [0.0; 8];
    pixdim[0] = 1.0;
    pixdim[1] = 2.0;
    pixdim[2] = 2.0;
    pixdim[3] = 3.0;
    let header = NiftiHeader {
        srow_x: [0.0, 0.0, 0.0, 0.0],
        srow_y: [0.0, 0.0, 0.0, 0.0],
        srow_z: [0.0, 0.0, 0.0, 0.0],
        sform_code: 0,
        qform_code: 1,
        pixdim,
        quatern_b: 0.0,
        quatern_c: 0.0,
        quatern_d: 0.0,
        ..NiftiHeader::default()
    };

    let orientation = extract_orientation_from_header(&header);
    assert_eq!(orientation, [0.0, 0.0, 0.0, 1.0]);
}
