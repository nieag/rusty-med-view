# Subplan 6.6 Voxel-to-Contour Extraction Handoff

This document is the implementation brief for `Subplan 6.6: Voxel-to-Contour Extraction V1`.

Use this as the source of truth for the next implementation passes. The goal is to make loaded voxel labelmaps usable as editable contour-primary ROIs while preserving the representation model: voxel data may be the source of extraction, but contour data becomes the authoritative editable state for the extracted ROI.

## Scope

This handoff covers:

1. extracting contour loops from voxel label data on a selected plane family
2. constructing `ContourData` in `PlaneLocalMm` from ROI-owned voxel geometry
3. creating or initializing contour-authoritative ROIs from voxel label inputs
4. preserving the existing runtime/editing contracts so extracted contours behave like native contour ROIs
5. keeping current image, voxel label, contour editing, contour-to-voxel rebuild, navigation, and 3D behavior stable

It does not cover:

- mesh representation, mesh generation, mesh deformation, or mesh-derived caches
- SDF/TSDF caches
- smoothing, simplification, contour interpolation, boolean operations, or margin expansion
- registration or resampling
- import/export
- GPU acceleration
- renderer expansion beyond current contour/voxel paths
- automatic bidirectional live sync between voxel-authoritative and contour-authoritative ROIs after extraction

## Current State

The repository already has:

- voxel-authoritative ROI loading from labelmaps
- contour-authoritative ROI creation and editing
- `Subplan 6.5` contour-to-voxel rebuilds through runtime/job state
- shared voxel/world/plane-local transform helpers in `src/convert/geometry.rs`
- `VoxelData`, `VoxelGeometry`, `ContourData`, `ContourSlice`, `ContourLoop`, and `ContourPoint`
- `Roi::new_contour(...)`
- runtime contour mutation APIs such as `replace_contour_data(...)`
- `PlaneFamily` and `PlaneDefinition`

Subplan 6.6 must build on those pieces. Do not create a parallel contour store, do not introduce ad hoc screen-space contour geometry, and do not mutate voxel-authoritative source data during extraction.

## Locked Decisions

### 1. Extraction creates contour-authoritative state

Voxel-to-contour extraction is not just a read-only display adapter.

The result of extraction should be one of these explicit flows:

- create a new contour-authoritative ROI from a voxel ROI source, or
- replace the authoritative state of a dedicated target ROI that is already contour-primary

For V1, prefer the first option because it keeps ownership and user intent explicit.

Expected result:

- source voxel ROI remains unchanged
- extracted contour ROI is `PrimaryRepresentation::Contour`
- extracted contour ROI can immediately use the existing contour editing workflow
- contour edits on the extracted ROI queue contour-derived voxel rebuilds through the existing `6.5` path

### 2. Voxel source geometry is authoritative for extraction

Extraction must use the source ROI's own voxel geometry, not the current main volume by convention.

Rules:

- read `VoxelData.geometry` from the voxel source being extracted
- derive slice planes and contour point placement from that geometry
- do not substitute main-volume geometry unless the source ROI explicitly lacks geometry, which should not happen in the current model

This keeps extracted contours spatially aligned with the actual loaded labelmap, including non-identical label/image grids.

### 3. V1 extraction remains slice-based and deterministic

Keep the first extraction algorithm deliberately conservative:

- extract slice-by-slice for one selected `PlaneFamily`
- support axial/coronal/sagittal first
- keep oblique extraction out of scope for V1
- generate closed loops in `PlaneLocalMm`
- preserve disconnected components as separate loops
- holes are desirable, but if the first implementation cannot represent them robustly, document the limitation explicitly and keep the behavior deterministic

Do not add smoothing or simplification that changes topology or moves contours significantly. Raw, editable boundaries are better than “nicer-looking” but unstable contours in V1.

### 4. Plane-family ownership stays explicit

Extracted contours must still obey the contour model:

- `ContourData.active_plane_family` is set explicitly at extraction time
- extracted slices belong only to that plane family
- switching plane family after extraction should continue to follow the existing runtime rules from Subplan 5

V1 does not need multi-family extraction in a single ROI.

### 5. Runtime/UI action must be explicit

Do not make contour extraction a silent side effect of loading a labelmap.

Provide an explicit runtime/UI action such as:

- “Create Contour ROI From Label”
- “Extract Contours From Active Voxel ROI”

The user should be able to tell whether they are editing:

- the original voxel ROI, or
- a contour-authoritative ROI derived from it

This explicit action should be placed so it is easy to use immediately after loading a voxel labelmap. Existing labelmap data is the main real-data verification path for `6.6`, so the workflow from “load label” to “extract contours” should be short and obvious even though extraction is not automatic.

### 6. Conversion boundary

The extraction boundary should be pure and testable before it is wired into ROI creation.

Suggested shape:

```rust
pub fn extract_contours_from_voxel_data(
    voxel_data: &VoxelData,
    family: PlaneFamily,
) -> Result<ContourData, VoxelContourExtractionError>
```

This function should not depend on ECS, GUI state, WGPU, or the current main volume.

## V1 Algorithm Direction

Use a simple, explicit slice-by-slice contour extraction algorithm.

Suggested approach:

1. iterate slices for the selected orthogonal family
2. build a 2D binary mask for each slice from `VoxelData.raw_data`
3. detect boundary edges between filled and empty cells
4. stitch boundary edges into closed loops
5. convert loop vertices from slice-local/grid coordinates into `PlaneLocalMm` using the ROI-owned `VoxelGeometry` and the shared geometry helpers
6. emit one `ContourSlice` per non-empty slice

For V1:

- nearest cell-edge boundaries are acceptable
- axis-aligned “pixel contour” output is acceptable
- sub-voxel interpolation is not required
- topology correctness and transform correctness matter more than visual smoothness

## Implementation Steps

### Step 6.6A: Extraction model and pure API

Goal:
- define the extraction boundary and keep it independent from runtime/UI wiring.

Tasks:

- add a conversion module, suggested path: `src/convert/voxel_contour_extract.rs`
- add `VoxelContourExtractionError`
- expose a pure function:

```rust
pub fn extract_contours_from_voxel_data(
    voxel_data: &VoxelData,
    family: PlaneFamily,
) -> Result<ContourData, VoxelContourExtractionError>
```

- reject unsupported families explicitly for V1 if only orthogonal families are implemented
- add unit tests for:
  - empty voxel data -> empty `ContourData`
  - single connected component on one axial slice -> one closed loop
  - multiple disconnected components -> multiple loops
  - returned `ContourData.active_plane_family` matches the requested family

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

### Step 6.6B: Deterministic orthogonal slice extraction

Goal:
- implement the first practical extraction algorithm for axial/coronal/sagittal slices.

Tasks:

- add helpers to:
  - read 2D occupancy masks from `VoxelData`
  - detect cell-edge boundaries
  - stitch boundaries into closed loops
  - convert extracted vertices into `PlaneLocalMm`
- keep loops closed and valid for contour editing
- skip degenerate loops deterministically
- add unit tests for:
  - axial extraction geometry
  - coronal extraction geometry
  - sagittal extraction geometry
  - geometry/origin/spacing preservation through extracted contour placement

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

### Step 6.6C: Runtime creation path

Goal:
- turn pure extracted contour data into a usable contour-authoritative ROI.

Tasks:

- add a runtime helper in `src/app/roi_runtime.rs` with a shape like:

```rust
pub fn create_contour_roi_from_voxel_roi(
    world: &mut World,
    source_roi: hecs::Entity,
    family: PlaneFamily,
) -> Result<hecs::Entity, VoxelContourCreationError>
```

- validate that the source ROI is voxel-authoritative
- clone source `VoxelData`
- call the pure extraction function
- create a new contour ROI using `Roi::new_contour(...)`
- set the new ROI as active when appropriate in the caller path
- keep the source voxel ROI unchanged
- add tests for:
  - source voxel ROI unchanged after extraction
  - new ROI is contour-primary
  - extracted contour data is present and editable
  - invalid/non-voxel source is rejected safely

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

### Step 6.6D: UI integration for real labelmap workflows

Goal:
- make existing loaded voxel label data usable as contour test/edit data.

Tasks:

- add an explicit UI action for contour extraction from the active voxel ROI
- place that action directly in the label/ROI workflow so a user can load a voxel labelmap and immediately trigger contour extraction without hunting through unrelated tools
- surface clear status messages for:
  - missing active ROI
  - non-voxel active ROI
  - extraction success
  - extraction failure
- preserve current label loading behavior; do not auto-convert on load
- keep contour editing behavior unchanged once the new contour ROI is created

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`
- manual check: load image + voxel label, run extraction action, expected: a new contour ROI appears and can enter contour editing

### Step 6.6E: Interop with existing contour editing and 6.5 rebuilds

Goal:
- ensure extracted contours are not a dead-end representation.

Tasks:

- verify extracted contour ROIs work with:
  - contour selection
  - contour move/insert/delete
  - contour-to-voxel rebuilds from `6.5`
- add tests or focused regression coverage where practical:
  - extracted contour ROI can be passed through `replace_contour_data(...)`
  - extracted contour ROI queues `RebuildVoxelCache` on edit
  - extracted contour ROI remains valid after one edit and rebuild cycle

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`
- manual check: extract contours from a loaded labelmap, edit them, expected: edited contour ROI rebuilds its derived voxel cache and remains editable

### Step 6.6F: Closeout and regression review

Goal:
- finalize `6.6` and document V1 limitations honestly.

Tasks:

- verify all Step `6.6A` through `6.6E` acceptance criteria
- update `docs/segmentation-reimplementation-plan.md` with:
  - completed checkpoints
  - commit hashes
  - validation commands
  - manual verification checklist with expected vs observed behavior
- document deferred limitations such as:
  - no oblique extraction in V1
  - no smoothing/simplification
  - possible hole/topology limitations if unresolved in V1
  - no registration/resampling
  - no import/export
- run final required verification

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`
- `cargo clippy --all-targets --all-features -- -D warnings`
- manual regression checklist:
  - load image NIfTI and voxel label NIfTI; expected: baseline image/label behavior preserved
  - extract contours from loaded voxel ROI; expected: new contour ROI is created with editable loops
  - edit extracted contour ROI; expected: contour editing works
  - rebuild extracted contour ROI back to voxel cache; expected: `6.5` path still works
  - pan/zoom/slice scroll/3D view; expected: existing viewer behavior remains unchanged

## Acceptance Criteria

Subplan 6.6 is complete when all of the following are true:

- a loaded voxel ROI can initialize an editable contour-authoritative ROI
- extraction uses ROI-owned voxel geometry rather than the current main volume by convention
- extracted contours are stored in `PlaneLocalMm`
- disconnected components are represented deterministically
- source voxel ROI remains unchanged
- extracted contour ROIs work with the existing contour editing path
- extracted contour ROIs can enter the existing `6.5` contour-to-voxel rebuild flow after edits
- unsupported extraction cases fail safely with clear status/log behavior
- existing image load, voxel label load, contour editing, contour-to-voxel rebuild, navigation, and 3D behavior remain stable

## Final Verification For Each Code Step

Run these before summarizing or committing a completed code step:

```sh
cargo test -q
cargo check --target wasm32-unknown-unknown -q
cargo fmt --all
```

For the final closeout step, also run:

```sh
cargo clippy --all-targets --all-features -- -D warnings
```

If a step is docs-only, run at least:

```sh
git diff --check
```

Every implementation summary must include a concrete manual verification checklist with expected vs observed behavior when the step changes user-visible behavior.

## Implementer Prompt Template

Use this prompt shape for lower-tier implementation passes:

```text
Continue the segmentation reimplementation using docs/subplan-6-6-voxel-to-contour-handoff.md as the source of truth. Implement only Step 6.6A first. Do not start runtime ROI creation, UI extraction actions, mesh work, SDF/TSDF, smoothing, interpolation, registration/resampling, import/export, or later Subplan 6.6 steps. Preserve current image, voxel label, contour editing, contour-to-voxel rebuild, 2D navigation, and 3D viewer behavior. Update docs/segmentation-reimplementation-plan.md with the completed checkpoint, run cargo test -q, cargo check --target wasm32-unknown-unknown -q, and cargo fmt --all, and include a concrete manual verification checklist with expected vs observed behavior before summarizing.
```
