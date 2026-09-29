# ROI Workstation Plan

Prioritized backlog from the 2026-09-29 full-codebase review. Work top-to-bottom; a later phase never starts before the earlier phase's exit criteria hold, except items marked *parallel*. Every item leaves `cargo test`, `cargo clippy --all-targets --all-features -D warnings`, `cargo check --target wasm32-unknown-unknown`, and `cargo fmt --check` green and lands as its own commit. Effort: S under half a day, M 1 to 3 days, L over 3 days.

Findings marked **[repro]** were reproduced with a throwaway test during the review. Each becomes a permanent regression test that fails before its fix.

## Product constraints

These drive the design.

- Each representation (voxel, contour, mesh) can be authoritative. Voxel authority is a read-only source, for example a deep-learning prediction. Users edit contours and meshes, not voxels.
- Switching between representations is smooth and automatic. No manual "promote" step, no visible geometry swap.
- Exactly one contour plane family is primary and editable at a time; all other views are derived and stay consistent with it. The primary family follows the view the user edits in.
- Oblique views are derived, per-slice views only. They never become the primary contour set.
- Switching primary view without editing is lossless and instant. A switch is an undo step, never a history wipe.

## Done

- [x] Dead code: duplicate plane-compatibility check, test-only delegates, deprecated `SlicePlane::from_viewport`.
- [x] Docs archive pruned to the three documents still cited (`docs-archive-full` tag holds the rest).
- [x] Inline test modules moved to sibling `tests.rs` files.
- [x] QA snapshot and sample bootstrap moved out of `app/mod.rs` into `app/qa/`.
- [x] Baseline: 331 tests pass, 6 ignored milestone tests pass in release (about 3 s), clippy, wasm, and rustfmt clean.

## Phase 0: Correctness and crashes (do first, all small)

Exit: every item has a test that failed before the fix; no known silent data loss or reachable panic.

- [x] **0.1 Voxel cache drops contour slices [repro] (S, highest priority).** Two slice commits before one rebuild leave the voxel cache holding only the last slice, flagged current. In `process_contour_voxel_rebuild_for_entity`, never trim the contour data to the dirty slice unless a valid base cache exists; fall back to a full rebuild otherwise. Also stop `replace_contour_data_for_slice` from discarding earlier queued dirty regions (merge them).
- [x] **0.2 Full vs incremental rasterizer disagree [repro] (S).** A plane between two voxel layers (default cursor on an even-sized axis) fills 2 layers in the full path and 1 in the incremental path. Snap contour slice planes to voxel-layer centers when created, and make both paths use one depth rule.
- [x] **0.3 Annotation notes panic on non-ASCII (S).** `annotations.rs` slices `&ann.note[..61]` by bytes. Truncate on a char boundary.
- [x] **0.4 Wrong slice number in the viewport overlay (S).** Overlay uses `round(uv * dim)`; navigation uses `uv * (dim-1)`. Use `slice_index_from_cursor_uv` and one indexing convention.
- [x] **0.5 Label import can panic (S).** `VoxelGeometry::new` accepts spacing 1e-5 but `RoiGeometry::from_legacy_parts` rejects it as singular, and `Roi::new_voxel_with_cache` `expect`s the result. Return the error to the caller and show it in the status area.
- [x] **0.6 Guard tests for the switch (M).** `tests/switch_guard.rs` on the liver sample (run in release: `cargo test --release --test switch_guard`, a CI step; debug builds ignore them because they take about 17 s there). Measured in release: extracting a full plane family from the voxel cache takes about 30 ms; rasterizing contours back to voxels takes 170 to 215 ms; re-deriving is lossless today (Dice exactly 1.0 for all three families) and switching away and back restores the original loops exactly, also after three switches. Consequences for Phase 2: a switch that extracts straight from a current voxel cache fits the 100 ms budget inline; a switch that must re-rasterize first (about 200 ms) has to run in the background. Contour simplification (3.3) will introduce loss and must keep these tests within an explicit Dice bound.

## Phase 1: Orientation and geometry foundation

Exit: one geometry type and one voxel-center convention everywhere; a LAS, an RAS, and a permuted-axis fixture all display, register, and label correctly.

- [x] **1.1 Preserve reflection in NIfTI orientation [repro] (M).** The loader now builds the full affine from the sform (`sform_code > 0`) or qform, so a LAS volume places voxel 9 at x=82 mm instead of 118 mm. Behaviour change: an `srow` with `sform_code == 0` is now ignored, per the NIfTI spec.
- [x] **1.2 One IJK-to-world affine (L).** `VoxelGeometry` is the only geometry type: dimensions plus an affine and its cached inverse, validated at construction (singular or non-finite transforms are rejected, which also removes the old label-import panic path at its root). `RoiGeometry` and `from_legacy_parts` are deleted; `VolumeData` and the loader outputs carry a validated geometry; overlay registration composes the affines exactly. `spacing()`, `origin()`, `orientation()` remain as derived views. `f32` world coordinates were kept where they were.
- [x] **1.3 One voxel-center convention (M).** Cell-centred, edge-to-edge UV (`(index + 0.5) / dim`) everywhere, per the ADR (now documented there). `src/convert/coord_mapping.rs` is the single home of the index/UV mapping; the overlay shader path mirrors it; the conflicting helpers are gone; cursors start on voxel centres (volume load, QA preset); HU readout uses the shared helper. Tests pin image texel, overlay cell, and CPU sample index to agree at every UV, and resolution independence.
- [x] **1.4 Orientation-aware display (M).** The R/L/A/P/S/I markers and the 3D gizmo letters are derived from the affine (`index_axis_letters`, `SlicePlane::edge_letters`) instead of assuming RAS storage, so LAS, LPS, and permuted volumes are labelled truthfully, and a viewport that shows a different anatomical plane than its name (permuted axes) says so. The QA snapshot exposes each 2D viewport's `edge_letters`. Not done: the 2D display is still index-space, so LAS data is shown in storage order (labelled truthfully) rather than reoriented; true patient-axis reslicing is a separate, larger change and remains open.
- [x] **1.5 One slice-match tolerance (S).** `planes_are_same_slice` in `convert/geometry.rs` replaces the three copies of the fixed 0.5 mm rule (render, editing, extraction alignment): orthogonal planes match when they select the same voxel layer (the same rule the rasterizer uses), oblique planes within half the smallest spacing. Adjacent layers of sub-half-millimetre grids no longer merge into one contour.
- [x] **1.6 Oblique mapping is skewed and off-plane [repro] (L).** The oblique plane is now defined by rotating in the volume's physical frame (`VoxelGeometry::axis_frame`, the same frame the 3D view rotates), so its axes are orthonormal in millimetres and contour local coordinates are true millimetres. The uv basis passed to the shader is now volume-UV per millimetre with a millimetre window sized to the volume's projection onto the plane axes (the shader formula is unchanged, only the uniforms' meaning). The reslice is planar and unsheared, its window has the true physical aspect, and no corner is cropped (the old window was in normalized units and cropped volumes whose axes differ in size). Tests cover planarity, orthogonality, window aspect, full-volume identity view, and round trips; the ignored target test is enabled. Behaviour changes: rotated oblique views on anisotropic volumes now show a different (correct) plane and are zoomed out to contain the volume. Verified by CPU property tests and a clean live run; the rendered oblique image itself has not been checked visually.
- [x] **1.7 Import hardening (S).** `xyzt_units` is honoured (metres and microns are converted to millimetres; unknown is treated as millimetres); label values must be integers in 0..=255 (probability maps and wide ids are rejected with the voxel and value instead of being clamped); gzip expansion is bounded at 2 GiB; voxel read errors propagate instead of becoming zeros.

## Phase 2: Automatic switching and the ROI model

Exit: the sidebar has no promotion controls; switching primary view keeps undo history; Phase 0.7 tests pass with the numbers recorded here.

- [ ] **2.1 Generic `Derived<T>` cache (M).** One type for value plus source revision plus geometry identity, replacing hand-rolled voxel, contour-view, mesh, and preview cache logic. The previous family's derived contours stay cached until an edit invalidates them (gives the lossless no-edit switch).
- [ ] **2.2 `RoiBody` enum with per-authority state (L).** `Voxel`, `Contour`, `Mesh` variants own their edit state, preview, and history. Editing code takes the concrete type, so wrong-authority errors and `Missing*` variants disappear. Also fixes the split between global `EditorState` history and per-ROI caches.
- [ ] **2.3 Automatic primary-view switch (L).** First edit gesture in a non-primary view makes that view's family primary: the clicked view's derived loops start the gesture, a background job re-derives the full set, and the edit commits when ready. Voxel and mesh ROIs become contour-primary on the first contour gesture, keeping the voxel source as an immutable baseline. The switch is an undo step (snapshot already includes the family).
- [ ] **2.4 Remove manual promotion (M).** Delete the family dropdown, "Make displayed view editable", `RequiresConversion`, the seven promotion error enums, and `clear_roi_edit_history_for_roi` on switch. One error type for real failures. Report loss (volume delta or Dice) on each conversion.
- [ ] **2.5 Scope decisions (S).** Decided: a contour tool on a mesh-primary ROI converts it to contour authority through the voxel cache, with the loss reported, and the conversion is one undo step (the mesh stays reachable through undo). Still to do: remove voxel-to-authority promotion (`promote_current_voxel_cache_to_authority`, `VoxelAuthorityPromotionError`) if confirmed unneeded; voxel stays a derived export form.

## Phase 3: Editing features and data completeness

Exit: a deep-learning multi-organ segmentation can be imported, corrected, and exported without merging structures.

- [ ] **3.1 Multi-label ROIs (M).** Decided: import one ROI per label. Today the overlay colours per label but SDF, contour, and mesh derivation use `!= 0`, and rasterization writes `1`, so a multi-organ import merges into one structure. Split a labelmap into one binary voxel ROI per non-zero label sharing the label grid's geometry, named after the file and label id, with the label's LUT colour. Memory note: N full-size binary masks; cropping each mask to its bounding box is tracked in 4.5. Requires the cache integrity fix in 0.1 (done).
- [ ] **3.2 Subtract/erase drawing (M).** Contour drawing is union-only and a loop inside an existing loop cannot make a hole. Add subtract and hole-creating modes on `geo` boolean ops.
- [ ] **3.3 Simplify extracted contours (M).** Voxel to contour produces a per-pixel staircase (hundreds of vertices per loop), which makes point-editing painful and inflates undo snapshots. Add simplification (Douglas-Peucker or marching squares with a tolerance) with a measured Dice bound.
- [ ] **3.4 NIfTI labelmap export (M).** Export a selected ROI from its owned geometry, refuse stale-cache export, reload and verify geometry and occupied bounds. Depends on 0.1 and 1.1.
- [ ] **3.5 Status area and messages (S).** Persistent active-ROI status (name, authority, tool, lock, derived-work state); concise processing, ready, and actionable failure messages; visible failure for GPU init on web.

## Phase 4: Performance and scale

Do after Phases 0 to 2 so measurements reflect the final structure. Measure each against a large volume (for example 512x512x300) before and after.

- [ ] **4.1 Loader (M).** Per-voxel `get_f64` loop and up to three CPU copies of the intensities; wasm parse runs on the main thread with no progress. Fast typed path, drop copies, chunk or yield with progress.
- [ ] **4.2 Image texture format (M).** R32Float is 4 bytes per voxel and nearest-only (oblique slices look blocky). Evaluate R16Float or normalized u16 with `float32-filterable` or linear sampling.
- [ ] **4.3 GPU mesh rendering (L).** Meshes are projected on the CPU every frame, compared and re-uploaded whole, and drawn flat and translucent with no depth or lighting. Upload world-space vertices once, project on the GPU with a uniform matrix, add depth and normals.
- [ ] **4.4 Real incremental mesh rebuild, or delete the plumbing (M).** `begin_for_voxel_aabb` ignores its AABB and rebuilds the full SDF plus all chunks on every edit. Either implement banded SDF updates or remove the dirty-region machinery around it.
- [ ] **4.5 Memory (M).** Undo keeps up to 32 full mesh and contour clones; the SDF path allocates about five full-volume arrays; each mesh-drag event clones and re-validates the mesh. Use delta or shared-structure snapshots, and avoid clone per drag event.
- [ ] **4.6 Small hot spots (S).** `roi_voxel_stats` scans every voxel of every ROI per frame while the Layers panel is open; cache by generation. Rebuild of overlay primitives runs twice per frame.

## Phase 5: Structure cleanup

- [ ] **5.1 Split `roi_runtime.rs` and `components.rs` by concern (M).** After Phase 2 they are much smaller: coordinator, per-representation rebuild, import, view building; viewport, ROI state, editing.
- [ ] **5.2 Delete unused public items (S).** 7 have no callers (`mesh_cache_mut`, `create_voxel_roi_from_label`, `uv_to_world_axis`, `cursor_uv_to_world_depth`, `ijk_to_world_affine`, `world_to_ijk_affine`, `create_blank_labelmap`); 22 are test-only. Verify each; much overlaps Phase 1.
- [ ] **5.3 Test-support module (S).** Move the remaining `cfg(test)` ROI creation helpers out of `roi_runtime.rs`.
- [ ] **5.4 One viewport-mapping implementation (M).** The `screen_aspect / slice_aspect` logic is duplicated in the shader, `geometry.rs`, `picking.rs`, and `input.rs`; `ORTHOGRAPHIC_VIEW_SCALE` is duplicated in Rust and WGSL. Unify in `convert/` and pass constants via uniforms. Collapse `SlicePlane`, `PlaneFamily`, and `ViewMode` overlap.
- [ ] **5.5 Sidebar and input dedupe (S).** Repeated status-message match arms in `input.rs` and `sidebar.rs`, and a duplicated `focus_cursor_on_first_extracted_slice` branch.
- [ ] **5.6 Dependency audit (S).** Check `geo`, `parry3d`, `uuid`, and others are all needed; each adds build time.

## Phase 6: Platform, release, and hygiene (*parallel* with Phases 3 to 5)

- [ ] **6.1 WebGL fallback limits (S).** `request_device` uses default limits, which normally fail on a WebGL2 adapter even though the `webgl` feature is enabled. Use downlevel limits with the adapter's resolution, or drop the feature.
- [ ] **6.2 Keep QA data out of production (S).** `index.html` copies `qa_samples` (28 MB) into the Pages deploy and `?qa=1` enables the debug API there. Gate behind a build feature or a dev-only Trunk config.
- [~] **6.3 CI (S).** Done: the QA spec (`tests/qa1_viewerqa.spec.js`) is a real gate: it launches Chrome with WebGPU (macOS defaults; override with `QA_CHROME_ARGS`), fails clearly when the app cannot initialize (`QA_ALLOW_NO_GPU=1` opts into the old lenient behaviour), waits for readiness instead of a fixed delay, and a new `qa-4` asserts the liver geometry, orientation letters, MPR viewport facts, and zero errors or warnings. `Trunk.toml` now ignores non-source directories so test artifacts no longer live-reload the page mid-run (restart `trunk serve` to pick it up). Still open: CI does not run the spec (GitHub runners have no WebGPU adapter; needs a software adapter), CI clippy still omits `--all-targets --all-features`, and the toolchain and `trunk` are unpinned; align the clippy flags documented in `clippy.toml`, `AGENTS.md`, and `ci.yml`.
- [ ] **6.4 LICENSE and attribution (S, needs your input).** No LICENSE file; the marching-cubes table derives from an Apache-2.0 crate with only a code comment. Add a LICENSE and a NOTICE.
- [ ] **6.5 GPU readback for visual QA (M).** Headless and headed Playwright cannot capture the WebGPU canvas. Add a wgpu readback behind `__viewerQa.screenshot()` for visual regression checks.
- [ ] **6.6 Small UX and platform items (S).** Windowing sliders, presets, and "HU" labels are CT-specific; make ranges follow `intensity_range`. `ScreenDescriptor` uses `window.scale_factor()` while layout uses `ctx.pixels_per_point()` and diverges under egui zoom. File picker leaks its `<input>` on cancel. Scroll accumulator is shared across viewports. Viewport mouse capture during drags.
- [ ] **6.7 Repo hygiene (S).** `qa_samples/` (28 MB) in git history; decide LFS. `target/` is 24 GB.

## Decisions

Resolved:

1. Multi-label handling (3.1): one ROI per label.
2. Voxel-centre convention (1.3): cell-centred, edge-to-edge (implemented).
3. Contour tool on a mesh-primary ROI (2.5): convert with loss reported, as one undo step.

Still open:

4. Whether non-CT modalities (MR, PET) are in scope (6.6).
5. License choice (6.4).
6. Visual check of the oblique view and a LAS-stored file (1.4, 1.6): deferred by request.

## Not tasks now

- A general ECS rewrite or generic job framework. Phase 2 is a targeted change to how ROIs are typed and accessed.
- New contour, voxel, or meshing algorithms beyond 3.2 and 3.3. Meshing work is tracked in `docs/mesh-authority-and-meshing-plan.md`.
- GPU compute migration without a measured bottleneck.

## Carried over from the earlier backlog

Product items not scheduled above: ROI catalog (select, visibility, opacity, rename, lock, eight-overlay limit), contour authoring polish (hover target, close-loop guidance, shortcut hints), sidebar overlay-cap message and its regression test, annotation scope decision (native WGPU or provisional).

## Later, only with evidence

- Profile representative large and multi-ROI workloads before optimizing further.
- GPU compute only if a measured conversion path misses its interaction budget.
- Session persistence, registration/resampling, DICOM, or collaboration after the author-to-export loop is useful.
