# Subplan 8 Mesh Workflow Handoff

This document is the implementation brief for `Subplan 8: Mesh Workflow`.

Use this as the source of truth for the next implementation passes. The goal is to make mesh a usable workflow, not just an architectural type. That requires the smallest practical combination of original `Subplan 8` and the mesh-specific prerequisites from original `Subplan 9`.

Post-review correction:

- the first implementation of this handoff exposed mesh placement, viewport clipping, binary-label semantics, and renderer ownership issues during manual verification
- do not treat Subplan 8 as complete until [docs/subplan-8-mesh-rendering-fixup-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-8-mesh-rendering-fixup-handoff.md:1) is implemented and verified
- the fixup handoff supersedes the loose `Step 8C` wording where native mesh rendering, main-display-volume projection, and viewport clipping are concerned

## Scope

This handoff covers:

1. extracting a first practical mesh from voxel data
2. creating mesh-primary ROIs from existing voxel or contour-driven inputs
3. adding the minimal mesh render/view path required to inspect and verify mesh state
4. establishing the first mesh-primary workflow baseline that later deformation tools can build on

It does not cover:

- full rendering abstraction for all ROI representations
- slice-facing deformation controls yet
- mesh-to-voxel or mesh-to-contour regeneration algorithms
- SDF/TSDF work
- smoothing, decimation, boolean operations, or margin tools
- import/export
- registration or resampling
- broad performance work

## Why This Handoff Combines Pieces

Original `Subplan 8` and `Subplan 9` are close in nature for the next practical step:

- mesh workflow is not useful unless mesh can be created from existing ROI data
- mesh workflow is not verifiable unless mesh can be rendered
- full render abstraction is still too broad for the next pass

So this handoff intentionally includes only the minimal mesh-specific rendering work required to make the first mesh workflow real. The broader representation-agnostic rendering layer remains deferred.

## Current State

The repository already has:

- completed mesh ROI architecture from `Subplan 7`
- voxel-authoritative ROIs
- contour-authoritative ROIs
- contour-derived voxel rebuilds
- voxel-to-contour extraction
- 2D contour rendering and current voxel overlay rendering
- a 3D viewer for the main volume / current scene setup

What is missing is the first useful mesh path:

- no mesh extraction algorithm
- no mesh creation path from current ROI data
- no mesh visualization path
- no mesh-primary user workflow

## Locked Decisions

### 1. First useful mesh source is voxel data

Do not start by trying to reconstruct a mesh directly from contours.

The first practical mesh source should be voxel occupancy:

- voxel-authoritative ROI data directly, or
- contour-authoritative ROI current derived voxel cache

This keeps the first algorithm and geometry contract much simpler.

### 2. Mesh creation should not mutate the source ROI

Like voxel-to-contour extraction:

- the source ROI remains unchanged
- the created mesh ROI is a new mesh-primary ROI
- ownership stays explicit

Do not silently flip a voxel or contour ROI into mesh-primary mode.

### 3. Minimal rendering is in scope, broad rendering abstraction is not

This phase may add the smallest dedicated mesh render path needed to inspect mesh results.

That does not authorize:

- a broad new rendering architecture rewrite
- mesh rendering in every viewport mode immediately
- premature unification of every ROI render path

For V1, 3D-view-only mesh rendering is acceptable if it is explicit and stable.

### 4. Use ROI-owned geometry for extraction

Voxel-to-mesh extraction must use the source voxel geometry:

- source voxel ROI geometry if voxel-primary
- current derived voxel cache geometry if contour-primary and current voxel cache exists

Do not assume main-volume geometry by convention.

### 5. Keep the first algorithm conservative

The first surface extraction should prioritize determinism and correctness over quality polish.

Acceptable for V1:

- binary occupancy input
- marching cubes or a similarly standard deterministic surface extraction
- no smoothing
- no decimation
- no topology repair beyond what the chosen extraction algorithm naturally provides

## Desired End State

At the end of this handoff, the repo should have:

- a mesh extraction algorithm from voxel data
- a runtime/UI path to create a mesh-primary ROI from current ROI data
- a minimal mesh render path to inspect results
- a credible baseline for later mesh deformation work

## Implementation Steps

### Step 8A: Pure voxel-to-mesh extraction

Goal:
- add a pure, testable voxel-to-mesh extraction boundary.

Tasks:

- add a conversion module, suggested path: `src/convert/voxel_mesh_extract.rs`
- add an API shaped like:

```rust
pub fn extract_mesh_from_voxel_data(
    voxel_data: &VoxelData,
) -> Result<MeshData, VoxelMeshExtractionError>
```

- use a deterministic binary-occupancy surface extraction algorithm
- store mesh vertices in patient/world coordinates
- keep the function independent from ECS, GUI, and rendering
- add tests for:
  - empty voxel data -> empty mesh
  - simple occupied shape -> non-empty mesh
  - ROI-owned geometry affects vertex placement correctly
  - deterministic output for the same voxel input

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

### Step 8B: Runtime mesh ROI creation from existing ROI data

Goal:
- create mesh-primary ROIs from practical existing sources.

Tasks:

- add runtime helper(s) such as:
  - `create_mesh_roi_from_voxel_roi(...)`
  - optionally `create_mesh_roi_from_contour_roi(...)` if contour ROI has a current voxel cache
- keep the source ROI unchanged
- create a new mesh-primary ROI with extracted `MeshData`
- reject unsupported or unavailable source states safely
- add tests for:
  - voxel source success path
  - contour source success path only when current voxel cache exists
  - source ROI unchanged after mesh creation
  - missing/non-supported source rejection

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

### Step 8C: Minimal mesh render path

Goal:
- make created mesh ROIs visible enough to verify the workflow.

Tasks:

- add a minimal mesh render/view path
- keep the first version deliberately narrow:
  - 3D viewport support first
  - explicit no-op in unsupported view modes is acceptable
- prepare mesh render data from mesh-primary ROI state
- keep rendering logic separate from extraction logic
- add tests for:
  - empty render payload safety
  - non-empty mesh render preparation
  - stable handling when no mesh ROI is active/visible

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

### Step 8D: UI/workflow integration

Goal:
- make the mesh workflow reachable from the current ROI workflow.

Tasks:

- add an explicit action to create a mesh ROI from the active ROI
- support at least the voxel-primary source path in UI
- if contour-primary source path is supported, gate it on current voxel-cache availability and fail clearly otherwise
- surface status messages for:
  - missing active ROI
  - unsupported active ROI
  - missing current voxel cache for contour source
  - mesh creation success
  - mesh extraction failure

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

### Step 8E: Closeout and route to deformation

Goal:
- finish the first mesh workflow baseline and set up the next phase.

Tasks:

- rerun full validation
- update `docs/segmentation-reimplementation-plan.md`
- record what mesh can now do:
  - be extracted
  - be created as a mesh-primary ROI
  - be inspected visually
- record what remains deferred:
  - actual deformation tools
  - mesh-derived voxel/contour rebuild algorithms
  - broad rendering integration

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`
- `cargo clippy --all-targets --all-features -- -D warnings`

## Acceptance Checklist

This handoff is complete when all of the following are true:

- voxel data can produce `MeshData` through a pure extraction API
- a new mesh-primary ROI can be created from current ROI data without mutating the source ROI
- a created mesh ROI can be inspected in the app
- the mesh workflow is explicit and does not blur representation ownership
- the repo is ready for a later deformation-focused pass

## Expected Summary Format

Every implementation summary for this handoff must include:

1. which `8.x` step was completed
2. what changed in practical terms
3. what mesh can now do in the app
4. what remains deferred
5. validation commands run
6. manual verification checklist with expected vs observed behavior

## Prompt Shape

Use this prompt shape for lower-tier implementation passes:

```text
Continue the segmentation reimplementation using docs/subplan-8-mesh-workflow-handoff.md as the source of truth. Implement only Step 8A first. Do not start deformation tools, mesh-to-voxel regeneration, mesh-to-contour regeneration, broad rendering abstraction, SDF/TSDF work, smoothing/decimation/booleans, import/export, registration/resampling, or later Step 8 sections. Preserve current image, voxel label, contour extraction, contour editing, contour-to-voxel rebuild, 2D navigation, and 3D viewer behavior. Update docs/segmentation-reimplementation-plan.md with the completed checkpoint, run cargo test -q, cargo check --target wasm32-unknown-unknown -q, and cargo fmt --all before summarizing.
```
