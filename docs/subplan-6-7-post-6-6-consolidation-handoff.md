# Subplan 6.7 Post-6.6 Consolidation Handoff

This document is the implementation brief for `Subplan 6.7: Post-6.6 Consolidation`.

Use this as the source of truth for the next implementation pass. The goal is to verify and tighten the first bidirectional voxel/contour workflow on real data before mesh architecture begins.

## Scope

This handoff covers:

1. manual verification of the full `label load -> contour extraction -> contour edit -> contour-derived voxel rebuild` workflow
2. targeted fixups for issues that make the current voxel/contour workflow confusing, fragile, or practically unusable
3. explicit documentation of remaining known limitations and follow-up items
4. plan/status updates so the repo has a clear checkpoint before `Subplan 7`

It does not cover:

- mesh representation, mesh generation, or mesh deformation
- SDF/TSDF caches
- contour smoothing, simplification, interpolation, boolean operations, or margin expansion
- oblique extraction support
- registration or resampling
- import/export
- broad renderer expansion beyond issues required to stabilize the current voxel/contour workflow
- speculative performance optimization beyond measured, directly user-visible bottlenecks

## Current State

The repository already has:

- voxel-authoritative ROI loading from labelmaps
- contour-authoritative ROI creation and editing
- `Subplan 6.5` contour-to-voxel rebuilds through runtime/job state
- `Subplan 6.6` voxel-to-contour extraction from loaded voxel ROIs
- ROI-owned geometry for voxel and contour workflows
- explicit UI action to extract contours from the active voxel ROI
- regression coverage for contour extraction placement/orientation and contour rebuild/runtime behavior

This subplan is not for adding a new representation. It is for proving that the current voxel/contour loop is coherent enough to build on.

## Locked Decisions

### 1. Consolidation is a targeted stabilization pass

This phase is not a vague polish bucket.

Work in this phase should be limited to issues uncovered by the current real-data workflow:

- wrong or confusing ROI activation/selection behavior
- unclear or misleading status messaging
- contour extraction or rebuild behavior that is technically correct but operationally awkward
- measurable responsiveness problems in the current contour-derived voxel rebuild path
- obvious regressions in 2D navigation, 3D viewing, or overlay visibility triggered by the new voxel/contour workflow

Do not use this phase to start mesh work early.

### 2. Representation boundaries remain explicit

Consolidation must preserve the current model:

- loaded label ROI remains voxel-authoritative
- extracted ROI remains contour-authoritative
- contour edits rebuild a derived voxel cache through the existing `6.5` path

Do not hide representation switches behind silent auto-conversion or implicit state mutation.

### 3. Manual verification drives this phase

The main purpose of `6.7` is to close the gap between “passes unit tests” and “works coherently on real datasets”.

Every implementation step must include:

- concrete manual actions performed
- expected result
- observed result
- pass/fail
- explicit note of anything not manually verified

### 4. Performance work is scoped to evidence

If the current contour-derived voxel rebuild path feels slow, first measure and localize the cost.

Allowed work:

- add timing/log instrumentation around rebuild and upload steps
- remove obvious unnecessary full refreshes or repeated work
- reuse buffers or narrow dirty work if the change is local and low-risk

Disallowed work in this phase:

- speculative GPU conversion work
- new worker/job systems
- broad async runtime changes
- large cache redesigns that belong in `Subplan 10`

## Desired End State

At the end of `6.7`, the repo should have:

- a verified label-driven voxel/contour workflow that is understandable and usable
- documented limitations for what still is not supported
- any blocking usability or correctness issues fixed
- a clear, credible base for `Subplan 7: Mesh Representation Architecture`

## Implementation Steps

### Step 6.7A: Workflow verification baseline

Goal:
- verify the current end-to-end voxel/contour workflow on representative datasets before making changes.

Tasks:

- run a manual verification pass for:
  - load image NIfTI
  - load voxel label NIfTI
  - extract contour ROI from active voxel ROI
  - inspect extracted contours in axial/coronal/sagittal views
  - edit extracted contour ROI
  - verify contour-derived voxel rebuild after edit
  - inspect 3D view and normal navigation behavior
- capture findings in the implementation summary and in the main plan status if they materially affect the route forward
- if no issues are found, state that explicitly rather than inventing work

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

### Step 6.7B: ROI workflow and status fixups

Goal:
- remove obvious workflow confusion around source voxel ROI vs extracted contour ROI.

Tasks:

- tighten ROI naming, selection, or status behavior if current interactions make it unclear which ROI is being edited
- ensure extraction success/failure messaging is specific and actionable
- ensure contour-authoritative and voxel-authoritative ROI behavior remains explicit in UI/runtime behavior
- add focused tests for any non-trivial runtime/UI logic that changes

Examples of acceptable fixes:

- clearer extraction result naming
- clearer active-ROI selection after extraction
- clearer rebuild/failure/success messaging
- small sidebar/runtime adjustments that make the current workflow easier to reason about

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

### Step 6.7C: Measured responsiveness fixups

Goal:
- address only directly observed responsiveness problems in the current contour-derived voxel rebuild path.

Tasks:

- add minimal instrumentation or logging if needed to identify whether delay is dominated by:
  - contour-to-voxel rasterization
  - CPU buffer allocation/clearing
  - GPU upload
  - repeated cache/bind-group refresh
- make only small, defensible improvements supported by that evidence
- preserve the current architecture and runtime contracts
- add tests for any new deterministic helper logic introduced

Examples of acceptable fixes:

- avoid unnecessary reallocation
- avoid repeated refresh when no material cache change occurred
- narrow obvious full-volume work when the current code already has enough context to do so safely

Examples of out-of-scope changes:

- GPU rasterization
- new async background execution model
- major cache invalidation redesign

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`

### Step 6.7D: Closeout and route-to-mesh checkpoint

Goal:
- finish consolidation with a clear statement of what is solid, what remains deferred, and whether the repo is ready for `Subplan 7`.

Tasks:

- rerun the full validation set
- update `docs/segmentation-reimplementation-plan.md`
- record:
  - what was verified manually
  - what was fixed
  - what remains deferred
  - whether any new blocking issue was discovered
- if there are no blockers, mark `6.7` complete and set the next checkpoint to `Subplan 7`

Required verification:

- `cargo test -q`
- `cargo check --target wasm32-unknown-unknown -q`
- `cargo fmt --all`
- `cargo clippy --all-targets --all-features -- -D warnings`

## Acceptance Checklist

`Subplan 6.7` is complete when all of the following are true:

- the current voxel/contour roundtrip has been manually verified on real data
- the current workflow no longer has unresolved confusion around source voxel ROI vs extracted contour ROI
- any directly observed correctness or usability blockers found during consolidation are fixed or explicitly documented
- any performance discussion in this phase is tied to observed behavior, not guesswork
- the main plan clearly reflects readiness to start `Subplan 7`

## Expected Summary Format

Every implementation summary for this subplan must include:

1. which `6.7` step was completed
2. what changed in practical terms
3. the exact manual verification checklist with expected vs observed behavior
4. which issues remain deferred
5. whether the repo is ready to proceed to `Subplan 7`

## Prompt Shape

Use this prompt shape for lower-tier implementation passes:

```text
Continue the segmentation reimplementation using docs/subplan-6-7-post-6-6-consolidation-handoff.md as the source of truth. Implement only Step 6.7A first. Do not start mesh work, SDF/TSDF, oblique extraction, smoothing/simplification, import/export, registration/resampling, or broad rendering/performance refactors. Preserve current image load, voxel label, contour extraction, contour editing, contour-to-voxel rebuild, 2D navigation, and 3D viewer behavior. Update docs/segmentation-reimplementation-plan.md with the completed checkpoint, run cargo test -q, cargo check --target wasm32-unknown-unknown -q, and cargo fmt --all, and include a concrete manual verification checklist with expected vs observed behavior before summarizing.
```
