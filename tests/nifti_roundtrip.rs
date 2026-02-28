/// Integration tests: load sample NIfTI files and verify basic properties.
///
/// Requires liver_0.nii and liver_0_label.nii in the crate root.
use rusty_med_view::nifti_loader::{load_label_from_bytes, load_nifti_from_bytes};

#[test]
fn load_liver_volume() {
    let data = std::fs::read("liver_0.nii").expect("liver_0.nii not found in crate root");
    let vol = load_nifti_from_bytes(&data).expect("Failed to load liver_0.nii");

    // Dimensions must be non-zero on all axes
    assert!(vol.dimensions[0] > 0);
    assert!(vol.dimensions[1] > 0);
    assert!(vol.dimensions[2] > 0);

    // Voxel spacing must be positive
    assert!(vol.spacing[0] > 0.0);
    assert!(vol.spacing[1] > 0.0);
    assert!(vol.spacing[2] > 0.0);

    // Data length must match declared dimensions
    let expected_len = (vol.dimensions[0] * vol.dimensions[1] * vol.dimensions[2]) as usize;
    assert_eq!(vol.float_data.len(), expected_len);

    // Intensity range must be ordered and finite
    assert!(vol.intensity_range[0].is_finite());
    assert!(vol.intensity_range[1].is_finite());
    assert!(vol.intensity_range[0] <= vol.intensity_range[1]);
}

#[test]
fn load_liver_label() {
    let data =
        std::fs::read("liver_0_label.nii").expect("liver_0_label.nii not found in crate root");
    let label = load_label_from_bytes(&data, "liver_0_label.nii".to_string())
        .expect("Failed to load label");

    // Dimensions must be non-zero on all axes
    assert!(label.dimensions[0] > 0);
    assert!(label.dimensions[1] > 0);
    assert!(label.dimensions[2] > 0);

    // Data length must match declared dimensions
    let expected_len = (label.dimensions[0] * label.dimensions[1] * label.dimensions[2]) as usize;
    assert_eq!(label.data.len(), expected_len);

    // Filename should be preserved
    assert_eq!(label.filename, "liver_0_label.nii");
}
