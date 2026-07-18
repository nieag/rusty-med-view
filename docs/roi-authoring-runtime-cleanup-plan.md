# ROI Authoring Runtime Cleanup Plan

## Goal

Restructure the implemented multi-representation ROI runtime into clear ownership boundaries before adding broader segmentation-authoring tools and automatic primary-representation switching.

The cleanup must preserve the working behavior established by the ROI closeout:

- exactly one authoritative representation per ROI
- voxel, contour, and mesh derived session caches
- revisioned interactive previews
- generation-safe conversion jobs
- explicit current, stale, rebuilding, blocked, and unsupported states
- native WGPU rendering for all viewer-scene content
- shared patient/world geometry and viewport mapping

This is a behavior-preserving architecture pass first. New segmentation tools consume the resulting interfaces later.

## Why This Is Needed

The representation model is sound, but most orchestration currently lives in `src/app/roi_runtime.rs`. That module owns request reporting, promotion, authority mutations, history, previews, invalidation, job scheduling, conversion execution, GPU cache installation, and QA-facing state.

This concentration creates three risks:

1. A change for one workflow can disturb unrelated representation transitions.
2. Redundant state can become contradictory when mutations bypass a required update.
3. Future tools may couple directly to caches and conversions instead of requesting an editing capability.

The cleanup should make invalid state difficult to create and make the supported edit lifecycle obvious to code, tests, and future agents.

## Scope

In scope:

- split ROI model and runtime responsibilities into explicit modules
- retain one public ROI application facade during migration
- centralize authoritative mutations and primary promotion
- centralize cache invalidation and result installation
- isolate preview lifecycle from committed authority
- isolate history capture and restore
- isolate job policy from conversion execution
- remove or derive redundant state after characterization coverage exists
- ensure render preparation only consumes prepared views and submits requests
- introduce a representation-transition planning contract for future segmentation tools
- add invariant and state-transition tests
- retain native/WASM parity and frame-budgeted execution

## Non-Goals

- no voxel brush or eraser implementation
- no contour subtract, smoothing, interpolation, or region-growing implementation
- no new mesh sculpt algorithm
- no change to conversion geometry or representation semantics
- no new authoritative representation
- no general task executor framework
- no worker-thread or Web Worker rewrite unless performance acceptance requires it
- no import/export, registration, or resampling work
- no visual redesign beyond replacing direct runtime calls with the new facade

## Locked Invariants

1. `RoiAuthoritativeData` is the only writable segmentation source of truth.
2. Derived caches never silently mutate authority.
3. Authority changes occur only through an explicit command or prepared tool edit.
4. Every derived result identifies its source authority generation.
5. Every preview result also identifies its preview revision.
6. Stale generation or revision results are rejected before cache or GPU installation.
7. Only current derived data may be promoted or edited.
8. Pointer motion updates preview state; pointer release commits authority once.
9. Conversion algorithms remain pure code under `src/convert/` where practical.
10. Rendering never performs unbounded representation conversion.
11. Patient/world millimetres remain the cross-representation spatial contract.
12. Viewer-scene content remains native WGPU, not egui painting.

## Target Module Structure

```text
src/app/roi/
  mod.rs              Public facade and explicit exports
  model.rs            ROI authority, cache, generation, and job value types
  authority.rs        Authoritative mutations and promotion
  cache.rs            Invalidation, freshness, and result installation
  preview.rs          Preview sessions, revisions, commit, and cancel
  history.rs          Undo/redo capture and restore
  requests.rs         Read-only representation readiness and blockers
  scheduler.rs        Job policy, priority, dependencies, and supersession
  executor.rs         Frame-budgeted conversion work and completion
```

`src/app/roi_runtime.rs` remains a compatibility facade during migration. It should shrink stepwise and be removed or reduced to re-exports only after all callers use the new boundary.

Pure algorithms remain in:

```text
src/convert/
  contour_boolean.rs
  contour_raster.rs
  voxel_contour_extract.rs
  voxel_mesh_extract.rs
  mesh_plane_intersect.rs
  mesh_voxelize.rs
  geometry.rs
```

Interaction systems should depend on the ROI facade, not cache internals:

```text
src/systems/
  contour_editing.rs
  mesh_editing.rs
  input.rs
```

Render code may read render views and submit missing-view requests. It must not promote authority or execute conversions.

## Dependency Rules

- `model` depends only on shared geometry/value types.
- `authority` may use `model`, `cache`, and `history` contracts.
- `preview` may request scheduler work but cannot commit authority except through `authority`.
- `scheduler` owns job ordering and supersession, not conversion algorithms.
- `executor` invokes `src/convert/` and returns typed results.
- `cache` is the only module that installs derived results.
- `requests` is read-only and cannot enqueue work.
- GUI and input systems call facade commands and queries only.
- render adapters consume `RoiRenderViews` and prepared caches only.

Avoid a generic representation graph or universal cache abstraction unless it removes demonstrated duplication. Explicit voxel, contour-view, and mesh behavior is preferable when their semantics differ.

## State Simplification Direction

### Primary representation

`primary_representation` duplicates the variant of `RoiAuthoritativeData`. During migration:

1. Add a derived `Roi::primary_representation()` query.
2. Move callers from direct field reads to the query.
3. Remove the stored field after tests prove no serialization/API requirement depends on it.

### Cache status

Current dirty booleans, cache generations, cache presence, contour-view state, and job state jointly describe readiness. Introduce one read-only status calculation per cache kind/view. Do not initially replace storage with a generic wrapper.

Required invariant checks:

- `Current` implies matching authority generation and non-dirty state.
- dirty or missing cache cannot report current.
- preview cache revision matches active ROI preview revision.
- stopped preview has no promotable preview results.
- blocked/unsupported views cannot be promoted.

### Job state

Remove compatibility projections such as duplicated `running` and `running_request`, or `queued` and `pending`, after callers use typed request queries. The scheduler should own the canonical queue.

## Future Segmentation Tool Contract

The cleanup must leave a stable seam for automatic representation preparation without implementing the tools themselves.

Proposed intent shape:

```rust
enum SegmentationToolIntent {
    Navigate,
    VoxelBrush,
    VoxelErase,
    ContourEdit { view: ContourViewKey },
    ContourAdd { view: ContourViewKey },
    ContourSubtract { view: ContourViewKey },
    MeshDeform,
}
```

The runtime resolves an intent into a read-only plan:

```rust
struct RepresentationTransitionPlan {
    required_authority: Option<PrimaryRepresentation>,
    readiness: TransitionReadiness,
    lossiness: TransitionLossiness,
    required_jobs: Vec<RoiJobRequest>,
    source_generation: u64,
}
```

Policy:

- selecting a tool may prepare caches but does not immediately change authority
- the first actual edit performs lazy promotion from the validated plan
- an already-correct authority begins immediately
- a current exact cache may promote automatically when policy allows
- a missing cache queues required work
- stale data never promotes
- materially lossy transitions require confirmation
- viewport navigation alone never changes authority
- selecting and abandoning a tool creates no history entry

The plan must be recomputed or rejected if authority generation changes before the first edit.

## Implementation Phases

### C0: Baseline and Characterization

Deliver:

- finish current Oblique alignment and performance acceptance
- capture current native/WASM validation results
- add black-box tests for contour and mesh preview/commit flows
- add a transition matrix covering authority, invalidation, jobs, and history
- record current interaction and conversion metrics on the liver sample

Exit criteria:

- current behavior has deterministic test coverage at the application facade
- refactoring regressions can be distinguished from existing limitations
- representative performance baseline exists

### C1: Model Extraction

Deliver:

- create `src/app/roi/model.rs`
- move ROI-specific value types out of the general components module
- preserve explicit compatibility imports while callers migrate
- add `Roi::primary_representation()` derived query

Exit criteria:

- no behavior change
- no broad wildcard public re-export
- all existing tests and WASM compilation pass

### C2: Read-Only Requests and Render Boundary

Deliver:

- move representation status/readiness logic into `requests.rs`
- keep `RoiRenderViews` as a read-only adapter
- audit per-frame cache synchronization for hidden heavy work
- make missing-view submission explicit outside render preparation

Exit criteria:

- render preparation performs no unbounded conversion
- request queries have no mutation side effects
- current/stale/preview/blocker reporting remains unchanged

### C3: Authority and History

Deliver:

- move voxel, contour, and mesh authoritative mutations into `authority.rs`
- move promotion into typed authority commands
- move undo/redo ownership into `history.rs`
- define one commit path that increments authority generation exactly once

Exit criteria:

- systems cannot mutate authoritative data directly
- promotion preserves entity identity and compatible current caches
- no-op edits and tool selection create no history entries

### C4: Cache Ownership

Deliver:

- move invalidation and freshness calculations into `cache.rs`
- add typed cache-result installation APIs
- enforce generation/revision validation at installation
- centralize GPU resource attachment behind cache installation outcomes

Exit criteria:

- no external caller independently changes cache data, generation, and dirty flags
- stale and superseded results are rejected in focused tests
- cache state invariants have debug assertions or invariant tests

### C5: Preview Lifecycle

Deliver:

- move begin/update/cancel/commit preview behavior into `preview.rs`
- represent one active ROI edit session explicitly
- keep contour and mesh preview strategies specialized behind the shared lifecycle
- preserve tool-switch and ROI-switch cleanup

Exit criteria:

- previews cannot outlive their authority generation
- commit occurs once on pointer release
- cancel never changes authority or history
- contour and mesh interaction behavior remains visually equivalent

### C6: Scheduler and Executor

Deliver:

- move priority, dependency, coalescing, and supersession into `scheduler.rs`
- move resumable conversion execution into `executor.rs`
- eliminate duplicated compatibility job fields
- keep the four-millisecond frame budget and visible/interactive priority

Exit criteria:

- one canonical pending/running state exists
- superseded preview jobs stop before further conversion or installation
- viewport rotation and slice scrolling cannot grow work unboundedly
- native and WASM use the same observable state contract

### C7: Tool-Intent Transition Seam

Deliver:

- introduce `SegmentationToolIntent`
- implement read-only transition planning
- implement lazy promotion for existing contour-edit and mesh-deform tools
- preserve confirmation for materially lossy transitions
- replace direct GUI conversion/tool sequencing with one facade request

Exit criteria:

- existing tools activate through capability requests
- stale plans fail safely after generation changes
- passive navigation never changes authority
- no new brush/subtract/interpolation algorithms are added in this phase

### C8: Consolidation and Closeout

Deliver:

- remove the compatibility facade and dead state where possible
- remove obsolete imports and duplicate helpers
- update architecture/current-state documents
- rerun native, WASM, lint, and manual performance acceptance

Exit criteria:

- no single ROI runtime module owns unrelated responsibilities
- public ROI mutations are explicit and narrowly exported
- performance is no worse than the C0 baseline
- future segmentation tools can request editing capability without touching caches

## Test Strategy

Each phase requires focused tests plus the full suite.

Required state-transition coverage:

- voxel authority -> contour preparation -> lazy promotion -> contour edit commit
- voxel authority -> mesh preparation -> lazy promotion -> mesh edit commit
- contour edit preview -> superseded preview -> latest result only
- mesh edit preview -> cancel -> unchanged authority
- commit -> generation increment -> derived invalidation -> exact convergence
- undo/redo through normal invalidation and rebuild paths
- stale cache and stale transition-plan rejection
- Oblique exact-view preparation and promotion
- ROI/tool switch cleanup during active preview

Validation commands:

- `cargo fmt --all`
- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo clippy --all-targets --all-features -- -D warnings`
- manual liver-sample contour, mesh, Oblique, undo/redo, and performance checks

## Commit Discipline

Each phase should land as a focused checkpoint with its hash recorded below. Do not mix new segmentation algorithms into cleanup commits. A phase that changes behavior must state the behavior change and update acceptance coverage explicitly.

## Implementation Status

State: planned; implementation not started.

Current checkpoint: complete ROI closeout acceptance and establish the C0 characterization baseline.

Completed prerequisites:

- `90d94ae` R6: implement bidirectional ROI editing closeout
- `4860bf2` Docs: record ROI closeout checkpoint

Pending:

- C0 baseline and characterization
- C1 model extraction
- C2 read-only requests and render boundary
- C3 authority and history
- C4 cache ownership
- C5 preview lifecycle
- C6 scheduler and executor
- C7 tool-intent transition seam
- C8 consolidation and closeout

Implementation checkpoints:

- none
