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

- timestamp/frame
- level: `debug`, `info`, `warn`, `error`
- target/category
- message
- structured fields object

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

- `image_label_axial`: load/display image + label, axial view centered on label bounds.
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
- load local bundled sample files only in development/trunk builds
- reuse the same NIfTI loaders and ROI creation paths as manual loading

The QA path must not create separate loading semantics that can pass while real loading is broken.

## Screenshot Workflow

The browser automation layer can capture screenshots once the app exposes stable readiness and viewport rectangles.

Required app support:

- `waitForReady(...)` reports that loading/conversions/rendering have reached a stable state
- `state().viewports[*].rect` gives browser-pixel crop rectangles
- status/logs expose load or render failures before screenshot capture

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

### QA-2: Deterministic Samples and Presets

Deliver:

- dev/sample autoload path for known image + label data
- query-param preset entry point
- `setPreset(...)` and `waitForReady(...)`
- presets for image+label and voxel->mesh alignment

Acceptance:

- opening a single URL can produce a known image+label viewer state
- automation can wait until the requested sample/preset is ready
- the state snapshot records expected loaded sample, active ROI, viewports, and visible renderables
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

- Should QA mode be controlled by a Cargo feature, URL flag, debug build check, or a combination?
- Where should sample data live so it is available to `trunk serve` without bloating production builds?
- How much WGPU error information can be captured into structured logs across native and WASM?
- Should screenshot cropping be handled only by browser automation, or should the app expose canvas capture helpers?
- Which presets are stable enough to become future CI visual artifacts?

## First Implementer Prompt

Implement QA-1 from `docs/viewer-qa-debug-tooling-plan.md`. Add a WASM-only QA/debug query surface for structured state, logs, metrics, and last error. Do not add sample autoload, screenshot automation, debug overlays, visual regression tests, or production telemetry yet. Preserve current viewer behavior when QA mode is disabled. Keep viewport scene rendering out of egui. Add focused tests for pure state/log snapshot helpers where practical, update docs with the completed checkpoint, and run `cargo test -q`, `cargo check --target wasm32-unknown-unknown -q`, and `cargo fmt --all` before summarizing.
