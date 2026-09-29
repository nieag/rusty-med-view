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

- [ ] **0.1 Voxel cache drops contour slices [repro] (S, highest priority).** Two slice commits before one rebuild leave the voxel cache holding only the last slice, flagged current. In `process_contour_voxel_rebuild_for_entity`, never trim the contour data to the dirty slice unless a valid base cache exists; fall back to a full rebuild otherwise. Also stop `replace_contour_data_for_slice` from discarding earlier queued dirty regions (merge them).
- [ ] **0.2 Full vs incremental rasterizer disagree [repro] (S).** A plane between two voxel layers (default cursor on an even-sized axis) fills 2 layers in the full path and 1 in the incremental path. Snap contour slice planes to voxel-layer centers when created, and make both paths use one depth rule.
- [ ] **0.3 Annotation notes panic on non-ASCII (S).** `annotations.rs` slices `&ann.note[..61]` by bytes. Truncate on a char boundary.
- [ ] **0.4 Wrong slice number in the viewport overlay (S).** Overlay uses `round(uv * dim)`; navigation uses `uv * (dim-1)`. Use `slice_index_from_cursor_uv` and one indexing convention.
- [ ] **0.5 Label import can panic (S).** `VoxelGeometry::new` accepts spacing 1e-5 but `RoiGeometry::from_legacy_parts` rejects it as singular, and `Roi::new_voxel_with_cache` `expect`s the result. Return the error to the caller and show it in the status area.
- [ ] **0.6 Oblique plane axes not orthogonal [repro] (S).** Compound rotations on anisotropic spacing give u·v = -0.21 and a 1.5 to 2 mm local/world round-trip error. Orthonormalize in `oblique_plane_from_view_rotation`.
- [ ] **0.7 Guard tests for the switch (M).** On the liver sample: time to re-derive all slices in a new plane family (decides background job vs inline within the 100 ms / 200 ms budget), Dice across a contour to voxel to contour round trip, and exact restore of the original loops after a no-edit switch away and back.

## Phase 1: Orientation and geometry foundation

Exit: one geometry type and one voxel-center convention everywhere; a LAS, an RAS, and a permuted-axis fixture all display, register, and label correctly.

- [ ] **1.1 Preserve reflection in NIfTI orientation [repro] (M).** A LAS sform currently loses its flip: voxel 9 lands at x=118 mm instead of 82 mm. Keep the full affine from sform or qform; delete the quaternion decomposition path. Replace the weak `test_calculate_orientation_las` with real placement assertions.
- [ ] **1.2 One IJK-to-world affine (L).** `RoiGeometry` (`f64` affine) becomes the only geometry type; delete `VoxelGeometry` decomposition and `from_legacy_parts`. `f32` world coordinates are sufficient at millimetre scale; keep them where they are.
- [ ] **1.3 One voxel-center convention (M).** The main image is sampled with edge-to-edge texture coordinates (`uv*dim`), while overlays, contours, picking, and HU readout use `uv*(dim-1)`. The mismatch is 0 at the center and up to 0.5 voxel at the edges. Adopt the ADR's edge-to-edge convention (integer IJK are centers; bounds `[-0.5, dim-0.5]`) in the shader and CPU helpers, delete the conflicting `coord_mapping` helpers, and add CPU-side golden tests for image/overlay/contour agreement.
- [ ] **1.4 Orientation-aware display (M).** Anatomical marker letters (R/L/A/P/S/I) are hard-coded and the 2D planes are index-space, so they can be wrong for LAS or permuted volumes. Derive letters from the affine and name planes by anatomical axis, or reslice to patient axes; until then label the display as index-space.
- [ ] **1.5 One slice-match tolerance (S).** The fixed 0.5 mm "same slice" tolerance is duplicated in `roi_views.rs`, `contour_editing.rs`, and `voxel_contour_extract.rs`. Replace with one shared function that is half the voxel spacing along the plane normal.
- [ ] **1.6 Import hardening (S).** Read `xyzt_units`, warn or convert on non-mm; reject or warn on labels above 255; cap gzip expansion size.

## Phase 2: Automatic switching and the ROI model

Exit: the sidebar has no promotion controls; switching primary view keeps undo history; Phase 0.7 tests pass with the numbers recorded here.

- [ ] **2.1 Generic `Derived<T>` cache (M).** One type for value plus source revision plus geometry identity, replacing hand-rolled voxel, contour-view, mesh, and preview cache logic. The previous family's derived contours stay cached until an edit invalidates them (gives the lossless no-edit switch).
- [ ] **2.2 `RoiBody` enum with per-authority state (L).** `Voxel`, `Contour`, `Mesh` variants own their edit state, preview, and history. Editing code takes the concrete type, so wrong-authority errors and `Missing*` variants disappear. Also fixes the split between global `EditorState` history and per-ROI caches.
- [ ] **2.3 Automatic primary-view switch (L).** First edit gesture in a non-primary view makes that view's family primary: the clicked view's derived loops start the gesture, a background job re-derives the full set, and the edit commits when ready. Voxel and mesh ROIs become contour-primary on the first contour gesture, keeping the voxel source as an immutable baseline. The switch is an undo step (snapshot already includes the family).
- [ ] **2.4 Remove manual promotion (M).** Delete the family dropdown, "Make displayed view editable", `RequiresConversion`, the seven promotion error enums, and `clear_roi_edit_history_for_roi` on switch. One error type for real failures. Report loss (volume delta or Dice) on each conversion.
- [ ] **2.5 Scope decisions (S, needs your input).** Confirm voxel-to-authority promotion (`promote_current_voxel_cache_to_authority`) is removed; decide what a contour tool does to a mesh-primary ROI (convert with loss, undoable, or refuse).

## Phase 3: Editing features and data completeness

Exit: a deep-learning multi-organ segmentation can be imported, corrected, and exported without merging structures.

- [ ] **3.1 Multi-label ROIs (M, needs your decision).** Today the overlay colors per label but SDF, contour, and mesh derivation use `!= 0`, and rasterization writes `1`, so a multi-organ import merges into one structure. Decide: one ROI per label at import (recommended) or a label-aware ROI. Requires the cache integrity fix in 0.1.
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
- [ ] **6.3 CI (S).** Run clippy with `--all-targets --all-features`, run the Playwright QA spec against the built bundle, pin the Rust toolchain and `trunk`, and align the clippy flags documented in `clippy.toml`, `AGENTS.md`, and `ci.yml`.
- [ ] **6.4 LICENSE and attribution (S, needs your input).** No LICENSE file; the marching-cubes table derives from an Apache-2.0 crate with only a code comment. Add a LICENSE and a NOTICE.
- [ ] **6.5 GPU readback for visual QA (M).** Headless and headed Playwright cannot capture the WebGPU canvas. Add a wgpu readback behind `__viewerQa.screenshot()` for visual regression checks.
- [ ] **6.6 Small UX and platform items (S).** Windowing sliders, presets, and "HU" labels are CT-specific; make ranges follow `intensity_range`. `ScreenDescriptor` uses `window.scale_factor()` while layout uses `ctx.pixels_per_point()` and diverges under egui zoom. File picker leaks its `<input>` on cancel. Scroll accumulator is shared across viewports. Viewport mouse capture during drags.
- [ ] **6.7 Repo hygiene (S).** `qa_samples/` (28 MB) in git history; decide LFS. `target/` is 24 GB.

## Decisions needed from you

1. Multi-label handling (3.1): one ROI per label, or a label-aware ROI.
2. Voxel-center convention (1.3): edge-to-edge per the ADR (recommended) vs node-based.
3. What a contour tool does to a mesh-primary ROI (2.5).
4. Whether non-CT modalities (MR, PET) are in scope (1.6, 6.6).
5. License choice (6.4).

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
