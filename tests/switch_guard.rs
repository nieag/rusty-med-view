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
    let geometry = label.geometry;
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

/// The mesh brush runs on every pointer move of a drag, so its per-update cost is a budget too.
/// A push into the liver must stay valid (the collision limit) and fast enough to feel live.
#[test]
#[cfg_attr(debug_assertions, ignore = "slow in debug builds; run with --release")]
fn test_mesh_deform_update_is_fast_and_keeps_the_liver_valid() {
    use rusty_med_view::convert::{
        deform_mesh_surface_brush_limited, extract_mesh_from_voxel_data,
        validate_mesh_for_voxelization, MeshDeformBase,
    };
    let source = liver_label();
    let mesh = extract_mesh_from_voxel_data(&source).unwrap();

    let started = Instant::now();
    let base = MeshDeformBase::new(&mesh).unwrap();
    let base_ms = started.elapsed().as_secs_f64() * 1000.0;

    // Push the topmost triangle straight down through the liver, deeper than it is thick.
    let (face, top) = mesh
        .faces
        .iter()
        .map(|face| {
            let z = face
                .vertex_indices
                .iter()
                .map(|i| mesh.vertices[*i as usize].world_mm[2])
                .sum::<f32>()
                / 3.0;
            (face, z)
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    let anchor = mesh.vertices[face.vertex_indices[0] as usize].world_mm;
    let mut worst_ms = 0.0_f64;
    let mut last = (mesh.clone(), 1.0);
    for depth in [5.0_f32, 20.0, 60.0, 200.0] {
        let started = Instant::now();
        last = deform_mesh_surface_brush_limited(
            &base,
            &mesh,
            face.vertex_indices,
            anchor,
            [0.0, 0.0, -depth],
            12.0,
            1.0,
        );
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        println!(
            "push {depth} mm (top z {top:.1}): fraction {:.3}, {ms:.1} ms",
            last.1
        );
        // A 200 mm push moves the whole liver; it only has to stay valid, not stay interactive.
        if depth <= 60.0 {
            worst_ms = worst_ms.max(ms);
        }
    }
    println!("base build {base_ms:.1} ms, worst update up to 60 mm {worst_ms:.1} ms");
    validate_mesh_for_voxelization(&last.0).expect("the limited push keeps the liver valid");
    if !cfg!(debug_assertions) {
        assert!(base_ms < 400.0, "base build {base_ms:.1} ms");
        assert!(worst_ms < 250.0, "worst deform update {worst_ms:.1} ms");
    }
}

/// A dirty-region mesh rebuild must equal a clean full rebuild on the real liver; also records
/// the timing of the incremental setup and chunk work.
#[test]
#[cfg_attr(debug_assertions, ignore = "slow in debug builds; run with --release")]
fn test_liver_dirty_mesh_rebuild_matches_clean_full_rebuild() {
    use rusty_med_view::convert::{
        ChunkedMeshData, IncrementalChunkedMeshRebuild, DEFAULT_MESH_CHUNK_SIZE,
    };
    use rusty_med_view::model::{MeshData, VoxelData};

    fn full_rebuild(voxels: &VoxelData) -> ChunkedMeshData {
        let mut work =
            IncrementalChunkedMeshRebuild::begin_full(voxels, DEFAULT_MESH_CHUNK_SIZE).unwrap();
        while !work.step(voxels).unwrap() {}
        work.into_result().unwrap()
    }

    fn canonical_triangle_bits(mesh: &MeshData) -> Vec<[[u32; 3]; 3]> {
        let mut triangles = mesh
            .faces
            .iter()
            .map(|face| {
                let mut points = face
                    .vertex_indices
                    .map(|index| mesh.vertices[index as usize].world_mm.map(f32::to_bits));
                points.sort();
                points
            })
            .collect::<Vec<_>>();
        triangles.sort();
        triangles
    }

    let mut voxels = liver_label();
    let base = full_rebuild(&voxels);
    let changed_index = voxels
        .raw_data
        .iter()
        .position(|value| *value != 0)
        .unwrap();
    voxels.raw_data[changed_index] = 0;
    let [width, height, _] = voxels.geometry.dimensions();
    let changed = [
        changed_index as u32 % width,
        (changed_index as u32 / width) % height,
        changed_index as u32 / (width * height),
    ];
    let setup_started = Instant::now();
    let mut work = IncrementalChunkedMeshRebuild::begin_for_voxel_aabb(
        base,
        &voxels,
        changed,
        changed.map(|value| value + 1),
    )
    .unwrap();
    let setup_duration = setup_started.elapsed();
    let chunk_started = Instant::now();
    while !work.step(&voxels).unwrap() {}
    let chunk_duration = chunk_started.elapsed();
    let rebuilt = work.into_result().unwrap();
    println!("liver dirty mesh rebuild: setup={setup_duration:?}, chunks={chunk_duration:?}");
    let clean = full_rebuild(&voxels);
    assert_eq!(
        canonical_triangle_bits(&rebuilt.merged_mesh()),
        canonical_triangle_bits(&clean.merged_mesh())
    );
}
