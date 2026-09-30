/// Integration tests: load sample NIfTI files and verify basic properties.
use rusty_med_view::nifti_loader::{load_label_from_bytes, load_nifti_from_bytes};
use std::path::PathBuf;

fn sample_path(name: &str) -> PathBuf {
    let qa_path = PathBuf::from("qa_samples").join(name);
    if qa_path.exists() {
        qa_path
    } else {
        PathBuf::from(name)
    }
}

#[test]
fn load_liver_volume() {
    let data = std::fs::read(sample_path("liver_0.nii"))
        .expect("liver_0.nii not found in qa_samples/ or crate root");
    let vol = load_nifti_from_bytes(&data).expect("Failed to load liver_0.nii");

    // Dimensions must be non-zero on all axes
    assert!(vol.dimensions[0] > 0);
    assert!(vol.dimensions[1] > 0);
    assert!(vol.dimensions[2] > 0);

    // Voxel spacing must be positive
    let spacing = vol.geometry.spacing();
    assert!(spacing[0] > 0.0);
    assert!(spacing[1] > 0.0);
    assert!(spacing[2] > 0.0);

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
    let data = std::fs::read(sample_path("liver_0_label.nii"))
        .expect("liver_0_label.nii not found in qa_samples/ or crate root");
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

#[test]
fn liver_label_splits_into_separate_liver_and_tumor_masks() {
    use rusty_med_view::app::roi::label_import::{present_label_ids, split_labelmap};

    let data = std::fs::read(sample_path("liver_0_label.nii"))
        .expect("liver_0_label.nii not found in qa_samples/ or crate root");
    let label = load_label_from_bytes(&data, "liver_0_label.nii".to_string())
        .expect("Failed to load label");

    // The sample holds liver (1) and tumor (2); importing it as one ROI merged the tumor into the
    // liver on the first contour or mesh conversion.
    let ids = present_label_ids(&label.data);
    assert_eq!(ids, vec![1, 2]);

    let masks = split_labelmap(&label.data, label.geometry.dimensions(), &ids).unwrap();
    let counts: Vec<usize> = masks
        .iter()
        .map(|mask| mask.data.iter().filter(|value| **value != 0).count())
        .collect();
    assert_eq!(counts, vec![113_169, 546]);
    // Each mask is cropped to its label: smaller than the volume, and the tumor much smaller
    // than the liver.
    assert!(masks.iter().all(|mask| mask.data.len() < label.data.len()));
    assert!(masks[1].data.len() < masks[0].data.len());
}
