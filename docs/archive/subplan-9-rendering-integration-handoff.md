# Subplan 9 Rendering Integration Layer Handoff

Purpose: make viewer rendering consume representation-agnostic prepared view data, then validate through QA state facts instead of screenshots.

Audience: weaker implementation model. Keep scope tight. Do not redesign renderer.

## Constraints

- No egui painting for viewer scene content.
- WGPU render modules own viewport scene rendering.
- Projection/coordinate math stays in `src/convert/` or `src/util/orientation.rs`.
- Render passes draw prepared data only. No conversion algorithms inside pass bodies.
- QA stays state/log based. Do not add screenshot capture, crop tooling, pixel matching, or image artifacts.
- Keep current two-overlay voxel renderer cap unless this subplan explicitly replaces it. Recommended: keep cap, make behavior visible and tested.

## Current Seams

- Voxel overlay selection and slot order:
  - `src/app/roi_runtime.rs`: `renderable_voxel_overlay_rois`, `renderable_voxel_overlay_count`, scene bind-group recreation.
  - `src/systems/render_prep.rs`: per-viewport uniforms and overlay geometry transforms.
- Contour rendering:
  - `src/render/contours.rs`: `prepare_contour_render_data`, upload, render.
  - Current limitation: active contour ROI only.
- Mesh rendering:
  - `src/render/meshes.rs`: `prepare_mesh_render_data`, upload, render.
  - Current limitation: visible mesh-primary ROIs in 3D.
- Frame orchestration:
  - `src/render/pipeline.rs`: frame systems, uniforms, volume pass, mesh pass, contour pass, GUI pass, `RenderFrameStats`.
- QA state:
  - `src/app/mod.rs`: `qa_state_snapshot`, `qa_metrics_snapshot`.
  - `src/app/qa.rs`: serializable QA snapshot structs.
  - `tests/qa1_viewerqa.spec.js`: QA-1/QA-2/QA-3 browser assertions.

## Recommended Design

Add a small render-view adapter layer under `src/render/`.

Recommended file:

- `src/render/roi_views.rs`

Recommended outputs:

- `RoiRenderViews`
- `VoxelOverlayView`
- `ContourOverlayView`
- `MeshOverlayView`
- `OverlayCapReport`
- `RenderRepresentationRequest`

This layer should answer:

- which visible ROIs are renderable as voxel overlays for this frame
- which contour ROIs have contour data usable by contour prep
- which mesh ROIs have mesh data usable by mesh prep
- which visible ROIs are skipped, and why
- whether overlay cap truncated visible voxel overlays

Keep it read-only. It should not mutate ROI caches, enqueue jobs, upload GPU data, or rebuild bind groups.

## QA Placement

Add QA after render-view adapters exist, not before.

Where:

- `src/app/qa.rs`: add fields only if current snapshot cannot express adapter results.
- `src/app/mod.rs`: compute QA facts from render-view adapters, not from duplicate ad hoc ROI logic.
- `src/render/pipeline.rs`: continue reporting frame-level facts from actual prepared render data (`viewport_uniform_count`, `contour_batch_count`, `mesh_batch_count`, warnings/errors).
- `tests/qa1_viewerqa.spec.js`: extend QA-3 only with state assertions; no screenshots.

Recommended QA facts for Subplan 9:

- keep existing `image_renderable`, `overlay_renderable`, `contour_renderable`, `mesh_renderable`
- add/derive blockers from adapter skip reasons, not duplicated special cases
- expose overlay cap behavior through existing `overlay_slots_used`/`overlay_slots_max`; add `overlay_truncated_count` only if weak model can wire it cleanly
- add QA assertion that voxel overlay slot count never exceeds cap
- add QA assertion that skipped/truncated visible ROIs have structured blocker/reason

Acceptance rule:

- `qa.ready=true` means QA sample/preset render state is valid.
- Structured WGPU failure (`wgpu.surface`, `wgpu.adapter`, `wgpu.device`) is accepted only as no-GPU environment outcome, not visual/render pass.

## Implementation Steps

### Step 9A: Read-Only Render View Adapters

Files:

- `src/render/mod.rs`
- `src/render/roi_views.rs`
- unit tests in `src/render/roi_views.rs`

Implement:

- collect visible ROI render candidates by representation
- preserve current active-ROI-first voxel overlay slot ordering
- preserve current two-overlay voxel cap
- return skipped/truncated reasons as stable snake_case strings
- no GPU objects in adapter output except entity/id references to existing cache/resource owners

Tests:

- active visible voxel ROI gets slot 0
- more than two visible voxel ROIs produce two selected plus truncated reason
- contour ROI with loops produces contour candidate
- mesh ROI with faces produces mesh candidate
- dirty/missing voxel GPU cache produces non-renderable reason

### Step 9B: Route Voxel Slot Accounting Through Adapter

Files:

- `src/app/roi_runtime.rs`
- `src/systems/render_prep.rs`
- tests near changed helpers

Implement:

- keep public helper names if possible to reduce churn
- internally call adapter for selected voxel overlays
- ensure bind-group order and uniform overlay rows match adapter-selected slots
- keep old behavior for current sample/preset

Tests:

- overlay count helper equals adapter selected length
- uniform prep ignores truncated overlays
- no silent third overlay binding

### Step 9C: Route Contour/Mesh Prep Through Adapter

Files:

- `src/render/contours.rs`
- `src/render/meshes.rs`
- tests already in those files

Implement:

- contour prep reads contour candidates from adapter
- mesh prep reads mesh candidates from adapter
- preserve current rendering limits unless explicitly expanded
- if keeping active-contour-only rendering, adapter reason must say so (`contour_inactive` or equivalent)

Tests:

- current active contour tests still pass
- non-active contour skip reason exists if not rendered
- visible mesh ROI still renders in 3D
- non-3D viewport still skips mesh

### Step 9D: QA Snapshot Uses Adapter Facts

Files:

- `src/app/qa.rs`
- `src/app/mod.rs`
- `tests/qa1_viewerqa.spec.js`

Implement:

- replace duplicate QA renderability logic with adapter-derived facts where practical
- keep existing QA JSON fields stable
- add fields only if needed for truncation/skips
- QA blockers must use same stable reason strings adapter emits

Tests:

- existing QA-1/QA-2/QA-3 pass unchanged first
- then add assertion for overlay cap/truncation only if deterministic fixture or test setup creates >2 ROIs
- no screenshot/crop additions

### Step 9E: Closeout Docs and Validation

Files:

- `docs/segmentation-reimplementation-plan.md`
- this handoff

Update:

- Implementation Status with completed Step 9 commits
- limitations kept: overlay cap, active-contour-only, no visual screenshot QA

Run:

```bash
cargo fmt --all
cargo test -q
cargo check --target wasm32-unknown-unknown -q
cargo clippy --all-targets --all-features -- -D warnings
```

Browser QA:

```text
http://localhost:8080/?qa=1&sample=liver_0&preset=image_label_mpr_basic
```

Run Playwright outside the sandbox when validating browser QA. The sandbox often lacks browser/WebGPU access, so a sandbox pass/fail is not authoritative for render QA.

Recommended sequence:

```bash
trunk serve
npx playwright test tests/qa1_viewerqa.spec.js --reporter=line
```

For actual render QA, use a GPU-capable Chrome launch with WebGPU enabled. Headless/no-GPU Playwright only validates the structured failure branch.

Assertions:

- `window.__viewerQa.state().qa.ready === true`
- axial/coronal/sagittal `image_renderable === true`
- axial/coronal/sagittal `overlay_renderable === true`
- `state.render.overlay_slots_used <= state.render.overlay_slots_max`
- `state.render.viewport_uniform_count >= 4`
- `state.render.last_error === null`
- `window.__viewerQa.lastError() === null`
- no QA log event with `level === "error"`

If browser lacks WebGPU:

- accept only structured `lastError.category` in `wgpu.surface`, `wgpu.adapter`, or `wgpu.device`
- do not mark rendering validation complete from no-GPU run

## Do Not Do

- Do not add screenshot or crop-based QA.
- Do not move conversion/raster/mesh extraction into WGPU passes.
- Do not add egui drawing for ROI scene content.
- Do not change coordinate math in `src/gui/`.
- Do not expand overlay compositing unless also replacing current cap with tests and QA facts.
- Do not mix performance/cache strategy work into Subplan 9.

## Completion Criteria

- Render modules consume prepared view data/adapters, not raw representation-specific discovery scattered across passes.
- Voxel overlay cap behavior is explicit in runtime and QA facts.
- QA assertions verify render state through `window.__viewerQa.state()`, `logs()`, `metrics()`, and `lastError()`.
- Existing sample preset remains ready on GPU-capable browser.

## Implementation Status

Status: complete.

Completed:
- Step 9A: add `src/render/roi_views.rs` read-only adapter with tests and stable skip reasons.
- Step 9B: route voxel overlay selection/count helpers through adapter; preserve active-first ordering and two-overlay cap.
- Step 9C: route contour/mesh render-prep entry through adapter candidates; preserve active-contour-only and current mesh viewport behavior.
- Step 9D: route QA snapshot renderability/blockers through adapter facts where practical; keep QA JSON contract stable; add QA-3 cap invariant assertion.
- Step 9E: update plan/status docs and run required validation commands plus Playwright run.

Validation run:
- `cargo fmt --all`
- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `npx playwright test tests/qa1_viewerqa.spec.js --reporter=line` (3 passed)

Retained limitations:
- two-overlay cap remains in effect
- contour rendering remains active-ROI-only
- QA remains structured state/log based (no screenshot/crop/pixel checks)
