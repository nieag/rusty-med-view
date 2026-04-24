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
- shared mapping utilities between viewport, world, plane-local, and ROI-local space
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

Deliver:
- contour slice storage
- contour loop representation
- active contour plane-family ownership
- contour-derived voxel generation
- contour regeneration rules for non-authoritative planes

Acceptance:
- contour-authoritative ROIs can exist without authoring tools yet
- voxel volume can be regenerated from contour state
- plane-family switching has explicit invalidation rules

### 6. Contour Editing V1

Purpose:
- implement actual contour authoring on the contour architecture

Deliver:
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

- contour-authoritative ROI regeneration to voxels
- orthogonal and oblique contour storage correctness
- plane-family switching without state drift

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
- `Pre-Subplan 4 Course Corrections`

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

Pending:
- add explicit spatial metadata or shared grid/world transform data to authoritative ROI voxel state
- stop deriving voxel ROI stats and future voxel conversions from borrowed main-volume spacing
- make the current two-overlay renderer limit explicit in either runtime/UI behavior or rendering scope
- execute the concrete handoff in `docs/pre-subplan-4-implementer-handoff.md`
- after those corrections, begin strengthening the shared transform/orientation layer before contour or mesh workflows begin
