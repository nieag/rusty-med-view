//! Guard for the scale target (backlog 2b.10): 100 to 200 ROIs on a large volume.
//!
//! The case is synthetic but its count and sizes are real: a 512 x 512 x 300 labelmap with 150
//! labels, each a copy of the QA liver mask (clipped to its cell of a 6 x 5 x 5 grid). Before
//! labels were cropped to their bounding boxes, each ROI held three full-volume copies (body,
//! cache, GPU texture), about 236 MB, so this import needed about 35 GB.
//!
//! Run with `cargo test --release --test scale_guard -- --nocapture` (CI does).
use rusty_med_view::app::roi::label_import::{present_label_ids, split_labelmap};
use rusty_med_view::components::{Roi, RoiId};
use rusty_med_view::model::{VoxelData, VoxelGeometry};
use rusty_med_view::nifti_loader::load_label_from_bytes;
use std::time::Instant;

const DIMENSIONS: [u32; 3] = [512, 512, 300];
const GRID: [usize; 3] = [6, 5, 5];

/// What one ROI costs on the GPU for its box (one byte per voxel), which `approx_bytes` leaves
/// out when the test builds ROIs without a device.
fn gpu_mirror_bytes(roi: &Roi) -> usize {
    roi.voxel_cache()
        .map_or(0, |cache| cache.data.raw_data.len())
}

/// The ceiling for the 150-ROI case, in bytes, counting the CPU copies and the GPU mirror.
/// Measured about 130 MB; the ceiling leaves room for the larger liver crops and history.
const MEMORY_CEILING_BYTES: usize = 400 * 1024 * 1024;

fn synthetic_labelmap() -> Vec<u8> {
    let bytes =
        std::fs::read("qa_samples/liver_0_label.nii").expect("qa_samples/liver_0_label.nii");
    let label = load_label_from_bytes(&bytes, "liver_0_label.nii".to_string()).expect("label");
    let source = label.geometry.dimensions().map(|value| value as usize);
    let at = |x: usize, y: usize, z: usize| label.data[(z * source[1] + y) * source[0] + x] != 0;
    // The liver's bounding box is the template.
    let mut min = [usize::MAX; 3];
    let mut max = [0usize; 3];
    for z in 0..source[2] {
        for y in 0..source[1] {
            for x in 0..source[0] {
                if at(x, y, z) {
                    for (axis, c) in [x, y, z].into_iter().enumerate() {
                        min[axis] = min[axis].min(c);
                        max[axis] = max[axis].max(c);
                    }
                }
            }
        }
    }
    let [width, height, depth] = DIMENSIONS.map(|value| value as usize);
    let cell = [width / GRID[0], height / GRID[1], depth / GRID[2]];
    let mut map = vec![0_u8; width * height * depth];
    for index in 0..GRID[0] * GRID[1] * GRID[2] {
        let cell_index = [
            index % GRID[0],
            (index / GRID[0]) % GRID[1],
            index / (GRID[0] * GRID[1]),
        ];
        let label = (index + 1) as u8;
        for dz in 0..cell[2].min(max[2] - min[2] + 1) {
            for dy in 0..cell[1].min(max[1] - min[1] + 1) {
                for dx in 0..cell[0].min(max[0] - min[0] + 1) {
                    if at(min[0] + dx, min[1] + dy, min[2] + dz) {
                        let x = cell_index[0] * cell[0] + dx;
                        let y = cell_index[1] * cell[1] + dy;
                        let z = cell_index[2] * cell[2] + dz;
                        map[(z * height + y) * width + x] = label;
                    }
                }
            }
        }
    }
    map
}

#[test]
#[cfg_attr(debug_assertions, ignore = "slow in debug builds; run with --release")]
fn test_150_labels_on_a_large_volume_fit_the_memory_ceiling() {
    let map = synthetic_labelmap();
    let geometry =
        VoxelGeometry::new(DIMENSIONS, [0.8, 0.8, 1.0], [0.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap();
    let labels = present_label_ids(&map);
    assert_eq!(labels.len(), 150, "the synthetic case must hold 150 labels");

    let started = Instant::now();
    let masks = split_labelmap(&map, DIMENSIONS, &labels).expect("within the import budget");
    let split_ms = started.elapsed().as_secs_f64() * 1000.0;

    let started = Instant::now();
    let rois: Vec<Roi> = masks
        .into_iter()
        .map(|mask| {
            let mask_geometry = geometry.cropped(mask.min, mask.dimensions).unwrap();
            Roi::new_voxel_in_grid(
                RoiId(u64::from(mask.label)),
                format!("label {}", mask.label),
                geometry,
                VoxelData {
                    geometry: mask_geometry,
                    raw_data: mask.data,
                },
                None,
            )
            .0
        })
        .collect();
    let build_ms = started.elapsed().as_secs_f64() * 1000.0;

    let total: usize = rois
        .iter()
        .map(|roi| roi.approx_bytes() + gpu_mirror_bytes(roi))
        .sum();
    let full_volume = DIMENSIONS.iter().product::<u32>() as usize;
    println!(
        "150 ROIs: {:.0} MB (vs {:.0} MB uncropped), split {split_ms:.0} ms, build {build_ms:.0} ms",
        total as f64 / 1e6,
        (150 * 3 * full_volume) as f64 / 1e6,
    );
    assert!(
        total < MEMORY_CEILING_BYTES,
        "150 ROIs hold {:.0} MB, over the {:.0} MB ceiling",
        total as f64 / 1e6,
        MEMORY_CEILING_BYTES as f64 / 1e6
    );
}
