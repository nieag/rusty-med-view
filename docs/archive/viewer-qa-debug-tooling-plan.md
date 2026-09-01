# Viewer QA and Debug Tooling Plan

## Purpose

Make the running viewer inspectable enough that an agent can verify visual behavior from structured state and logs.

This tooling is for development and QA. It must not become part of normal clinical/viewer UI behavior.

Primary goals:

- deterministic app setup for known visual scenarios
- queryable structured runtime state
- structured logs and metrics for rendering/conversion/debugging
- structured render facts for viewport-level inspection
- enough geometry diagnostics to debug image/label/contour/mesh alignment problems

## Non-Goals

Do not build:

- a full automated visual regression framework in the first pass
- pixel-perfect image comparisons
- browser crop automation as a required QA path
- production telemetry
- renderer rewrites just to support QA
- egui-based drawing of viewport scene content

Image capture is out of scope for automated QA. Browser/canvas coordinate spaces vary across automation environments; use structured facts instead.

## Design Principles

- Debug state must be queryable from the browser console or automation runtime.
- Visual QA should rely on structured facts, not captured pixels.
- QA tooling must be deterministic: same sample, same view preset, same viewport layout, same active ROI.
- Geometry diagnostics must use the same central spatial contract as the app: representation-native space -> ROI geometry -> world millimetres -> viewport projection.
- Debug overlays that appear inside viewports must use native WGPU rendering, not egui painting.
- Debug tools must be gated behind development-only controls, URL flags, cargo features, or explicit runtime enablement.

## Proposed Browser QA Surface

Expose a WASM-only JavaScript debug object when QA mode is enabled:

```text
window.__viewerQa
```

Initial methods:

- `version()`: return QA API version and build metadata.
- `state()`: return the current structured viewer state snapshot.
- `metrics()`: return current counters/timings/resource metrics.
- `logs(options)`: return recent structured log entries from an in-memory ring buffer.
- `waitForReady(options)`: resolve when the app has reached a stable requested state.
- `setPreset(name, options)`: apply a deterministic viewport/tool/camera preset.
- `loadSample(name)`: load a known dev sample if sample autoload is available.
- `lastError()`: return the latest captured app/WGPU/load/conversion error.

The exact implementation can be Rust `wasm_bindgen` exports or a small JS wrapper around exported Rust functions. The important contract is stable queryability from browser automation.

## Resolved First Slice

The first QA contract is readiness. A preset is useful only if automation can wait until the viewer is in the requested state without relying on arbitrary timeouts.

First preset:

- `image_label_mpr_basic`

First sample:

- sample id: `liver_0`
- image file: `/qa_samples/liver_0.nii`
- label file: `/qa_samples/liver_0_label.nii`
- sample files should live under `qa_samples/` and be served by Trunk through an explicit copy rule
- these first fixtures are tracked because they were already part of the repository at the root and have only moved into a clearer QA asset path
- if a requested local sample is missing, report `qa.sample` in both `lastError()` and `logs()` with the missing path and sample id

Enablement:

- WASM only
- URL flag: `?qa=1`
- intended for debug/dev builds first
- future Cargo feature gating may be added only if release-like QA builds need it
- when `?qa=1` is absent, `window.__viewerQa` should not exist
- when `?qa=1` is present, `window.__viewerQa` should exist even if sample or preset setup fails

Example URL:

```text
/?qa=1&sample=liver_0&preset=image_label_mpr_basic
```

Readiness ownership:

- Rust app state owns QA readiness
- JavaScript wrapper/promise code only polls exported Rust state
- `waitForReady(...)` must not infer readiness from canvas presence or fixed sleeps
- `waitForReady(...)` should be a JavaScript promise wrapper over Rust-owned state, with timeout/polling ergonomics handled in JavaScript
- missing sample files should keep QA initialized with `ready=false` and a structured `qa.sample` error instead of causing automation to time out

`image_label_mpr_basic` readiness requires:

- app initialized
- sample volume loaded through the same loader path as manual loading after bytes are fetched
- sample label loaded through the same label/ROI creation path as manual loading after bytes are fetched
- label ROI exists, is active, is visible, and has non-empty voxel data
- axial, coronal, sagittal, and 3D viewports exist
- all viewport rects are non-zero and usable as diagnostics
- 2D cursors are centered from the non-zero label bounds using ROI-owned voxel geometry and the shared geometry contract
- axial, coronal, and sagittal viewports report both `ready` and `overlay_renderable`
- 3D viewport reports `ready` with a valid deterministic camera and no render error
- no 3D label mesh/geometry is required for this first preset
- active ROI is assigned to an overlay texture slot
- at least one frame has presented after the sample and preset state were applied
- `lastError()` is empty for relevant load, geometry, preset, and render categories

Readiness should fail on relevant errors only, not every warning. Blocking conditions include load failure, missing sample files, invalid geometry, missing active/visible label ROI, missing overlay slot, missing required viewport, and render fatal/error states. Non-blocking warnings should remain visible through logs and state.

Keep viewport readiness and overlay renderability separate:

- `viewports[*].ready`: viewport exists, rect is valid, mode/camera state is valid
- `viewports[*].overlay_renderable`: 2D-only fact that the label ROI should be drawable in that viewport

For the first pass, `overlay_renderable` means:

- label ROI is visible
- label ROI has non-empty voxel data and valid ROI-owned voxel geometry
- label ROI is assigned to an overlay slot
- the current viewport plane/slice intersects the label world bounds
- renderer reports no overlay warning/error after the frame

Pixel-level proof, color sampling, and golden image diffs are out of scope.

The first implementation should expose both:

- a global ROI `overlay_slot` fact, proving the renderer has assigned an overlay texture slot
- per-2D-viewport `overlay_renderable`, proving the current slice/plane should display that ROI

No visible QA UI should be added in QA-1 or QA-2. URL-driven sample/preset state changes, console warnings/errors, and the browser automation API are allowed. egui QA panels, visible QA badges, and debug overlay geometry are deferred.

State, logs, and errors should use stable snake_case string values rather than leaking Rust enum variant names into the browser contract.

The first boundary should use serde-backed Rust structs serialized to JSON strings, with a JavaScript wrapper parsing those strings into objects. This keeps the core QA state testable on native targets and keeps wasm-only types out of the snapshot model.

## Structured State Snapshot

`window.__viewerQa.state()` should return JSON-serializable data.

Recommended top-level shape:

```text
{
  app,
  volume,
  rois,
  viewports,
  tools,
  render,
  conversions,
  warnings
}
```

The first implementation may expose a minimal versioned subset for `image_label_mpr_basic`:

```text
{
  qa,
  app,
  volume,
  rois,
  viewports,
  render
}
```

Minimum first-slice fields:

- `qa`: enabled, version, requested sample, requested preset, ready, last error
- `app`: status, frame counter, last-present timestamp if available
- `volume`: loaded, dimensions, spacing, world bounds, sample id
- `rois`: id, name, visible, active flag, primary representation, voxel dimensions, non-empty voxel bounds, overlay slot
- `viewports`: mode, browser-pixel rect, cursor/plane summary, `ready`, `overlay_renderable` for 2D viewports
- `render`: frame counter, overlay slots used/max, last warning/error

Required `app` fields:

- build/profile info
- current status message
- active tool
- active ROI id/name
- loaded/ready flags
- frame counter and last-present timestamp

Required `volume` fields:

- loaded flag
- dimensions
- spacing
- origin
- orientation quaternion
- intensity range
- world bounds
- source filename/sample id when available

Required `rois` fields per ROI:

- id, name, visibility, color, opacity
- primary representation
- cache current/dirty flags and generation ids
- voxel geometry when voxel authoritative/cache exists
- contour plane families and loop counts
- mesh vertex/face counts
- world bounds for each available representation/cache
- renderability flags and reasons when not renderable

Required `viewports` fields per viewport:

- mode
- viewport rect
- slice/cursor position where relevant
- zoom/pan/pivot
- user rotation and composed 3D rotation
- display projection context summary
- visible ROI count
- viewport rect diagnostics

Required `render` fields:

- overlay texture slots used/max
- overlay ROI ids assigned to each slot
- contour batch counts
- mesh batch counts
- uniform stride and uniform byte size
- last GPU upload sizes where cheap to collect
- last renderer warning/error

Required `conversions` fields:

- queued jobs
- running jobs
- last completed jobs
- last failed jobs with reason
- generation ids involved in each job

Required `warnings` fields:

- geometry mismatch warnings
- missing cache warnings
- overlay truncation warnings
- out-of-bounds projection/sampling counts if tracked

## Logs

Add a development ring buffer for structured log events. It should be queryable by `window.__viewerQa.logs(...)` and optionally mirrored to `console`.

Minimum fields:

- monotonically increasing sequence number
- timestamp/frame
- level: `debug`, `info`, `warn`, `error`
- target/category
- message
- structured fields object

First-pass log buffer policy:

- store the latest 500 events
- drop oldest events when the buffer is full
- use string key/value structured fields first; expand to arbitrary JSON values only when needed
- mirror `warn` and `error` events to the browser console in WASM QA mode
- keep `info` and `debug` events in the ring buffer only by default
- implement the Rust-side ring buffer target-neutrally where practical; `window.__viewerQa.logs()` is the WASM-only browser reader

Important categories:

- `load.volume`
- `load.label`
- `roi.create`
- `roi.cache`
- `convert`
- `render.volume`
- `render.overlay`
- `render.contour`
- `render.mesh`
- `geometry`
- `wgpu`
- `qa`

Important events to log:

- volume loaded with geometry
- label loaded with geometry
- label/main geometry relation summary
- ROI creation and active ROI changes
- cache rebuild request/start/finish/fail
- mesh extraction counts and bounds
- contour extraction/rasterization counts and bounds
- overlay slot assignment and truncation
- WGPU validation errors or resource-size warnings where capture is possible

## Metrics

Add lightweight development metrics that can be queried without perturbing normal behavior too much.

Initial metrics:

- frame counter
- moving average frame time
- last frame render time if available
- number of visible ROIs
- overlay slots used/max
- mesh vertices/faces submitted
- contour vertices submitted
- voxel overlay texture dimensions per slot
- conversion job durations
- cache rebuild counts
- latest sample/load durations
- warnings/errors count by category

Geometry-specific metrics:

- world bounds for main volume
- world bounds for each ROI representation/cache
- overlap/intersection summary between ROI bounds and main volume bounds
- projected viewport bounds for mesh/contour batches where cheap
- count of mesh triangles skipped due to invalid indices/projection
- count of overlay samples outside ROI bounds if practical in CPU-side diagnostics

## Deterministic QA Presets

Add named presets that set the viewer to known states. Presets should be applied by `window.__viewerQa.setPreset(name)`.

Initial presets:

- `image_label_mpr_basic`: load/display image + label in axial, coronal, sagittal, and 3D viewports, with 2D cursors centered on label bounds.
- `image_label_mpr`: show axial/coronal/sagittal/3D layout with label visible.
- `voxel_mesh_alignment_3d`: create or select mesh from active voxel ROI, set 3D camera to a known rotation, show voxel overlay and mesh together.
- `contour_mesh_alignment_3d`: use a contour-derived voxel cache and mesh, set 3D camera to a known rotation.
- `contour_edit_axial`: set contour edit tool, active contour ROI, and a known slice.
- `geometry_debug`: show bounds/axes diagnostics if debug overlay rendering is enabled.

Each preset should define:

- required sample data
- required ROI setup
- view mode/layout
- active ROI/tool
- cursor/slice/camera
- expected readiness condition
- expected visible objects

## Sample Data Loading

Manual file picker loading is not enough for automated review.

Add one of these:

- URL query autoload, e.g. `?qa=voxel_mesh_alignment_3d&sample=liver`
- dev-only sample loader that fetches known files from the app bundle/public directory
- test-only JS helper that injects sample bytes into existing loading paths

Preferred first pass:

- support query params for sample and preset
- load local bundled sample files from `qa_samples/` only in development/trunk builds
- reuse the same NIfTI loaders and ROI creation paths as manual loading

The QA path must not create separate loading semantics that can pass while real loading is broken.

## Structured Render Fact Workflow

The browser automation layer should use structured state/logs as the verification surface.

Required app support:

- `waitForReady(...)` reports that loading/conversions/rendering have reached a stable state
- `state().viewports[*]` reports mode, rect validity, renderability facts, and blockers
- `state().rois[*]` reports overlay slot, voxel dimensions, non-empty bounds, and visibility facts
- `state().render` reports cheap render facts such as overlay slot usage and frame counters
- `logs()` exposes load, preset, geometry, and render failures

Useful structured facts:

- per-viewport `image_renderable`
- per-viewport `overlay_renderable`
- per-viewport `volume_slice_in_bounds`
- per-viewport `cursor_intersects_roi`
- ROI `overlay_slot`
- frame presented after preset
- render warnings/errors

Initial review flow:

1. Start `trunk serve`.
2. Open app URL with QA query params.
3. Wait for `window.__viewerQa.waitForReady(...)`.
4. Query `state()`, `metrics()`, and recent `logs()`.
5. Report pass/fail from structured facts and blockers.
6. Stop there; do not require image capture.

## Debug Overlays

Debug overlays should be optional and native-rendered.

Useful overlays:

- main volume world bounds
- ROI voxel bounds
- mesh world bounds
- contour plane outlines
- patient/world axes
- active viewport/scissor bounds
- sampled label/mesh alignment markers

Rules:

- no egui painting for viewport geometry
- overlays must be clearly marked debug-only
- overlays must be toggleable from QA state/preset
- overlays should stay opt-in if any future visual artifact path is reintroduced

## Implementation Phases

### QA-1: Queryable State and Logs

Deliver:

- `window.__viewerQa` object in WASM QA mode
- `state()`, `logs()`, `metrics()`, `lastError()`
- structured state for volume, ROIs, viewports, render slot usage, and conversion jobs
- in-memory structured log ring buffer

Acceptance:

- browser automation can query loaded volume geometry
- browser automation can query ROI geometry/bounds/counts
- recent load/conversion/render warnings are visible without reading the UI
- no behavior change when QA mode is disabled
- `cargo test -q`, `cargo check --target wasm32-unknown-unknown -q`, and `cargo fmt --all` pass

### QA-2: Readiness, Deterministic Samples, and Presets

Deliver:

- dev/sample autoload path for known image + label data
- query-param preset entry point
- `setPreset(...)` and `waitForReady(...)`
- `image_label_mpr_basic` preset using `liver_0`
- Rust-owned readiness state for the requested sample and preset
- separate viewport `ready` and 2D `overlay_renderable` state
- presets for voxel->mesh alignment may follow only after `image_label_mpr_basic` is stable

Acceptance:

- opening a single URL can produce a known image+label viewer state
- automation can wait until the requested sample/preset is ready
- the state snapshot records expected loaded sample, active ROI, all viewport rects, and 2D overlay renderability
- the 3D viewport is ready with a deterministic camera, without requiring label mesh geometry
- manual file loading still works unchanged

### QA-3: Structured Render Facts

Deliver:

- per-viewport renderability facts beyond QA-2 readiness
- render-facing counters/facts for image, overlay, contour, and mesh submissions where cheap
- structured blockers that explain why a viewport should or should not show content
- documented automation workflow that reports state/log pass/fail

Acceptance:

- an agent can verify image+label viewport readiness from `state()` and `logs()` alone
- an agent can explain missing image/overlay/mesh content with structured blockers
- captured pixels are not required for automated QA pass/fail

### QA-4: Geometry Debug Overlays

Deliver:

- optional native-rendered debug bounds/axes overlays
- QA preset that enables geometry debug visualization
- metrics/logs linking overlay bounds to ROI/main-volume bounds

Acceptance:

- structured debug facts can show whether voxel, contour, mesh, and main volume bounds overlap as expected
- debug overlays do not use egui painting
- normal viewer rendering is unchanged when debug overlays are disabled

### QA-5: Optional Automated Visual Regression

Deliver only after QA-1 through QA-4 are stable.

Possible additions:

- optional non-blocking visual artifacts only if a later plan reintroduces them
- perceptual/image-diff thresholds
- CI structured artifact upload rather than strict image fail initially
- browser-console failure collection in CI

Acceptance:

- visual artifacts remain outside the automated pass/fail path
- strict image comparison is deferred indefinitely unless a separate plan proves coordinate/capture stability

## Validation Checklist for Implementers

Every QA tooling implementation pass should report:

- commands run
- QA mode enabled/disabled behavior
- exact URL or preset used for manual/browser verification
- `state()` fields checked
- `logs()` warnings/errors observed
- structured state/log facts checked
- any cases where visual state and structured state disagree

Required commands unless the implementation is docs-only:

```bash
cargo test -q
cargo check --target wasm32-unknown-unknown -q
cargo fmt --all
```

## Open Questions

- How much WGPU error information can be captured into structured logs across native and WASM?
- Which structured render facts should become future CI gates?

## Implementer Handoffs

- QA-1: [docs/qa-1-debug-api-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/qa-1-debug-api-handoff.md:1)
- QA-2: [docs/qa-2-sample-preset-readiness-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/qa-2-sample-preset-readiness-handoff.md:1)
- QA-3: [docs/qa-3-structured-render-facts-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/qa-3-structured-render-facts-handoff.md:1)

## Implementation Status

- Current phase: QA-1 implemented.
- Completed:
  - first readiness contract selected
  - first preset defined as `image_label_mpr_basic`
  - first sample fixtures moved under `qa_samples/`
  - Trunk copy rule added for `qa_samples/`
  - QA-1 weaker-model implementation handoff written
  - added `src/app/qa.rs` with target-neutral QA runtime, snapshots, metrics, error model, and fixed-size log ring buffer (500)
  - wired `QaRuntime` into `AppState` and exposed minimal honest QA-1 state/metrics snapshots from live app state
  - installed WASM-only `window.__viewerQa` when `?qa=1` with `version()`, `state()`, `metrics()`, `logs()`, `lastError()`, and `waitForReady(options)`
  - mirrored QA runtime `error` events to `console.error` in WASM QA mode
  - added unit tests for QA log capacity/sequence behavior and QA JSON serialization stability
  - updated NIfTI roundtrip integration tests to load samples from `qa_samples/` (with crate-root fallback)
  - QA-2 implementation handoff written
- Pending:
  - QA-3 structured render facts
