# Subplan 4 Transform Handoff

This document is the implementation brief for `Subplan 4: Transform, Plane, and Geometry Context`.

Use this as the source of truth for the next implementation pass. The goal is to make coordinate and plane behavior explicit before contour or mesh workflows are added.

## Scope

This handoff covers exactly one architectural checkpoint:

1. introduce a shared geometry/plane API that owns coordinate-space conversion rules
2. migrate existing orthogonal and oblique viewport math onto that API
3. preserve current viewer behavior while making patient/world geometry explicit

It does not cover:

- contour storage
- contour editing tools
- mesh storage or deformation
- voxel-to-contour, contour-to-voxel, or mesh conversion algorithms
- multi-overlay renderer expansion beyond the current two-overlay cap
- image/label registration or resampling when geometries differ

## Current Problem

The repository has the correct ROI direction now, but transform logic is still split across several call sites:

- `src/util/orientation.rs` owns `SlicePlane` and several 3D projection helpers
- `src/systems/picking.rs` has private oblique-plane math
- `src/render/geometry.rs` projects annotation/world positions using viewport indices
- `src/gui/overlays.rs` duplicates 2D slice aspect and screen-to-volume math
- `src/app/components.rs` stores voxel dimensions, spacing, and orientation, but not origin/translation
- `src/io/nifti.rs` extracts orientation from NIfTI sform rows, but currently discards the sform translation

Subplan 4 should fix the shared architecture, not add segmentation algorithms.

## Locked Decisions

### 1. Coordinate-space names

Use these names consistently in code comments, tests, and function names:

- `VoxelIndex`: continuous voxel coordinates in image index space. Integer coordinates represent NIfTI voxel centers.
- `SampleIndex`: discrete `[u32; 3]` voxel indices used for texture/data lookup.
- `VolumeUv`: current normalized volume/object coordinates in `[0.0, 1.0]`.
- `PatientWorldMm`: patient/world coordinates in millimeters.
- `PlaneLocalMm`: 2D plane-local coordinates in millimeters.
- `ViewportUv`: normalized viewport coordinates with top-left `[0.0, 0.0]` and bottom-right `[1.0, 1.0]`.
- `EguiScreen`: egui pixel coordinates with Y increasing downward.
- `GpuNdc`: render-facing normalized/device coordinates.

Do not introduce new names like `world`, `object`, or `screen` without qualifying which of these spaces they mean.

### 2. Geometry must include origin

Extend voxel geometry to carry NIfTI translation/origin:

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoxelGeometry {
    pub dimensions: [u32; 3],
    pub spacing: [f32; 3],
    pub origin: [f32; 3],
    pub orientation: [f32; 4],
}
```

Apply the same origin field to the loaded volume/label path:

- `LoadedVolume`
- `LoadedLabel`
- `VolumeData`
- `VoxelGeometry`

For this checkpoint, extract origin from NIfTI sform translation:

```rust
let origin = [header.srow_x[3], header.srow_y[3], header.srow_z[3]];
```

If the sform is invalid and the current loader falls back to identity orientation, use `[0.0, 0.0, 0.0]` for origin. Full qform fallback can be a later loader-hardening task, but do not silently pretend that origin is part of orientation.

### 3. Patient/world transform convention

Use this transform contract:

```text
PatientWorldMm = origin + orientation_matrix * (VoxelIndex * spacing)
```

Where:

- `origin` is the patient/world coordinate of voxel index `[0.0, 0.0, 0.0]`
- `orientation_matrix` is derived from the existing `[x, y, z, w]` quaternion
- `spacing` is applied per axis before rotation
- inverse mapping subtracts origin, applies inverse rotation, then divides by spacing

Add helper functions with roundtrip tests:

```rust
pub fn volume_uv_to_voxel_index(uv: [f32; 3], dimensions: [u32; 3]) -> [f32; 3]
pub fn voxel_index_to_volume_uv(index: [f32; 3], dimensions: [u32; 3]) -> [f32; 3]
pub fn voxel_index_to_world_mm(index: [f32; 3], geometry: VoxelGeometry) -> [f32; 3]
pub fn world_mm_to_voxel_index(world: [f32; 3], geometry: VoxelGeometry) -> [f32; 3]
pub fn volume_uv_to_world_mm(uv: [f32; 3], geometry: VoxelGeometry) -> [f32; 3]
pub fn world_mm_to_volume_uv(world: [f32; 3], geometry: VoxelGeometry) -> [f32; 3]
pub fn sample_index_from_volume_uv(uv: [f32; 3], dimensions: [u32; 3]) -> [u32; 3]
```

Use this geometry conversion formula:

```text
VoxelIndex = VolumeUv * (dimensions - 1)
VolumeUv = VoxelIndex / (dimensions - 1)
```

When a dimension is `0` or `1`, map that axis to `0.0` instead of dividing by zero.

Keep `sample_index_from_volume_uv` compatible with current sampling behavior:

```text
sample = floor(uv * dimensions).clamp(0, dimensions - 1)
```

This preserves current HU and label lookup behavior while the geometry layer becomes more explicit.

### 4. Plane model

Introduce first-class plane types in the shared conversion layer:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlaneFamily {
    Axial,
    Coronal,
    Sagittal,
    Oblique,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaneDefinition {
    pub family: PlaneFamily,
    pub origin_mm: [f32; 3],
    pub u_axis_mm: [f32; 3],
    pub v_axis_mm: [f32; 3],
    pub normal_mm: [f32; 3],
}
```

Plane axes must be normalized in patient/world millimeters.

Add plane-local conversion helpers:

```rust
pub fn plane_local_mm_to_world_mm(local: [f32; 2], plane: PlaneDefinition) -> [f32; 3]
pub fn world_mm_to_plane_local_mm(world: [f32; 3], plane: PlaneDefinition) -> [f32; 2]
```

Orthogonal planes must preserve the current radiological viewport mapping:

- axial viewport U maps toward decreasing volume X
- axial viewport V maps toward decreasing volume Y
- coronal viewport U maps toward decreasing volume X
- coronal viewport V maps toward decreasing volume Z
- sagittal viewport U maps toward decreasing volume Y
- sagittal viewport V maps toward decreasing volume Z

Oblique planes must use the current user rotation semantics, but the math must move out of `src/systems/picking.rs`.

### 5. Module ownership

Put new shared conversion primitives in `src/convert/geometry.rs`.

Update `src/convert/mod.rs`:

```rust
pub mod geometry;
pub use geometry::*;
```

Keep `src/util/orientation.rs` as a compatibility layer during this subplan. Existing helpers can call into `convert::geometry`, but new segmentation code should not add more viewport math to `util/orientation.rs`.

Do not move all 3D camera/gizmo math in this checkpoint unless required by the plane API.

## Implementation Sequence

### Step 4A: Add geometry origin and transform helpers

Files expected:

- `src/app/components.rs`
- `src/io/nifti.rs`
- `src/io/volume.rs`
- `src/app/context.rs`
- `src/app/roi_runtime.rs`
- `src/convert/mod.rs`
- `src/convert/geometry.rs`

Required work:

- add `origin: [f32; 3]` to `VolumeData`, `LoadedVolume`, `LoadedLabel`, and `VoxelGeometry`
- populate origin for main volumes and labels from NIfTI sform translation
- use `[0.0, 0.0, 0.0]` in tests and app defaults
- update `main_volume_voxel_geometry` to include origin
- update label-vs-main geometry mismatch validation to compare origin as well as dimensions, spacing, and orientation
- add conversion helpers listed above

Tests required:

- `VolumeUv -> VoxelIndex -> VolumeUv` roundtrips for normal dimensions and handles `0`/`1` dimensions
- `VoxelGeometry` roundtrips `VoxelIndex -> PatientWorldMm -> VoxelIndex` with nonzero origin and anisotropic spacing
- identity geometry maps origin/index as expected
- label import preserves origin from synthetic sform rows
- ROI import preserves label origin even when the main volume origin differs
- sample-index conversion preserves current floor-and-clamp behavior

Suggested commit:

- `Phase 4A: add explicit voxel world geometry`

### Step 4B: Add plane definitions without behavior change

Files expected:

- `src/convert/geometry.rs`
- `src/util/orientation.rs`

Required work:

- add `PlaneFamily`
- add `PlaneDefinition`
- add constructors for axial, coronal, sagittal, and oblique planes
- keep `SlicePlane` as a compatibility enum for current call sites
- add adapter conversions between `SlicePlane` and `PlaneFamily`
- do not remove old `SlicePlane` methods yet

Required helper shape:

```rust
pub fn orthogonal_plane_from_volume_uv(
    family: PlaneFamily,
    cursor_uv: [f32; 3],
    geometry: VoxelGeometry,
) -> Option<PlaneDefinition>

pub fn oblique_plane_from_view_rotation(
    cursor_uv: [f32; 3],
    rotation: [f32; 4],
    geometry: VoxelGeometry,
) -> Option<PlaneDefinition>
```

Tests required:

- axial/coronal/sagittal plane axes match current radiological screen-axis behavior
- oblique plane axes are normalized
- plane normal is orthogonal to both plane axes
- `PlaneLocalMm -> PatientWorldMm -> PlaneLocalMm` roundtrips for a non-origin point

Suggested commit:

- `Phase 4B: define shared plane model`

### Step 4C: Centralize viewport-to-volume mapping

Files expected:

- `src/convert/geometry.rs`
- `src/systems/picking.rs`
- `src/gui/overlays.rs`
- `src/render/geometry.rs`

Required work:

- add a shared viewport transform helper for zoom, pan, pivot, and screen aspect
- replace private orthogonal screen-to-volume math in picking with the shared helper
- replace duplicated annotation drag math in overlays with the shared helper
- route render projection through the same helper where practical
- preserve existing visible behavior for axial, coronal, and sagittal views

Required helper shape:

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportMapping {
    pub zoom: f32,
    pub pan: [f32; 2],
    pub pivot: [f32; 2],
    pub screen_aspect: f32,
}

pub fn viewport_uv_to_volume_uv(
    viewport_uv: [f32; 2],
    plane: PlaneDefinition,
    geometry: VoxelGeometry,
    mapping: ViewportMapping,
) -> Option<[f32; 3]>

pub fn volume_uv_to_viewport_uv(
    volume_uv: [f32; 3],
    plane: PlaneDefinition,
    geometry: VoxelGeometry,
    mapping: ViewportMapping,
) -> Option<[f32; 2]>
```

Use `pivot: [0.5, 0.5]` where current code hard-codes center-pivot behavior. Do not silently change pan/zoom behavior.

Tests required:

- axial viewport-to-volume results match the old `SlicePlane::screen_uv_to_volume` behavior
- coronal viewport-to-volume results match old behavior
- sagittal viewport-to-volume results match old behavior
- volume-to-viewport is the inverse of viewport-to-volume within tolerance
- egui top-left Y-down convention is explicitly tested

Suggested commit:

- `Phase 4C: route orthogonal views through shared geometry`

### Step 4D: Centralize oblique mapping

Files expected:

- `src/convert/geometry.rs`
- `src/systems/picking.rs`

Required work:

- move `oblique_slice_aspect` and `oblique_screen_uv_to_volume` behavior out of `src/systems/picking.rs`
- implement equivalent behavior through `PlaneDefinition` and `ViewportMapping`
- keep existing oblique visual/picking behavior unless a test exposes a documented bug
- make oblique planes first-class instead of branching into private picking math

Tests required:

- current oblique helper output is preserved for at least one identity rotation case
- current oblique helper output is preserved for at least one non-identity rotation case
- output outside volume bounds is rejected consistently

Suggested commit:

- `Phase 4D: route oblique picking through shared geometry`

### Step 4E: Retire new viewport-index assumptions

Files expected:

- `src/render/geometry.rs`
- `src/gui/overlays.rs`
- `src/systems/picking.rs`
- `src/util/orientation.rs`
- `docs/segmentation-reimplementation-plan.md`

Required work:

- stop adding new code that maps `1 => axial`, `2 => coronal`, `3 => sagittal`
- keep legacy adapters only at UI/runtime boundaries where viewport IDs still exist
- update plan status with completed Subplan 4 checkpoints and commit hashes
- leave a clear note if any viewport-index usage remains intentionally deferred

Tests required:

- CPU picking, annotation overlay placement, and render projection agree for orthogonal views
- oblique picking uses the shared plane API
- WASM compile passes

Suggested commit:

- `Phase 4E: consolidate viewport plane mapping`

## Acceptance Criteria

Subplan 4 is complete when all of the following are true:

- voxel geometry includes dimensions, spacing, origin, and orientation
- loaded image and label NIfTI origin is preserved
- shared conversion helpers own index, UV, patient/world, viewport, and plane-local mapping
- `PlaneFamily` and `PlaneDefinition` exist and support orthogonal plus oblique planes
- oblique math no longer lives as private helper logic in `src/systems/picking.rs`
- picking, overlay dragging, and render projection use the same shared conversion APIs or documented adapters
- egui Y-down and render/GPU coordinate conventions are covered by tests
- `cargo test -q` passes
- `cargo check --target wasm32-unknown-unknown -q` passes

## Guardrails For A Lower-Tier Implementer

Do not:

- start contour or mesh implementation
- add conversion algorithms between voxel, contour, and mesh
- rewrite rendering shaders unless a compile error forces a tiny compatibility edit
- remove `SlicePlane` immediately
- change pan, zoom, cursor, or radiological flip behavior without a failing/updated test
- resample labels when geometry differs from the main volume
- use main-volume geometry as authoritative ROI geometry

Prefer small commits in the sequence above. If a step becomes too large, stop after adding tests and update this handoff with the discovered split.

## Validation Commands

Run these after each code checkpoint:

```bash
cargo test -q
cargo check --target wasm32-unknown-unknown -q
cargo fmt --all
```

Run `cargo fmt --all` before committing.

## Suggested Handoff Prompt

Use this prompt for the implementation model:

```text
Continue the segmentation reimplementation using docs/subplan-4-transform-handoff.md as the source of truth. Implement only Step 4A first. Do not start contours, meshes, conversion algorithms, renderer expansion, or registration/resampling. Preserve current viewer behavior. Update docs/segmentation-reimplementation-plan.md with the completed checkpoint and run cargo test -q, cargo check --target wasm32-unknown-unknown -q, and cargo fmt --all before summarizing.
```
