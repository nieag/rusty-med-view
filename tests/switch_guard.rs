//! Guard tests for automatic primary-view switching (backlog item 0.6).
//!
//! Switching the primary contour plane family re-derives contours through the voxel hub
//! (contours -> voxel raster -> contours in the new family). These tests record how long that
//! takes, how much overlap it loses, and whether switching away and back without editing
//! restores the original data, on the QA liver labelmap.
//!
//! They take about a second in release and much longer in debug, so debug builds ignore them.
//! Run with `cargo test --release --test switch_guard` (CI does).
use rusty_med_view::app::roi::{ContourData, VoxelData, VoxelGeometry};
use rusty_med_view::convert::{
    extract_contours_from_voxel_data, rasterize_contours_to_voxel_data, PlaneFamily,
};
use rusty_med_view::nifti_loader::load_label_from_bytes;
use std::time::Instant;

fn liver_label() -> VoxelData {
    let bytes =
        std::fs::read("qa_samples/liver_0_label.nii").expect("qa_samples/liver_0_label.nii");
    let label = load_label_from_bytes(&bytes, "liver_0_label.nii".to_string()).expect("label");
    let geometry = VoxelGeometry::new(
        label.dimensions,
        label.spacing,
        label.origin,
        label.orientation,
    )
    .expect("geometry");
    // Collapse to a binary mask, which is what contour authority represents.
    let raw_data = label.data.iter().map(|v| u8::from(*v != 0)).collect();
    VoxelData { geometry, raw_data }
}

fn dice(a: &VoxelData, b: &VoxelData) -> f64 {
    assert_eq!(a.raw_data.len(), b.raw_data.len());
    let (mut both, mut left, mut right) = (0u64, 0u64, 0u64);
    for (x, y) in a.raw_data.iter().zip(&b.raw_data) {
        let (x, y) = (*x != 0, *y != 0);
        left += u64::from(x);
        right += u64::from(y);
        both += u64::from(x && y);
    }
    if left + right == 0 {
        return 1.0;
    }
    2.0 * both as f64 / (left + right) as f64
}

/// One primary-view switch as the app performs it today.
fn switch_family(from: &ContourData, geometry: VoxelGeometry, to: PlaneFamily) -> ContourData {
    let voxels = rasterize_contours_to_voxel_data(from, geometry).expect("raster");
    extract_contours_from_voxel_data(&voxels, to).expect("extract")
}

const FAMILIES: [PlaneFamily; 3] = [
    PlaneFamily::Axial,
    PlaneFamily::Coronal,
    PlaneFamily::Sagittal,
];

#[test]
#[cfg_attr(debug_assertions, ignore = "slow in debug builds; run with --release")]
fn test_deriving_contours_in_any_family_preserves_the_mask_exactly() {
    let source = liver_label();
    for family in FAMILIES {
        let contours = extract_contours_from_voxel_data(&source, family).unwrap();
        let raster = rasterize_contours_to_voxel_data(&contours, source.geometry).unwrap();

        assert!(!contours.slices.is_empty(), "{family:?} produced no slices");
        assert_eq!(
            dice(&source, &raster),
            1.0,
            "{family:?} round trip lost overlap"
        );
    }
}

#[test]
#[cfg_attr(debug_assertions, ignore = "slow in debug builds; run with --release")]
fn test_switching_family_and_back_without_edits_restores_the_original_contours() {
    let source = liver_label();
    let geometry = source.geometry;
    let axial = extract_contours_from_voxel_data(&source, PlaneFamily::Axial).unwrap();

    let coronal = switch_family(&axial, geometry, PlaneFamily::Coronal);
    let back = switch_family(&coronal, geometry, PlaneFamily::Axial);
    assert_eq!(back, axial, "one switch away and back must be lossless");

    let sagittal = switch_family(&back, geometry, PlaneFamily::Sagittal);
    let back_again = switch_family(&sagittal, geometry, PlaneFamily::Axial);
    assert_eq!(back_again, axial, "three switches must not drift");
}

/// Timing budget for a primary-view switch (backlog item 0.6).
///
/// Measured on the liver sample in release: extracting a full plane family from a current voxel
/// cache takes about 30 ms, but rasterizing contours back to voxels takes 170 to 215 ms, which is
/// over the 100 ms preview target. A switch that can extract straight from a current voxel cache
/// fits the budget inline; one that must re-rasterize first has to run in the background.
/// Timings are only asserted in release builds (`cargo test --release --test switch_guard`).
#[test]
#[cfg_attr(debug_assertions, ignore = "slow in debug builds; run with --release")]
fn test_switch_timing_budget() {
    let source = liver_label();
    for family in FAMILIES {
        let started = Instant::now();
        let contours = extract_contours_from_voxel_data(&source, family).unwrap();
        let extract_ms = started.elapsed().as_secs_f64() * 1000.0;

        let started = Instant::now();
        let _ = rasterize_contours_to_voxel_data(&contours, source.geometry).unwrap();
        let raster_ms = started.elapsed().as_secs_f64() * 1000.0;

        println!("{family:?}: extract {extract_ms:.1} ms, raster {raster_ms:.1} ms");
        if !cfg!(debug_assertions) {
            assert!(extract_ms < 100.0, "{family:?} extract {extract_ms:.1} ms");
            assert!(raster_ms < 600.0, "{family:?} raster {raster_ms:.1} ms");
        }
    }
}
