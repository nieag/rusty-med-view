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
- The renderer supports eight simultaneous ROI overlay textures in one pass; additional visible voxel ROIs remain explicitly truncated in runtime and QA state.
- `egui` is GUI only; ROI geometry, contours, mesh surfaces/wireframes, segmentation overlays, and viewport clipping must use native wgpu renderer paths. See [docs/rendering-architecture.md](/Users/nieage/dev/git/rust_starter_app/docs/rendering-architecture.md:1).
- ROI/image spatial alignment must use one central geometry contract: representation-native index/point space -> representation geometry -> patient/world millimetres -> viewport projection. Voxel overlays, contours, meshes, picking, and conversion helpers must not each invent local coordinate paths.

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

## Representation and Conversion Strategy

The representation model is intentionally asymmetric:

- the authoritative representation is the editable source of truth for an ROI
- derived representations are session caches and may be discarded/rebuilt
- conversion jobs must be explicit, observable through ROI runtime state, and never hidden inside render passes
- conversions are allowed to be lossy, but the loss/quality tradeoff must be owned by the conversion step and documented in the handoff for that step

Primary representations:

- `Voxel`: discrete label/occupancy data with ROI-owned grid geometry; primary source for labelmap compatibility and volume computation
- `Contour`: planar closed loops stored in patient/world plane-local millimeters; primary source for slice-based authoring
- `Mesh`: future surface state for 3D visualization and deformation workflows

Derived/session representations:

- voxel cache: rebuilt from contour or mesh primary data when volume, voxel overlay, or labelmap-compatible behavior is needed
- contour cache: rebuilt from voxel or mesh primary data when slice-facing contour inspection/edit initialization is needed
- mesh cache: rebuilt from voxel or contour primary data when 3D surface viewing or mesh-primary transition support is needed
- distance-field cache: optional future SDF/TSDF-style working cache for smoothing, margins, boolean operations, robust mesh regeneration, and mesh deformation support

Initial conversion order:

- `Contour -> Voxel` comes before mesh work because contour editing now exists, but contour ROIs cannot yet produce voxel-derived volume or voxel overlay caches
- `Voxel -> Contour` follows immediately after the first contour-derived voxel loop because loaded voxel labelmaps are the current practical test/import data and need a path into editable contours
- `Voxel -> Mesh` follows once voxel caches are reliable enough to support surface extraction
- `Mesh -> Voxel` follows mesh deformation so edited meshes can return to voxel-derived volume/label behavior
- `Mesh -> Contour` supports slice-facing inspection/editing after mesh deformation
- `Contour -> Mesh` should be deferred until a specific workflow requires it; the first practical route may be indirect through `Contour -> Voxel -> Mesh`

SDF/TSDF position:

- SDF/TSDF should not become the first core authoritative representation
- SDF/TSDF may be introduced later as a derived working cache once mesh deformation, smoothing, margin, boolean, or higher-quality surface reconstruction workflows need it
- any SDF/TSDF cache must be tied to explicit source generation, grid geometry, truncation/threshold semantics, and rebuild invalidation rules

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

### 6.5. Contour-to-Voxel Conversion V1

Purpose:
- close the first practical representation loop by converting contour-authoritative ROIs into voxel-derived caches
- make contour-edited ROIs usable for voxel-derived volume, voxel overlay behavior, and later labelmap-compatible export
- exercise the existing ROI runtime job scaffold with a real conversion algorithm before mesh architecture adds more conversion contracts

Implementation note:
- concrete implementer guidance lives in [docs/subplan-6-5-contour-to-voxel-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-6-5-contour-to-voxel-handoff.md:1)
- keep the first implementation deliberately conservative and testable
- do not introduce mesh, SDF/TSDF, smoothing, interpolation, margins, boolean operations, registration/resampling, or renderer expansion in this phase

Deliver:
- target voxel-grid selection policy for contour-derived voxel caches
  - first option should be explicit and deterministic, such as using the current main volume grid or a stored ROI/reference grid selected at conversion time
  - the selected grid must be stored with the derived voxel cache and must not be inferred later from the currently loaded main volume
- contour loop rasterization for closed loops on matching contour slice planes
- inside/outside fill rules for multiple loops on the same slice, including hole handling or an explicitly documented first-pass limitation
- voxel cache rebuild job path that consumes queued `RebuildVoxelCache` work for contour-primary ROIs
- voxel-derived volume update after successful contour rasterization
- safe failure/status behavior when contour data cannot be rasterized into the selected grid
- tests for geometry mapping, loop fill, cache generation, runtime job completion, and no-render-regression behavior

Acceptance:
- editing a contour ROI can rebuild a voxel-derived cache through the runtime boundary
- contour-derived voxel volume is available after rebuild
- contour ROIs can produce voxel overlay data without making contour data non-authoritative
- conversion behavior is deterministic across native and WASM-compatible execution
- registration/resampling and advanced interpolation remain explicitly deferred

### 6.6. Voxel-to-Contour Extraction V1

Purpose:
- make loaded voxel labelmaps usable as editable contour-primary ROIs
- provide real-data contour test inputs from existing labelmap datasets
- complete the first bidirectional voxel/contour workflow before mesh architecture begins

Implementation note:
- concrete implementer guidance lives in [docs/subplan-6-6-voxel-to-contour-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-6-6-voxel-to-contour-handoff.md:1)
- keep the first implementation deliberately conservative and slice-based
- do not introduce mesh, SDF/TSDF, smoothing, interpolation, boolean operations, registration/resampling, or import/export in this phase

Deliver:
- extraction from voxel-primary ROI data into contour slices for a selected `PlaneFamily`
- deterministic 2D boundary extraction on voxel slices
- contour loop construction in `PlaneLocalMm` using the shared geometry/plane APIs
- explicit handling of multiple disconnected components and holes, or a documented V1 limitation
- UI/runtime action for converting or initializing an editable contour ROI from a loaded voxel labelmap
  - this action should live directly in the label/ROI workflow so loaded labelmaps become the primary real-data test path for `6.6`
- tests using synthetic voxel labelmaps so behavior does not depend on external datasets

Acceptance:
- a loaded voxel labelmap can initialize contour-authoritative data for editing
- extracted contours preserve spatial placement through ROI-owned voxel geometry and shared transforms
- voxel authoritative data is not mutated by extraction
- unsupported extraction cases fail safely with status/log messaging
- contour editing still works on extracted loops

### 6.7. Post-6.6 Consolidation

Purpose:
- verify that the first bidirectional voxel/contour workflow is coherent on representative data before mesh expansion begins
- close obvious workflow, status, and responsiveness gaps exposed by real labelmap-driven editing

Implementation note:
- concrete implementer guidance lives in [docs/subplan-6-7-post-6-6-consolidation-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-6-7-post-6-6-consolidation-handoff.md:1)

Deliver:
- manual verification pass for `label load -> contour extraction -> contour edit -> contour-derived voxel rebuild`
- clarified ROI workflow/status behavior for source voxel ROI vs extracted contour ROI
- documented findings on contour-derived voxel rebuild responsiveness on extracted data
- targeted fixups for issues that block reliable day-to-day use of the current voxel/contour workflow

Acceptance:
- the voxel/contour roundtrip is usable on real datasets without representation ambiguity
- known limitations are documented explicitly rather than surfacing as silent behavior
- any remaining issues are either fixed or captured as concrete follow-up work before mesh implementation starts

### 7. Mesh Representation Architecture

Purpose:
- define mesh as a representation family with explicit primary-state rules

Implementation note:
- concrete implementer guidance lives in [docs/subplan-7-mesh-architecture-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-7-mesh-architecture-handoff.md:1)

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

Implementation note:
- concrete implementer guidance lives in [docs/subplan-8-mesh-workflow-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-8-mesh-workflow-handoff.md:1)
- the first `Subplan 8` implementation pass intentionally includes the minimal mesh-specific rendering prerequisites needed to make the mesh workflow testable before full deformation tooling begins

Deliver:
- mesh-primary editing session model
- slice-based deformation controls
- commit/update rules for mesh edits
- derived voxel and contour rebuild behavior after deformation

Acceptance:
- user can deform an ROI from 2D slice interactions
- resulting state is coherent in MPR and 3D views
- volume remains voxel-derived

### 8.1 Spatial Geometry Contract

Purpose:
- close the voxel-overlay/mesh-placement gap before deformation work continues
- make ROI-native geometry, patient/world space, and viewport projection a single tested contract

Implementation note:
- concrete implementer guidance lives in [docs/subplan-8-1-spatial-geometry-contract-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-8-1-spatial-geometry-contract-handoff.md:1)
- this checkpoint supersedes the nearest-neighbor display-grid resampling direction from the first Subplan 8 voxel display geometry fixup

Deliver:
- central ROI/image geometry mapping helpers in `src/convert/`
- geometry-aware voxel overlay sampling instead of assuming label texture coordinates equal main image texture coordinates
- mesh extraction that remains native to the source `VoxelData.geometry` and emits world-millimetre vertices
- geometry-aware voxel/mesh consistency checks that operate through world-space mapping rather than display-grid assumptions

Acceptance:
- voxel overlay, contour projection, and mesh projection agree through the same ROI-native-to-world-to-viewport contract
- geometry-mismatched labels are either displayed/converted correctly through world geometry or fail clearly
- no empty or displaced mesh ROI is created from non-empty voxel data

### 9. Rendering Integration Layer

Purpose:
- keep rendering representation-agnostic

Implementation note:
- concrete implementer guidance lives in [docs/subplan-9-rendering-integration-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-9-rendering-integration-handoff.md:1)

Deliver:
- ROI render-view adapters for voxel, contour, and mesh
- viewport-side representation requests
- placeholder/loading behavior while caches rebuild
- explicit multi-overlay compositing strategy or explicit runtime/UI cap while the renderer remains limited

Acceptance:
- render code only draws prepared view data
- conversion logic does not live inside render passes
- overlay-count behavior is explicit rather than silently truncating ROIs

### 9.1 Representation Orchestration and Contour View Caches

Purpose:
- tie the primary-representation ROI model, derived session caches, viewport render requests, and QA facts into one runtime contract

Implementation note:
- concrete implementer guidance lives in [docs/subplan-9-1-representation-orchestration-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-9-1-representation-orchestration-handoff.md:1)

Deliver:
- contour view-cache model for read-only per-view/per-plane contour display
- viewport representation request policy for voxel overlays, contour views, and 3D mesh display
- contour view promotion flow so editing another view makes that plane family authoritative
- explicit current/stale/rebuilding/blocked/unsupported cache states
- QA state/presets that prove representation requests and cache states without screenshots

Acceptance:
- each viewport can report which ROI representation it requests and why
- contour-primary ROIs keep one editable authoritative plane family while supporting derived contour views
- editing a derived contour view promotes it through an explicit runtime flow rather than creating multiple authoritative contour families
- contour edits invalidate/rebuild voxel, contour view, and mesh caches in a documented order
- 3D ROI display requests mesh cache/surface rather than contour-polyline hacks

### 9.2 Voxel/Contour Functional Closeout

Purpose:
- make voxel-primary and contour-primary workflows usable end-to-end before performance/GPU/chunking work begins

Implementation note:
- concrete implementer guidance lives in [docs/subplan-9-2-voxel-contour-functional-closeout-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-9-2-voxel-contour-functional-closeout-handoff.md:1)

Deliver:
- functional `Voxel -> Contour -> Voxel` workflow on sample data
- orthogonal contour view caches built from current voxel caches
- explicit contour view promotion UI/runtime path
- contour edit stale/rebuild/current state reflected in QA
- active oblique contour editing/display behavior, with explicit blockers for unsupported oblique voxel overlay or derived oblique extraction

Acceptance:
- user can load voxel ROI, extract/edit contour, rebuild voxel cache, and inspect updated overlay/stats
- derived orthogonal contour views are real caches, not state-only placeholders
- editing another contour view promotes it through runtime API
- oblique contour behavior is usable or blocked explicitly
- unsupported paths are visible in QA state and UI status, not silent

### 9.3 Mesh-Primary Functional Closeout

Purpose:
- make mesh-primary ROI workflow functional enough before performance/GPU/chunking work begins

Implementation note:
- concrete implementer guidance lives in [docs/subplan-9-3-mesh-primary-functional-closeout-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-9-3-mesh-primary-functional-closeout-handoff.md:1)

Deliver:
- CPU `Mesh -> Voxel` V1 conversion into target voxel grid
- `Mesh -> Contour` behavior through `Mesh -> Voxel -> Contour`
- runtime mesh-derived voxel rebuild path replacing current `NotImplemented` placeholder
- minimal mesh edit operation proving authoritative mesh mutation and derived-cache rebuild
- QA/manual workflow state for mesh-primary MPR/3D coherence

Acceptance:
- user can create mesh ROI, perform minimal mesh edit, rebuild voxel cache, and inspect MPR/3D state
- mesh-derived contour views route through current voxel cache or report explicit blockers
- invalid/open/unsupported mesh cases fail safely
- no SDF/TSDF, GPU voxelization, or full mesh UX polish is required yet

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

### ROI Multi-Representation Closeout

The original Subplan 9.3 and Subplan 10 scopes are necessary but not sufficient for live bidirectional editing. The canonical closeout sequence is now defined in [docs/roi-multi-representation-closeout-plan.md](/Users/nieage/dev/git/rust_starter_app/docs/roi-multi-representation-closeout-plan.md:1).

It adds:

- preview versus committed consistency semantics
- dependency-aware conversion scheduling and supersession
- mesh-to-voxel correctness plus direct mesh-plane preview intersections
- incremental contour-to-voxel-to-mesh updates
- measurable interactive latency gates
- multi-ROI and oblique rendering completion
- screenshot-assisted browser visual QA

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
9. Contour-to-voxel conversion v1
10. Voxel-to-contour extraction v1
11. Post-6.6 consolidation
12. Mesh representation architecture
13. Mesh deform workflow
14. Rendering integration layer
15. Representation orchestration and contour view caches
16. Voxel/contour functional closeout
17. Mesh-primary functional closeout
18. Performance and cache strategy

Rules:

- `1-5` must land before contour or mesh feature work
- contour editing must not begin before contour architecture exists
- mesh architecture should not begin until the first bidirectional voxel/contour conversion loop exists
- mesh implementation should be preceded by a short consolidation pass on the voxel/contour workflow
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
- contour-to-voxel rasterization maps plane-local contour loops into the selected voxel grid deterministically
- contour-derived voxel cache generation and voxel-derived volume update after rebuild
- invalid or unsupported contour conversion inputs fail safely without corrupting authoritative contour data
- voxel-to-contour extraction produces editable contour loops from synthetic voxel labelmaps
- extracted contour loops roundtrip through shared voxel/world/plane-local geometry without screen-space assumptions

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
- `Subplan 6.7: Post-6.6 Consolidation` complete
- `Subplan 7 Step 7A: Mesh core types and ROI state` complete
- `Subplan 7 Step 7B: Mesh cache slots and invalidation rules` complete
- `Subplan 7 Step 7C: Runtime contracts for mesh-primary ROIs` complete
- `Subplan 7 Step 7D: Explicit conversion contract scaffolding` complete
- `Subplan 7 Step 7E: Closeout` complete
- `Subplan 8 Step 8A: Pure voxel-to-mesh extraction` complete
- `Subplan 8 Step 8B: Runtime mesh ROI creation from existing ROI data` complete
- `Subplan 8 Step 8C/8D/8E` complete
- `Subplan 8 mesh rendering fixup` complete from [docs/subplan-8-mesh-rendering-fixup-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-8-mesh-rendering-fixup-handoff.md:1)
- `Subplan 8 voxel display geometry fixup` reviewed; nearest-neighbor display-grid resampling is not accepted as the final correctness direction
- `Subplan 8.1 Spatial Geometry Contract` complete from [docs/subplan-8-1-spatial-geometry-contract-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-8-1-spatial-geometry-contract-handoff.md:1)
- `Subplan 9: Rendering Integration Layer` complete (render-view adapters wired through runtime/render prep/QA facts; overlay cap explicit and asserted)
- `Subplan 9.1: Representation Orchestration and Contour View Caches` complete from [docs/subplan-9-1-representation-orchestration-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-9-1-representation-orchestration-handoff.md:1)
- `Subplan 9.2: Voxel/Contour Functional Closeout` complete from [docs/subplan-9-2-voxel-contour-functional-closeout-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-9-2-voxel-contour-functional-closeout-handoff.md:1)
- post-9.2 review fixes complete: main-display contour projection, requested-slice extraction, hole-preserving topology, cache/GPU state separation, adapter alignment, visibility, and contour scissoring
- `Subplan 9.3: Mesh-Primary Functional Closeout` implementation handoff written in [docs/subplan-9-3-mesh-primary-functional-closeout-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-9-3-mesh-primary-functional-closeout-handoff.md:1)
- `ROI Multi-Representation Closeout R1-R3` complete
- `ROI Multi-Representation Closeout R4` implementation complete: contour preview raster/cross-plane path, dirty-AABB chunked mesh extraction, four-millisecond resumable execution, and retained changed-chunk GPU uploads
- `ROI Multi-Representation Closeout R5` bidirectional editing core complete: projected 3D vertex selection, radius/strength mesh brush, revisioned preview, direct 2D intersections, WGPU handle, and release commit
- same-ROI authority promotion complete for voxel-derived contour families and current derived meshes; identity, metadata, and compatible caches are retained
- cache-gated reverse promotion complete: contour or mesh authority can return to current voxel authority, then reuse current contour/mesh representations without creating another ROI
- contour interaction fixup complete: egui-consumed pointer motion no longer leaves stale scene coordinates; axial/coronal/sagittal commits update retained voxel slabs and committed mesh extraction is frame-budgeted
- R5 authoritative commit undo/redo complete for contour and mesh edits; previews are excluded, history is bounded, and restores use normal cache invalidation/rebuild
- R6 eight-ROI voxel compositing complete: one shared LUT, per-ROI geometry/opacity uniforms, deterministic active-first ordering, and explicit ninth-overlay truncation
- R6 multi-contour rendering complete: all visible contour ROIs render with stable active-first ordering and per-layer opacity, while editing affordances remain active-only
- R6 oblique spatial/rendering path complete: shared CPU/GPU plane basis, geometry-aware voxel sampling, view-specific contour extraction, exact displayed-view promotion, and multi-plane-safe committed rasterization
- R6 ROI MPR protocol complete: axial, coronal, sagittal, RMB-rotatable oblique, and 3D viewports are simultaneously available in the QA workflow
- oblique dirty-slice edits currently use a correctness-first full-contour voxel rebuild; incremental arbitrary-plane rasterization remains performance work
- next checkpoint: manual oblique alignment acceptance, then representative-volume R7 performance/closeout QA

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
- note: loaded labelmaps preserve ROI-owned geometry and may be visible even when dimensions, spacing, origin, or orientation differ from the main image; registration/resampling belongs to explicit alignment/conversion workflows, not basic label visibility
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
- note: Subplan 5 implementation landed in `8d353db` together with the first Subplan 6 implementation batch; commit history was not rewritten, but this checkpoint was retrospectively reviewed in [docs/phase-1-6-retrospective-review.md](/Users/nieage/dev/git/rust_starter_app/docs/phase-1-6-retrospective-review.md:1)
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
  - add `EditorTool::{ContourSelect, ContourDraw}` while keeping `Navigation` as default
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
- complete `Subplan 6 Step 6G` from [docs/subplan-6-contour-editing-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-6-contour-editing-handoff.md:1):
  - add contour point move operation in contour edit/select mode using selected-point plane-local updates through shared mapping helpers
  - add contour point insertion for selected loops (selected-point segment or nearest segment fallback) with authoritative mutation via `replace_contour_data(...)`
  - add contour point/loop deletion behavior with valid-loop-size handling and selection cleanup when loops are removed
  - route all committed edit operations through runtime contour replacement so `RebuildVoxelCache` queueing remains consistent
  - add UI command buttons for insert/delete actions and preserve navigation behavior outside active edit operations
  - add Step 6G tests for move, insert, delete, loop-removal-at-min-size, and rebuild-queue contract
- complete `Subplan 6 Step 6H` from [docs/subplan-6-contour-editing-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-6-contour-editing-handoff.md:1):
  - finalize Subplan 6 implementation across commits `8d353db`, `1764ee2`, `de35bab`, `a4031eb`, `3b632e4`, and `87be697`
  - rerun required verification commands: `cargo test -q`, `cargo check --target wasm32-unknown-unknown -q`, and `cargo fmt --all`
  - manual regression checks completed:
    - action: load image NIfTI and voxel label NIfTI; expected: both load and existing image/label baseline behavior is preserved; observed: pass
    - action: inspect image plus voxel label in 2D and 3D views; expected: visible overlays remain stable; observed: pass
    - action: create contour ROI, draw a loop, select point/loop, move point, insert point, and delete point/loop on supported 2D plane families; expected: contour editing works without voxel rasterization; observed: pass
    - action: pan/zoom/slice scroll and inspect 3D viewer outside contour edit actions; expected: navigation and 3D behavior remain unchanged; observed: pass
  - deferred limitations documented and accepted for Subplan 6 scope:
    - no contour rasterization or contour-to-voxel conversion algorithm yet
    - no contour interpolation/smoothing/boolean/margin toolset yet
    - no mesh representation/deformation work yet
    - no renderer expansion/import-export/registration-resampling yet
- complete retrospective review and fixup pass from [docs/phase-1-6-retrospective-review.md](/Users/nieage/dev/git/rust_starter_app/docs/phase-1-6-retrospective-review.md:1):
  - confirm Phase 1 is aligned with the plan and requires no Phase 1-specific fixup
  - use a shared renderable voxel overlay view model for visibility slot accounting, render prep, and bind-group ordering
  - preserve label-owned geometry without hiding valid non-identical label grids
  - route oblique annotation/render projection through the shared oblique plane path
  - keep contour drag preview in editor preview state and commit through `replace_contour_data(...)` on release
  - remove stale separate point-move tool naming after unifying contour select/move behavior
- complete `Subplan 6.5 Step 6.5A` from [docs/subplan-6-5-contour-to-voxel-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-6-5-contour-to-voxel-handoff.md:1):
  - add CPU-backed `VoxelCache { data: VoxelData, gpu_resources: Option<GpuVolumeResources> }` in ROI components
  - change `RoiSessionCaches::voxel` to `Option<VoxelCache>`
  - keep voxel-authoritative ROI constructors populating session voxel cache data while preserving render behavior
  - add `Roi` accessors `voxel_cache`, `voxel_gpu_cache`, and keep `renderable_voxel_cache` rendering against current GPU resources only
  - update bind-group update path to target cached GPU resources through the new cache shape
  - add Step 6.5A tests for voxel cache data mirroring and no-GPU renderable behavior gating
  - rerun required verification commands: `cargo fmt --all`, `cargo test -q`, and `cargo check --target wasm32-unknown-unknown -q`
- complete `Subplan 6.5 Step 6.5B` from [docs/subplan-6-5-contour-to-voxel-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-6-5-contour-to-voxel-handoff.md:1):
  - add pure contour raster conversion module at `src/convert/contour_raster.rs`
  - add `rasterize_contours_to_voxel_data(contour, target_geometry) -> Result<VoxelData, ContourRasterizationError>`
  - implement deterministic voxel-center evaluation with plane-distance slab tolerance and even-odd multi-loop fill
  - skip invalid loops (`!is_closed` or fewer than three points) deterministically
  - keep result geometry equal to the requested target voxel grid
  - add Step 6.5B unit tests for simple axial square fill, empty contour result behavior, invalid open-loop skip, even-odd loop behavior, and geometry preservation
  - export the raster API from `src/convert/mod.rs`
  - rerun required verification commands: `cargo fmt --all`, `cargo test -q`, and `cargo check --target wasm32-unknown-unknown -q`
- complete `Subplan 6.5 Step 6.5C` from [docs/subplan-6-5-contour-to-voxel-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-6-5-contour-to-voxel-handoff.md:1):
  - add runtime contour voxel rebuild executor in `src/app/roi_runtime.rs` that consumes queued contour-primary `RebuildVoxelCache` jobs
  - select target grid from `main_volume_voxel_geometry(world)` and fail safely with log + requeue when main geometry is unavailable
  - capture authoritative generation and contour snapshot before conversion, run rasterization via `rasterize_contours_to_voxel_data`, and store derived CPU voxel cache data in `RoiSessionCaches::voxel`
  - discard stale conversion results when authoritative generation changes before commit, leaving voxel cache dirty and requeued
  - mark voxel cache current only for non-stale successful rebuilds
  - run the contour voxel rebuild executor in frame systems to connect runtime queueing to execution without adding async workers
  - add Step 6.5C tests for successful queued rebuild, missing-main-volume failure behavior, stale-generation discard behavior, and running/queued cleanup on success
  - rerun required verification commands: `cargo fmt --all`, `cargo test -q`, and `cargo check --target wasm32-unknown-unknown -q`
- complete `Subplan 6.5 Step 6.5D` from [docs/subplan-6-5-contour-to-voxel-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-6-5-contour-to-voxel-handoff.md:1):
  - add reusable `R8Uint` upload helpers in `src/io/volume.rs` for raw label bytes and `VoxelData`
  - upload contour-derived voxel cache bytes to a 3D texture on successful rebuild and store GPU resources in `VoxelCache.gpu_resources`
  - keep `renderable_voxel_cache()` gated on current voxel cache state and GPU resource presence
  - integrate GPU-aware contour rebuild execution into frame prep and recreate scene bind groups after successful contour voxel rebuilds
  - preserve existing overlay selection/cap behavior by reusing the same overlay/runtime bind-group path
  - add Step 6.5D accessor/cache-state coverage for current-cache-without-GPU renderability rejection
  - rerun required verification commands: `cargo fmt --all`, `cargo test -q`, and `cargo check --target wasm32-unknown-unknown -q`
- complete `Subplan 6.5 Step 6.5E` from [docs/subplan-6-5-contour-to-voxel-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-6-5-contour-to-voxel-handoff.md:1):
  - add `roi_voxel_stats(world, roi_entity)` supporting voxel-primary authoritative data and contour-primary current derived voxel caches
  - keep contour-primary stats returning `None` when no current derived voxel cache exists
  - add runtime status messages for contour voxel rebuild success and failure paths (missing geometry, rasterization failure, missing bind-group, GPU upload failure)
  - preserve contour edit queueing behavior through existing `replace_contour_data(...) -> RebuildVoxelCache` flow
  - keep V1 rebuild processing in the frame/update tick path (synchronous runtime helper; no worker/progress/cancel path added)
  - add Step 6.5E tests for contour-primary stats `None` before rebuild and non-empty derived stats after rebuild, while preserving voxel-primary stats coverage
  - rerun required verification commands: `cargo fmt --all`, `cargo test -q`, and `cargo check --target wasm32-unknown-unknown -q`
- complete `Subplan 6.5 Step 6.5F` from [docs/subplan-6-5-contour-to-voxel-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-6-5-contour-to-voxel-handoff.md:1):
  - confirm Steps `6.5A` through `6.5E` are implemented and validated
  - final verification commands run:
    - `cargo test -q`
    - `cargo check --target wasm32-unknown-unknown -q`
    - `cargo fmt --all`
    - `cargo clippy --all-targets --all-features -- -D warnings`
  - manual regression checklist completed:
    - action: load image NIfTI and voxel label NIfTI; expected: existing image/label behavior preserved; observed: pass
    - action: create contour ROI, draw closed loop, and wait for runtime rebuild; expected: contour-derived voxel overlay appears via existing voxel overlay path; observed: pass
    - action: edit contour points after rebuild; expected: contour remains authoritative/editable and derived voxel overlay/stats update after rebuild; observed: pass
    - action: inspect neighboring slices after single-slice contour draw; expected: V1 slab-tolerance can include adjacent-slice voxels; observed: pass (multi-slice thickness observed, expected by V1 semantics)
    - action: pan/zoom/slice scroll/3D viewer checks; expected: existing navigation and 3D behavior preserved; observed: pass
  - deferred limitations explicitly retained for post-6.5 work:
    - no interpolation between contour slices
    - no partial-volume calculation
    - no smoothing/margins/booleans
    - no mesh/SDF/TSDF integration
    - no registration/resampling
    - no import/export
- complete post-review fixes for Subplan 6.5 runtime safety:
  - invalidate contour-derived voxel caches when main volume geometry changes and queue contour voxel rebuilds against the new reference grid
  - stop automatic per-frame hard-failure requeue loops for missing-main-volume/rasterization/upload failures; keep retry behavior event-driven
  - retain stale-generation requeue behavior for superseded rebuild results
  - add tests for main-volume-change invalidation and hard-failure no-requeue behavior
  - rerun verification commands: `cargo fmt --all`, `cargo test -q`, `cargo check --target wasm32-unknown-unknown -q`, and `cargo clippy --all-targets --all-features -- -D warnings`
- complete `Subplan 9: Rendering Integration Layer` from [docs/subplan-9-rendering-integration-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-9-rendering-integration-handoff.md:1):
  - add read-only adapter module `src/render/roi_views.rs` with voxel/contour/mesh candidate views, overlay-cap report, and structured skip reasons
  - route voxel overlay slot selection/count through adapter-backed `roi_runtime::renderable_voxel_overlay_rois`
  - route contour/mesh prep entry through adapter candidates while preserving limits (active-contour-only, mesh in 3D path)
  - route QA snapshot renderability/blockers through adapter facts/reasons where practical
  - retain current two-overlay cap and expose cap facts through `overlay_slots_used`/`overlay_slots_max`
  - add QA-3 assertion `overlay_slots_used <= overlay_slots_max`
  - validation run:
    - `cargo fmt --all`
    - `cargo test -q`
    - `cargo check --target wasm32-unknown-unknown -q`
    - `cargo clippy --all-targets --all-features -- -D warnings`
    - `NO_COLOR=true trunk serve` (outside sandbox due local bind/browser requirements)
    - `npx playwright test tests/qa1_viewerqa.spec.js --reporter=line` (3 passed)
- complete `Subplan 6.6 Step 6.6A` from [docs/subplan-6-6-voxel-to-contour-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-6-6-voxel-to-contour-handoff.md:1):
  - add pure extraction module `src/convert/voxel_contour_extract.rs` with `VoxelContourExtractionError` and
    `extract_contours_from_voxel_data(voxel_data, family) -> Result<ContourData, ...>`
  - keep Step 6.6A boundary pure (no ECS, no runtime wiring, no UI integration)
  - implement deterministic V1 extraction support for axial family and explicit unsupported-family handling for out-of-scope families
  - extract closed contour loops slice-by-slice from voxel occupancy via deterministic connected-component + boundary edge stitching
  - populate contour points in `PlaneLocalMm` using ROI-owned `VoxelData.geometry` transform helpers
  - add Step 6.6A unit tests for empty data, single component, multiple components, active-plane-family propagation, and unsupported family rejection
  - run verification commands: `cargo test -q`, `cargo check --target wasm32-unknown-unknown -q`, and `cargo fmt --all`
- complete `Subplan 6.6 Step 6.6B` from [docs/subplan-6-6-voxel-to-contour-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-6-6-voxel-to-contour-handoff.md:1):
  - expand pure extraction to orthogonal families (`Axial`, `Coronal`, `Sagittal`) while keeping `Oblique` explicitly unsupported for V1
  - add reusable orthogonal-family helpers for mask extraction, family-axis indexing, per-slice plane selection, and family-aware voxel/index vertex mapping
  - preserve deterministic boundary edge stitching and closed-loop construction across all orthogonal families
  - add Step 6.6B tests for axial/coronal/sagittal extraction geometry plus origin/spacing preservation through extracted contour placement
  - run verification commands: `cargo test -q`, `cargo check --target wasm32-unknown-unknown -q`, and `cargo fmt --all`
- complete `Subplan 6.6 Step 6.6C` from [docs/subplan-6-6-voxel-to-contour-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-6-6-voxel-to-contour-handoff.md:1):
  - add runtime helper `create_contour_roi_from_voxel_roi(world, source_roi, family) -> Result<hecs::Entity, VoxelContourCreationError>` in `src/app/roi_runtime.rs`
  - validate source ROI existence and voxel-authoritative type before extraction
  - clone source `VoxelData` and run pure conversion via `extract_contours_from_voxel_data(...)`
  - create a new contour-authoritative ROI from extracted `ContourData` while keeping source voxel ROI unchanged
  - add Step 6.6C tests for unchanged source voxel ROI, new contour-primary ROI creation, extracted contour presence/editability (`ContourData` with loops), invalid source rejection, missing ROI rejection, and extraction-error propagation
  - run verification commands: `cargo test -q`, `cargo check --target wasm32-unknown-unknown -q`, and `cargo fmt --all`
- complete `Subplan 6.6 Step 6.6D` from [docs/subplan-6-6-voxel-to-contour-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-6-6-voxel-to-contour-handoff.md:1):
  - add explicit sidebar action in `src/gui/sidebar.rs` to extract contours from the active voxel ROI without auto-conversion on label load
  - place extraction controls directly in the ROI/layer workflow with explicit orthogonal family selection (`Axial`, `Coronal`, `Sagittal`) for immediate post-label-load use
  - surface clear runtime status messages for:
    - missing active ROI
    - non-voxel active ROI
    - extraction success (new contour ROI created and selected)
    - extraction failure (propagated extraction/runtime error)
  - keep existing label loading behavior unchanged and preserve existing contour editing paths after creation by reusing `create_contour_roi_from_voxel_roi(...)`
  - run verification commands: `cargo test -q`, `cargo check --target wasm32-unknown-unknown -q`, and `cargo fmt --all`
- complete `Subplan 6.6 Step 6.6E` from [docs/subplan-6-6-voxel-to-contour-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-6-6-voxel-to-contour-handoff.md:1):
  - add focused interop regressions in `src/app/roi_runtime.rs` verifying extracted contour ROIs are fully compatible with existing contour edit and `6.5` rebuild flow
  - verify extracted contour ROIs can be edited through `replace_contour_data(...)`
  - verify edits on extracted contour ROIs queue `RoiJobKind::RebuildVoxelCache`
  - verify extracted contour ROIs remain contour-authoritative and valid after one edit + contour-to-voxel rebuild cycle (`process_contour_voxel_rebuild_jobs`)
  - retain source voxel ROI unchanged and keep extraction/edit/rebuild representation boundaries explicit
  - run verification commands: `cargo test -q`, `cargo check --target wasm32-unknown-unknown -q`, and `cargo fmt --all`
- complete `Subplan 6.6 Step 6.6F` from [docs/subplan-6-6-voxel-to-contour-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-6-6-voxel-to-contour-handoff.md:1):
  - verify Step `6.6A` through `6.6E` acceptance with geometry-correct extraction baseline retained (no rollback/rework without new failing regression)
  - complete final verification commands:
    - `cargo test -q`
    - `cargo check --target wasm32-unknown-unknown -q`
    - `cargo fmt --all`
    - `cargo clippy --all-targets --all-features -- -D warnings`
  - manual regression checklist completed:
    - action: load image NIfTI and voxel label NIfTI; expected: baseline image/label behavior preserved; observed: pass
    - action: extract contours from loaded voxel ROI; expected: new contour ROI is created and stays contour-authoritative while source voxel ROI remains unchanged; observed: pass
    - action: scroll through extracted slices; expected: extracted contours remain visible across the represented slice range (no early-slice truncation); observed: pass after slice UV/index mapping unification fixes
    - action: compare extracted contour placement against source voxel label; expected: spatial alignment without half-voxel offset; observed: pass after node-based slice mapping and ROI-geometry render reference fixes
    - action: edit extracted contour ROI (select/move/insert/delete paths); expected: existing contour editing behavior preserved; observed: pass (validated by runtime interop regressions + manual edit checks)
    - action: rebuild extracted contour ROI through 6.5 contour-to-voxel path; expected: `RebuildVoxelCache` queue + rebuild cycle succeeds and ROI remains editable; observed: pass
    - action: pan/zoom/slice scroll/3D view checks; expected: existing navigation and 3D behavior preserved; observed: pass
  - document deferred V1 limitations retained for post-6.6 work:
    - no oblique extraction support
- complete `Subplan 8 Step 8A` from [docs/subplan-8-mesh-workflow-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-8-mesh-workflow-handoff.md:1):
  - add pure voxel-to-mesh extraction module `src/convert/voxel_mesh_extract.rs`
  - add API `extract_mesh_from_voxel_data(voxel_data) -> Result<MeshData, VoxelMeshExtractionError>`
  - implement deterministic binary occupancy surface extraction from voxel boundary faces
  - store extracted `MeshVertex` positions in world/patient coordinates via ROI-owned `VoxelGeometry`
  - keep Step 8A boundary pure (no ECS/runtime/render/UI coupling)
  - add Step 8A tests for empty voxel data, simple occupied shape, ROI geometry-controlled vertex placement, and deterministic output
  - run verification commands: `cargo test -q`, `cargo check --target wasm32-unknown-unknown -q`, and `cargo fmt --all`
- complete `Subplan 8 Step 8B` from [docs/subplan-8-mesh-workflow-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-8-mesh-workflow-handoff.md:1):
  - add runtime helpers `create_mesh_roi_from_voxel_roi(...)` and `create_mesh_roi_from_contour_roi(...)` in `src/app/roi_runtime.rs`
  - keep source ROIs unchanged and spawn new mesh-primary ROI entities with extracted `MeshData`
  - gate contour-source mesh creation on presence of a current contour-derived voxel cache
  - add Step 8B tests for voxel-source success + source unchanged, contour-source success when current voxel cache exists, and missing/unsupported source rejection paths
- complete `Subplan 8 Step 8C` from [docs/subplan-8-mesh-workflow-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-8-mesh-workflow-handoff.md:1):
  - add minimal mesh render-prep module `src/render/meshes.rs` that collects visible mesh ROI world-space wireframe segments
  - keep V1 scope explicit and narrow: render path only used for `ViewMode::ThreeD`
  - keep unsupported view modes as no-op by omission
  - add Step 8C tests for empty payload safety, non-empty mesh render preparation, and stable no-mesh-visible behavior
- complete `Subplan 8 Step 8D` from [docs/subplan-8-mesh-workflow-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-8-mesh-workflow-handoff.md:1):
  - add explicit sidebar action to create a mesh ROI from the active ROI in `src/gui/sidebar.rs`
  - support voxel-primary sources directly and contour-primary sources only when current voxel cache exists
  - add explicit status messaging for missing active ROI, unsupported active ROI, missing contour voxel cache, extraction failure, and success
- complete `Subplan 8 Step 8E` from [docs/subplan-8-mesh-workflow-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-8-mesh-workflow-handoff.md:1):
  - verify mesh can now be extracted from voxel data and created as new mesh-primary ROI entities from voxel or eligible contour sources
  - verify created mesh ROIs are inspectable in 3D viewport via wireframe overlay path
  - retain explicit deferrals:
    - no mesh deformation tools yet
    - no mesh-to-voxel regeneration
    - no mesh-to-contour regeneration
    - no broad rendering abstraction rewrite
    - no SDF/TSDF integration
    - no smoothing/decimation/boolean/margin tooling
    - no import/export
    - no registration/resampling
  - run verification commands: `cargo test -q`, `cargo check --target wasm32-unknown-unknown -q`, and `cargo fmt --all`
- post-review correction for `Subplan 8`:
  - manual verification found that the first mesh render path is not acceptable as a completion baseline
  - required fixup is documented in [docs/subplan-8-mesh-rendering-fixup-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-8-mesh-rendering-fixup-handoff.md:1)
  - deformation work remains blocked until mesh rendering uses native wgpu ownership, projects world-mm mesh vertices through main display volume geometry, clips to viewport bounds, and makes binary all-non-zero extraction semantics explicit or implements real label filtering
- begin `Subplan 6.7: Post-6.6 Consolidation`:
  - review the voxel/contour workflow for post-6.6 usability and readiness-to-mesh concerns
  - improve extracted contour ROI naming so derived contour ROIs carry explicit family context
  - improve extraction success messaging so source voxel ROI vs created contour ROI is explicit in the status text
  - preserve current representation boundaries and runtime behavior while making the workflow easier to interpret
  - rerun developer-side validation commands before requesting a fresh manual verification pass
- complete `Subplan 6.7: Post-6.6 Consolidation`:
  - confirm the updated voxel/contour workflow remains coherent on real data
  - manual verification completed:
    - action: load image NIfTI and voxel label NIfTI; expected: baseline image/label behavior preserved; observed: pass
    - action: extract contours from loaded voxel ROI; expected: the derived contour ROI is created with clear source-vs-derived naming/status messaging; observed: pass
    - action: edit extracted contour ROI and trigger contour-derived voxel rebuild; expected: contour-authoritative editing remains intact and rebuild behavior stays coherent; observed: pass
    - action: inspect normal navigation and viewer behavior; expected: 2D navigation and 3D viewing remain stable; observed: pass
  - developer-side validation rerun successfully:
    - `cargo fmt --all`
    - `cargo test -q`
    - `cargo check --target wasm32-unknown-unknown -q`
    - `cargo clippy --all-targets --all-features -- -D warnings`
  - retain deferred scope boundaries:
    - no mesh implementation yet
    - no oblique extraction expansion
    - no smoothing/simplification/interpolation/boolean operations
    - no registration/resampling
    - no import/export
- complete `Subplan 7 Step 7A` from [docs/subplan-7-mesh-architecture-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-7-mesh-architecture-handoff.md:1):
  - add mesh core model types in ROI components: `MeshVertex`, `MeshFace`, and `MeshData`
  - extend authoritative ROI state to explicit mesh data via `RoiAuthoritativeData::Mesh(MeshData)`
  - add mesh-primary constructor `Roi::new_mesh(...)`
  - add authoritative mesh accessor `Roi::mesh_data(&self)`
  - add focused Step 7A tests for mesh-primary ROI initialization, mesh accessor success, and non-mesh accessor rejection
- complete `Subplan 7 Step 7B` from [docs/subplan-7-mesh-architecture-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-7-mesh-architecture-handoff.md:1):
  - define mesh cache storage shape in ROI session caches as `MeshCache { data: MeshData }`
  - add mesh cache accessors on `Roi` (`mesh_cache`, `mesh_cache_mut`)
  - add mesh-authoritative invalidation helper `mark_mesh_authoritative_changed(...)`
  - enforce mesh-primary invalidation rules: mesh-authoritative edits dirty voxel + contour caches always, and dirty mesh cache only when a mesh derived cache exists
  - add Step 7B tests for mesh-authoritative dirty/current transitions and mesh cache generation/current behavior after mesh-cache rebuild completion
- complete `Subplan 7 Step 7C` from [docs/subplan-7-mesh-architecture-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-7-mesh-architecture-handoff.md:1):
  - add runtime mesh-primary ROI creation helper `create_empty_mesh_roi(...)` with active-ROI selection behavior aligned to contour ROI creation flow
  - add runtime mesh-authoritative mutation helper `replace_mesh_data(...)` with explicit `MissingRoi`/`NotMeshRoi` error contracts
  - route mesh-authoritative mutation through ROI invalidation contracts via `mark_mesh_authoritative_changed(...)` without queueing unimplemented mesh-derived rebuild jobs
  - add Step 7C tests for mesh ROI creation, mutation success, and non-mesh mutation rejection
- complete `Subplan 7 Step 7D` from [docs/subplan-7-mesh-architecture-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-7-mesh-architecture-handoff.md:1):
  - add explicit runtime placeholder contracts for mesh-derived rebuild requests:
    - `request_rebuild_voxel_cache_from_mesh(...)`
    - `request_rebuild_contour_cache_from_mesh(...)`
  - enforce safe unsupported behavior with explicit `MeshDerivedRebuildError::NotImplemented` instead of implicit success
  - keep placeholder request semantics side-effect-free when returning `NotImplemented` (no cache dirtying, no queued jobs)
  - add Step 7D tests for placeholder behavior plus missing/non-mesh rejection
- complete `Subplan 7 Step 7E` from [docs/subplan-7-mesh-architecture-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-7-mesh-architecture-handoff.md:1):
  - rerun validation commands:
    - `cargo test -q`
    - `cargo check --target wasm32-unknown-unknown -q`
    - `cargo fmt --all`
    - `cargo clippy --all-targets --all-features -- -D warnings`
  - confirm architecture-vs-algorithm boundary remains explicit:
    - no mesh generation algorithm implementation
    - no mesh editing/deformation workflow
    - no renderer expansion for mesh visualization paths
    - no SDF/TSDF, smoothing/interpolation, import/export, or registration/resampling work
- complete post-review runtime-contract honesty fixes for Subplan 7:
  - stop queueing unexecutable mesh-derived rebuild jobs from `replace_mesh_data(...)`
  - make mesh rebuild request placeholders (`request_rebuild_voxel_cache_from_mesh`, `request_rebuild_contour_cache_from_mesh`) explicitly side-effect-free on `NotImplemented`
  - align mesh mutation/runtime semantics so mesh edits invalidate derived cache state without advertising executable mesh job processing that does not yet exist
  - add/adjust runtime tests to lock side-effect-free `NotImplemented` behavior and no-queued-job mesh mutation behavior

Completed Subplan 8 Review:
- `Subplan 8` mesh rendering fixup from [docs/subplan-8-mesh-rendering-fixup-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-8-mesh-rendering-fixup-handoff.md:1):
  - add explicit main display geometry helper `main_volume_geometry(world)` and keep compatibility alias `main_volume_voxel_geometry(world)`
  - remove egui mesh drawing path from `src/gui/overlays.rs`
  - add native mesh renderer in `src/render/meshes.rs` with:
    - render prep from mesh ROI faces to GPU-facing vertices
    - projection of mesh world-mm vertices through main display volume geometry
    - per-viewport scissor enforcement in native wgpu pass
    - render execution after volume pass and before egui pass
  - keep mesh rendering limited to `ViewMode::ThreeD`; non-3D viewports emit no mesh batches
  - skip invalid mesh face indices safely
  - make binary extraction semantics explicit in UI text/status as all-non-zero voxel extraction
  - add extraction regressions:
    - single-voxel topology triangle count
    - solid `2x2x2` internal-face suppression
    - invalid raw-data length error
    - all non-zero label values treated as occupied
    - shifted-origin/non-unit-spacing world-bound checks retained
  - add mesh render-prep regressions for missing main volume, non-3D viewports, and invalid face handling
  - previous review blocker was mesh/voxel placement mismatch
  - mismatch is resolved by [docs/subplan-8-1-spatial-geometry-contract-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-8-1-spatial-geometry-contract-handoff.md:1)
  - final Subplan 8/8.1 review completed with no blocking findings before wrapping up the branch

Completed Subplan 8.1 Review:
- complete `Subplan 8.1 Spatial Geometry Contract` from [docs/subplan-8-1-spatial-geometry-contract-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-8-1-spatial-geometry-contract-handoff.md:1):
  - add central index-space/world-space affine helpers in `src/convert/geometry.rs` with roundtrip coverage
  - make voxel overlay sampling geometry-aware by mapping main-volume sample indices into ROI voxel index space per overlay
  - preserve contour and mesh placement via existing ROI-native/world-mm paths while removing the temporary mesh-creation geometry mismatch gate
  - update mesh-source runtime tests to validate mismatched-geometry acceptance under the shared world-space contract
  - fix dynamic uniform-buffer stride after geometry-aware overlay uniforms increased `Uniforms` size
  - fix NIfTI spatial loading to fall back from missing/invalid `sform` to `qform`, so main image and label origins can align when different headers store spatial metadata differently
  - manual verification completed:
    - action: load real image + label NIfTI; expected: label overlay visible and aligned; observed: pass after qform fallback
    - action: inspect browser console during load/render; expected: no uniform buffer write/dynamic-offset WGPU errors; observed: pass after dynamic uniform stride fix
    - action: create mesh from loaded voxel label; expected: mesh overlaps green voxel label in 3D; observed: pass
    - action: draw contours and convert contour-derived voxel cache to mesh; expected: generated mesh remains coherent in viewer; observed: pass
    - action: preserve existing image loading, label loading, contour editing, contour extraction, contour-to-voxel rebuild, 2D navigation, and 3D viewer behavior; expected: no regressions observed during manual checks; observed: pass
