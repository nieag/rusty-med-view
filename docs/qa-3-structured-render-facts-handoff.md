# QA-3 Structured Render Facts Handoff

Purpose: make agents verify viewer/render behavior from `window.__viewerQa.state()`, `logs()`, and `metrics()` without relying on screenshot crop stability.

Screenshots are optional/manual diagnostics. Do not make screenshot capture or crop correctness an acceptance gate in QA-3.

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
- pixel comparison
- golden image storage
- debug overlays
- broad renderer refactors

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

Optional screenshots may be saved as artifacts but must not decide pass/fail.
