# Foundation review, 2026-09-30

A review of the codebase after Phase 2 (automatic representation switching) and the 3D
performance work. It asks whether the foundation is right, in behaviour and in structure,
before the drawing tools (Phase 3) are built on it. The resulting work is Phase 2b of
[roi-workstation-todos.md](roi-workstation-todos.md); this document holds the evidence and the
reasoning that the plan items only summarise.

## 1. Verdict

The ROI model is sound. Every ROI has one authoritative form, the others are derived caches, and
switching between forms is automatic and undoable. Chained operations do not make the shape
drift. What is missing is structure (a layering cycle, one oversized module, a partial type),
completeness (derived forms exist only for the active ROI), and robustness (no GPU error
handling). None of it blocks, but all of it is cheaper to fix before more features land.

## 2. Target the design is checked against

One case holds 100 to 200 ROIs (for example a multi-organ model output) plus notes,
measurements, in-view comments, and points of interest, hundreds of small annotations in all.
The current sample has two ROIs, so several choices that look fine there do not survive this
target (section 6).

## 3. What was verified

The ROI lifecycle tests in `src/app/roi_runtime/tests.rs` chain operations on one ROI and check
the shape after each step, using a 14x14x14 volume with a ball and a small second ball.

| Test | Steps | Result |
|---|---|---|
| `test_shape_survives_conversions_and_undo_across_every_representation` | voxel, axial contours, coronal contours, undo, redo, mesh, undo through every switch | Exact (Dice 1.0) through the contour forms and through undo; the mesh step resamples, so it is close, not exact; the original voxel body is restored exactly |
| `test_edit_updates_every_derived_view_and_undo_restores_the_shape` | erase one slice's loops, check the voxel cache and a derived coronal view, undo | The voxel cache follows the edit, the derived view equals a direct extraction from the edited voxels, undo restores both |
| `test_mesh_edit_then_contour_conversion_keeps_the_edited_shape_and_undo_restores_the_mesh` | convert to mesh, shift the surface one voxel, commit, convert to contours, undo twice | Contours follow the edit (Dice above 0.85 against the shifted source); undo restores the mesh exactly |

These found one bug, now fixed (section 4, R1). They are the regression net for the structural
work below: every item in Phase 2b must leave them passing unchanged.

## 4. Findings

Each finding is in the plan as the item named on its line.

**R1 Slice-scoped commit could leave stale voxels. Fixed (commit 4cdf8ff).**
`replace_contour_data_for_slice` always scheduled a slice-local voxel rebuild. When the commit
removed the slice, the rebuild had no old slab to locate and clear, so the deleted shape stayed
in the voxel cache and in every view derived from it. The editing tools use a full rebuild for
deletion, so the bug was not reachable from the interface; it was a trap in the API. The rule is
now in `authority.rs` (`dirty_region_for_slice_edit` for commits, `dirty_region_for_slice_swap`
for undo and redo).

**R2 The `convert` layer and `app::roi` import each other. Plan: 2b.1.**
Pure geometry and conversion code imports its data types (`MeshData`, `ContourData`,
`VoxelData`, `VoxelGeometry`) from `app::roi::model`, and `app::roi` calls `convert`. `render`
takes 29 imports from `app::components` and 5 from `app::roi_runtime`; `util` and `io` also
import from `app`. Consequences: an algorithm cannot be reused or tested without ECS types, and
layering fixes touch many files. Fix: one dependency-free `model` module, then the order model,
convert, `app::roi`, runtime, systems, render, gui, enforced by a test that scans `use crate::`
lines.

**R3 `PlaneFamily::Oblique` makes authoritative contour data partial. Plan: 2b.2.**
`ContourData::active_plane_family` can name Oblique although only orthogonal families are
editable. Ten `unreachable!` calls (`contour_raster.rs`, `voxel_contour_extract.rs`,
`geometry.rs`) and several `if family == Oblique` guards cope with a state the design forbids.
Fix: an orthogonal-family type for authoritative data; `PlaneFamily` with Oblique stays for
derived per-slice view keys only.

**R4 Only the active ROI gets derived contours and a mesh. Plan: 2b.3.**
`sync_roi_contour_view_caches_for_viewports` and `sync_active_roi_mesh_cache_for_viewports`
create demand for the active ROI and mesh-authority ROIs only. The second label of a multi-label
import shows a voxel overlay and nothing else, in 2D and 3D, which contradicts the constraint
that all views stay consistent. Needs a rule (every visible ROI within a budget, active first).

**R5 Stale derived contour views render after an edit, unmarked. Plan: 2b.6.**
After an edit, views in other planes show the pre-edit contours until the voxel rebuild finishes
(200 ms and more on large volumes). This is stale-while-revalidate and may be the right choice,
but the display can silently disagree with the edit. Decide between "stale but marked (dimmed or
dashed)" and "hidden until current", record it in ADR 0004, pin it with a test.

**R6 No GPU error handling. Plan: 2b.5 (includes 6.1).**
No device-lost or uncaptured-error handler is installed, so one wgpu validation error panics the
wasm module (it happened once during this session, from a browser-only timestamp subtraction).
`request_device` uses default limits, which normally fail on a WebGL2 adapter.

**R7 Memory hot spots. Plan: 4.5 and 2b.10.**
A mesh rebuild clones the whole voxel volume and the chunk set; each switch snapshot of a voxel
body clones the volume (up to 32 history steps); each mesh drag update clones the mesh (the
deform topology is now built once per drag, in 6 ms on the liver).

**R8 `roi_runtime.rs` is 2,075 lines. Plan: 2b.7 (moved from 5.1).**
It mixes the job coordinator, three rebuild pipelines, contour view caches, and label import.
Split after the layering so the split follows real boundaries.

**R9 The docs lag the code. Plan: 2b.8.**
`current-state.md` and `code-map.md` do not describe `app/roi/switch.rs`,
`render/view3d_cache.rs`, `convert/mesh_deform.rs`, or the GPU mesh renderer.

## 5. The ECS

Measured on 2026-09-30: 291 component lookups by entity handle (131 `Roi`, 42 `EditorState`,
32 `InputState`, 16 `Viewport`, 15 `Transform`), 64 queries of which 9 span several components,
16 spawn sites. Nine pieces of state that exist once (`editor`, `input`, `gui_state`, `cursor`,
`volume_windowing`, `annotations`, `overlay`, `protocol`, `window_settings`, in `AppEntities`)
are entities.

What it costs today:

- About 110 lookups of those nine are fallible and force error variants that cannot occur in a
  running app (`MissingEditorState`, `MissingViewportState`, `MissingCursor`, ...), and about 45
  `.ok()` calls swallow the same possibility.
- Nearly every function takes `(world, entities)`, so its signature does not say which state it
  reads or changes, and every test builds a nine-entity world first.
- `Roi` is one 18-field component, so composition is unused; annotations are a `Vec` inside one
  singleton component, so the entity-shaped things are not entities.

What is fair use: stable generational handles for ROIs and viewports, and temporary work
components attached to an ROI while a background job runs.

Assessment: at two ROIs the ECS was overhead. At the target scale (section 2) the scene holds
many small heterogeneous entities, which is what an ECS is for, so the decision is to keep it
and use it properly (plan 2b.9): singletons become plain typed fields, `Roi` splits into small
components, and notes, measurements, points, and comments become entities (plan 3.6). The
speed argument does not apply (a scan over 200 ROIs is cheap); the reasons are clear
signatures, honest types, and systems that select by component. The job-as-component scheduler
is a trial, kept only if it shrinks the coordinator.

## 6. What does not scale to the target, and it is not the ECS

- **Memory.** Each label imports as its own full-volume mask, cache, and GPU texture. 100 ROIs on
  a 512x512x300 scan are 7.9 GB of masks before any GPU copies. The fix is to crop each ROI to
  its bounding box using its own geometry (the affine model already allows a cropped origin).
- **Display.** The voxel overlay has 8 GPU slots (`MAX_VOXEL_OVERLAY_SLOTS`), so at most 8 ROIs
  show as voxels. ROIs beyond the slots need a shared display labelmap or contours.
- **Markers.** Overlay markers go through a shader array capped at 64 primitives
  (`MAX_OVERLAY_PRIMITIVES`), which cannot hold hundreds of notes and points.
- **Interface and scheduling.** The layer list has no search, filter, or virtualisation, and the
  job scheduler has no work budget across many ROIs.

Plan: 2b.10 (memory, display, interface, budget), 3.6 (annotations as entities, no cap), 3.7
(session save and load).

## 7. Open decisions

1. **Derived forms for every visible ROI (2b.3).** Recommendation: yes, active first, with a
   budget.
2. **Stale derived views (2b.6).** Recommendation: keep them visible but marked, so the display
   never flickers and never silently disagrees.
3. **How an in-view comment is anchored (ADR 0005).** Either it belongs to a view position (a
   comment made in the axial view at slice 50 shows there only), or it is a 3D point that follows
   the anatomy into every view. This decides the components of the annotation entities.
4. **Scope of image modalities and the license (6.4, 6.6).** Unchanged from the plan.

## 8. Order

2b.1 (layering) first, then 2b.2 (orthogonal-family type). 2b.9 (scene model and ADR 0005) and
2b.10 (scale) come next; measure 2b.10's memory ceiling early because it shows how hard the
target is. 2b.3 to 2b.6 are independent of each other. 2b.7 follows 2b.1 and 2b.9; 2b.8 is last.
Phase 3 starts after Phase 2b.
