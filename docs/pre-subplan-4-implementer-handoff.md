# Pre-Subplan 4 Implementer Handoff

This document is the implementation brief for the work that must land before `Subplan 4: Transform, Plane, and Geometry Context`.

Use this document as the source of truth for the next coding step. Do not re-decide the architecture described here while implementing it.

## Scope

This handoff covers exactly two corrections:

1. make voxel-authoritative ROIs self-describing in space
2. make the current two-overlay renderer limit explicit in runtime/UI behavior

It does **not** cover:

- contour data structures
- mesh data structures
- render-pipeline expansion beyond the current two-overlay limit
- plane or transform refactors

## Locked Decisions

### 1. ROI voxel geometry shape

Add a new struct in `src/app/components.rs`:

```rust
pub struct VoxelGeometry {
    pub dimensions: [u32; 3],
    pub spacing: [f32; 3],
    pub orientation: [f32; 4],
}
```

Use this exact ownership model:

- `VoxelData` must store `geometry: VoxelGeometry`
- `VoxelData` must keep `raw_data: Vec<u8>`
- ROI voxel stats must read spacing from `VoxelData.geometry`
- later contour/mesh work may replace this with a more general shared grid/world transform type, but that is out of scope for this checkpoint

Do **not**:

- keep borrowing spacing/orientation from the current main volume for ROI stats
- add a second parallel geometry type for voxel ROIs in another module
- add contour or mesh geometry fields yet

### 2. Label import geometry rule

Current label import does not provide independent spatial metadata. For this checkpoint, use the following temporary rule:

- a loaded label ROI copies `spacing` and `orientation` from the current main volume
- the ROI keeps its own `dimensions` from the loaded label
- after creation, the ROI owns this geometry and must not borrow it again from the main volume

If there is no main volume loaded, label ROI creation must fail rather than guessing geometry.

If label dimensions differ from the current main volume dimensions:

- still create the ROI using the label dimensions
- still copy spacing and orientation from the current main volume
- log a warning describing the mismatch

This is a temporary compatibility rule for the current loader, not the final import architecture.

### 3. Explicit overlay-cap rule

The current renderer only supports two ROI overlay textures. Make that explicit with a temporary runtime/UI cap.

Use this exact temporary policy:

- define `MAX_SIMULTANEOUS_ROI_OVERLAYS: usize = 2`
- no more than two ROIs may be visible at once
- if a new ROI is loaded while fewer than two ROIs are visible, it should start visible
- if a new ROI is loaded while two ROIs are already visible, it should start hidden
- if the user tries to enable visibility on a third ROI, reject the change and keep it hidden

User-facing message for rejected visibility changes:

- `Only two ROI overlays can be visible at once in the current renderer.`

Do **not**:

- silently allow more than two visible ROIs and rely on bind-group truncation
- auto-hide another ROI to make room
- expand the renderer to support more than two overlays in this checkpoint

## Files To Touch

Expected files:

- `src/app/components.rs`
- `src/app/roi_runtime.rs`
- `src/io/handlers.rs`
- `src/gui/sidebar.rs`
- `docs/segmentation-reimplementation-plan.md`

Touch other files only if needed for compilation or tests.

## Required Changes

### A. Make voxel ROI geometry explicit

In `src/app/components.rs`:

- add `VoxelGeometry`
- update `VoxelData` to:
  - remove the top-level `dimensions` field
  - add `geometry: VoxelGeometry`
  - keep `raw_data: Vec<u8>`
- update `Roi::new_voxel(...)` and `Roi::new_voxel_with_cache(...)` to take `VoxelGeometry` instead of raw dimensions
- update affected tests to assert through `voxel.geometry`

### B. Add main-volume geometry lookup helper

In `src/app/roi_runtime.rs`:

- add a small helper that returns the current main-volume voxel geometry
- build that helper from `VolumeData { dimensions, spacing, orientation }`

Suggested shape:

```rust
pub fn main_volume_voxel_geometry(world: &World) -> Option<VoxelGeometry>
```

### C. Move label ROI creation to owned geometry

In `src/app/roi_runtime.rs`:

- update `create_voxel_roi_from_label(...)`
- copy spacing/orientation from `main_volume_voxel_geometry(world)`
- use label dimensions as the ROI dimensions
- if no main volume exists, fail explicitly rather than guessing
- if dimensions mismatch, log a warning

Recommended API change:

```rust
pub fn create_voxel_roi_from_label(...) -> Result<hecs::Entity, String>
```

Then update callers in `src/io/handlers.rs` to surface the error cleanly.

### D. Stop borrowing geometry in voxel stats

In `src/app/roi_runtime.rs`:

- update `voxel_roi_stats(...)` to compute `volume_mm3` from `VoxelData.geometry.spacing`
- do not query `MainVolumeTag` for this calculation anymore

### E. Make overlay-cap behavior explicit

In `src/app/roi_runtime.rs`:

- add `MAX_SIMULTANEOUS_ROI_OVERLAYS: usize = 2`
- add helpers for:
  - counting visible ROIs
  - checking whether another ROI may become visible

Suggested shapes:

```rust
pub fn visible_roi_count(world: &World) -> usize
pub fn can_enable_roi_visibility(world: &World, roi_entity: hecs::Entity) -> bool
```

Use those helpers in two places:

1. during label ROI creation
   - set `is_visible` according to the cap rule
2. in sidebar visibility toggles
   - reject enabling a third ROI
   - keep checkbox state false
   - surface the status message

### F. Keep plan tracking in sync

Update `docs/segmentation-reimplementation-plan.md` in the same code commit:

- mark the implementer handoff as active guidance for the current phase
- when the work is done, move `Current Phase` from `Pre-Subplan 4 Course Corrections` to `Subplan 4: Transform, Plane, and Geometry Context`

## Status Message Handling

When visibility is rejected by the overlay cap:

- set GUI status to:
  - `Only two ROI overlays can be visible at once in the current renderer.`

When label ROI creation fails because no main volume is loaded:

- return an explicit error from the runtime/helper path
- surface a human-readable status message rather than panicking

## Tests To Add

Add unit tests for:

1. voxel ROI constructors preserve `VoxelGeometry`
2. voxel ROI stats use ROI-owned spacing rather than main-volume spacing
3. label ROI creation fails when no main volume is present
4. label ROI creation copies spacing/orientation from the main volume
5. label ROI creation preserves label dimensions even when they differ from the main volume
6. visible-ROI cap allows one and two visible ROIs but rejects a third

Keep using:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`

## Acceptance Criteria

This checkpoint is complete when all of the following are true:

- a voxel-authoritative ROI owns `dimensions`, `spacing`, and `orientation`
- voxel ROI stats no longer depend on global main-volume spacing
- label ROI creation does not guess geometry when no main volume is present
- more than two visible ROIs cannot occur silently
- the plan doc is updated in the same commit

## Suggested Commit Message

Use a focused commit message in this shape:

- `Phase 3A: make voxel ROI geometry explicit`

If you split the work into two commits, use:

- `Phase 3A: make voxel ROI geometry explicit`
- `Fix: make ROI overlay limit explicit`
