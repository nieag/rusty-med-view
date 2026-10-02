//! Guard tests for automatic primary-view switching (backlog item 0.6).
//!
//! Switching the primary contour plane family re-derives contours through the voxel hub
//! (contours -> voxel raster -> contours in the new family). These tests record how long that
//! takes, how much overlap it loses, and whether switching away and back without editing
//! restores the original data, on the QA liver labelmap.
//!
//! They take about a second in release and much longer in debug, so debug builds ignore them.
//! Run with `cargo test --release --test switch_guard` (CI does).
use rusty_med_view::convert::{
    changed_mesh_chunks, contour_distance_field, contours_from_mesh,
    extract_contours_from_voxel_data, extract_mesh_from_voxel_data, mesh_from_contour_field,
    rasterize_contours_to_voxel_data, smooth_mesh_field_from_signed_distance, ContourFieldBuild,
    ContourFieldState, IncrementalChunkedMeshRebuild, IncrementalMeshVoxelization,
    DEFAULT_MESH_CHUNK_SIZE,
};
use rusty_med_view::model::{ContourData, OrthogonalFamily, VoxelData, VoxelGeometry};
use rusty_med_view::nifti_loader::load_label_from_bytes;
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

/// These tests assert timings, so they must not compete for CPU with each other: cargo runs the
/// tests of one file in parallel, and a timing measured under that contention means nothing.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Runs `work` a few times and returns the fastest time in milliseconds with its last result.
/// The minimum measures the code; a single run also measures whatever else the machine is doing.
fn best_of<T>(runs: usize, mut work: impl FnMut() -> T) -> (f64, T) {
    let mut best = f64::INFINITY;
    let mut result = None;
    for _ in 0..runs {
        let started = Instant::now();
        let value = work();
        best = best.min(started.elapsed().as_secs_f64() * 1000.0);
        result = Some(value);
    }
    (best, result.expect("at least one run"))
}

/// Timing runs per measurement.
const TIMING_RUNS: usize = 3;

// Budgets are about five times the measurement on an idle machine (extract 30 to 100 ms, raster
// 170 to 215 ms, deform base 6 ms and update up to 60 mm about 100 ms). They exist to catch an
// order-of-magnitude regression, and must not flake when the machine is busy: the rasterizer and
// the mesh code use several threads, so a loaded machine is 3 to 4 times slower. The numbers the
// tests print are the record to compare by hand.
const EXTRACT_BUDGET_MS: f64 = 500.0;
const RASTER_BUDGET_MS: f64 = 1_500.0;
const DEFORM_BASE_BUDGET_MS: f64 = 1_000.0;
const DEFORM_UPDATE_BUDGET_MS: f64 = 1_000.0;
const DIRTY_MESH_BUDGET_MS: f64 = 150.0;
const MESH_VOXELIZE_BUDGET_MS: f64 = 2_000.0;
const MESH_CUT_BUDGET_MS: f64 = 100.0;
const CONTOUR_FIELD_BUDGET_MS: f64 = 400.0;
const FIELD_EDIT_BUDGET_MS: f64 = 60.0;

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
fn switch_family(from: &ContourData, geometry: VoxelGeometry, to: OrthogonalFamily) -> ContourData {
    let voxels = rasterize_contours_to_voxel_data(from, geometry).expect("raster");
    extract_contours_from_voxel_data(&voxels, to).expect("extract")
}

const FAMILIES: [OrthogonalFamily; 3] = [
    OrthogonalFamily::Axial,
    OrthogonalFamily::Coronal,
    OrthogonalFamily::Sagittal,
];

#[test]
#[cfg_attr(debug_assertions, ignore = "slow in debug builds; run with --release")]
fn test_deriving_contours_in_any_family_preserves_the_mask_exactly() {
    let _serial = serial();
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
    let _serial = serial();
    let source = liver_label();
    let geometry = source.geometry;
    let axial = extract_contours_from_voxel_data(&source, OrthogonalFamily::Axial).unwrap();

    let coronal = switch_family(&axial, geometry, OrthogonalFamily::Coronal);
    let back = switch_family(&coronal, geometry, OrthogonalFamily::Axial);
    assert_eq!(back, axial, "one switch away and back must be lossless");

    let sagittal = switch_family(&back, geometry, OrthogonalFamily::Sagittal);
    let back_again = switch_family(&sagittal, geometry, OrthogonalFamily::Axial);
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
    let _serial = serial();
    let source = liver_label();
    for family in FAMILIES {
        let (extract_ms, contours) = best_of(TIMING_RUNS, || {
            extract_contours_from_voxel_data(&source, family).unwrap()
        });
        let (raster_ms, _) = best_of(TIMING_RUNS, || {
            rasterize_contours_to_voxel_data(&contours, source.geometry).unwrap()
        });

        println!("{family:?}: extract {extract_ms:.1} ms, raster {raster_ms:.1} ms");
        if !cfg!(debug_assertions) {
            assert!(
                extract_ms < EXTRACT_BUDGET_MS,
                "{family:?} extract {extract_ms:.1} ms"
            );
            assert!(
                raster_ms < RASTER_BUDGET_MS,
                "{family:?} raster {raster_ms:.1} ms"
            );
        }
    }
}

/// The mesh brush runs on every pointer move of a drag, so its per-update cost is a budget too.
/// A push into the liver must stay valid (the collision limit) and fast enough to feel live.
#[test]
#[cfg_attr(debug_assertions, ignore = "slow in debug builds; run with --release")]
fn test_mesh_deform_update_is_fast_and_keeps_the_liver_valid() {
    let _serial = serial();
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
        let (ms, result) = best_of(TIMING_RUNS, || {
            deform_mesh_surface_brush_limited(
                &base,
                &mesh,
                face.vertex_indices,
                anchor,
                [0.0, 0.0, -depth],
                12.0,
                1.0,
            )
        });
        last = result;
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
        assert!(
            base_ms < DEFORM_BASE_BUDGET_MS,
            "base build {base_ms:.1} ms"
        );
        assert!(
            worst_ms < DEFORM_UPDATE_BUDGET_MS,
            "worst deform update {worst_ms:.1} ms"
        );
    }
}

/// A dirty-region mesh rebuild must equal a clean full rebuild on the real liver; also records
/// the timing of the incremental setup and chunk work.
#[test]
#[cfg_attr(debug_assertions, ignore = "slow in debug builds; run with --release")]
fn test_liver_dirty_mesh_rebuild_matches_clean_full_rebuild() {
    let _serial = serial();
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
    // A local edit rebuilds a few chunks (about 7 ms idle; the full rebuild is about 800 ms).
    assert!(
        (setup_duration + chunk_duration).as_secs_f64() * 1000.0 < DIRTY_MESH_BUDGET_MS,
        "a one-voxel edit must not rebuild the whole mesh"
    );
    let clean = full_rebuild(&voxels);
    assert_eq!(
        canonical_triangle_bits(&rebuilt.merged_mesh()),
        canonical_triangle_bits(&clean.merged_mesh())
    );
}

/// The mesh to voxels step of a mesh to contour switch. It was a point-in-mesh query per voxel
/// (2.5 s on the liver); the scanline version takes about 0.2 s including the self-intersection
/// validation and about 25 ms without it, and must reproduce the mask exactly.
#[test]
#[cfg_attr(debug_assertions, ignore = "slow in debug builds; run with --release")]
fn test_mesh_to_voxels_is_fast_and_exact_on_the_liver() {
    let _serial = serial();
    let source = liver_label();
    let mesh = extract_mesh_from_voxel_data(&source).expect("mesh");
    let (best_ms, voxelized) = best_of(TIMING_RUNS, || {
        let mut work = IncrementalMeshVoxelization::begin(&mesh, source.geometry).expect("begin");
        work.step(usize::MAX);
        work.into_result().expect("complete")
    });
    println!("liver mesh to voxels (validating): {best_ms:.0} ms");
    assert_eq!(
        voxelized.raw_data, source.raw_data,
        "the round trip must be exact"
    );
    assert!(
        best_ms < MESH_VOXELIZE_BUDGET_MS,
        "mesh voxelization took {best_ms:.0} ms (budget {MESH_VOXELIZE_BUDGET_MS} ms)"
    );
}

/// Mesh to contours by cutting the mesh with the layer planes: no voxels, exact. The cut contours
/// of the liver surface, filled back into voxels, must give the liver mask again, and the cut must
/// take a few tens of milliseconds at most (about 15 ms idle).
#[test]
#[cfg_attr(debug_assertions, ignore = "slow in debug builds; run with --release")]
fn test_cutting_the_liver_mesh_into_contours_is_fast_and_exact() {
    let _serial = serial();
    let source = liver_label();
    let mesh = extract_mesh_from_voxel_data(&source).expect("mesh");
    let (best_ms, cut) = best_of(TIMING_RUNS, || {
        contours_from_mesh(&mesh, source.geometry, OrthogonalFamily::Axial).expect("cut")
    });
    println!(
        "liver mesh to contours by cutting: {best_ms:.1} ms ({} slices)",
        cut.slices.len()
    );
    let filled = rasterize_contours_to_voxel_data(&cut, source.geometry).expect("raster");
    assert_eq!(
        filled.raw_data, source.raw_data,
        "cut and fill must reproduce the mask"
    );
    assert!(
        best_ms < MESH_CUT_BUDGET_MS,
        "cutting took {best_ms:.0} ms (budget {MESH_CUT_BUDGET_MS} ms)"
    );
}

/// Contours to a mesh through the signed distance field (ADR 0006): the first full build of a
/// liver-sized stack. Later edits update only a local region, so this is the worst case.
#[test]
#[cfg_attr(debug_assertions, ignore = "slow in debug builds; run with --release")]
fn test_liver_contour_distance_field_and_mesh_timing() {
    let _serial = serial();
    let source = liver_label();
    let contours =
        extract_contours_from_voxel_data(&source, OrthogonalFamily::Axial).expect("contours");
    let (field_ms, field) = best_of(TIMING_RUNS, || {
        contour_distance_field(&contours, source.geometry, None)
            .expect("field")
            .expect("non-empty")
    });
    let (mesh_ms, mesh) = best_of(TIMING_RUNS, || mesh_from_contour_field(&field));
    println!(
        "liver contours -> field: {field_ms:.0} ms ({} samples), field -> mesh: {mesh_ms:.0} ms ({} triangles)",
        field.values.len(),
        mesh.faces.len()
    );
    assert!(!mesh.faces.is_empty());
    assert!(
        field_ms + mesh_ms < CONTOUR_FIELD_BUDGET_MS,
        "field and mesh took {:.0} ms (budget {CONTOUR_FIELD_BUDGET_MS} ms)",
        field_ms + mesh_ms
    );
}

/// One edited slice of a liver stack: the field is updated from the previous state (only that
/// slice's distances are recomputed) and only the chunks the change reaches are re-meshed.
#[test]
#[cfg_attr(debug_assertions, ignore = "slow in debug builds; run with --release")]
fn test_editing_one_liver_contour_slice_updates_the_field_locally() {
    let _serial = serial();
    let source = liver_label();
    let contours =
        extract_contours_from_voxel_data(&source, OrthogonalFamily::Axial).expect("contours");
    let before = ContourFieldState::build(None, &contours, source.geometry, None)
        .expect("field")
        .expect("non-empty");
    let mut edited = contours.clone();
    let middle = edited.slices.len() / 2;
    for contour_loop in &mut edited.slices[middle].loops {
        for point in &mut contour_loop.points {
            point.local_mm[0] += 1.7;
        }
    }
    let keep = Some(before.field.geometry);
    let (build_ms, after) = best_of(TIMING_RUNS, || {
        ContourFieldState::build(Some(&before), &edited, source.geometry, keep)
            .expect("field")
            .expect("non-empty")
    });
    let (reuse_ms, _) = best_of(TIMING_RUNS, || {
        ContourFieldState::build(Some(&before), &contours, source.geometry, keep)
            .expect("field")
            .expect("non-empty")
    });
    println!("liver field rebuilt with nothing changed: {reuse_ms:.0} ms");
    let (diff_ms, changed) = best_of(TIMING_RUNS, || {
        changed_mesh_chunks(
            after.field.geometry,
            &before.field.values,
            &after.field.values,
            DEFAULT_MESH_CHUNK_SIZE,
        )
    });
    let smooth = smooth_mesh_field_from_signed_distance(after.field.geometry, &after.field.values);
    let old_mesh = IncrementalChunkedMeshRebuild::begin_full_from_field(
        before.field.geometry,
        smooth_mesh_field_from_signed_distance(before.field.geometry, &before.field.values),
        DEFAULT_MESH_CHUNK_SIZE,
    )
    .map(|mut rebuild| {
        while !rebuild.step_field().expect("step") {}
        rebuild.into_result().expect("mesh")
    })
    .expect("full mesh");
    let (mesh_ms, _) = best_of(TIMING_RUNS, || {
        let mut rebuild = IncrementalChunkedMeshRebuild::begin_changed_from_field(
            old_mesh.clone(),
            after.field.geometry,
            smooth.clone(),
            changed.clone(),
        )
        .expect("rebuild");
        while !rebuild.step_field().expect("step") {}
        rebuild.into_result().expect("mesh")
    });
    println!(
        "liver slice edit: field {build_ms:.0} ms, diff {diff_ms:.0} ms, {} chunks re-meshed in {mesh_ms:.0} ms",
        changed.len()
    );
    assert!(!changed.is_empty());
    let total = build_ms + diff_ms + mesh_ms;
    assert!(
        total < FIELD_EDIT_BUDGET_MS,
        "a slice edit took {total:.0} ms (budget {FIELD_EDIT_BUDGET_MS} ms)"
    );
}

/// An edit that grows the box (a loop drawn well outside the shape): the per-slice distances of
/// the unchanged slices are kept and only the new border is computed, so this is far cheaper than
/// the first build.
#[test]
#[cfg_attr(debug_assertions, ignore = "slow in debug builds; run with --release")]
fn test_a_box_growing_edit_of_the_liver_field_does_not_rebuild_every_slice() {
    let _serial = serial();
    let source = liver_label();
    let contours =
        extract_contours_from_voxel_data(&source, OrthogonalFamily::Axial).expect("contours");
    let before = ContourFieldState::build(None, &contours, source.geometry, None)
        .expect("field")
        .expect("non-empty");
    let mut edited = contours.clone();
    let middle = edited.slices.len() / 2;
    let mut far = edited.slices[middle].loops[0].clone();
    for point in &mut far.points {
        point.local_mm[0] += 25.0;
        point.local_mm[1] += 20.0;
    }
    edited.slices[middle].loops.push(far);
    let keep = Some(before.field.geometry);
    let (grow_ms, after) = best_of(TIMING_RUNS, || {
        ContourFieldBuild::begin(Some(&before), &edited, source.geometry, keep)
            .expect("field")
            .map(|mut build| {
                while !build.step(None) {}
                build.finish()
            })
            .expect("non-empty")
    });
    let (full_ms, _) = best_of(TIMING_RUNS, || {
        ContourFieldState::build(None, &edited, source.geometry, keep)
            .expect("field")
            .expect("non-empty")
    });
    println!(
        "liver box-growing edit: {grow_ms:.0} ms with the old slices kept, {full_ms:.0} ms from scratch ({:?} -> {:?})",
        before.field.geometry.dimensions(),
        after.field.geometry.dimensions()
    );
    assert_ne!(
        before.field.geometry.identity(),
        after.field.geometry.identity(),
        "the box grew"
    );
    assert!(
        grow_ms < full_ms * 0.6,
        "growing took {grow_ms:.0} ms against {full_ms:.0} ms from scratch"
    );
}
