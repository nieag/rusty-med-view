# QA-3 Structured Render Facts Handoff

Purpose: make agents verify viewer/render behavior from `window.__viewerQa.state()`, `logs()`, and `metrics()`.

QA-3 is state-only. Do not add screenshot capture, viewport crop automation, pixel matching, or image artifacts.

## Scope

Implement structured facts that explain what each viewport should render and why.

Add facts for:

- image renderability per viewport
- ROI overlay renderability per viewport
- contour renderability per viewport when available
- mesh renderability per viewport when available
- render submission counts where cheap
- readiness blockers and render blockers with stable snake_case strings
- last render warning/error by category

Do not implement:

- app-side screenshot capture
- browser screenshot automation
- viewport crop coordinate work
- viewport rect conversion between physical, CSS, canvas, or page spaces
- pixel comparison
- golden image storage
- debug overlays
- broad renderer refactors

Do not treat `state().viewports[*].rect` as a screenshot crop contract. It may remain a renderer/app diagnostic rect only.

## Likely Files

- `src/app/qa.rs`
- `src/app/mod.rs`
- `src/render/pipeline.rs`
- `src/render/contours.rs`
- `src/render/meshes.rs`
- `src/app/roi_runtime.rs`
- `tests/qa1_viewerqa.spec.js`
- `docs/viewer-qa-debug-tooling-plan.md`
- `docs/qa-3-structured-render-facts-handoff.md`

Avoid shader changes unless a fact cannot be exposed from CPU-side render prep/state.

## Proposed State Shape

Extend `state().viewports[*]` with:

- `image_renderable: bool`
- `overlay_renderable: bool`
- `contour_renderable: bool`
- `mesh_renderable: bool`
- `volume_slice_in_bounds: Option<bool>`
- `cursor_intersects_active_roi: Option<bool>`
- `render_blockers: Vec<String>`

Extend `state().render` with:

- `frame_counter`
- `last_presented_frame`
- `viewport_uniform_count`
- `overlay_slots_used`
- `overlay_slots_max`
- `contour_batch_count`
- `mesh_batch_count`
- `last_warning`
- `last_error`

Extend `logs()` categories as needed:

- `render.viewport`
- `render.uniforms`
- `render.overlay`
- `render.contour`
- `render.mesh`

## Acceptance

For `/?qa=1&sample=liver_0&preset=image_label_mpr_basic` in a GPU-capable browser:

- `waitForReady(...)` resolves
- axial/coronal/sagittal report `image_renderable=true`
- axial/coronal/sagittal report `overlay_renderable=true`
- 3D reports `image_renderable` or an explicit non-blocking reason if current 3D path is not image-render factored
- `render.overlay_slots_used >= 1`
- render logs have no `error` events
- every false renderability fact has a useful blocker
- no screenshot, crop, or pixel assertion is required

In no-GPU/headless environments:

- app does not panic
- `lastError().category` is one of `wgpu.surface`, `wgpu.adapter`, `wgpu.device`
- `qa.ready=false`
- blockers explain missing context

## Validation

Required:

```bash
cargo fmt --all
cargo test -q
cargo check --target wasm32-unknown-unknown -q
```

Browser automation:

- keep QA-1 API test
- keep QA-2 ready-or-structured-WGPU-fail test
- add QA-3 assertions for structured render facts

No screenshot artifacts are required or expected for QA-3.
