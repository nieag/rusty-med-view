# Subplan 5 Contour Handoff

This document is the implementation brief for `Subplan 5: Contour Representation Architecture`.

Use this as the source of truth for the next implementation passes. The goal is to make contour-authoritative ROIs real in the core model before adding editing tools, rasterization, mesh conversion, or renderer expansion.

## Scope

This handoff covers one architectural checkpoint split into small implementation steps:

1. add contour data types that use the shared plane and coordinate model from Subplan 4
2. add contour-authoritative ROI construction and accessors
3. define active contour plane-family ownership rules
4. define runtime mutation and cache invalidation contracts
5. update plan tracking and verification after each completed step

It does not cover:

- contour drawing, editing, dragging, selection, or deletion UI
- contour-to-voxel rasterization algorithms
- voxel-to-contour extraction algorithms
- mesh generation, deformation, or mesh-to-voxel conversion
- renderer expansion beyond current voxel overlay behavior
- registration, resampling, or geometry reconciliation when representations differ
- import/export of contour formats

## Current State

The repository already has:

- `PrimaryRepresentation::{Voxel, Contour, Mesh}` in `src/app/components.rs`
- voxel-authoritative ROI data through `RoiAuthoritativeData::Voxel(VoxelData)`
- placeholder `RoiAuthoritativeData::Contour` and `RoiAuthoritativeData::Mesh`
- placeholder `ContourCache` and `MeshCache`
- ROI dirty state, cache generation, and queued/running job state
- runtime helpers for cache status, rebuild requests, job start, and rebuild completion
- shared `PlaneFamily` and `PlaneDefinition` in `src/convert/geometry.rs`
- shared plane-local/world conversion helpers from Subplan 4

Subplan 5 should build on those pieces. Do not introduce a parallel coordinate system.

## Locked Decisions

### 1. Contour storage space

Contour points are stored in `PlaneLocalMm`.

Each contour slice carries the `PlaneDefinition` that anchors those local millimeter points in patient/world space.

Do not store contour points in:

- viewport pixels
- egui coordinates
- normalized volume UV coordinates
- voxel indices
- GPU NDC

Those are view or sampling spaces. Contours are patient/world geometry.

### 2. Contour data model

Add these data types in `src/app/components.rs` unless a clearer app-owned module already exists by the time this is implemented:

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContourPoint {
    pub local_mm: [f32; 2],
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContourLoop {
    pub points: Vec<ContourPoint>,
    pub is_closed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContourSlice {
    pub plane: PlaneDefinition,
    pub loops: Vec<ContourLoop>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContourData {
    pub active_plane_family: PlaneFamily,
    pub slices: Vec<ContourSlice>,
}
```

`ContourData` is authoritative contour state. It is not a session cache.

`ContourCache` remains for derived or render-facing contour state. Do not store authoritative contour loops in `ContourCache`.

### 3. Active plane-family ownership

`ContourData.active_plane_family` defines the plane family that owns authoritative contour authoring.

For this checkpoint, use this invariant:

- authoritative slices in `ContourData.slices` must belong to `active_plane_family`
- non-authoritative reformatted contours must not be stored in `ContourData`
- future regenerated contour views belong in `ContourCache`, not authoritative data

Plane-family switching must be explicit and non-destructive:

- switching to the current family is a no-op
- switching an empty contour ROI is allowed
- switching a contour ROI that already contains loops must fail or return a clear error until a later conversion/migration step exists
- no implementation may silently reinterpret, rotate, project, or clear existing contour loops

This is intentionally conservative. It avoids losing contour data or pretending that cross-family contour migration exists before the conversion algorithms are implemented.

### 4. Dirty-state meaning

Authoritative contour changes should mark derived representations stale.

Do not treat `contour_cache_dirty` as "the authoritative contour is dirty." It means a derived/session contour cache is dirty.

For a contour-authoritative ROI:

- authoritative data lives in `RoiAuthoritativeData::Contour(ContourData)`
- voxel cache is derived
- mesh cache is derived
- contour cache is optional derived/session state, not the source of truth

When contour data changes:

- increment the authoritative generation
- mark voxel-derived state dirty
- mark mesh-derived state dirty
- mark contour cache dirty only if the cache represents derived/session contour data
- enqueue a voxel rebuild job only as a contract hook; do not implement rasterization in Subplan 5

### 5. Conversion algorithms are out of scope

Subplan 5 may schedule or represent the need for a contour-derived voxel rebuild.

It must not implement:

- scanline filling
- polygon rasterization
- marching squares
- contour interpolation
- oblique slice resampling
- mesh extraction

If code needs a result from one of those algorithms, return `None`, keep the cache dirty, or leave the rebuild job queued/running according to the existing runtime contract.

## Implementation Sequence

Implement one step at a time. Do not start the next step until the current step compiles, has tests, and the plan status is updated if the step advances the plan.

### Step 5A: Add contour data types

Goal:

- replace the placeholder contour authoring shape with explicit data types
- keep behavior unchanged

Files expected:

- `src/app/components.rs`
- `docs/segmentation-reimplementation-plan.md`

Required work:

- import or reference `PlaneDefinition` and `PlaneFamily` from `src/convert/geometry.rs`
- add `ContourPoint`, `ContourLoop`, `ContourSlice`, and `ContourData`
- change `RoiAuthoritativeData::Contour` to `RoiAuthoritativeData::Contour(ContourData)`
- update existing matches that handle `RoiAuthoritativeData::Contour`
- add small helper methods if useful, for example:
  - `ContourLoop::is_valid_closed_loop() -> bool`
  - `ContourData::is_empty() -> bool`
  - `ContourData::has_loops() -> bool`

Tests required:

- empty `ContourData` preserves its `active_plane_family`
- a `ContourSlice` preserves its `PlaneDefinition`
- a closed loop with at least three points is considered valid if such a helper is added
- a closed loop with fewer than three points is not considered valid if such a helper is added
- voxel ROI behavior and current viewer behavior remain unchanged

Verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

Suggested commit:

- `Phase 5A: add contour data model`

### Step 5B: Add contour-authoritative ROI construction

Goal:

- allow contour-authoritative ROIs to exist without editing tools

Files expected:

- `src/app/components.rs`
- `src/app/roi_runtime.rs` if world-level construction is added here
- `docs/segmentation-reimplementation-plan.md`

Required work:

- add `Roi::new_contour(roi_id, name, contour_data) -> Self`
- set `primary_representation` to `PrimaryRepresentation::Contour`
- set `authoritative_data` to `RoiAuthoritativeData::Contour(contour_data)`
- do not create a voxel GPU cache
- keep `session_caches.contour` as `None` unless a real derived contour cache is added
- add `Roi::contour_data(&self) -> Option<&ContourData>`
- add `Roi::contour_data_mut(&mut self) -> Option<&mut ContourData>` only if mutation call sites can preserve invalidation rules
- add a world-level constructor only if it keeps callers simpler

Tests required:

- `Roi::new_contour` creates a contour-primary ROI
- contour ROI exposes its authoritative `ContourData`
- contour ROI starts without a voxel GPU cache
- contour ROI cache state clearly marks missing derived voxel state as not current
- voxel ROI accessors still reject contour data access

Verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

Suggested commit:

- `Phase 5B: add contour ROI construction`

### Step 5C: Add active plane-family switching contract

Goal:

- make plane-family ownership explicit without adding migration algorithms

Files expected:

- `src/app/roi_runtime.rs`
- `src/app/components.rs` if helper methods are needed
- `docs/segmentation-reimplementation-plan.md`

Required work:

- add a runtime helper for changing active contour plane family
- reject missing ROI entities
- reject non-contour ROIs
- return success without mutation when the requested family is already active
- allow switching only when the contour data has no loops
- reject switching when authoritative loops exist
- when a switch succeeds, mark derived cache state dirty using the same runtime rules as other contour mutations

Suggested shape:

```rust
pub enum ContourPlaneFamilySwitchError {
    MissingRoi,
    NotContourRoi,
    RequiresConversion,
}

pub fn set_active_contour_plane_family(
    world: &mut World,
    roi_entity: hecs::Entity,
    family: PlaneFamily,
) -> Result<(), ContourPlaneFamilySwitchError>
```

Tests required:

- switching an empty contour ROI updates `active_plane_family`
- switching to the same family is a no-op
- switching a contour ROI with loops returns `RequiresConversion`
- switching a voxel ROI returns `NotContourRoi`
- successful switching marks derived voxel or mesh cache state dirty according to the chosen invalidation helper

Verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

Suggested commit:

- `Phase 5C: define contour plane ownership`

### Step 5D: Add contour mutation and rebuild scheduling contract

Goal:

- make contour mutations affect runtime state without implementing conversion algorithms

Files expected:

- `src/app/roi_runtime.rs`
- `src/app/components.rs` if mutation helpers live on `Roi`
- `docs/segmentation-reimplementation-plan.md`

Required work:

- add a safe runtime helper for replacing or mutating contour data
- ensure contour changes increment the authoritative generation
- mark derived voxel and mesh cache state dirty
- enqueue `RoiJobKind::RebuildVoxelCache` as the future contour-to-voxel rebuild hook
- do not complete the rebuild job unless real voxel data is produced
- keep existing voxel-authoritative ROI behavior unchanged

Suggested shapes:

```rust
pub enum ContourMutationError {
    MissingRoi,
    NotContourRoi,
}

pub fn replace_contour_data(
    world: &mut World,
    roi_entity: hecs::Entity,
    contour_data: ContourData,
) -> Result<(), ContourMutationError>
```

Tests required:

- replacing contour data increments authoritative generation
- replacing contour data marks voxel-derived cache dirty
- replacing contour data marks mesh-derived cache dirty
- replacing contour data queues `RebuildVoxelCache`
- replacing contour data on a voxel ROI returns `NotContourRoi`
- queued rebuild jobs do not require a contour-to-voxel algorithm yet

Verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

Suggested commit:

- `Phase 5D: add contour mutation runtime contract`

### Step 5E: Finish Subplan 5 status tracking

Goal:

- make the plan reflect the actual implementation state before moving to contour editing or conversion algorithms

Files expected:

- `docs/segmentation-reimplementation-plan.md`

Required work:

- update the `Implementation Status` log after each completed step
- include commit hashes once commits exist
- mark Subplan 5 complete only after Steps 5A through 5D are implemented and verified
- keep pending contour editing, rasterization, and mesh work in later subplans

Tests required:

- no additional code tests beyond the step-level tests

Verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

Suggested commit:

- include status updates with the implementation commit they describe

## Acceptance Criteria

Subplan 5 is complete when all of the following are true:

- contour-authoritative ROIs can exist in the core model without authoring tools
- contour points are stored as plane-local millimeter coordinates tied to `PlaneDefinition`
- active contour plane-family ownership is explicit
- plane-family switching is non-destructive and rejects cases that require missing conversion algorithms
- contour mutations dirty derived voxel and mesh state
- contour mutations can enqueue the future voxel rebuild job without implementing rasterization
- current voxel ROI loading, visibility, rendering, stats, and transform behavior remain unchanged
- `docs/segmentation-reimplementation-plan.md` records completed checkpoints and commit hashes

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

## Implementer Prompt Template

Use this prompt shape for lower-tier implementation passes:

```text
Continue the segmentation reimplementation using docs/subplan-5-contour-handoff.md as the source of truth. Implement only Step 5A first. Do not start contour editing, conversion algorithms, renderer expansion, mesh work, registration/resampling, or later Subplan 5 steps. Preserve current viewer behavior. Update docs/segmentation-reimplementation-plan.md with the completed checkpoint and run cargo test -q, cargo check --target wasm32-unknown-unknown -q, and cargo fmt --all before summarizing.
```
