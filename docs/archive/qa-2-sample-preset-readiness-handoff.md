# QA-2 Sample Preset Readiness Handoff

## Implementation Status

- State: implemented in codebase
- Completed:
  - QA sample/preset runtime phases and readiness blockers
  - WASM sample autoload (`liver_0` image then label) via existing load handlers
  - deterministic `image_label_mpr_basic` preset apply
  - ROI overlay slot, voxel dims, non-empty bounds facts
  - viewport readiness + `overlay_renderable` facts
  - Playwright QA-2 URL readiness test
- Pending:
  - none in QA-2 scope

This is the implementation handoff for the first agent-usable structured QA slice.

Goal: make `/?qa=1&sample=liver_0&preset=image_label_mpr_basic` load a deterministic scene, expose enough structured state to verify readiness from state/logs, and make `waitForReady(...)` meaningful for that scene.

Keep QA-2 scoped. Do not add screenshot capture, golden image comparison, debug overlays, mesh extraction, contour conversion, or a visible QA panel.

## Scope

Implement only:

- QA sample fetch/autoload for `liver_0`
- sequential image then label load using the same NIfTI parsing and load handlers as manual loading
- `image_label_mpr_basic` preset application
- readiness state for the requested sample/preset
- viewport rect validity for axial, coronal, sagittal, and 3D viewports
- global ROI overlay slot fact and 2D `overlay_renderable` facts
- QA logs/errors for sample load, preset application, and readiness blockers
- Playwright/browser check for the full QA-2 URL

Do not implement:

- app-owned screenshot capture
- browser screenshot/crop automation
- automated pixel matching
- `voxel_mesh_alignment_3d`
- contour/mesh readiness
- debug geometry overlays
- production telemetry

## Files

Expected files:

- `src/app/qa.rs`
- `src/app/mod.rs`
- `src/app/events.rs`
- `src/lib.rs`
- `src/io/handlers.rs` only if a small helper is needed to reuse existing load handling cleanly
- `src/render/protocols.rs` only if preset protocol selection needs a tiny helper
- `tests/qa1_viewerqa.spec.js` or a renamed/added QA-2 Playwright spec
- `docs/viewer-qa-debug-tooling-plan.md`

Avoid renderer/shader changes in QA-2 unless existing state cannot expose required rect/slot facts.

## Step 1: Extend QA Runtime State

Add explicit QA workflow state to `QaRuntime`:

- requested sample id
- requested preset id
- sample load phase:
  - `not_requested`
  - `fetching_volume`
  - `loading_volume`
  - `fetching_label`
  - `loading_label`
  - `loaded`
  - `failed`
- preset phase:
  - `not_requested`
  - `pending_sample`
  - `applying`
  - `applied`
  - `failed`
- `preset_applied_frame: Option<u64>`
- `last_presented_frame: Option<u64>` or reuse/increment existing QA frame counter if it corresponds to rendered frames
- active QA ROI id/entity debug string when known

Expose these in `qa` snapshot fields using stable snake_case strings.

## Step 2: Add QA Sample Fetch On WASM

When `?qa=1&sample=liver_0` is present:

- fetch `/qa_samples/liver_0.nii`
- parse with `load_nifti_from_bytes(...)`
- send existing `AppEvent::VolumeLoaded(Ok(LoadResult::Volume(...)))`
- after volume load is handled, fetch `/qa_samples/liver_0_label.nii`
- parse with `load_label_from_bytes(..., "liver_0_label.nii".to_string())`
- send existing `AppEvent::VolumeLoaded(Ok(LoadResult::Label(...)))`

The fetch path must reuse the same parser and event/load handler flow as toolbar/manual loading after bytes are available. Do not create a separate ECS mutation path that can pass while manual loading is broken.

If fetch fails:

- `qa.ready = false`
- set `lastError()` category `qa.sample`
- log an `error` event with sample id and missing/failing path
- keep `window.__viewerQa` callable

Native behavior may remain unchanged in QA-2 unless a native QA runner is added later.

## Step 3: Sequence Sample Loading Safely

Do not fire volume and label loads blindly at the same time.

Recommended sequence:

1. QA startup requests volume fetch.
2. `AppEvent::VolumeLoaded(Ok(LoadResult::Volume(...)))` is handled by existing code.
3. QA runtime records volume loaded and requests label fetch.
4. `AppEvent::VolumeLoaded(Ok(LoadResult::Label(...)))` is handled by existing code.
5. QA runtime records label loaded and captures active ROI from `EditorState`.

Reason: label import depends on main volume geometry. Sequential loading matches user workflow and avoids races.

## Step 4: Apply `image_label_mpr_basic`

After label load succeeds:

- apply `"Standard 2x2"` protocol
- ensure active ROI is the loaded label ROI
- ensure label ROI visibility is true
- reset 3D viewport rotation/camera state to deterministic defaults already used by the app
- set cursor to the center of the non-empty label voxel bounds, converted through ROI-owned voxel geometry into main volume/cursor space
- request bind group rebuild if visibility or active ROI changed
- record `preset_applied_frame = current_frame`

If computing non-empty label bounds is too large for QA-2, stop and add a small tested helper rather than hardcoding `liver_0` coordinates.

## Step 5: Compute Readiness

For `image_label_mpr_basic`, `qa.ready` is true only when all are true:

- QA enabled
- requested sample is `liver_0`
- requested preset is `image_label_mpr_basic`
- sample phase is `loaded`
- preset phase is `applied`
- main volume loaded
- active ROI is the label ROI
- active ROI is visible
- label ROI voxel data is non-empty
- active ROI has a renderable voxel cache/GPU resources
- axial, coronal, sagittal, and 3D viewports exist
- all four viewport rects are non-zero
- axial/coronal/sagittal `overlay_renderable = true`
- 3D viewport `ready = true`
- at least one rendered frame occurred after `preset_applied_frame`
- no relevant `lastError()`

Readiness should remain false with a structured blocker reason if any required fact is missing.

## Step 6: Overlay Slot And Per-Viewport Facts

Expose in ROI snapshot:

- `overlay_slot: Option<u32>`
- `voxel_dimensions`
- `non_empty_voxel_bounds`

Expose in viewport snapshot:

- `mode`
- browser-pixel `rect`
- `ready`
- `overlay_renderable`
- optional `readiness_blockers: Vec<String>`

For QA-2, `overlay_renderable` for axial/coronal/sagittal means:

- label ROI is visible
- label ROI is assigned to overlay slot 0 or 1
- label voxel data is non-empty
- current plane/slice intersects label world bounds
- no renderer overlay error is present

Do not require pixel sampling yet.

## Step 7: Browser/Playwright Check

Extend the existing Playwright check or add a QA-2-specific spec:

```text
http://localhost:8080/?qa=1&sample=liver_0&preset=image_label_mpr_basic
```

Required checks:

- `window.__viewerQa` exists
- `await window.__viewerQa.waitForReady({ timeoutMs: 10000 })` resolves
- `state.qa.ready === true`
- `state.qa.requested_sample === "liver_0"`
- `state.qa.requested_preset === "image_label_mpr_basic"`
- state has exactly/at least axial, coronal, sagittal, and three_d viewports
- every viewport rect has positive width/height
- 2D viewports have `overlay_renderable === true`
- active ROI is visible and has an overlay slot
- `lastError()` is `null`
- logs contain sample/preset progress events and no `error` events

No screenshot artifacts are required for QA-2. Readiness is state/log based.

## Step 8: Validation

Required commands:

```bash
cargo fmt --all
cargo test -q
cargo check --target wasm32-unknown-unknown -q
```

Manual/browser validation:

- `/` keeps `window.__viewerQa === undefined`
- `/?qa=1` still exposes QA-1 API and resolves trivial `waitForReady(...)`
- `/?qa=1&sample=liver_0&preset=image_label_mpr_basic` reaches ready
- `state()`, `logs()`, `metrics()`, and `lastError()` remain objects/null rather than JSON strings

## Completion Bar

QA-2 is complete only when:

- the full QA-2 URL loads sample image and label without manual interaction
- `waitForReady(...)` resolves only after the preset scene is actually ready
- all required viewport rects are present and positive
- 2D overlay renderability is true for axial, coronal, and sagittal
- 3D readiness does not require label mesh geometry
- missing sample files produce structured `qa.sample` errors instead of timeout-only failures
- normal app behavior without `?qa=1` is unchanged
- validation commands pass
