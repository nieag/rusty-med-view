# Architecture Review

Date: 2026-08-30

## Executive summary

The architecture is fundamentally sound. The important domain boundaries are already in the right places:

- each ROI has one authoritative representation;
- contours, voxels, and meshes are derived caches of the same ROI identity;
- conversion code is separated from rendering;
- WGPU owns viewer-scene rendering while egui owns application UI;
- generation and revision checks protect asynchronous cache installation;
- ROI spatial metadata is explicit instead of inherited from whichever volume happens to be active.

The main weakness is unfinished consolidation. New focused ROI modules were added, but much of the old `roi_runtime` facade, catch-all component storage, broad exports, and historical documentation remain. The result is more surface area than the application needs and frame-order behavior that is harder to reason about than the underlying domain model.

For a stable v0, preserve the current model and remove transitional layers. Do not replace the ECS, introduce a generic representation graph, or build a general job framework.

## Current structure

```text
input / GUI
    -> app events and editor state
    -> contour authoring systems
    -> authoritative ROI mutation
    -> dirty bounds + generation/revision update
    -> ROI work scheduling
    -> contour/voxel/mesh conversion
    -> validated derived-cache installation
    -> retained WGPU resources
    -> viewport rendering
```

The principal module boundaries are:

| Area | Current responsibility | Assessment |
| --- | --- | --- |
| `src/app/roi/` | authority, cache state transitions, history, preview, requests, scheduler | Good boundary; should become the clear ROI API |
| `src/app/roi_runtime.rs` | compatibility facade, conversion execution, imports, GPU synchronization, tests | Transitional catch-all; largest cleanup target |
| `src/app/components.rs` | ECS components plus much of the ROI runtime state | Too broad; hides ownership of ROI concepts |
| `src/systems/` | interaction, contour editing, picking, render preparation | Appropriate, though contour editing is large |
| `src/convert/` | pure coordinate, rasterization, extraction, and meshing algorithms | Strong and testable |
| `src/render/` | native WGPU pipelines, resources, and frame rendering | Strong boundary; a few duplicated helpers remain |
| `src/app/mod.rs` | application lifecycle, event loop, QA data, presets, WASM bridge | Too many unrelated application-level concerns |
| `docs/` | active architecture, plans, and many historical handoffs | Useful source material, but too many documents look current |

## What should remain

### One authoritative representation per ROI

ADRs 0001 and 0003 provide the right model. An ROI is the stable domain object; contour, voxel, and mesh forms do not need separate ROI identities. This avoids synchronization between duplicate objects and gives cache invalidation a clear direction.

### Pure conversion modules

Rasterization, coordinate conversion, voxel contour extraction, and mesh generation belong in `src/convert/`. Keeping these functions independent of ECS and GPU state makes them testable and reusable by both interactive and batch paths.

### Native rendering boundary

ADR 0002 is a good constraint. Medical images, contours, overlays, and meshes should continue through WGPU rather than accumulating egui drawing paths. The renderer-owned viewport clipping and shared coordinate helpers prevent subtle disagreement between display modes.

### ECS for scene composition

The ECS adds value for entities whose components are independently consumed by input, authoring, scheduling, and rendering systems: ROIs, viewports, volumes, and temporary interaction state. It is less useful as the public workflow API. The ROI workflow should remain an explicit, small facade over ECS queries rather than exposing system ordering to callers.

### Revision-checked cache installation

Generation IDs, source revisions, dirty bounds, and stale-result rejection are necessary correctness mechanisms. They should not be simplified away while optimizing the implementation.

## Findings and recommended changes

### 1. Finish removing the `roi_runtime` compatibility layer

`src/app/roi_runtime.rs` remains the largest conceptual hotspot. Focused modules now implement authority, cache, history, preview, requests, and scheduling, but the runtime file still re-exports or delegates part of that API while also executing conversions and synchronizing GPU resources.

Several public functions have no production callers outside this file, including duplicate-ROI creation helpers, mesh translation preview helpers, and mesh-to-cache request helpers. They appear to be remnants of the earlier workflow where representations could become separate ROI entities.

Recommended action:

1. Remove uncalled compatibility wrappers and their wrapper-only tests.
2. Keep tests with the focused module that owns the behavior.
3. Rename the remaining runtime responsibility around what it actually does, such as `roi/executor.rs`.
4. Move GPU resource synchronization only if that produces an obvious ownership boundary; do not create a framework merely to make the file smaller.

Expected result: hundreds of Rust lines can likely be removed, and there will be one obvious place to look for each ROI state transition.

### 2. Make ROI work advancement one explicit operation

The frame pipeline currently invokes several conversion processors and synchronization steps in a required order. The recent mesh-cache trigger failure demonstrated the risk: an intermediate synchronization step could make a pending mesh job look current before it executed.

Recommended action: expose one operation such as `advance_roi_work(world, gpu_context, budget)` that owns the ordering of:

- queued conversion execution;
- stale-result validation;
- cache installation;
- GPU synchronization;
- reporting whether work remains.

This should be a concrete coordinator, not a generic task framework. A regression test should prove that a queued mesh rebuild cannot be suppressed by an earlier synchronization pass.

This also gives the event loop a reliable signal for requesting another frame while work remains, instead of relying on incidental GUI repaint behavior.

### 3. Move ROI runtime state out of the catch-all component module

`src/app/components.rs` contains viewer state, GPU-facing resources, editor state, volume state, ROI cache/job/history/preview types, and the central entity registry. The file is large because unrelated ownership boundaries converge there.

Recommended action: move ROI-specific state beside the ROI behavior, for example:

- authoritative representation and identity in `app/roi/model.rs`;
- derived cache and job state in `app/roi/cache.rs` or a small `app/roi/state.rs`;
- preview and history state in their existing focused modules.

Do this after the compatibility API is removed, so types are moved once rather than carried through another temporary layer. Keep general viewer and render components in `components.rs`.

### 4. Narrow the public API

`lib.rs`, `systems.rs`, and some module roots use broad wildcard re-exports. This makes internal types look supported, obscures dependencies, and allows new code to bypass the focused ROI API.

Recommended action:

- export only the application entry point and intentionally supported loading/test APIs from `lib.rs`;
- remove legacy module aliases when internal imports no longer need them;
- replace wildcard exports with explicit items or direct module paths;
- keep `app::roi` as the intended mutation and request surface.

This is not urgent for runtime stability, but it prevents the cleanup from regressing.

### 5. Separate application lifecycle concerns

`src/app/mod.rs` combines application construction, presets/sample loading, event dispatch, QA snapshot assembly, WASM exports, and the winit application handler.

Recommended action: extract only cohesive sections that already have distinct callers. The first useful candidates are the QA bridge and sample/preset loading. Leave the central `App` lifecycle together unless a split removes real coupling.

Avoid adding application traits, dependency injection containers, or factories. There is one application implementation.

### 6. Remove duplicated and stale small pieces

Small cleanup items with low risk:

- consolidate the two `planes_are_slice_compatible` implementations in `render/contours.rs` and `render/roi_views.rs` into an existing shared plane/orientation module;
- delete stale commented imports and obsolete comments;
- review public helpers in `roi_runtime.rs` with zero non-test callers and delete them rather than preserving speculative API compatibility.

The hand-written orientation math is not an immediate deletion target. It overlaps with `glam`, but its explicit array layout helps maintain CPU/WGSL parity. Replace it only if the shader-parity tests remain equally clear.

### 7. Prune documentation that competes with current truth

The documentation directory contains active plans alongside many historical handoffs and subplans. Git already preserves history; keeping every checkpoint in the primary documentation directory makes it difficult to tell what governs the current implementation.

Keep prominent:

- `docs/current-state.md`;
- `docs/rendering-architecture.md`;
- `docs/adr/`;
- the active ROI closeout and cleanup plans.

Archive or delete completed handoffs and superseded implementation plans. If retained, put them under `docs/archive/` with one index stating that they are historical.

`CLAUDE.md` is materially stale: it describes the contour/SDF/mesh stack as removed and awaiting redesign. Replace it with a short pointer to `AGENTS.md`, `docs/current-state.md`, and the ADRs, or remove it if no active tool requires it.

Most removable lines are documentation, likely several thousand. This is worthwhile because it removes conflicting instructions, not because line count is itself a goal.

## Comparison with `rust-image-viewer`

The sibling `rust-image-viewer` repository is a useful architectural reference, but not a feature-equivalent implementation. It is a clean-room vertical slice whose native contour, mask, and mesh editing tools are still planned. This application has already absorbed the complexity of multiple ROI entities, interactive contour authoring, previews, mesh editing, annotations, QA control, and frame-budgeted derived work.

Snapshot comparison:

| Dimension | This application | `rust-image-viewer` |
| --- | ---: | ---: |
| Rust source | about 26,300 lines | about 10,500 lines |
| Rust files | 53 | 59 |
| Rust tests | about 285 | about 136 |
| Architecture/planning docs | about 8,600 lines | about 800 lines |
| Package shape | one application crate | `viewer-core`, `viewer-engine`, `viewer-app` workspace |
| Runtime state | ECS with multiple independent entities | one `MedicalWorkspace` containing one segmentation |
| Editing maturity | interactive contour and mesh paths exist | editing block is mostly pending |

Line counts are only orientation. The important distinction is that `rust-image-viewer` has fewer implemented product paths, so some of its cleanliness has not yet been tested by equivalent interaction complexity.

### Where `rust-image-viewer` is better

#### Compiler-enforced dependency direction

Its crate graph is explicitly:

```text
viewer-app -> viewer-engine -> viewer-core
      \---------------------> viewer-core
```

`viewer-core` cannot import WGPU, windowing, browser, or UI code. `viewer-engine` cannot import iced. Integration tests inspect the dependency graph and enforce those rules. This is stronger than directory conventions alone.

The equivalent boundaries in this application are conceptually good but easier to bypass because `app`, `convert`, `render`, and `systems` live in one crate with broad exports.

#### One domain aggregate owns representation state transitions

`viewer-core::Segmentation` owns authority, revision, derived records, pending requests, failures, and undo/redo. `WorkspaceMaterializer` resolves the current authority, executes the conversion route, and accepts or rejects the result. Callers request a target; they do not manually assemble a valid sequence of cache mutations.

This is the clearest lesson for this codebase. Each ECS ROI should remain an entity, but its authority/cache/request state should behave as one aggregate behind the `app::roi` API. The frame pipeline should call one coordinator rather than knowing the order of individual conversion and synchronization systems.

#### Invalid state is harder to express

`RepresentationTarget::Contours(PlaneFamily)` combines representation and contour family in one value. Requests, cache keys, conversion plans, and renderer mirrors use the same target type. This avoids separate kind/family fields whose combinations can disagree.

The current app should adopt this idea only where it removes existing parallel fields or conditionals. It does not need the entire sibling type hierarchy.

#### GPU state is explicitly a mirror

The sibling renderer keys uploaded resources to accepted CPU representation records and rebuilds them when the materialization epoch or representation identity changes. Domain authority never lives in the renderer.

This application follows the same principle, but the implementation is spread across ROI runtime processing and render synchronization. Making the accepted cache record the sole input to GPU synchronization would make that invariant easier to inspect.

#### Documentation has one current plan

`rust-image-viewer` has one active OpenSpec change with proposal, design, and task list. It is formal, but there are not dozens of competing handoffs. This app should copy the single-current-plan property, not necessarily the OpenSpec machinery.

### Where this application is better or further along

#### It has exercised the real interaction loop

This application has working authoring state, picking, preview, atomic commit, queued conversion, retained rendering, visibility, multiple ROIs, and QA inspection. The mesh-trigger bug was caused by a real frame-order interaction that the sibling's synchronous diagnostic materializer does not yet face.

The sibling architecture is therefore evidence for cleaner boundaries, not evidence that its current implementation already solves the harder runtime problem.

#### ECS fits the larger scene model

`rust-image-viewer` currently stores one optional workspace with one segmentation. Its direct aggregate model is simpler because it does not yet need independent lifecycles for several ROIs, volumes, viewports, annotations, and tool previews.

Replacing this application's ECS with one large workspace object would lose useful composition. The better hybrid is:

- ECS for entity identity, scene membership, visibility, and independently consumed state;
- a cohesive ROI aggregate/API for authority and derived-work invariants;
- renderer resources as rebuildable mirrors.

#### It uses proven libraries for difficult geometry

The sibling project intentionally owns domain geometry and conversion algorithms. This app uses libraries such as `geo` and `parry3d` where appropriate. There is no v0 benefit in replacing those dependencies with local implementations solely to match the sibling repository.

### Problems visible in `rust-image-viewer`

It is cleaner, but not free of concentration:

- `viewer-engine/src/renderer.rs` is about 1,000 lines and owns synchronization plus several rendering paths;
- `viewer-app/src/app.rs` is about 875 lines and combines event handling, import, diagnostics, materialization requirements, and presentation;
- current materialization is CPU-synchronous, so its waiting/request model has not yet been fully exercised by background work;
- the renderer still performs coarse representation uploads, including cloning complete mask data;
- the provenance model—conversion plans, algorithm IDs, versions, parameter hashes, geometry identities, purposes, and generation tokens—is robust but heavier than this app needs for v0;
- stable IDs for rings, contour vertices, mesh vertices, and triangles add value for direct element editing, but are premature if a path does not use them.

Its architecture may also become less compact once the pending native editing, preview, incremental conversion, and GPU acceleration blocks are implemented.

### What to borrow

1. A single ROI-domain coordinator that owns request, execution, acceptance, and failure ordering.
2. One target value that includes contour plane family and is reused consistently across requests and cache lookup.
3. Architecture tests for dependency direction and rendering-boundary invariants.
4. Accepted CPU cache records as the only authority for rebuilding GPU mirrors.
5. One clearly marked current plan, with historical work removed from the main documentation path.

### What not to borrow for v0

1. A three-crate migration. First clean the boundaries inside the current crate; split only if imports keep crossing them afterward.
2. Iced or the sibling runtime stack. The existing egui/WGPU integration is not the architectural problem.
3. A fully generic conversion-plan/provenance model. Current generation, revision, source identity, and algorithm parameters are sufficient until persistence or interchangeable backends require more.
4. Owned replacements for working geometry dependencies.
5. A single-workspace state object in place of ECS.

### Revised verdict

`rust-image-viewer` is the cleaner architectural sketch; this application is the more valuable product implementation. The goal should not be to merge or rewrite one into the other. Use the sibling's core/materializer boundary as the reference for finishing `app::roi`, while retaining this application's ECS scene model, existing renderer, and proven geometry stack.

## Performance implications

The largest performance costs are not evidence of a bad domain architecture. They are implementation choices behind otherwise appropriate boundaries:

- uploading a full 3D voxel texture after a local contour edit;
- recreating GPU binding resources when only a subregion changed;
- synchronous rasterization or contour extraction on the interaction path;
- generating derived forms that are not currently requested or visible;
- full-volume work for oblique edits.

After the workflow coordinator is explicit, optimization can happen behind it without changing user-facing behavior. Prioritize measurement and incremental uploads before changing the representation model.

## What not to build for v0

- No generic representation dependency graph. Three known representations and explicit transitions are easier to audit.
- No general-purpose async job framework. The current bounded ROI scheduler is enough until measured workloads require more.
- No replacement for the ECS. The problem is API and ownership locality, not entity/component storage itself.
- No second rendering path for previews or contours.
- No duplicate ROI entities for extracted representations.
- No speculative plugin or tool architecture around ROI operations.

## Recommended sequence for a stable v0

1. Lock in the current contour-to-mesh behavior with the regression test and acceptance checks.
2. Add the single ROI work-advancement seam, including reliable redraw-while-pending behavior.
3. Remove dead duplicate-ROI helpers and compatibility wrappers from `roi_runtime`.
4. Re-home remaining ROI state and executor code into the focused `app/roi` boundary.
5. Measure the contour edit path, then optimize full-volume uploads and unnecessary derived work.
6. Narrow exports and prune stale documentation.
7. Split `app/mod.rs` only where the previous work reveals a clear, cohesive boundary.

The first four steps stabilize the architecture without changing the product model. Steps five through seven improve performance and maintainability after the workflow is mechanically reliable.

## Bottom line

The codebase does not need a redesign. Its core decisions are good, but the migration to them is incomplete. The highest-value work is deletion and consolidation: one ROI API, one ordered work coordinator, focused state ownership, fewer public escape hatches, and fewer documents claiming to be current.
