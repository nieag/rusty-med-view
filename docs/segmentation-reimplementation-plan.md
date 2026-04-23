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
- explicit primary-switch requests
- cache generation tracking
- frame-budgeted conversion scheduling

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
- job queue and job lifecycle
- generation counters
- cache lookup and invalidation API
- frame-budgeted progress API
- WASM-safe cooperative execution behavior

Acceptance:
- caches can be dirtied selectively
- rebuilds can be enqueued, progressed, cancelled, and superseded
- conversions do not run inline inside render passes

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

### 4. Plane and Geometry Context

Purpose:
- define the shared plane model required for contour and deformation tools

Deliver:
- `PlaneFamily`
- `PlaneDefinition`
- orthogonal and oblique plane support
- shared mapping utilities between viewport, world, and ROI-local space

Acceptance:
- all future editing workflows use the same plane abstraction
- oblique planes are first-class, not special-case math

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

Acceptance:
- render code only draws prepared view data
- conversion logic does not live inside render passes

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
5. Plane and geometry context
6. Contour representation architecture
7. Contour editing v1
8. Mesh representation architecture
9. Mesh deform workflow
10. Rendering integration layer
11. Performance and cache strategy

Rules:

- `1-4` must land before contour or mesh feature work
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

- WASM-safe incremental job progression
- missing derivative caches trigger rebuild scheduling instead of panics
- large edits remain correct even when rebuilds span multiple frames

## Implementation Status

Current Phase:
- `Subplan 1: ROI Core Model`

Completed:
- `1453511` Baseline: remove legacy segmentation stack
- add canonical segmentation reimplementation plan document

Pending:
- define the ROI core model in code
- migrate current label overlays onto ROI instances
- introduce cache and job-state scaffolding without changing current viewer behavior
