# Subplan 6.5 Contour-to-Voxel Conversion Handoff

This document is the implementation brief for `Subplan 6.5: Contour-to-Voxel Conversion V1`.

Use this as the source of truth for the next implementation passes. The goal is to close the first practical representation loop: contour-authoritative ROIs should be able to rebuild a voxel-derived cache for volume calculation and voxel overlay rendering, without making contour data non-authoritative.

## Scope

This handoff covers:

1. adding a CPU-backed derived voxel cache shape
2. defining the target voxel grid for contour-derived voxel caches
3. implementing deterministic contour loop rasterization into that grid
4. wiring `RebuildVoxelCache` jobs for contour-primary ROIs
5. updating voxel-derived volume/stat helpers to use current derived voxel caches
6. preserving current image, voxel label, contour editing, navigation, and 3D behavior

It does not cover:

- voxel-to-contour extraction
- mesh representation, mesh generation, mesh deformation, or mesh-derived caches
- SDF/TSDF caches
- interpolation between contour slices
- smoothing, simplification, boolean operations, or margin expansion
- registration or resampling
- import/export
- renderer expansion beyond the current voxel overlay texture path
- async workers or full frame-budgeted scheduling

Voxel-to-contour extraction is intentionally not part of this handoff, but it is not optional in the larger plan. Existing real test data is currently voxel labelmaps, so `Subplan 6.6: Voxel-to-Contour Extraction V1` should follow before mesh architecture work. This handoff only closes the edited-contour-to-voxel-cache direction.

## Current State

The repository already has:

- ROI authoritative state in `src/app/components.rs`
- `RoiAuthoritativeData::Voxel(VoxelData)` and `RoiAuthoritativeData::Contour(ContourData)`
- `VoxelData { geometry: VoxelGeometry, raw_data: Vec<u8> }`
- `ContourData`, `ContourSlice`, `ContourLoop`, and `ContourPoint`
- `RoiSessionCaches { voxel, contour, mesh }`, where `voxel` currently stores GPU resources only
- `RoiJobKind::RebuildVoxelCache`
- `replace_contour_data(...)`, which queues `RebuildVoxelCache`
- shared geometry helpers in `src/convert/geometry.rs`, including:
  - `voxel_index_to_world_mm(...)`
  - `world_mm_to_voxel_index(...)`
  - `volume_uv_to_world_mm(...)`
  - `world_mm_to_volume_uv(...)`
  - `PlaneDefinition`
  - `PlaneFamily`
- native contour rendering through a dedicated WGPU contour pass
- voxel overlay rendering through ROI renderable voxel cache accessors

Subplan 6.5 must build on these pieces. Do not create a second ROI store, do not bypass ROI runtime state, and do not mutate contour-authoritative data during conversion.

## Locked Decisions

### 1. Authoritative state stays contour-primary

For a contour ROI:

- `RoiAuthoritativeData::Contour(ContourData)` remains the editable source of truth
- conversion writes only derived/session voxel cache state
- conversion must not rewrite `ContourData`
- conversion must not change `primary_representation`

Expected result after a successful rebuild:

- authoritative contour data is unchanged
- voxel cache is current for the authoritative generation used by the rebuild
- voxel-derived volume/stat helpers can read the derived voxel cache
- voxel overlay rendering can use the derived voxel cache through the existing render path

### 2. Derived voxel cache must be CPU-backed

The current `RoiSessionCaches::voxel` stores only `GpuVolumeResources`. That is not enough for contour-derived volume, testability, or later export.

Add a cache shape that owns CPU voxel data plus optional GPU resources. Suggested shape:

```rust
pub struct VoxelCache {
    pub data: VoxelData,
    pub gpu_resources: Option<GpuVolumeResources>,
}
```

Then change:

```rust
pub struct RoiSessionCaches {
    pub voxel: Option<VoxelCache>,
    pub contour: Option<ContourCache>,
    pub mesh: Option<MeshCache>,
}
```

Keep existing render-facing accessors stable where possible:

```rust
pub fn voxel_cache(&self) -> Option<&VoxelCache>
pub fn voxel_gpu_cache(&self) -> Option<&GpuVolumeResources>
pub fn renderable_voxel_cache(&self) -> Option<&GpuVolumeResources>
```

Existing voxel-label behavior must keep working. Voxel-primary ROIs should wrap their authoritative `VoxelData` plus GPU resources in the same cache shape so the renderer sees no behavioral regression.

### 3. Target voxel grid policy

V1 should use one explicit target grid policy:

- if a main volume exists, use the current main volume `VoxelGeometry` as the target grid for contour-derived voxel caches
- store that selected `VoxelGeometry` in the derived `VoxelData`
- do not infer the derived cache geometry later from whatever main volume happens to be loaded
- if no main volume exists, fail the rebuild safely with a status/log message and leave the authoritative contour data unchanged

This policy is intentionally conservative. It does not mean label/ROI geometry must match the main image. It only means V1 contour rasterization needs a deterministic grid target, and the current main image grid is the first practical default.

Future work may add explicit user-selected reference grids or ROI-owned rasterization grids.

### 4. Rasterization semantics

V1 rasterization should be deterministic and simple:

- rasterize closed loops only
- skip invalid loops with fewer than three points
- fill voxels by testing voxel centers
- for each contour slice, include a voxel if its center is within the slice slab tolerance for that contour plane
- map included voxel centers into the contour slice's `PlaneDefinition` local 2D coordinates
- use an even-odd fill rule across all closed loops on the slice
- write label value `1` for inside voxels and `0` elsewhere
- no interpolation between contour slices
- no partial volume weighting
- no smoothing/margins/booleans

Suggested slice slab tolerance:

- use half of the target grid spacing projected along the plane normal when practical
- for orthogonal planes, this should correspond to half the target spacing on the slice axis
- keep the helper deterministic and covered by unit tests

If oblique contour slices exist, the same plane-distance and plane-local point-in-polygon path should work. If a limitation is found, reject oblique rasterization explicitly with a status/log message rather than silently producing wrong data.

### 5. Runtime and generation contract

Conversion must run through the ROI runtime boundary.

For a synchronous V1 rebuild, the implementation may process one rebuild directly in a system or helper, but it must still respect the job state:

1. find contour-primary ROIs with queued `RoiJobKind::RebuildVoxelCache`
2. capture the authoritative generation before conversion
3. begin the queued job
4. build the derived `VoxelData`
5. upload the voxel texture to GPU resources
6. store `VoxelCache { data, gpu_resources: Some(...) }`
7. complete `RoiCacheKind::Voxel` only if the ROI still has the same authoritative generation
8. if the generation changed during rebuild, discard the stale result and leave or requeue the cache dirty

The first implementation can stay synchronous because contour volumes are small enough for correctness work. Do not introduce workers, cancellation UI, or full frame-budgeted scheduling in this subplan.

### 6. Volume/stat behavior

Voxel-derived volume must be available for both:

- voxel-primary ROIs from authoritative voxel data
- contour-primary ROIs from current derived voxel cache data

Add or update a helper with semantics like:

```rust
pub fn roi_voxel_stats(world: &World, roi_entity: hecs::Entity) -> Option<VoxelRoiStats>
```

Rules:

- if ROI is voxel-primary, stats may use authoritative `VoxelData`
- if ROI is contour-primary, stats must use a current derived voxel cache
- if no current derived voxel cache exists, return `None` rather than deriving stale stats
- stats must use the geometry stored with the voxel data being counted

## Implementation Steps

### Step 6.5A: CPU-backed voxel cache model

Goal:
- make derived voxel caches capable of carrying CPU data and GPU resources.

Tasks:

- add `VoxelCache` in `src/app/components.rs`
- change `RoiSessionCaches::voxel` from `Option<GpuVolumeResources>` to `Option<VoxelCache>`
- update voxel-primary ROI constructors to populate `VoxelCache { data, gpu_resources }`
- update ROI accessors so existing render code can still request `GpuVolumeResources`
- update render prep and bind-group code only as needed to compile against the new accessors
- add tests showing voxel-primary ROIs still have renderable current voxel caches
- do not implement contour rasterization in this step

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`
- manual check: load image and voxel label; expected: label still appears as before

### Step 6.5B: Target grid and pure raster primitives

Goal:
- add pure, CPU-testable conversion primitives without touching job execution yet.

Tasks:

- add a conversion module, suggested path: `src/convert/contour_raster.rs`
- expose a pure function shape like:

```rust
pub fn rasterize_contours_to_voxel_data(
    contour: &ContourData,
    target_geometry: VoxelGeometry,
) -> Result<VoxelData, ContourRasterizationError>
```

- implement point-in-polygon using an even-odd rule
- implement voxel-center world mapping through shared geometry helpers
- implement plane-distance slab inclusion
- skip invalid/non-closed loops deterministically
- add errors for unsupported/invalid input only when conversion cannot safely continue
- add unit tests for:
  - simple square fill on an axial slice
  - empty contour data creates an all-zero voxel cache or returns a documented empty result
  - invalid open loop is skipped
  - multiple loops use even-odd behavior
  - geometry stored on returned `VoxelData` equals the target grid

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

### Step 6.5C: Runtime rebuild helper

Goal:
- connect contour-primary queued `RebuildVoxelCache` jobs to the pure rasterizer.

Tasks:

- add a runtime helper in `src/app/roi_runtime.rs` or a small dedicated system module
- select target geometry from `main_volume_voxel_geometry(world)`
- reject rebuild with a clear status/log if no target geometry exists
- clone/capture contour data and authoritative generation before conversion
- begin the queued voxel rebuild job
- call the pure rasterizer
- store the resulting CPU `VoxelData` in `RoiSessionCaches::voxel`
- complete the voxel cache rebuild only if generation is still current
- add tests for:
  - queued contour ROI rebuild creates a current voxel cache
  - missing main volume fails without mutating contour data
  - stale generation result does not mark voxel cache current
  - successful rebuild clears/runs job state correctly

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

### Step 6.5D: GPU upload and overlay integration

Goal:
- make rebuilt contour-derived voxel caches render through the existing voxel overlay path.

Tasks:

- add a reusable texture upload helper for `VoxelData`/`&[u8]` label data, likely near `src/io/volume.rs`
- after rasterization, upload the derived voxel bytes into an `R8Uint` 3D texture
- store `GpuVolumeResources` inside the derived `VoxelCache`
- ensure `renderable_voxel_cache()` returns the derived GPU resources only when the voxel cache is current
- ensure bind-group recreation happens after a successful rebuild
- preserve the two-overlay cap behavior from the retrospective fixup
- add tests where possible around accessors and cache state; GPU upload itself may rely on compile/manual verification

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`
- manual check: create contour ROI, draw a closed loop, trigger rebuild, expected: voxel overlay appears for the contour-derived cache while contour lines remain authoritative/editable

### Step 6.5E: Stats, status, and UI integration

Goal:
- expose the practical result of conversion without expanding the renderer or tool scope.

Tasks:

- add/update `roi_voxel_stats(...)` to support current derived voxel caches for contour-primary ROIs
- surface a concise status message when a contour voxel rebuild succeeds or fails
- ensure contour edits continue to queue voxel rebuilds after every committed edit
- decide when rebuilds are processed in V1:
  - acceptable: process queued contour voxel rebuilds synchronously after commit or during the next update tick
  - not required: progress UI, cancellation, worker scheduling, or partial rebuild
- add tests for stat behavior:
  - voxel-primary stats still work
  - contour-primary stats return `None` before rebuild
  - contour-primary stats return occupied voxel count and volume after rebuild
- update `docs/segmentation-reimplementation-plan.md` with completed checkpoints as steps land

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`
- manual check: edit a contour point after rebuild; expected: contour remains editable and derived voxel overlay/stat updates after rebuild

### Step 6.5F: Closeout and regression review

Goal:
- finalize Subplan 6.5 and document limitations honestly.

Tasks:

- verify all Step 6.5A through 6.5E acceptance criteria
- update `docs/segmentation-reimplementation-plan.md` Implementation Status with:
  - completed steps
  - commit hashes
  - validation commands
  - manual verification checklist with expected vs observed behavior
- document deferred limitations:
  - no interpolation between contour slices
  - no partial-volume calculation
  - no smoothing/margins/booleans
  - no mesh/SDF/TSDF
  - no registration/resampling
  - no import/export
- run final required verification

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`
- `cargo clippy --all-targets --all-features -- -D warnings`
- manual regression checklist:
  - load image NIfTI; expected: image renders as before
  - load voxel label NIfTI; expected: label overlay renders as before
  - create contour ROI and draw a closed loop; expected: contour line renders natively
  - rebuild contour-derived voxel cache; expected: voxel overlay appears and volume/stat is non-zero
  - move/delete contour points; expected: contour authority remains editable and derived voxel cache updates only through rebuild
  - pan/zoom/slice scroll/3D view; expected: existing viewer behavior remains unchanged

## Acceptance Criteria

Subplan 6.5 is complete when all of the following are true:

- contour-primary ROIs can rebuild a current voxel-derived cache
- derived voxel cache stores CPU `VoxelData` plus GPU resources
- derived voxel cache geometry is explicit and stored with the cache
- contour-authoritative data remains unchanged by conversion
- voxel-derived stats work for contour-primary ROIs after rebuild
- voxel overlay rendering can use contour-derived voxel caches
- every committed contour edit continues to dirty/queue voxel cache rebuild work
- invalid or unsupported conversion input fails safely without corrupting authoritative data
- current image load, voxel label load, contour editing, navigation, and 3D view behavior remain unchanged

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
Continue the segmentation reimplementation using docs/subplan-6-5-contour-to-voxel-handoff.md as the source of truth. Implement only Step 6.5A first. Do not start contour rasterization, runtime rebuild execution, GPU upload changes beyond the cache model, stats/UI integration, mesh work, SDF/TSDF, interpolation, smoothing, renderer expansion, import/export, registration/resampling, or later Subplan 6.5 steps. Preserve current image, voxel label, contour editing, 2D navigation, and 3D viewer behavior. Update docs/segmentation-reimplementation-plan.md with the completed checkpoint, run cargo test -q, cargo check --target wasm32-unknown-unknown -q, and cargo fmt --all, and include a concrete manual verification checklist with expected vs observed behavior before summarizing.
```
