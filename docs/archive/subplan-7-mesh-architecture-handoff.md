# Subplan 7 Mesh Representation Architecture Handoff

This document is the implementation brief for `Subplan 7: Mesh Representation Architecture`.

Use this as the source of truth for the next implementation passes. The goal is to introduce mesh as a first-class representation family with explicit authoritative/derived rules, without starting mesh editing or broad renderer expansion yet.

## Scope

This handoff covers:

1. adding mesh-authoritative ROI state to the core model
2. defining mesh cache ownership and invalidation relationships
3. defining explicit mesh-to-voxel and mesh-to-contour rebuild contracts
4. exposing runtime-safe creation and access paths for mesh-primary ROIs
5. updating the main plan status as the phase advances

It does not cover:

- mesh generation algorithms
- mesh deformation/editing tools
- mesh rendering expansion beyond what is needed for representation plumbing
- SDF/TSDF work
- contour smoothing/simplification/interpolation
- registration or resampling
- import/export
- speculative performance work

## Current State

The repository already has:

- voxel-authoritative ROI support
- contour-authoritative ROI support
- contour-derived voxel rebuilds through `Subplan 6.5`
- voxel-to-contour extraction through `Subplan 6.6`
- a completed post-`6.6` consolidation checkpoint
- shared ROI runtime/job scaffolding

`Subplan 7` must extend that model, not bypass it.

## Locked Decisions

### 1. Mesh is a representation family, not just a render artifact

Mesh must follow the same architecture shape as voxel and contour:

- it can be authoritative
- it can have derived caches
- it participates in explicit conversion contracts

Do not introduce mesh as “just something the renderer happens to draw”.

### 2. Architecture first, algorithms later

This phase is about state, contracts, and invalidation rules.

Do not start:

- marching cubes or other surface extraction
- mesh slicing algorithms
- mesh deformation tools
- mesh voxelization

Those belong to later phases once the representation boundaries are stable.

### 3. Volume remains voxel-derived

Even after mesh is introduced, volume/stat behavior should still conceptually come from voxel data, not directly from mesh geometry.

That means mesh-primary ROIs need explicit rebuild paths toward voxel-derived state rather than ad hoc direct measurement rules.

### 4. Mesh should fit the existing ROI runtime

Do not create a separate mesh subsystem with its own independent lifecycle.

Mesh state should plug into:

- `RoiAuthoritativeData`
- `RoiSessionCaches`
- dirty/cache generation tracking
- queued/running job state
- runtime rebuild APIs

## Desired End State

At the end of `Subplan 7`, the repo should have:

- mesh-authoritative ROI state in the core model
- explicit mesh cache slots/relationships
- explicit dirty/invalidation rules for mesh-primary mutation
- explicit rebuild contracts for:
  - mesh -> voxel
  - mesh -> contour
- no ambiguity about how mesh fits the multi-representation architecture

## Implementation Steps

### Step 7A: Mesh core types and ROI state

Goal:
- introduce the minimal mesh data model and mesh-primary ROI construction.

Tasks:

- add mesh core types in the ROI model, for example:
  - `MeshVertex`
  - `MeshFace` or equivalent indexed-triangle representation
  - `MeshData`
- extend `RoiAuthoritativeData` so mesh-primary authoritative state is explicit
- add `Roi::new_mesh(...)`
- add accessors for authoritative mesh data similar to voxel/contour accessors
- add focused tests for:
  - mesh-primary ROI initialization
  - mesh authoritative accessors
  - non-mesh ROIs rejecting mesh accessors appropriately

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

### Step 7B: Mesh cache slots and invalidation rules

Goal:
- make mesh participate in the existing cache/runtime model.

Tasks:

- decide and implement how mesh cache data is stored in `RoiSessionCaches`
- define invalidation rules for mesh-primary authoritative edits:
  - voxel cache dirty
  - contour cache dirty
  - mesh cache behavior explicit
- define generation/current-state expectations for mesh-related caches
- add tests for dirty/current behavior after mesh-authoritative mutation

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

### Step 7C: Runtime contracts for mesh-primary ROIs

Goal:
- expose mesh-primary creation and mutation boundaries through runtime APIs instead of raw field mutation.

Tasks:

- add runtime helper(s) for creating mesh-primary ROIs
- add a mesh-authoritative replacement/mutation helper if needed
- ensure mutations route through the same invalidation/job contract style used by contour edits
- add tests for:
  - mesh ROI creation
  - mesh mutation success
  - derived-cache dirtying/job queue behavior

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

### Step 7D: Explicit conversion contract scaffolding

Goal:
- make later mesh conversion phases plug into named runtime contracts rather than ad hoc future work.

Tasks:

- define explicit job/runtime hooks or placeholder contract points for:
  - `RebuildVoxelCache` from mesh-primary data
  - `RebuildContourCache` from mesh-primary data
- document clearly that the actual algorithms are deferred
- ensure unsupported execution paths fail safely rather than silently pretending mesh-derived data exists
- add tests for placeholder/failure behavior if new runtime entry points are added

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

### Step 7E: Closeout

Goal:
- finish the architecture phase with a clear route into mesh workflow implementation.

Tasks:

- rerun full validation
- update `docs/segmentation-reimplementation-plan.md`
- record what is now defined vs what remains deferred to `Subplan 8`
- keep the final summary explicit about what mesh can and cannot do yet

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`
- `cargo clippy --all-targets --all-features -- -D warnings`

## Acceptance Checklist

`Subplan 7` is complete when all of the following are true:

- mesh-primary ROI state exists in the core model
- mesh fits the existing ROI runtime/cache architecture
- mesh-to-voxel and mesh-to-contour rebuild contracts are explicit
- unsupported mesh conversion behavior is explicit and safe
- the repo is ready to start `Subplan 8` without redefining mesh ownership rules

## Expected Summary Format

Every implementation summary for this subplan must include:

1. which `7.x` step was completed
2. what changed in practical terms
3. which mesh behaviors are now defined architecturally
4. which mesh algorithms/workflows remain deferred
5. validation commands run

## Prompt Shape

Use this prompt shape for lower-tier implementation passes:

```text
Continue the segmentation reimplementation using docs/subplan-7-mesh-architecture-handoff.md as the source of truth. Implement only Step 7A first. Do not start mesh generation algorithms, mesh editing/deformation, renderer expansion, SDF/TSDF work, contour smoothing/interpolation, import/export, registration/resampling, or later Subplan 7 steps. Preserve current image, voxel label, contour extraction, contour editing, contour-to-voxel rebuild, 2D navigation, and 3D viewer behavior. Update docs/segmentation-reimplementation-plan.md with the completed checkpoint, run cargo test -q, cargo check --target wasm32-unknown-unknown -q, and cargo fmt --all before summarizing.
```
