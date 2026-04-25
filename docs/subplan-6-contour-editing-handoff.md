# Subplan 6 Contour Editing Handoff

This document is the implementation brief for `Subplan 6: Contour Editing V1`.

Use this as the source of truth for contour editing implementation passes. The goal is to add practical contour authoring on top of the Subplan 5 contour data model and runtime contracts, while preserving existing image, voxel label, navigation, and overlay behavior.

## Scope

This handoff covers the first user-facing contour editing workflow:

1. create an empty contour-authoritative ROI
2. enter an explicit contour editing tool mode
3. map viewport clicks and drags into `PlaneLocalMm`
4. add native WGPU contour render infrastructure
5. display stored contour loops through the native render path
6. draw, close, select, move, insert, and delete contour points/loops
7. schedule contour-derived voxel rebuilds through the existing runtime contract

It does not cover:

- contour-to-voxel rasterization algorithms
- voxel-to-contour extraction algorithms
- interpolation between slices
- smoothing, simplification, boolean operations, or margin expansion
- mesh generation, mesh editing, or mesh-derived caches
- renderer expansion beyond current voxel overlay behavior
- import/export of contour formats
- registration or resampling

## Current State

The repository already has:

- `EditorTool::Navigation` and `EditorState { active_roi, active_tool }` in `src/app/components.rs`
- layer selection and visibility controls in `src/gui/sidebar.rs`
- load buttons and status display in `src/gui/toolbar.rs`
- egui viewport labels, orientation markers, and annotation controls in `src/gui/overlays.rs`
- a WGPU render pass in `src/render/pipeline.rs`
- an existing GPU overlay primitive buffer used by the current volume shader
- mouse input handling in `src/systems/input.rs`
- shared `PlaneDefinition`, `PlaneFamily`, `ViewportMapping`, and viewport/plane conversion helpers in `src/convert/geometry.rs`
- contour-authoritative state through `RoiAuthoritativeData::Contour(ContourData)`
- `Roi::new_contour(...)`, `Roi::contour_data(...)`, `set_active_contour_plane_family(...)`, and `replace_contour_data(...)`

Subplan 6 should use those pieces directly. Do not create a second contour store or bypass `replace_contour_data(...)` for authoritative contour edits.

## Locked Decisions

### 1. Tool model

Start with these tool states:

```rust
pub enum EditorTool {
    Navigation,
    ContourSelect,
    ContourDraw,
    ContourPointMove,
}
```

Use `Navigation` as the default. Existing pan, rotate, zoom, slice scroll, and crosshair behavior must remain unchanged when `active_tool == Navigation`.

Contour tools must only consume left-click/drag interactions that they own. Middle-click panning, right-click rotation, and modifier-based navigation should keep working unless a later explicit UX decision changes that.

### 2. Authoritative edit path

All committed contour edits must update `ContourData` through `replace_contour_data(...)`.

Do not mutate `ContourData` directly from UI widgets or input systems unless the mutation goes through a helper that applies the same invalidation and rebuild scheduling contract.

Expected result of a committed edit:

- authoritative contour data changes
- authoritative generation increments
- voxel and mesh derived caches become dirty
- `RebuildVoxelCache` is queued
- no rasterization is executed in Subplan 6

### 3. Coordinate space

User interactions may start in viewport UV or egui pixels, but stored contour points remain `PlaneLocalMm`.

Use this conversion flow:

```text
ViewportUv -> PlaneDefinition + ViewportMapping -> VolumeUv -> PatientWorldMm -> PlaneLocalMm
```

For drawing overlays, use the inverse flow:

```text
PlaneLocalMm -> PatientWorldMm -> VolumeUv -> ViewportUv -> EguiScreen
```

Do not store points in viewport pixels, egui coordinates, volume UV, voxel indices, or GPU NDC.

### 4. Plane ownership

Contour authoring is allowed only on the active contour plane family.

For V1:

- axial/coronal/sagittal editing should be implemented first
- oblique support should use the same `PlaneDefinition` path when a visible oblique viewport is available
- if no oblique viewport is exposed in the UI yet, keep the code path generic but do not invent a new visible oblique UI in this subplan

When a user tries to edit a contour ROI from a viewport whose family does not match `ContourData.active_plane_family`, the system should reject the edit with a status message rather than silently storing inconsistent slices.

### 5. Draft versus committed state

Use explicit draft state for in-progress drawing.

Suggested shape:

```rust
pub struct ContourDraft {
    pub roi_entity: hecs::Entity,
    pub plane: PlaneDefinition,
    pub points: Vec<ContourPoint>,
}

pub struct ContourSelection {
    pub roi_entity: hecs::Entity,
    pub slice_index: usize,
    pub loop_index: usize,
    pub point_index: Option<usize>,
}
```

Draft state belongs in `EditorState`, not `ContourData`.

Only closed loops belong in authoritative `ContourData` during V1. Do not store partial draw state as authoritative contour loops.

### 6. Initial hit testing

V1 hit testing can be simple and deterministic:

- nearest point within a fixed screen-space radius selects a point
- nearest segment within a fixed screen-space radius selects a segment for insertion
- nearest loop can be selected by any point/segment match
- no polygon containment selection is required for V1

Screen-space thresholds should be stable constants with tests for the pure geometry pieces.

### 7. UI placement

Prefer small additions to existing UI surfaces:

- toolbar: tool mode buttons for navigation/select/draw/move
- layer/sidebar: create empty contour ROI and plane-family controls
- native render pass: draw existing contour loops, draft loops, selected points, and hover states

Do not add a new major panel unless the existing surfaces cannot carry the workflow.

### 8. Native contour rendering

Contour geometry must be drawn by our renderer, not by egui painter primitives.

Use egui only for controls, labels, status text, and panels. Do not draw contour polylines, points, handles, draft loops, hover state, or selection state with egui.

The implementation should introduce a dedicated render-side contour overlay path instead of bolting contour drawing into UI code:

- CPU prepares per-viewport contour vertices from `ContourData`, `ContourDraft`, and `ContourSelection`
- vertices are in viewport or screen/NDC space after using the shared plane/viewport conversion helpers
- WGPU owns buffers, pipeline state, blending, and the draw call
- rendering runs after the volume pass and before egui UI rendering
- egui labels and panels remain visually above native contours

For line thickness, prefer a triangle-strip/polyline mesh generated on CPU for WebGPU portability. Do not depend on wide hardware lines.

### 9. Manual verification discipline

Each implementation step must include explicit manual checks tied to visible behavior, not only compile/test commands.

At minimum, every step summary should report:

- exact manual actions performed
- expected result for each action
- observed result for each action
- pass/fail status
- whether any behavior was not verified manually and why

Use concrete checks relevant to this project, for example:

- load main image NIfTI
- load voxel label NIfTI
- verify overlays or contours appear when expected
- verify tools/modes switch correctly
- verify interaction feels responsive while dragging/editing
- verify 2D navigation and 3D view still work

If a step cannot be manually verified (for example CI-only context), state that explicitly and include the specific gap.

The existing annotation overlay primitive path may inform data flow, but contour rendering should not be constrained by the current `MAX_OVERLAY_PRIMITIVES` circle/ring shader model.

## Implementation Sequence

Implement one step at a time. Do not start the next step until the current step compiles, has focused tests, and the plan status is updated if the step advances the plan.

### Step 6A: Add contour edit mode and empty contour ROI creation

Goal:

- let the app create and select an empty contour-authoritative ROI without drawing yet
- expose explicit contour tool mode state without changing current navigation behavior

Files expected:

- `src/app/components.rs`
- `src/app/roi_runtime.rs`
- `src/gui/toolbar.rs`
- `src/gui/sidebar.rs`
- `docs/segmentation-reimplementation-plan.md`

Required work:

- add `EditorTool::{ContourSelect, ContourDraw, ContourPointMove}` while preserving `Navigation` default
- add runtime helper for creating an empty contour ROI with `ContourData { active_plane_family, slices: vec![] }`
- set the new contour ROI as `EditorState.active_roi`
- keep empty contour ROIs invisible in voxel overlay rendering unless a future contour overlay is drawn
- add basic toolbar or sidebar controls to switch editor tools
- add status messages for invalid actions, for example no active ROI or active ROI is not contour-primary

Tests required:

- editor tool default remains `Navigation`
- empty contour ROI creation produces `PrimaryRepresentation::Contour`
- empty contour ROI stores requested `active_plane_family`
- active ROI selection updates to the new contour ROI
- voxel label load/render code paths are unchanged

Verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

Suggested commit:

- `Phase 6A: add contour edit mode and ROI creation`

### Step 6B: Add contour viewport mapping helpers

Goal:

- provide testable helpers that map between viewport interaction coordinates and contour plane-local coordinates

Files expected:

- `src/convert/geometry.rs`
- `src/systems` or a new small contour editing helper module if cleaner
- `docs/segmentation-reimplementation-plan.md`

Required work:

- add helper to resolve the active viewport plane for a contour edit
- add helper to convert `ViewportUv` to `PlaneLocalMm`
- add helper to convert `PlaneLocalMm` to `ViewportUv`
- reject 3D view editing for V1
- reject viewport families that do not match `ContourData.active_plane_family`
- support oblique through `PlaneDefinition` if the viewport mode is available, but do not expose new oblique UI here

Tests required:

- axial/coronal/sagittal click roundtrips through plane-local coordinates
- viewport-family mismatch is rejected
- 3D viewport edit attempts are rejected
- oblique helper path uses `PlaneDefinition` rather than viewport-index assumptions

Verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

Suggested commit:

- `Phase 6B: add contour viewport mapping`

### Step 6C: Add native contour render infrastructure

Goal:

- add a renderer-owned contour drawing path before connecting it to contour semantics

Files expected:

- `src/render/contours.rs` or an equivalent render-owned contour module
- `src/render/pipeline.rs`
- `src/app/context.rs`
- `docs/segmentation-reimplementation-plan.md`

Required work:

- add contour render resources, including buffer ownership and a contour render pipeline
- define a compact contour vertex format for screen/NDC-space line and point geometry
- add CPU helpers to build thick 2D line segments as triangles for WebGPU portability
- add a render pass that draws contour vertices after the volume pass and before `Gui::render(...)`
- add empty-buffer behavior so the pass is a no-op when no contours exist
- keep voxel overlay rendering unchanged
- keep egui out of contour drawing

Tests required:

- contour line mesh generation produces stable triangle geometry
- zero-contour render data produces an empty/no-op draw payload
- renderer resource setup still compiles for native and WASM

Manual verification:

- creating or loading voxel labels still renders as before
- viewer frame renders normally with an empty contour render pass

Verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

Suggested commit:

- `Phase 6C: add native contour renderer`

### Step 6D: Render stored contour loops

Goal:

- make stored contour loops visible in 2D viewports through the native contour renderer without enabling editing yet

Files expected:

- native contour render module added in Step 6C
- `src/render/pipeline.rs`
- `src/systems` if render data preparation lives there
- `src/convert/geometry.rs` if projection helpers need small additions
- `docs/segmentation-reimplementation-plan.md`

Required work:

- prepare contour render vertices for the active contour ROI on matching plane-family viewports
- draw only loops whose `ContourSlice.plane` matches the displayed slice plane within a small tolerance
- draw points and line segments with WGPU, not egui
- use ROI color and selection state where available
- keep voxel overlay rendering unchanged
- do not draw contour overlays in 3D for V1

Tests required:

- pure projection helper tests for `PlaneLocalMm -> ViewportUv`
- render vertex preparation tests for line segments and point markers
- slice-plane tolerance test
- native contour render preparation handles empty contours without panics

Manual verification:

- creating or loading voxel labels still renders as before
- a synthetic contour ROI can be made visible through the native contour render path without breaking viewer layout

Verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

Suggested commit:

- `Phase 6D: render stored contours`

### Step 6E: Add contour draw and loop closure

Goal:

- allow users to place points on the active contour plane and commit a closed loop

Files expected:

- `src/app/components.rs`
- `src/app/roi_runtime.rs`
- `src/systems/input.rs`
- native contour render module added in Step 6C
- `docs/segmentation-reimplementation-plan.md`

Required work:

- add `ContourDraft` state to `EditorState`
- in `ContourDraw`, left-click on a matching 2D viewport appends a draft point
- close the draft loop when clicking near the first point after at least three points
- commit closed loop into `ContourData` through `replace_contour_data(...)`
- cancel draft when switching tools or active ROI if needed
- preserve panning, zooming, rotation, and crosshair behavior outside contour draw interactions

Tests required:

- draft append adds plane-local points without changing authoritative data
- loop closure with fewer than three points is rejected
- closed loop commit adds a `ContourLoop` to the matching `ContourSlice`
- commit queues `RebuildVoxelCache`
- switching away from draw mode clears draft state according to the chosen rule

Manual verification:

- draw a simple triangle on an axial/coronal/sagittal contour ROI
- confirm the loop remains visible after tool switch and slice navigation
- confirm image + voxel label rendering still works

Verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

Suggested commit:

- `Phase 6E: add contour draw loop`

### Step 6F: Add point and loop selection

Goal:

- allow deterministic selection of existing contour points and loops

Files expected:

- `src/app/components.rs`
- native contour render module added in Step 6C
- `src/systems/input.rs`
- `docs/segmentation-reimplementation-plan.md`

Required work:

- add `ContourSelection` state to `EditorState`
- in `ContourSelect`, select nearest point or loop on click
- draw selected point/loop distinctly in the overlay
- clear selection when active ROI changes or selected loop is deleted later
- keep hit testing as pure helper functions where possible

Tests required:

- nearest point hit test chooses expected point
- hit test respects screen-space threshold
- selection rejects non-contour active ROI
- selection rejects non-matching plane family

Manual verification:

- click existing contour points and see selected state update
- clicking away clears or preserves selection according to documented rule

Verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

Suggested commit:

- `Phase 6F: add contour selection`

### Step 6G: Add point move, insert, and delete operations

Goal:

- provide the minimum practical contour editing operations after a loop exists

Files expected:

- `src/app/roi_runtime.rs`
- `src/systems/input.rs`
- `src/gui/toolbar.rs` or sidebar if command buttons are added there
- native contour render module added in Step 6C
- `docs/segmentation-reimplementation-plan.md`

Required work:

- in `ContourPointMove`, drag selected points in plane-local space
- add point insertion on selected or nearest segment
- add point deletion for selected points
- delete the loop if deletion would leave fewer than three valid closed-loop points
- add loop deletion for selected loops
- route every committed operation through `replace_contour_data(...)`
- keep rebuild scheduling contract intact

Tests required:

- move point updates only selected point
- insert point adds at expected loop position
- delete point removes expected point
- deleting below valid loop size removes loop or rejects according to documented rule
- each committed operation queues `RebuildVoxelCache`

Manual verification:

- create a loop, move one point, insert one point, delete one point, delete loop
- confirm navigation shortcuts still work when not actively dragging contour points

Verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

Suggested commit:

- `Phase 6G: add contour point editing`

### Step 6H: Finish Subplan 6 status and regression pass

Goal:

- mark contour editing V1 complete only after the basic edit workflow is usable and validated

Files expected:

- `docs/segmentation-reimplementation-plan.md`

Required work:

- update implementation status after each completed step
- include commit hashes once commits exist
- document any deferred contour editing limitations
- mark Subplan 6 complete only after Steps 6A through 6G are implemented and verified

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`
- manual image + voxel label regression check
- manual contour draw/select/move/delete check

Suggested commit:

- include status updates with the implementation commit they describe

## Acceptance Criteria

Subplan 6 is complete when all of the following are true:

- a user can create an empty contour-authoritative ROI
- a user can enter and leave contour editing tools without breaking navigation
- a user can draw a valid closed loop on a matching 2D plane
- native contour render infrastructure exists independently of egui
- stored loops render through the native contour render path
- a user can select, move, insert, and delete points
- a user can delete a loop
- every committed edit updates `ContourData` through the runtime mutation contract
- every committed edit queues contour-derived voxel rebuild work without requiring rasterization
- axial/coronal/sagittal use the same plane-local coordinate path
- oblique support remains model-compatible even if the visible oblique viewport is not exposed yet
- existing image load, voxel label load, overlay visibility, 2D navigation, and 3D view behavior remain unchanged

## Final Verification For Each Code Step

Run these before summarizing or committing a completed code step:

```sh
cargo test -q
cargo check --target wasm32-unknown-unknown -q
cargo fmt --all
```

If a step is docs-only, run at least:

```sh
git diff --check
```

In addition to commands, include a short manual verification checklist in every step summary.

## Implementer Prompt Template

Use this prompt shape for lower-tier implementation passes:

```text
Continue the segmentation reimplementation using docs/subplan-6-contour-editing-handoff.md as the source of truth. Implement only Step 6A first. Do not start native contour rendering, contour drawing, contour rasterization, interpolation, smoothing, mesh work, renderer expansion, import/export, registration/resampling, or later Subplan 6 steps. Preserve current image, voxel label, 2D navigation, and 3D viewer behavior. Update docs/segmentation-reimplementation-plan.md with the completed checkpoint, run cargo test -q, cargo check --target wasm32-unknown-unknown -q, and cargo fmt --all, and include a concrete manual verification checklist with expected vs observed behavior (loading data, visibility, interaction, and regressions) before summarizing.
```
