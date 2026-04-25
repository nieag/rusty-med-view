# Segmentation Reimplementation Plan

## Summary

This document is the canonical implementation plan for reintroducing segmentation into the repository after the legacy contour/SDF/mesh stack was removed.

The new design is based on a **primary-shape multi-representation model**:

- `Voxel`: robust volumetric representation used for volume calculation and labelmap compatibility
- `Contour`: editable planar representation used for slice-based authoring, including oblique planes
- `Mesh`: representation used for 3D deformation workflows and 3D visualization

Each ROI has exactly one authoritative representation at a time. Other representations are treated as derived session caches and may be rebuilt on demand.

## Locked Decisions

- Use a `PrimaryRepresentation` model rather than trying to keep all representations equally editable at once.
- Support full MPR parity, including oblique contour editing.
- Keep native and browser behavior aligned; architecture must work with WASM-safe incremental execution.
- Make primary-representation switches explicit in the UI and tool workflow.
- Compute ROI volume from voxelized data, regardless of which representation is primary.
- Keep derivative caches in memory for the session only.
- Keep export out of scope for the first implementation wave.
- Treat mesh editing as an explicit mesh-primary deformation workflow, not a general-purpose mesh editor.
- Allow only one active contour plane family to be editable at a time per ROI.
- Authoritative voxel ROI state must carry its own spatial metadata; later phases must not rely on borrowing geometry from the current main volume by convention.
- The current renderer only supports two simultaneous ROI overlay textures; until that changes, the limit must be explicit in the runtime or UI rather than silently truncating visible ROIs.

## Architecture Direction

The redesign should avoid embedding conversion logic directly into rendering or ad hoc ECS systems.

The implementation should be separated into:

- persistent ROI state
- editor and viewport interaction state
- derived representation caches
- conversion and invalidation jobs
- rendering view models

Core ROI concepts to introduce:

- `RoiId`
- `RoiMetadata`
- `PrimaryRepresentation`
- `RoiAuthoritativeData`
- `RoiSessionCaches`
- `RoiDirtyState`
- `RoiJobState`

Core runtime concepts to introduce:

- `PlaneFamily`
- `PlaneDefinition`
- named coordinate spaces and transform contracts
- explicit primary-switch requests
- cache generation tracking
- frame-budgeted conversion scheduling

The current `util/orientation.rs` module should be treated as the seed of the future shared transform layer, not bypassed by ad hoc math in tools or rendering code.

## Subplans

### 0. Baseline Wrap-Up

Purpose:
- finalize removal of the legacy segmentation stack
- establish the current viewer/overlay application as the clean starting point

Acceptance:
- repository builds and tests cleanly
- old segmentation docs and code are removed
- repo state is documented honestly

### 1. ROI Core Model

Purpose:
- replace the current ad hoc segmentation entity model with a real ROI model

Deliver:
- `Roi` identity and metadata
- authoritative representation enum
- session cache containers
- dirty state and job-state scaffolding
- migration path from current label overlay entities into ROI instances

Acceptance:
- current label overlays can be represented as ROIs
- active ROI selection and visibility still work
- render code does not depend on direct representation internals

### 2. Conversion and Job Runtime

Purpose:
- define how derived representations are invalidated, rebuilt, and tracked

Deliver:
- minimal job queue and job lifecycle scaffold
- generation counters
- cache lookup and invalidation API
- runtime boundary for requesting and observing rebuild work
- deferred design note for future frame-budgeted progress API and WASM-safe cooperative execution behavior

Acceptance:
- caches can be dirtied selectively
- rebuilds can be enqueued and superseded through the runtime boundary
- conversions do not run inline inside render passes
- progress/cancel/frame-budget execution may remain deferred until real conversion work exists

### 3. Voxel Baseline Integration

Purpose:
- move current labelmap behavior onto the new ROI and job architecture

Deliver:
- voxel-authoritative ROI path
- voxel-derived volume computation
- current overlay rendering through ROI view data
- label loading rewritten against the ROI model

Acceptance:
- current viewer behavior is preserved
- overlay rendering works through ROI state
- this becomes the stable base for contour and mesh work

### Pre-Subplan 4 Course Corrections

Purpose:
- correct the remaining architectural assumptions exposed by the first implementation wave before transform and plane work begins

Implementation note:
- the concrete implementation brief for this phase lives in [docs/pre-subplan-4-implementer-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/pre-subplan-4-implementer-handoff.md:1)

Deliver:
- add explicit voxel spatial metadata to authoritative ROI voxel state
  - minimum requirement: dimensions plus spacing and orientation
  - preferred shape: a shared grid/world transform abstraction that can later serve contour and mesh regeneration too
- stop deriving voxel ROI volume and related stats by borrowing geometry from the current main volume entity
- align the plan wording with the current runtime implementation so Subplan 2 is tracked as a scaffold rather than a completed full scheduler
- make the current two-overlay renderer ceiling explicit
  - either cap visible ROI overlays in the UI/runtime for now
  - or pull the multi-overlay compositing decision forward into rendering work before contour features expand ROI usage

Acceptance:
- a voxel-authoritative ROI is self-describing in space
- voxel ROI stats and future voxel conversions use ROI-owned geometry rather than global viewer assumptions
- the plan status accurately reflects the runtime that exists today
- visible ROI overlay behavior is explicit when more than two ROIs are enabled

### 4. Transform, Plane, and Geometry Context

Purpose:
- define the shared transform stack and plane model required for contour and deformation tools

Prerequisite:
- the Pre-Subplan 4 course corrections above are complete so voxel ROI geometry is explicit before transform work depends on it

Implementation note:
- the concrete implementation brief for this phase lives in [docs/subplan-4-transform-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-4-transform-handoff.md:1)

Deliver:
- canonical coordinate-space definitions for:
  - voxel/index space
  - volume UV space
  - world/patient space
  - plane-local 2D space
  - viewport UV space
  - egui screen space
  - render-facing GPU/NDC space
- `PlaneFamily`
- `PlaneDefinition`
- orthogonal and oblique plane support
- shared mapping utilities between viewport, patient/world, plane-local, and ROI-local space
- explicit voxel geometry origin/translation handling instead of orientation-only geometry
- explicit egui/wgpu convention reconciliation rules
- migration of oblique-plane math into the shared transform/orientation layer
- removal of viewport-index-based plane assumptions from new segmentation code

Acceptance:
- all future editing workflows use the same transform and plane abstraction
- oblique planes are first-class, not special-case math
- overlay placement, picking, and render projection use the same shared conversion APIs
- CPU and GPU transform paths have parity tests for orthogonal and oblique views

### 5. Contour Representation Architecture

Purpose:
- define contour-authoritative ROI behavior before editing tools are added

Implementation note:
- concrete implementer guidance lives in [docs/subplan-5-contour-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-5-contour-handoff.md:1)

Deliver:
- contour slice storage
- contour loop representation
- active contour plane-family ownership
- contour-authoritative ROI construction and runtime accessors
- contour-derived voxel rebuild contract and scheduling hooks
- explicit invalidation and rejection rules for plane-family changes that would require missing conversion algorithms

Acceptance:
- contour-authoritative ROIs can exist without authoring tools yet
- orthogonal and oblique contour slices can be stored using the shared `PlaneDefinition` model
- contour mutations and allowed plane-family changes have explicit cache invalidation and rebuild scheduling rules
- no contour editing UI or contour-to-voxel rasterization algorithm is required for this checkpoint

### 6. Contour Editing V1

Purpose:
- implement actual contour authoring on the contour architecture

Implementation note:
- concrete implementer guidance lives in [docs/subplan-6-contour-editing-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-6-contour-editing-handoff.md:1)

Deliver:
- explicit contour edit mode/tool plumbing
- empty contour-authoritative ROI creation
- viewport-to-plane-local editing coordinate helpers
- native read-only contour rendering
- create/select/move/delete workflows
- viewport integration for axial/coronal/sagittal/oblique editing
- explicit switch into contour-primary mode
- contour-to-voxel rebuild scheduling

Acceptance:
- contour editing works in any selected plane family
- oblique editing works through the same model
- repeated switching does not drift state

### 7. Mesh Representation Architecture

Purpose:
- define mesh as a representation family with explicit primary-state rules

Deliver:
- mesh-authoritative ROI state
- mesh cache relationships
- mesh-to-voxel regeneration contract
- mesh-to-contour regeneration contract

Acceptance:
- a mesh-authoritative ROI can remain coherent with derived voxel and contour caches
- mesh mode is explicit and isolated from rendering internals

### 8. Mesh Deform Workflow

Purpose:
- implement slice-facing mesh deformation

Deliver:
- mesh-primary editing session model
- slice-based deformation controls
- commit/update rules for mesh edits
- derived voxel and contour rebuild behavior after deformation

Acceptance:
- user can deform an ROI from 2D slice interactions
- resulting state is coherent in MPR and 3D views
- volume remains voxel-derived

### 9. Rendering Integration Layer

Purpose:
- keep rendering representation-agnostic

Deliver:
- ROI render-view adapters for voxel, contour, and mesh
- viewport-side representation requests
- placeholder/loading behavior while caches rebuild
- explicit multi-overlay compositing strategy or explicit runtime/UI cap while the renderer remains limited

Acceptance:
- render code only draws prepared view data
- conversion logic does not live inside render passes
- overlay-count behavior is explicit rather than silently truncating ROIs

### 10. Performance and Cache Strategy

Purpose:
- make the architecture practical at WASM parity after correctness is established

Deliver:
- selective invalidation
- oblique cache reuse and eviction
- partial rebuild strategies where safe
- coarse rebuild fallback where partial rebuilds are unavailable
- browser-focused frame-budget tuning

Acceptance:
- large edits degrade gracefully instead of blocking the app
- cache growth is bounded
- conversion work remains observable and incremental

## Dependency Order

Required implementation order:

1. Baseline wrap-up
2. ROI core model
3. Conversion and job runtime
4. Voxel baseline integration
5. Pre-Subplan 4 course corrections
6. Plane and geometry context
7. Contour representation architecture
8. Contour editing v1
9. Mesh representation architecture
10. Mesh deform workflow
11. Rendering integration layer
12. Performance and cache strategy

Rules:

- `1-5` must land before contour or mesh feature work
- contour editing must not begin before contour architecture exists
- mesh deformation must not begin before mesh-primary rules exist
- performance work must not drive early architecture choices

## Test Strategy

Foundational tests:

- ROI creation, metadata, visibility, and selection behavior
- explicit primary-switch invariants
- cache invalidation and generation correctness
- job enqueue/progress/cancel/supersede behavior

Voxel baseline tests:

- labelmap load into ROI state
- voxel-derived volume computation
- voxel ROI geometry is owned by the ROI rather than borrowed from the main volume entity
- overlay rendering parity with current viewer behavior

Contour tests:

- contour-authoritative ROI creation and storage
- contour-derived voxel rebuild scheduling without requiring rasterization yet
- orthogonal and oblique contour storage correctness
- plane-family switching rejection when conversion would be required

Mesh tests:

- mesh-primary mode entry and exit
- deformation updates propagate into voxel-derived volume
- contour and 3D views remain generation-consistent after mesh edits

Runtime tests:

- runtime scaffold behavior for cache invalidation, enqueue, begin, and completion
- missing derivative caches trigger rebuild scheduling instead of panics
- frame-budgeted progression remains a pending design item until real conversions exist

Transform and parity tests:

- orthogonal plane roundtrip tests between screen, viewport UV, plane-local, and volume space
- oblique plane roundtrip tests through the shared plane definition APIs
- parity tests between CPU picking, egui overlay placement, and GPU-facing projection inputs
- regression tests for radiological orientation and screen-axis flip behavior

## Implementation Status

Current Phase:
- `Subplan 6: Contour Editing V1`

Completed:
- `1453511` Baseline: remove legacy segmentation stack
- add canonical segmentation reimplementation plan document
- introduce `Roi`, `RoiMetadata`, `RoiId`, `PrimaryRepresentation`, `RoiAuthoritativeData`, `RoiSessionCaches`, `RoiDirtyState`, and `RoiJobState` in code
- migrate current loaded label overlays from `Segmentation` entities to `Roi` entities
- rename editor selection from `active_layer` to `active_roi`
- complete `Subplan 1: ROI Core Model`
- begin `Subplan 2` with ROI cache generation helpers, dirty/current checks, and typed queued/running job state
- route current overlay bind-group rebuild access through ROI runtime helpers instead of open-coding raw voxel-cache access in handlers
- move scene bind-group rebuild orchestration out of load handlers into `app::roi_runtime` as the first explicit ROI runtime/service boundary
- add world-level ROI runtime APIs for cache status, rebuild request, job start, and rebuild completion so later systems can target a runtime boundary instead of entity internals
- complete `Subplan 2: Conversion and Job Runtime` as a minimal runtime scaffold
- begin `Subplan 3` by moving voxel ROI creation into `app::roi_runtime` and adding explicit voxel ROI occupancy/volume stats from authoritative voxel data
- complete `Subplan 3: Voxel Baseline Integration`
- `40416de` Fix: show loaded ROI overlays by default
- `ad37e27` complete the Pre-Subplan 4 course corrections:
  - add explicit voxel geometry to authoritative ROI voxel state
  - stop borrowing voxel ROI stats from the main volume entity
  - preserve label NIfTI geometry directly on ROI import and use the main volume only for validation
  - make the two-overlay renderer limit explicit in runtime/UI behavior
- complete `Subplan 4 Step 4A` from [docs/subplan-4-transform-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-4-transform-handoff.md:1):
  - add `origin` to `VolumeData`, `LoadedVolume`, `LoadedLabel`, and `VoxelGeometry`
  - preserve NIfTI sform translation/origin for image and label loads (with invalid-sform fallback to `[0.0, 0.0, 0.0]`)
  - add shared conversion helpers in `src/convert/geometry.rs` for `VolumeUv <-> VoxelIndex <-> PatientWorldMm`
  - compare label/main origin in ROI import geometry validation
- complete `Subplan 4 Step 4B` from [docs/subplan-4-transform-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-4-transform-handoff.md:1):
  - add `PlaneFamily` and `PlaneDefinition` in shared conversion code
  - add orthogonal and oblique plane constructors with normalized patient/world axes
  - add plane-local/world conversion helpers and Step 4B geometry tests
  - add `SlicePlane <-> PlaneFamily` compatibility adapters without changing current viewport behavior
- complete `Subplan 4 Step 4C` from [docs/subplan-4-transform-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-4-transform-handoff.md:1):
  - add shared `ViewportMapping` plus `viewport_uv_to_volume_uv` and `volume_uv_to_viewport_uv` in shared conversion code
  - route orthogonal picking and annotation dragging through shared viewport/geometry conversion helpers
  - route 2D annotation projection through the same shared viewport/geometry conversion helper path (with legacy fallback)
  - add Step 4C parity tests for orthogonal mapping and egui top-left Y-down behavior
- complete `Subplan 4 Step 4D` from [docs/subplan-4-transform-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-4-transform-handoff.md:1):
  - centralize oblique viewport-to-volume mapping in shared conversion helpers
  - remove private oblique mapping math from `src/systems/picking.rs`
  - route oblique picking through `PlaneDefinition` + `ViewportMapping`
  - add oblique parity tests for identity and non-identity rotations against legacy behavior
- complete `Subplan 4 Step 4E` from [docs/subplan-4-transform-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-4-transform-handoff.md:1):
  - remove `viewport_idx -> SlicePlane` assumptions from shared projection/mapping paths
  - route projection and overlay mapping through `ViewMode -> SlicePlane` adapters in shared conversion flows
  - keep viewport-index handling only at UI/runtime boundary points
  - retain compatibility adapter `SlicePlane::from_viewport` only at the orientation boundary
- define `Subplan 5: Contour Representation Architecture` implementation breakdown in [docs/subplan-5-contour-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-5-contour-handoff.md:1)
- complete `Subplan 5 Step 5A` from [docs/subplan-5-contour-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-5-contour-handoff.md:1):
  - add `ContourPoint`, `ContourLoop`, `ContourSlice`, and `ContourData` in the ROI core model
  - change `RoiAuthoritativeData::Contour` to `RoiAuthoritativeData::Contour(ContourData)`
  - add Step 5A contour data-model tests for plane-family ownership, plane-definition preservation, and closed-loop validation
- complete `Subplan 5 Step 5B` from [docs/subplan-5-contour-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-5-contour-handoff.md:1):
  - add `Roi::new_contour(...)` for contour-authoritative ROI construction
  - add `Roi::contour_data(&self)` accessor for authoritative contour state
  - add Step 5B tests for contour-primary ROI initialization, missing voxel-cache current state, and voxel-ROI accessor rejection
- complete `Subplan 5 Step 5C` from [docs/subplan-5-contour-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-5-contour-handoff.md:1):
  - add `set_active_contour_plane_family(...)` runtime API with `MissingRoi`, `NotContourRoi`, and `RequiresConversion` errors
  - allow switching only for empty contour-authoritative ROI data and no-op unchanged family requests
  - mark authoritative generation and derived cache states dirty on successful switches
  - add Step 5C runtime tests for success, no-op, and rejection paths
- complete `Subplan 5 Step 5D` from [docs/subplan-5-contour-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-5-contour-handoff.md:1):
  - add `replace_contour_data(...)` runtime API with `MissingRoi` and `NotContourRoi` mutation errors
  - add contour-authoritative invalidation helper to dirty derived caches and preserve contour cache cleanliness unless a derived contour cache exists
  - enqueue `RebuildVoxelCache` on contour-authoritative data replacement without requiring contour-to-voxel rasterization yet
  - add Step 5D runtime tests for mutation success, queued rebuild job contract, and rejection paths
- complete `Subplan 5 Step 5E` from [docs/subplan-5-contour-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-5-contour-handoff.md:1):
  - confirm Steps 5A through 5D are implemented and validated (`cargo test -q`, `cargo check --target wasm32-unknown-unknown -q`, `cargo fmt --all`)
  - mark `Subplan 5: Contour Representation Architecture` complete and move the active phase to contour editing
- define `Subplan 6: Contour Editing V1` implementation breakdown in [docs/subplan-6-contour-editing-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-6-contour-editing-handoff.md:1)
- complete `Subplan 6 Step 6A` from [docs/subplan-6-contour-editing-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-6-contour-editing-handoff.md:1):
  - add `EditorTool::{ContourSelect, ContourDraw, ContourPointMove}` while keeping `Navigation` as default
  - add runtime helper `create_empty_contour_roi(...)` that creates contour-primary empty ROI state and sets `EditorState.active_roi`
  - add sidebar controls for creating empty contour ROIs and selecting contour active plane family
  - add toolbar tool-mode controls with invalid-action status messaging for missing/non-contour active ROI
  - keep contour ROIs non-renderable through the voxel overlay path until a contour render path exists
- complete `Subplan 6 Step 6B` from [docs/subplan-6-contour-editing-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-6-contour-editing-handoff.md:1):
  - add shared helpers to convert `ViewportUv <-> PlaneLocalMm` through `PlaneDefinition` and existing volume/world mapping APIs
  - add contour edit viewport resolver that rejects 3D mode, enforces contour/viewport plane-family match, and supports oblique through `PlaneDefinition`
  - add Step 6B tests for orthogonal click roundtrips, viewport-family mismatch rejection, 3D rejection, and oblique plane-definition path roundtrip
- complete `Subplan 6 Step 6C` from [docs/subplan-6-contour-editing-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-6-contour-editing-handoff.md:1):
  - add native contour render module with dedicated contour vertex format and WGPU pipeline ownership
  - add CPU thick-line polyline triangle generation helper for WebGPU-portable contour line rendering
  - add contour renderer vertex-buffer upload path with explicit empty-data no-op behavior
  - add contour render pass ordering after the volume pass and before `Gui::render(...)`
  - keep Step 6C contour render data preparation empty so current voxel/image viewer behavior remains unchanged
  - add Step 6C tests for thick-line triangle stability and empty render payload no-op behavior
- complete `Subplan 6 Step 6D` from [docs/subplan-6-contour-editing-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-6-contour-editing-handoff.md:1):
  - prepare native contour render vertices from active contour ROI slices/loops on matching plane-family 2D viewports
  - project contour `PlaneLocalMm` points through world/volume into viewport UV and screen NDC using shared conversion helpers
  - filter rendered contour slices by displayed-plane tolerance and skip 3D viewports for V1
  - render contour line segments and point markers through the native contour pass while preserving existing voxel/image rendering behavior
  - add Step 6D tests for point-marker geometry, slice-plane tolerance, projection stability, empty safety, and non-empty matching-slice render preparation
- complete `Subplan 6 Step 6E` from [docs/subplan-6-contour-editing-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-6-contour-editing-handoff.md:1):
  - add `ContourDraft` in editor state and route `ContourDraw` left-click interactions to draft point appends using shared viewport/plane mapping helpers
  - implement near-first-point loop closure with minimum three-point validation and authoritative commit via `replace_contour_data(...)`
  - commit closed loops into matching contour slices and preserve existing contour-plane ownership rules
  - clear draft when leaving draw mode or switching active ROI
  - render in-progress draft points/segments through the native contour renderer
  - add Step 6E tests for draft append behavior, closure rejection for fewer than three points, successful loop commit/rebuild queue contract, and draft-clear rule
- complete `Subplan 6 Step 6F` from [docs/subplan-6-contour-editing-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-6-contour-editing-handoff.md:1):
  - add `ContourSelection` editor state with ROI/slice/loop/optional-point identity
  - add deterministic screen-space hit testing helpers for nearest point and nearest loop segment selection
  - route `ContourSelect` left-click interactions through contour-aware selection mapping and reject non-contour/mismatched-family cases
  - clear selection when leaving select mode and when active ROI changes
  - render selected loops/points distinctly in native contour overlay rendering
  - add Step 6F tests for nearest-point selection, threshold behavior, and rejection paths

Pending:
- implement `Subplan 6 Step 6G` from [docs/subplan-6-contour-editing-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-6-contour-editing-handoff.md:1): point move, insert, and delete operations
