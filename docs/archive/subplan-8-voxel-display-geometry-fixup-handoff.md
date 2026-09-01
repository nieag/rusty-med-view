# Subplan 8 Voxel Display Geometry Fixup Handoff

This document is the source of truth for fixing the remaining mesh-vs-voxel placement mismatch seen after the native mesh rendering/projection fix.

Status update: the nearest-neighbor display-grid resampling approach in this document is not accepted as the final correctness step. The follow-up source of truth is [docs/subplan-8-1-spatial-geometry-contract-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-8-1-spatial-geometry-contract-handoff.md:1), which implements a central ROI-native-to-world-to-viewport geometry contract and geometry-aware voxel overlay sampling.

## Problem

Manual verification shows:

- the green voxel label overlay appears in the correct image space
- the red mesh created from that voxel ROI appears displaced

Code inspection shows why this can happen:

- the voxel overlay shader samples the label texture in the main volume texture/display coordinate path
- mesh creation currently extracts from the voxel ROI's authoritative `VoxelData`
- mesh extraction converts voxel indices to `world_mm` using that authoritative `VoxelData.geometry`

Those are not guaranteed to be the same display grid.

If a loaded label's stored geometry differs from how the current overlay path displays the label, mesh extraction can be geometrically self-consistent but visually inconsistent with the green overlay. That is unacceptable for the current mesh workflow because the user action is "create mesh from the displayed voxel ROI".

## Goal

Define and implement a display-compatible voxel source for surface extraction.

The mesh created from a visible voxel ROI must occupy the same screen-space/physical region as the displayed voxel overlay for that ROI.

This should be done through an explicit derived display voxel cache or view model, not by silently overriding authoritative ROI geometry.

## Non-Goals

Do not implement:

- mesh deformation
- mesh-to-voxel regeneration
- mesh-to-contour regeneration
- SDF/TSDF
- smoothing, decimation, booleans, margins
- import/export
- full registration/resampling framework
- broad renderer abstraction rewrite

Nearest-neighbor display-grid resampling is not the accepted Subplan 8.1 solution. Loaded labels retain authoritative geometry, voxel overlay sampling maps through ROI-owned geometry, and mesh extraction remains native to the source voxel geometry.

## Required Architecture

### 1. Preserve authoritative voxel geometry

Loaded voxel/label ROIs must keep their authoritative `VoxelData.geometry`.

Do not "fix" this by replacing the authoritative geometry with main volume geometry.

Authoritative geometry remains the source of truth for:

- imported label identity
- future export
- future proper registration/resampling
- representation conversion that explicitly targets native label space

### 2. Add an explicit display-compatible voxel source

Introduce a named helper or cache boundary for voxel data as displayed in the current viewer.

Suggested helper shape:

```rust
pub enum DisplayVoxelSourceError {
    MissingRoi,
    NotVoxelRoi,
    MissingMainVolume,
    UnsupportedGeometry, // only if the implementation chooses fail-fast instead of nearest-neighbor resampling
}

pub fn voxel_data_for_display_surface_extraction(
    world: &World,
    source_roi: hecs::Entity,
) -> Result<VoxelData, DisplayVoxelSourceError>
```

The exact name may differ, but the contract is required:

- return voxel data in the same grid/geometry used by the displayed voxel overlay
- for geometry matching main volume, this can return the authoritative voxel data unchanged
- for geometry differing from main volume, this must either:
  - produce a nearest-neighbor resampled display voxel grid in main volume geometry, or
  - fail clearly with a user-facing unsupported message
- do not silently use authoritative label geometry if that is not what the green overlay displays

Preferred V1:

- produce a nearest-neighbor display voxel grid in main volume geometry
- use the same all-non-zero binary semantics already documented for mesh extraction
- keep the cache/session-derived nature explicit

Fail-fast V1 is acceptable only if it blocks mesh creation for mismatched geometry with a clear message. It is not acceptable if the UI still creates a visibly displaced mesh.

### 3. Route mesh creation through the display-compatible voxel source

Update `create_mesh_roi_from_voxel_roi(...)`:

- source voxel ROI remains unchanged
- mesh-primary ROI is still newly spawned
- extraction uses `voxel_data_for_display_surface_extraction(...)` or equivalent
- status/error messages are explicit when a display-compatible voxel source cannot be produced

For contour-source mesh creation:

- if the contour ROI already has a current voxel cache in main display geometry, use it
- otherwise require/create display-compatible voxel data before extraction
- do not silently mix contour-derived native geometry with display geometry

### 4. Keep native mesh projection path

Do not undo the native mesh rendering work.

The mesh renderer must still:

- render through wgpu, not egui
- use the shared display projection context
- project mesh `world_mm` through main display volume geometry
- clip to viewport bounds

This fixup changes the voxel source used to create mesh geometry, not the renderer back-end.

## Required Tests

Add tests that encode the actual invariant:

```text
mesh created from displayed voxel ROI overlaps the displayed voxel ROI region
```

Minimum tests:

- geometry-matching voxel ROI returns authoritative voxel data unchanged for display surface extraction
- mismatched voxel ROI returns display-compatible voxel data in main volume geometry, or fails clearly if fail-fast V1 is chosen
- mesh creation from voxel ROI uses the display-compatible voxel source, not the raw authoritative geometry, when geometry differs
- the created mesh's vertex world bounds correspond to the display grid/main volume geometry for the occupied voxels
- non-3D viewport and invalid face tests from the mesh renderer still pass

Regression test shape:

1. create a main volume with geometry `A`
2. create a voxel ROI with raw data containing a compact occupied region but geometry `B` that is shifted/scaled relative to `A`
3. mark it as the same data the current overlay path would display in main-volume texture coordinates
4. create mesh from the voxel ROI
5. assert the mesh world bounds correspond to geometry `A` for those occupied indices, not geometry `B`

If the implementation chooses fail-fast instead:

1. create main geometry `A`
2. create voxel ROI geometry `B`
3. attempt mesh creation
4. assert a clear `UnsupportedGeometry`-style error and no mesh ROI spawned

Do not add a test that merely asserts the mesh has vertices. That missed the original bug.

## Manual Verification

Run these checks before summarizing:

- Action: load the usual image NIfTI.
- Expected: image displays as before.
- Observed: record pass/fail.

- Action: load the usual voxel label NIfTI.
- Expected: green voxel overlay appears in the correct image space.
- Observed: record pass/fail.

- Action: create mesh from the active voxel ROI.
- Expected: red mesh overlays the same region as the green voxel overlay.
- Observed: record pass/fail.

- Action: rotate, pan, and zoom the 3D view.
- Expected: image, green voxel overlay, and red mesh remain locked together.
- Observed: record pass/fail.

- Action: switch viewport protocols or zoom to viewport edge.
- Expected: mesh remains clipped to the 3D viewport.
- Observed: record pass/fail.

## Documentation Updates

Update `docs/segmentation-reimplementation-plan.md`:

- keep Subplan 8 under review until this display-geometry fix passes
- record whether V1 uses nearest-neighbor display-grid resampling or fail-fast unsupported geometry handling
- record that mesh-from-voxel visual workflow consumes display-compatible voxel data, while authoritative voxel geometry remains preserved

Update `docs/rendering-architecture.md`:

- record that voxel overlays and geometry-derived renderables must share a display-compatible source grid when used for visual comparison or workflow creation

## Validation Commands

Run:

```bash
cargo test -q
cargo check --target wasm32-unknown-unknown -q
cargo fmt --all
```

If shader/render pipeline code changes:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

## Continuation Prompt

Use this prompt for the implementer:

```text
Continue the Subplan 8 fixup using docs/subplan-8-voxel-display-geometry-fixup-handoff.md as the source of truth. Fix only the voxel-display-geometry/source-grid mismatch causing meshes created from voxel ROIs to appear displaced from the green voxel overlay. Do not start mesh deformation, mesh-to-voxel regeneration, mesh-to-contour regeneration, SDF/TSDF, smoothing/decimation/booleans, import/export, registration/resampling, or later phases. Preserve authoritative voxel ROI geometry; do not silently overwrite label geometry with main volume geometry. Introduce a display-compatible voxel source/helper or cache for surface extraction. Mesh creation from a displayed voxel ROI must use that display-compatible voxel data, or fail clearly if a display-compatible source cannot be produced. Keep native wgpu mesh rendering and the shared display projection context. Add tests proving that mesh creation from a mismatched-geometry voxel ROI uses main display geometry/display-compatible voxel data rather than raw authoritative geometry, or fails clearly in fail-fast V1. Update docs/segmentation-reimplementation-plan.md and docs/rendering-architecture.md, then run cargo test -q, cargo check --target wasm32-unknown-unknown -q, and cargo fmt --all before summarizing. Include manual verification expected vs observed for image load, voxel label display, mesh creation, 3D rotate/pan/zoom lockstep, and viewport clipping.
```
