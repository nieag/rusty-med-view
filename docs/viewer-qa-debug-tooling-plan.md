# Viewer QA and Debug Tooling Plan

## Purpose

Make the running viewer inspectable enough that an agent or reviewer can verify visual behavior in the web app without relying only on manual screenshots and human descriptions.

This tooling is for development and QA. It must not become part of normal clinical/viewer UI behavior.

Primary goals:

- deterministic app setup for known visual scenarios
- queryable structured runtime state
- structured logs and metrics for rendering/conversion/debugging
- screenshot capture workflow for viewport-level visual inspection
- enough geometry diagnostics to debug image/label/contour/mesh alignment problems

## Non-Goals

Do not build:

- a full automated visual regression framework in the first pass
- pixel-perfect CI screenshot comparisons
- production telemetry
- renderer rewrites just to support QA
- egui-based drawing of viewport scene content

Screenshots are initially for agent-assisted/manual review. Automated image comparisons can be added later only after the capture path is stable.

## Design Principles

- Debug state must be queryable from the browser console or automation runtime.
- Visual QA should combine screenshots with structured facts; screenshots alone are not enough.
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
- all viewport crop rects are non-zero browser-pixel rectangles inside the canvas/browser bounds
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

Pixel-level proof, screenshot color sampling, and golden image diffs are out of scope for the first pass.

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
- screenshot/crop rect in browser pixels

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

## Screenshot Workflow

The browser automation layer can capture screenshots once the app exposes stable readiness and viewport rectangles.

Required app support:

- `waitForReady(...)` reports that loading/conversions/rendering have reached a stable state
- `state().viewports[*].rect` gives browser-pixel crop rectangles
- status/logs expose load or render failures before screenshot capture
- screenshot capture itself should stay outside the app for the first pass; browser automation should capture full screenshots and viewport crops from exposed rects

Screenshot types:

- full app screenshot for layout/status review
- per-viewport crop for image/overlay/mesh inspection
- optional debug-overlay screenshot for geometry diagnostics

Initial review flow:

1. Start `trunk serve`.
2. Open app URL with QA query params.
3. Wait for `window.__viewerQa.waitForReady(...)`.
4. Query `state()`, `metrics()`, and recent `logs()`.
5. Capture full screenshot and relevant viewport crop.
6. Compare visible output against structured expected state.

## Debug Overlays

Debug overlays should be optional and native-rendered.

Useful overlays:

- main volume world bounds
- ROI voxel bounds
- mesh world bounds
- contour plane outlines
- patient/world axes
- active viewport crop/scissor bounds
- sampled label/mesh alignment markers

Rules:

- no egui painting for viewport geometry
- overlays must be clearly marked debug-only
- overlays must be toggleable from QA state/preset
- overlays should be excluded from normal screenshots unless explicitly requested

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

### QA-3: Screenshot Review Workflow

Deliver:

- documented browser automation workflow
- viewport rects/crop data in debug state
- screenshot capture examples for full app and selected viewport
- expected-vs-observed checklist template

Acceptance:

- an agent can capture a 3D viewport screenshot after loading sample data
- an agent can inspect screenshot plus structured state to diagnose alignment
- user no longer needs to manually provide screenshots for every visual review pass

### QA-4: Geometry Debug Overlays

Deliver:

- optional native-rendered debug bounds/axes overlays
- QA preset that enables geometry debug visualization
- metrics/logs linking overlay bounds to ROI/main-volume bounds

Acceptance:

- debug screenshots can show whether voxel, contour, mesh, and main volume bounds overlap as expected
- debug overlays do not use egui painting
- normal viewer rendering is unchanged when debug overlays are disabled

### QA-5: Optional Automated Visual Regression

Deliver only after QA-1 through QA-4 are stable.

Possible additions:

- golden screenshot capture for stable presets
- perceptual/image-diff thresholds
- CI artifact upload rather than strict fail initially
- browser-console failure collection in CI

Acceptance:

- visual artifacts are useful for review without causing flaky CI failures
- strict image comparison is introduced only for stable, deterministic scenes

## Validation Checklist for Implementers

Every QA tooling implementation pass should report:

- commands run
- QA mode enabled/disabled behavior
- exact URL or preset used for manual/browser verification
- `state()` fields checked
- `logs()` warnings/errors observed
- screenshots captured, with expected vs observed notes
- any cases where visual state and structured state disagree

Required commands unless the implementation is docs-only:

```bash
cargo test -q
cargo check --target wasm32-unknown-unknown -q
cargo fmt --all
```

## Open Questions

- How much WGPU error information can be captured into structured logs across native and WASM?
- Should screenshot cropping be handled only by browser automation, or should the app expose canvas capture helpers?
- Which presets are stable enough to become future CI visual artifacts?

## First Implementer Prompt

Implement QA-1 from `docs/viewer-qa-debug-tooling-plan.md`. Add a WASM-only QA/debug query surface for structured state, logs, metrics, and last error. Do not add sample autoload, screenshot automation, debug overlays, visual regression tests, or production telemetry yet. Preserve current viewer behavior when QA mode is disabled. Keep viewport scene rendering out of egui. Add focused tests for pure state/log snapshot helpers where practical, update docs with the completed checkpoint, and run `cargo test -q`, `cargo check --target wasm32-unknown-unknown -q`, and `cargo fmt --all` before summarizing.

## QA-1 Implementation Handoff

This section is the step-by-step implementation plan for a lower-context coding pass. Keep the first pass narrow. Do not implement sample autoload, presets, screenshot capture, debug overlays, or visual regression.

### Scope

Implement only:

- a target-neutral Rust QA state/log module
- serde-backed JSON snapshots for state/logs/metrics/last error
- WASM-only `window.__viewerQa` installation when `?qa=1` is present
- a minimal state snapshot with honest placeholder values where real data is not wired yet
- focused unit tests for the pure Rust QA helpers

Do not implement:

- `image_label_mpr_basic` sample loading
- `setPreset(...)`
- true readiness for sample/preset workflows
- viewport crop computation beyond cheap existing facts if they are already available
- app-side screenshot capture
- egui QA panels
- native-rendered debug overlays

### Files to Touch

Expected files:

- `Cargo.toml`
- `src/app/mod.rs`
- `src/app/qa.rs` (new)
- `src/lib.rs`
- `index.html` only if the JS wrapper cannot be installed cleanly from Rust
- `docs/viewer-qa-debug-tooling-plan.md`

Avoid touching renderer/shader files in QA-1 unless a compile error forces a tiny integration adjustment.

### Step 1: Add Dependencies

Add direct dependencies:

```toml
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

Rationale: serde already exists transitively, but QA snapshot structs should depend on it explicitly.

### Step 2: Add `src/app/qa.rs`

Create target-neutral QA structs and helpers:

- `QA_API_VERSION: u32 = 1`
- `QaLevel`: stable snake_case values `debug`, `info`, `warn`, `error`
- `QaError`: `category`, `message`, string `fields`
- `QaLogEvent`: `seq`, `frame`, `level`, `category`, `message`, string `fields`
- `QaLogBuffer`: fixed capacity 500, monotonically increasing `seq`, drops oldest when full
- `QaSnapshot`: minimal versioned state shape
- `QaMetricsSnapshot`: minimal metrics shape
- `QaRuntime`: enabled flag, requested sample/preset strings, log buffer, last error

Use `BTreeMap<String, String>` for structured fields in QA-1.

Suggested JSON helpers:

```rust
pub fn to_json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string())
}
```

Tests:

- buffer keeps latest 500 events
- dropped events preserve increasing sequence numbers
- `last_error` is set and serialized
- snapshot serializes stable snake_case strings

### Step 3: Wire `QaRuntime` Into `AppState`

In `src/app/mod.rs`:

- add `pub mod qa;`
- add `qa: qa::QaRuntime` to `AppState`
- initialize it in `App::new()`

On native, default can be disabled. On WASM, enable only when URL query contains `qa=1`.

Do not change normal viewer behavior when QA is disabled.

### Step 4: Build Minimal Snapshots From Existing App State

Add methods that can produce JSON while holding the existing `AppState` mutex briefly:

- if `context` is `None`, return `qa.enabled`, `ready=false`, empty `viewports`, no volume loaded
- if `context` exists, fill cheap facts:
  - status message if available from `GuiState`
  - main volume dimensions/spacing/world bounds if available
  - active ROI id/name if cheap to query
  - overlay slot counts if already available cheaply
  - render/frame counters may be `0` in QA-1 if no counter exists yet

Honesty rule: if a field is not wired, return `null`, `0`, `false`, or an empty array rather than inventing readiness.

For QA-1, `qa.ready` should be:

- `false` when sample or preset query params are present, because QA-2 owns readiness
- `true` only for the trivial case where QA is enabled and no sample/preset was requested, if that is useful

### Step 5: Install `window.__viewerQa` On WASM Only

Preferred approach: install from Rust because `run()` already owns startup and has access to `App.state`.

In `src/lib.rs`, after `App::new()` and before/after event loop setup as needed:

- detect `?qa=1`
- if absent, do not create `window.__viewerQa`
- if present, install a JS object with:
  - `version()`
  - `state()`
  - `metrics()`
  - `logs()`
  - `lastError()`
  - `waitForReady(options)`

The methods should parse JSON in JS and return objects. The Rust boundary can return JSON strings.

`waitForReady(options)` should:

- poll `state().qa.ready`
- use default timeout, e.g. 10 seconds
- use default poll interval, e.g. 50 ms
- reject with current `lastError()` or state summary on timeout

If Rust-installed JS object becomes awkward, use a tiny inline wrapper in `index.html`, but keep the public API identical.

### Step 6: Console Mirroring

When QA is enabled on WASM:

- mirror `warn` events to `console.warn`
- mirror `error` events to `console.error`
- do not mirror `info` or `debug` by default

QA-1 only needs this for events that pass through `QaRuntime`; do not try to capture all `log` crate output.

### Step 7: Validation

Required commands:

```bash
cargo fmt --all
cargo test -q
cargo check --target wasm32-unknown-unknown -q
```

Manual browser checks after `trunk serve`:

- without `?qa=1`: `window.__viewerQa === undefined`
- with `?qa=1`: `window.__viewerQa` exists
- `window.__viewerQa.version()` returns API version
- `window.__viewerQa.state()` returns an object, not a JSON string
- `window.__viewerQa.logs()` returns an array or object containing log events
- `window.__viewerQa.lastError()` returns `null` or a structured error
- `window.__viewerQa.waitForReady({ timeoutMs: 100 })` resolves only if `qa.ready` is true, otherwise rejects with useful state/error data

### QA-1 Completion Bar

QA-1 is complete only when:

- normal app behavior is unchanged without `?qa=1`
- browser QA object is absent without `?qa=1`
- browser QA object exists with `?qa=1`
- Rust unit tests cover the log buffer and JSON helpers
- no sample/preset loading has been added
- plan status below is updated
- required Rust validation commands pass

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
  - QA-2 sample autoload and `image_label_mpr_basic` readiness

## QA-2 Implementation Handoff

This section is the implementation plan for the first agent-usable visual QA slice. QA-2 should make `/?qa=1&sample=liver_0&preset=image_label_mpr_basic` load a deterministic scene, expose enough structured state to crop screenshots, and make `waitForReady(...)` meaningful for that scene.

Keep QA-2 scoped. Do not add screenshot capture inside the app, golden image comparison, debug overlays, mesh extraction, contour conversion, or a visible QA panel.

### Scope

Implement only:

- QA sample fetch/autoload for `liver_0`
- sequential image then label load using the same NIfTI parsing and load handlers as manual loading
- `image_label_mpr_basic` preset application
- readiness state for the requested sample/preset
- viewport crop rects for axial, coronal, sagittal, and 3D viewports
- global ROI overlay slot fact and 2D `overlay_renderable` facts
- QA logs/errors for sample load, preset application, and readiness blockers
- Playwright/browser check for the full QA-2 URL

Do not implement:

- app-owned screenshot capture
- automated pixel matching
- `voxel_mesh_alignment_3d`
- contour/mesh readiness
- debug geometry overlays
- production telemetry

### Files to Touch

Expected files:

- `src/app/qa.rs`
- `src/app/mod.rs`
- `src/app/events.rs`
- `src/lib.rs`
- `src/io/handlers.rs` only if a small helper is needed to reuse existing load handling cleanly
- `src/render/protocols.rs` only if preset protocol selection needs a tiny helper
- `tests/qa1_viewerqa.spec.js` or a renamed/added QA-2 Playwright spec
- `docs/viewer-qa-debug-tooling-plan.md`

Avoid renderer/shader changes in QA-2 unless the existing state does not expose required rect/slot facts.

### Step 1: Extend QA Runtime State

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
- `last_presented_frame: Option<u64>` or reuse/increment the existing QA frame counter if it corresponds to rendered frames
- active QA ROI id/entity debug string when known

Expose these in `qa` snapshot fields using stable snake_case strings.

### Step 2: Add QA Sample Fetch on WASM

When `?qa=1&sample=liver_0` is present:

- fetch `/qa_samples/liver_0.nii`
- parse with `load_nifti_from_bytes(...)`
- send existing `AppEvent::VolumeLoaded(Ok(LoadResult::Volume(...)))`
- after volume load is handled, fetch `/qa_samples/liver_0_label.nii`
- parse with `load_label_from_bytes(..., "liver_0_label.nii".to_string())`
- send existing `AppEvent::VolumeLoaded(Ok(LoadResult::Label(...)))`

The fetch path must reuse the same parser and event/load handler flow as toolbar/manual loading after bytes are available. Do not create a separate ECS mutation path that could pass while manual loading is broken.

If fetch fails:

- `qa.ready = false`
- set `lastError()` category `qa.sample`
- log an `error` event with sample id and missing/failing path
- keep `window.__viewerQa` callable

Native behavior may remain unchanged in QA-2 unless a native QA runner is added later.

### Step 3: Sequence Sample Loading Safely

Do not fire volume and label loads blindly at the same time.

Recommended sequence:

1. QA startup requests volume fetch.
2. `AppEvent::VolumeLoaded(Ok(LoadResult::Volume(...)))` is handled by existing code.
3. QA runtime records volume loaded and requests label fetch.
4. `AppEvent::VolumeLoaded(Ok(LoadResult::Label(...)))` is handled by existing code.
5. QA runtime records label loaded and captures active ROI from `EditorState`.

Reason: label import depends on main volume geometry. Sequential loading matches user workflow and avoids races.

### Step 4: Apply `image_label_mpr_basic`

After label load succeeds:

- apply `"Standard 2x2"` protocol
- ensure active ROI is the loaded label ROI
- ensure label ROI visibility is true
- reset 3D viewport rotation/camera state to deterministic defaults already used by the app
- set cursor to the center of the non-empty label voxel bounds, converted through ROI-owned voxel geometry into main volume/cursor space
- request bind group rebuild if visibility or active ROI changed
- record `preset_applied_frame = current_frame`

If computing non-empty label bounds is too large for QA-2, stop and add a small tested helper rather than hardcoding `liver_0` coordinates.

### Step 5: Compute Readiness

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

### Step 6: Overlay Slot and Per-Viewport Facts

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

### Step 7: Browser/Playwright Check

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

Optional but useful:

- capture full app screenshot as Playwright artifact
- capture each viewport crop using exposed rects

Screenshots remain external artifacts; the app should not implement screenshot capture in QA-2.

### Step 8: Validation

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

### QA-2 Completion Bar

QA-2 is complete only when:

- the full QA-2 URL loads sample image and label without manual interaction
- `waitForReady(...)` resolves only after the preset scene is actually ready
- all required viewport rects are present and positive
- 2D overlay renderability is true for axial, coronal, and sagittal
- 3D readiness does not require label mesh geometry
- missing sample files produce structured `qa.sample` errors instead of timeout-only failures
- normal app behavior without `?qa=1` is unchanged
- validation commands pass
