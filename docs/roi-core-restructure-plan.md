# ROI Core Restructure Plan

Date: 2026-08-30  
Status: In progress  
Scope: Coordinate ownership, ROI state orchestration, derived caches, and removal of transitional runtime code

## Purpose

Restructure the ROI core in place without rebooting the application.

The existing renderer, interaction systems, conversion algorithms, ECS scene, QA tooling, and native/WASM shell remain. The work replaces the weak center: coordinate ownership, representation state transitions, cache scheduling, and GPU synchronization ordering.

This document turns the findings in [architecture-review.md](architecture-review.md) into an implementation sequence. The existing [roi-multi-representation-closeout-plan.md](roi-multi-representation-closeout-plan.md) remains the behavioral and latency acceptance reference. Once this plan is accepted, it supersedes the structural cleanup sequence in [roi-authoring-runtime-cleanup-plan.md](roi-authoring-runtime-cleanup-plan.md).

## Decision summary

| Area | Decision |
| --- | --- |
| Repository | Keep this application; do not start a third implementation |
| ECS | Keep for scene composition and entity identity |
| ROI semantics | Put behind one cohesive ROI domain API |
| Coordinate ownership | Every ROI owns immutable validated reference-grid geometry |
| World interchange | Convert independent objects through world/patient millimetres only |
| Authority | Exactly one authoritative representation per ROI |
| Derived work | Demand-driven, revision-checked, and geometry-checked |
| Runtime ordering | One coordinator advances ROI work |
| GPU state | Rebuildable mirror of accepted CPU caches |
| Rendering | Keep native WGPU viewer rendering |
| UI | Keep egui for application UI only |
| Geometry algorithms | Keep current algorithms and dependencies |
| Crate structure | Keep one crate during v0 stabilization |

## Problems being fixed

### Coordinate ownership is incomplete

`VoxelGeometry` currently belongs to `VoxelData`. When contour or mesh data is authoritative, voxelization can obtain its target geometry from the current main volume. That means an unchanged ROI can be interpreted differently after the display volume changes.

Geometry fields are publicly constructible origin/spacing/quaternion values. Validation is distributed, and an invalid quaternion can silently normalize to identity. Coordinate-space meaning is often carried by raw `[f32; 2]` and `[f32; 3]` arrays.

### ROI work ordering is distributed

The frame pipeline calls multiple conversion processors and GPU synchronization functions in a required order. The mesh-trigger regression showed that an intermediate synchronization pass can accidentally suppress queued work.

### The old and new ROI APIs coexist

Focused modules under `src/app/roi/` own much of the intended behavior, while `src/app/roi_runtime.rs` still contains compatibility wrappers, duplicate-ROI helpers, execution, import behavior, GPU synchronization, and many tests.

### Derived work is broader than current demand

Some representation rebuilds and GPU uploads happen even when the output is not visible or required by an active tool. Full-volume uploads remain the common fallback after local edits.

## Target architecture

```text
GUI / input
    -> authoring systems
    -> ROI domain operation
         - validate edit
         - commit authority revision
         - invalidate derived caches
         - enqueue required work
    -> advance_roi_work(world, gpu, budget)
         - select demanded work
         - execute conversion
         - reject stale/mismatched result
         - install CPU cache
         - synchronize GPU mirror
         - report whether work remains
    -> WGPU rendering
```

ECS continues to answer which entities exist and which systems consume them. The ROI domain API answers which state transitions are valid.

## Coordinate contract

### Canonical spaces

```text
Screen pixels
    <-> viewport/camera
Plane-local millimetres
    <-> validated plane frame
World/patient millimetres
    <-> ROI-owned grid affine
ROI IJK coordinates

World/patient millimetres
    <-> volume-owned grid affine
Display-volume IJK coordinates
```

Rules:

1. World/patient millimetres are the only interchange space between independently owned objects.
2. Integer IJK coordinates identify voxel centres.
3. The continuous grid bounds are `[-0.5, dimension - 0.5]` on each axis.
4. CPU geometry calculations use `f64`; conversion to `f32` occurs at GPU upload boundaries.
5. Normalized volume UV is navigation/render state, not persistent ROI geometry.
6. ROI-to-display alignment goes ROI IJK -> world -> display IJK. Direct ROI UV -> display UV mapping is forbidden.
7. Import code owns normalization into the selected world convention.

Before implementation, the NIfTI import path must be audited and the canonical world convention recorded in ADR 0003. Do not silently change existing orientation semantics during the restructure.

### Validated grid geometry

Replace publicly constructible `VoxelGeometry` with a validated value using the existing `glam` dependency:

```rust
pub struct RoiGeometry {
    dimensions: [u32; 3],
    ijk_to_world: DMat4,
    world_to_ijk: DMat4,
    identity: GeometryIdentity,
}
```

Exact names may follow existing module vocabulary. Construction must reject:

- zero dimensions;
- non-finite matrices;
- non-affine matrices;
- singular transforms;
- zero-length grid axes.

The inverse and identity are computed only by the validated constructor. Fields remain private.

Use `src/convert/geometry.rs` initially rather than creating another geometry package. Split it later only if a real dependency boundary requires it.

### Typed boundary values

Introduce minimal newtypes at public conversion boundaries:

```rust
struct IjkPoint(DVec3);
struct WorldPoint(DVec3);
struct PlanePoint(DVec2);
struct ScreenPoint(DVec2);
```

Do not rewrite every local vector immediately. Algorithms may unwrap to `glam` internally. The purpose is to prevent cross-space mistakes at module boundaries.

### Plane ownership

Keep arbitrary and oblique contour planes. Replace independently constructible origin, U axis, V axis, and normal fields with a validated plane frame. Derive the normal from non-collinear U and V axes so stored values cannot disagree.

Contour vertices remain plane-local millimetres. Mesh vertices remain world millimetres. Mask samples remain ROI IJK values tied to `RoiGeometry`.

### ROI reference geometry

Every v0 ROI owns reference geometry even when contour or mesh is authoritative:

```rust
struct Roi {
    metadata: RoiMetadata,
    geometry: RoiGeometry,
    authority: RoiAuthoritativeData,
    // existing cache, job, preview, and history state
}
```

Creation policy:

- contour drawn on a volume snapshots that volume geometry into the new ROI;
- imported labelmap uses its own labelmap geometry;
- imported mesh requires an explicitly selected reference grid before voxelization;
- changing the displayed volume never mutates ROI geometry.

Geometry is immutable for v0. Resampling is a future explicit operation, not a field update.

## Representation and cache contract

Exactly one representation is authoritative. Voxel, contour, and mesh caches remain on the same ROI identity.

Every accepted derived cache must identify at least:

- ROI identity;
- source authority representation;
- source authority revision/generation;
- ROI geometry identity;
- conversion parameters that affect output.

A mismatch makes the result stale and prevents installation. A derived conversion never transfers authority. Only a validated user edit or explicit promotion does that.

Contour plane family should eventually be part of the representation target value rather than a parallel optional field, but only where this removes existing state and branches. Do not copy the sibling application's full conversion-plan/provenance hierarchy for v0.

## Work coordinator contract

Expose one application-level entry point:

```rust
fn advance_roi_work(world: &mut World, gpu: &GpuContext, budget: WorkBudget) -> WorkStatus;
```

The concrete signature can follow current ownership constraints. It must own:

1. selecting the highest-priority demanded job;
2. validating its source revision and geometry;
3. executing or advancing conversion work;
4. validating completion again;
5. installing the CPU cache;
6. synchronizing affected GPU mirrors;
7. reporting pending work so the event loop requests another frame.

Internally reuse the existing processors first. Do not create a generic task framework or trait hierarchy.

Priority order remains:

1. active interaction preview;
2. visible committed representation;
3. active-tool prerequisite;
4. explicit export or QA request;
5. background cache warming, if retained at all.

## Demand contract

A stale derived cache is not automatically a job. Work is scheduled when demanded by:

- a visible viewport;
- the active authoring tool;
- export;
- explicit QA or testing.

For example, committing a contour while its mesh is hidden marks the mesh stale. It need not rebuild until the 3D mesh view or a mesh-dependent tool requests it.

Demand changes must request a redraw when new work is queued. Superseded preview work should be discarded before expensive conversion where practical and always before cache or GPU installation.

## Migration phases

Each phase must leave the application runnable. Do not maintain a permanent legacy/new feature flag.

### Phase 0: Freeze behavior and coordinate conventions

Deliver:

- document the current NIfTI/world convention in ADR 0003;
- retain the contour-to-mesh trigger regression test;
- add black-box acceptance coverage for contour commit -> voxel -> mesh -> GPU-visible result;
- capture current QA fields needed to identify authority, geometry, caches, and queued work.

Exit criteria:

- current behavior is reproducible before structural changes;
- voxel-centre and orientation conventions are explicit.

### Phase 1: Introduce validated ROI geometry

Deliver:

- validated affine-based `RoiGeometry` and geometry identity;
- IJK/world round-trip tests, including rotation, reflection, anisotropic spacing, and supported affine cases;
- rejection tests for invalid geometry;
- compatibility conversion at import boundaries from current volume metadata;
- ROI creation snapshots explicit geometry.

Exit criteria:

- new ROIs always own valid geometry;
- no new code constructs raw geometry fields.

### Phase 2: Migrate the contour tracer path

Migrate one complete path:

```text
draw contour
-> commit contour authority
-> rasterize into ROI grid
-> derive mesh in world space
-> install caches
-> upload and render
```

Deliver:

- conversions use `roi.geometry`, never the main volume;
- contour planes use the validated plane contract;
- cache acceptance checks geometry identity;
- display alignment maps through world space.

Exit criteria:

- switching the displayed main volume cannot change the committed ROI's world geometry;
- mismatched ROI and display geometries render through deliberate world mapping.

### Phase 3: Centralize ROI work advancement

Deliver:

- one `advance_roi_work` entry point;
- existing processors hidden behind it;
- pending work reliably drives redraw;
- regression coverage for queued mesh work surviving earlier synchronization;
- rapid edits reject superseded results.

Exit criteria:

- the frame pipeline no longer knows individual ROI conversion ordering;
- QA reaches an empty pending queue with caches matching the latest authority revision.

### Phase 4: Migrate voxel and mesh authority paths

Deliver:

- voxel import/edit uses ROI-owned geometry;
- mesh edit/voxelization requires explicit ROI reference geometry;
- undo/redo restores authority state without changing geometry;
- all derived paths validate geometry identity;
- representation demand includes contour plane family where needed.

Exit criteria:

- all authority-to-derived transitions use the same geometry and acceptance contracts;
- no ROI conversion calls `main_volume_voxel_geometry()`.

### Phase 5: Delete transitional code

Remove:

- `VoxelGeometry` public field construction;
- invalid-orientation fallback to identity;
- main-volume geometry fallback for ROI conversion;
- duplicate-ROI conversion helpers;
- uncalled mesh translation and cache request APIs;
- delegating `roi_runtime` compatibility wrappers;
- duplicate coordinate and plane-compatibility helpers;
- broad exports that expose deleted internals.

Move the remaining conversion execution into `app/roi/executor.rs` only if that is its actual final responsibility. Move ROI-specific state out of `components.rs` after callers use the focused API.

Exit criteria:

- `roi_runtime.rs` is removed or contains no compatibility facade;
- one obvious module owns each ROI transition;
- no production callers use legacy APIs.

### Phase 6: Optimize measured bottlenecks

Measure separately:

- queue delay;
- contour rasterization;
- mesh extraction;
- CPU cache installation;
- CPU-to-GPU transfer;
- visible convergence latency.

Then implement only measured wins:

- demand-driven mesh generation;
- dirty texture-region uploads;
- retained bind groups when resource shape is unchanged;
- affected mesh-chunk updates;
- stronger work coalescing.

Keep full rebuild as the correctness fallback.

## Acceptance matrix

| Scenario | Required outcome |
| --- | --- |
| Draw and commit contour | One authority revision; demanded voxel/mesh caches converge |
| Rapid consecutive contour edits | Only latest revision can install caches or GPU resources |
| Switch displayed volume | ROI world geometry remains unchanged |
| Imported labelmap differs from display volume | Overlay and mesh align through world space |
| Oblique contour | Plane-local points rasterize using ROI geometry without main-volume fallback |
| Mesh-authoritative voxelization | Explicit ROI reference grid is used |
| Invalid affine/orientation | Import or construction fails without silently changing orientation |
| Undo/redo | Authority and caches reflect restored revision; ROI geometry remains stable |
| Hidden mesh | Mesh may remain stale without consuming rebuild time |
| Mesh becomes visible | Demand queues rebuild and redraw continues until current |
| Stale completion | Result is discarded before CPU cache and GPU installation |
| Native and WASM builds | Same coordinate and cache acceptance semantics |

## QA visibility

The QA snapshot should expose enough state to distinguish waiting from incorrectness:

- ROI ID and authority representation;
- authority revision/generation;
- geometry identity and dimensions;
- cache state and source revision per representation;
- queued/running work kind;
- last job duration and failure reason;
- visible demand per representation;
- whether redraw is required for pending work.

Avoid exposing internal scheduler structures that tests do not need.

## Explicit non-goals

- replacing ECS;
- switching egui or WGPU;
- splitting into three crates during v0 stabilization;
- copying the full `rust-image-viewer` provenance model;
- replacing `geo`, `parry3d`, or `glam`;
- rewriting conversion algorithms solely for architectural purity;
- adding a generic representation graph or task framework;
- supporting geometry mutation or resampling in place;
- introducing a third repository or long-lived parallel implementation.

## Risks and controls

| Risk | Control |
| --- | --- |
| Coordinate convention changes visual orientation | Freeze convention in Phase 0 and retain visual/round-trip tests |
| Temporary duplicate geometry types | Migrate one vertical slice, then delete the old type promptly |
| Coordinator becomes another catch-all | Limit it to ordering; keep conversion math in `src/convert/` and rendering in `src/render/` |
| Demand-driven work hides expected output | Visibility/tool/export demand is explicit and visible through QA |
| Large refactor masks behavior regressions | One runnable phase and focused commit at a time |
| Full affine increases GPU complexity | Convert validated matrices to existing GPU uniforms at the render boundary |

## Implementation status

Current phase: Phase 4 authority-path migration

Completed:

- [x] Existing architecture and runtime flow reviewed
- [x] Compared against `rust-image-viewer`
- [x] Chosen in-place ROI-core replacement over a full reboot
- [x] Defined target coordinate ownership and work-coordinator contracts
- [x] Drafted migration and deletion sequence
- [x] Documented the current NIfTI sform/qform/fallback compatibility convention in ADR 0003
- [x] Added validated affine-based `RoiGeometry`, stable geometry identity, and conversion tests
- [x] New contour ROIs snapshot a reference grid and rebuild without main-volume geometry
- [x] Main-volume changes no longer invalidate contour ROI caches that own reference geometry
- [x] Added `advance_roi_work` as the single render-frame ROI work coordinator
- [x] Pending ROI work now requests another frame independently of GUI repaint timing
- [x] Reject voxel and preview-voxel cache installation when geometry differs from ROI reference
- [x] Mesh-authoritative voxel rebuilds use the ROI cache grid, not the displayed main-volume grid
- [x] Legacy voxel-to-contour and voxel-to-mesh conversions snapshot the source grid and work without a displayed main volume
- [x] Coordinator-level contour commit acceptance coverage reaches current voxel and mesh caches
- [x] Removed uncalled empty-mesh ROI creation API
- [x] ROI reference geometry is mandatory; main-volume contour-cache invalidation compatibility hook removed
- [x] Accepted mesh and preview-mesh caches carry the ROI geometry identity
- [x] Accepted contour-view caches carry the ROI geometry identity
- [x] Production plane factories use a validated constructor that derives normals without changing legacy oblique mapping
- [x] Removed uncalled mesh cache-request compatibility APIs
- [x] Moved uncalled duplicate ROI-creation compatibility APIs out of production builds
- [x] Current-cache checks reject accepted mesh or contour data with a mismatched geometry stamp
- [x] QA exposes authority plus per-cache generation and current-state diagnostics
- [x] ROI job metrics record queue delay for measured Phase 6 optimization work
- [x] Removed duplicate voxel ROI statistics compatibility wrapper
- [x] Affine ROI/display mapping rejects invalid orientations instead of silently using identity
- [x] Moved test-only cache-control compatibility APIs out of production builds

Pending:

- [x] Accept plan and begin implementation
- [x] Phase 0: add end-to-end contour-to-mesh acceptance coverage
- [x] Phase 0: expose ROI geometry identity and dimensions through QA
- [x] Phase 1: attach validated geometry to every ROI and import path
- [ ] Phase 2: carry geometry identity on every cache and migrate validated plane frames
- [x] Phase 3: move remaining direct processor callers behind the coordinator
- [ ] Phase 4: attach geometry to remaining legacy mesh/import creation paths
- [ ] Phase 5: transitional deletion
- [ ] Phase 6: measured optimization

Plan-relevant commits: `d7b4704`, `a5eed49`, `876d103`, `2ed81e0`, `cd905ac`, `88857f0`, `694503a`, `3193068`, `e7e236e`.

## Stable-v0 boundary

Stable v0 requires Phases 0 through 5 plus performance work necessary to meet the existing interaction budget. Phase 6 optimizations that are not supported by measurements are deferred.

The completion condition is one mechanically reliable loop:

```text
explicit ROI geometry
-> validated authority edit
-> demanded derived work
-> revision/geometry-checked cache installation
-> GPU mirror update
-> visible latest result
-> empty pending queue
```
