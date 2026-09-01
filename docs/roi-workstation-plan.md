# ROI Workstation Direction

Status: Proposed

## Product outcome

Make the existing viewer feel useful for one complete local workflow:

```text
open an image
-> create or import an ROI
-> author and inspect it in 2D/3D
-> understand its state and measurement
-> export a labelmap result
```

This is a usability/product pass over the accepted ROI core. It does not replace the conversion algorithms, coordinate contract, ECS, or WGPU renderer.

## Target user loop

An operator can load a volume, create or select an ROI, draw/edit a contour, see immediately when its derived views are ready, inspect it across MPR and 3D, read its basic volume, and export the current ROI as a labelmap on its own reference grid.

## Foundation gate

Clear these small, known inconsistencies before presenting further polish as stable product work:

1. **Overlay-cap contract.** The renderer and QA surface support eight geometry-aware voxel overlays, but the sidebar still tells users the old limit of two. Derive the message from the one shared cap and add a regression check for eight accepted overlays and an explicit ninth rejection.
2. **Annotation rendering ownership.** Crosshairs are already rendered natively in `shader.wgsl`, but annotation markers, labels, and hit testing remain in `src/gui/overlays.rs`. Decide and execute one of two explicit scopes: migrate that scene feature to native WGPU plus input systems, or mark annotations provisional and remove them from stable-product claims. Do not add new viewport scene features through egui.
3. **Repeatable baseline.** Write one short manual QA script for image load, label import, contour authoring, derived-cache readiness, and viewport inspection. The existing browser QA remains the contract/timing layer.

These are bounded correctness and ownership fixes. They do not justify an ECS rewrite, typed-coordinate rewrite, generic job system, or project-file format.

## Priorities

| Priority | Deliverable | Why it comes now |
| --- | --- | --- |
| F0 | Foundation gate | Removes known contract and rendering-boundary ambiguity. |
| P0 | Clear active-ROI, tool, readiness, and error feedback | Makes existing functionality understandable. |
| P1 | Practical ROI catalog: select, rename, show/hide, lock, inspect | Makes more than one ROI manageable. |
| P2 | Authoring ergonomics: clear draw/edit lifecycle and viewport feedback | Makes contouring trustworthy without new algorithms. |
| P3 | Measurement and NIfTI labelmap export | Produces a useful result outside the app. |
| P4 | Repeatable end-to-end QA and targeted performance polish | Protects the workflow before feature expansion. |

## Delivery slices

### 1. Make state legible

Expose the selected ROI name, authority, active tool, and derived-work state in one persistent place. Show concise busy, ready, and failure feedback; never leave an edit waiting on an implicit conversion. Preserve the existing QA diagnostics as debug-only support.

Acceptance: after a contour commit, a user can tell which ROI changed, whether it is still processing, and when voxel/mesh views are ready without opening browser QA.

### 2. Make ROIs manageable

Turn the current layer list into a small catalog. Keep selection, visibility, opacity, and current volume; add rename and lock/unlock. Keep operations scoped to the active ROI and make a lock block edits rather than silently changing another ROI.

Acceptance: with ten ROIs, a user can identify, select, rename, hide/show, lock, and inspect any ROI. Rendering limits remain explicit rather than changing the ROI data model.

### 3. Make contour authoring predictable

Polish the existing tools, not their algorithms: visible tool state, pointer/hover feedback, clear loop completion/cancel behavior, concise shortcuts, and a single post-commit result state. Keep world-mm contour ownership and the current CPU conversion pipeline.

Acceptance: a first-time operator can create an axial or oblique contour, close it, edit it, undo/redo it, and see the derived 2D/3D result without asking when meshing occurs.

### 4. Deliver an output

Show basic ROI facts from the current voxel cache: name, authority, readiness, occupied voxel count, and volume in mm³. Export one selected current ROI as a NIfTI labelmap using its owned reference geometry. Refuse export while the voxel cache is stale, with an actionable message.

Acceptance: an exported ROI reloads as a label ROI with matching geometry and occupied-voxel bounds. This is the durable deliverable; project/session persistence is not required for the first pass.

### 5. Close the usability loop

Add a short manual QA script covering load, import/create, author, inspect, measure, and export/reload. Keep the existing browser QA cache/timing assertions. Profile only the interactions that miss the existing latency budget.

Acceptance: the script succeeds on the liver sample in a GPU-capable browser; the contour loop remains within the accepted stable-v0 timings.

## Explicit non-goals

- New contour, meshing, voxelization, or GPU-compute algorithms.
- Registration or automatic resampling across ROI grids.
- DICOM, collaboration, cloud storage, or a generic project format.
- A broad UI redesign or a generic workflow framework.
- Removing ECS or rewriting `roi_runtime.rs` as part of product polish.

## Sequencing rule

Build and test one vertical slice at a time: foundation gate, state feedback, catalog, authoring polish, then measurement/export. Do not begin export or persistence design until the authoring state is understandable in the app.

## Implementation status

Current phase: proposed; no implementation started.

Completed:

- [x] Accepted the stable-v0 ROI core and its measured contour loop.
- [x] Archived prior implementation plans and established current documentation.
- [x] Defined the first product workflow and non-goals.
- [x] Identified the overlay-cap mismatch and annotation rendering-boundary decision.

Pending:

- [ ] Confirm this ROI-workstation outcome as the next active direction.
- [ ] Clear the foundation gate.
- [ ] Implement Slice 1: state feedback.
- [ ] Implement Slice 2: ROI catalog.
- [ ] Implement Slice 3: authoring polish.
- [ ] Implement Slice 4: measurement and export.
- [ ] Run workflow QA and evaluate remaining polish.
