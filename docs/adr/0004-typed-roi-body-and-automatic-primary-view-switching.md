# Typed ROI Body and Automatic Primary-View Switching

Status: Proposed. Implements backlog Phase 2 (items 2.1 to 2.5). Review before code.

## Context

ADR 0001 gives every ROI one authoritative representation (voxel, contour, or mesh) and treats the others as derived caches. That model is sound, but it is implemented as runtime checks over one `Roi` struct:

- `Roi` carries authoritative data, three hand-built caches, two preview slots, dirty flags, a job queue, and metrics, and every operation begins by matching on the authority and returning a `NotXRoi`/`MissingX` error.
- Cache freshness (generation, dirty flag, geometry identity, "is current") is re-implemented for voxel, contour views, mesh, and previews.
- Changing which contour plane family is editable needs a manual dropdown or a "make displayed view editable" button, runs synchronously, can fail on cache state, and wipes the ROI's undo history.
- Undo lives in a global stack in `EditorState`, so undo can silently move the active ROI.

Product constraints: voxel is a read-only source (for example a model prediction); users edit contours and meshes; switching between representations must be smooth and automatic; exactly one contour plane family is primary and editable at a time while all other views are derived and consistent; oblique views are derived, per-slice views only; switching without editing is lossless and instant; a switch is an undo step, never a history wipe; a contour tool on a mesh ROI converts it with the loss reported, as one undo step.

Measurements (liver sample, release build, `tests/switch_guard.rs`): extracting every slice of a plane family from a current voxel cache takes about 30 ms; rasterizing contours back to voxels takes 170 to 215 ms; re-deriving is currently lossless (Dice 1.0) and switching away and back restores the original loops exactly.

## Decision

### 1. One freshness rule

Every derived cache records what it was built from and whether it was explicitly invalidated, and answers "is this current?" in one place:

```text
CacheFreshness { dirty: bool, built_from: Revision }
```

The voxel, contour, and mesh caches all use it, replacing the per-cache generation fields and dirty flags; geometry identity is still checked in `is_cache_current`. Each contour view records the `Revision` it was built from. (The original proposal wrapped each value in a `Derived<T>`; the values stay in `session_caches`, see the staging note.)

`Revision` has two parts so that a lossless representation change does not invalidate everything:

- `shape`: bumped when the ROI's shape changes (an edit, an undo, a lossy conversion). Voxel and mesh caches key on it.
- `form`: bumped when only the authoritative representation changes while the shape is preserved (a lossless primary-view switch). Contour views key on both.

A lossless family switch therefore keeps the voxel and mesh caches current: no re-rasterize, no mesh rebuild, no flicker.

### 2. `RoiBody`: authority as a type

```text
enum RoiBody { Voxel(VoxelBody), Contour(ContourBody), Mesh(MeshBody) }
```

Each body owns its authoritative data and its edit preview. Editing code takes the concrete body (`&mut ContourBody`), so wrong-authority errors and `Missing*` variants disappear. `Roi` keeps identity, metadata, geometry, the derived caches, the job queue, and the history. A `VoxelBody` is read-only: it has no edit operations.

**History lives on the `Roi`, per ROI, not inside the body.** Undo and redo act on the active ROI and never change it. It cannot live inside the body because an authority change replaces the body: the history entry for that change must survive the replacement. An entry is either an edit snapshot or, for an authority change, a snapshot of the previous body, so an authority change is one undo step. *Implemented in stage 2a* (`RoiHistory` on `Roi`, 32 steps per ROI, undo and redo act on the active ROI).

### 3. Automatic switching

One entry point, `ensure_editable(roi, target)`, where `target` is a plane family for contours or `Mesh`. Tools call it; the user never sees it.

- **Primary-view switch (contour to contour).** The first edit gesture in a non-primary 2D view makes that view's family primary. If the target family's derived set is current, or the voxel cache is current, the switch is inline (about 30 ms measured). Otherwise a background job re-derives it (about 200 ms plus extraction). The gesture that triggered it is held and replayed when the switch completes; further gestures are ignored until then.
- **Voxel to contour.** The first contour gesture on a voxel ROI extracts contours in the gesture's family. The original voxel data stays as an immutable baseline (revert and compare), at the cost of one extra copy of the mask.
- **Mesh to contour.** Converts through the voxel cache (scheduling the mesh to voxel job first when it is not current), reports the loss, and is one undo step whose snapshot restores the mesh.
- **Undo** of any switch restores the previous body exactly.
- **Failure or staleness.** A switch job carries the revision it started from; if the ROI changed meanwhile the result is discarded. Failures leave the body unchanged and show a message. There is no partial state.
- **Loss report.** Every conversion returns a `ConversionReport` (kind, Dice against the previous form when computable, volume change) shown in the status area. It is trivially perfect for lossless switches.

### 4. Removed

The family dropdown, "Make displayed view editable", `RequiresConversion`, the seven promotion error enums, `clear_roi_edit_history_for_roi` on switch, and voxel-to-authority promotion (voxel stays a derived export form).

## Staging

Each stage is its own commit series and must keep the guard tests (`tests/switch_guard.rs`), the strict QA spec, clippy, fmt, and the wasm check green.

1. **`Revision` and one freshness rule.** Pure refactor of the cache machinery, no behaviour change. *Implemented* as `Revision { shape, form }`, a per-cache `CacheFreshness { dirty, built_from }` held in `RoiDirtyState`, one `is_current` rule, and one invalidation routine driven by a per-cache rule. The cache values stay in `session_caches` rather than being wrapped in a `Derived<T>` container: the freshness rule is the part that was duplicated, and wrapping the values would only have added accessor churn at about 150 sites. Contour views record the full `Revision` they were built from.
2. **`RoiBody`.** Three commits. *2a (done):* per-ROI history; undo acts on the active ROI. *2b (done):* the in-flight edit preview lives on the `Roi` (`Roi::edit_preview`) instead of a single global slot in the editor state, so switching the active ROI cannot leave a preview pointing at the wrong ROI; `end_preview` discards it with the preview caches. *2c:* typed `RoiBody` variants owning their preview. Behaviour unchanged except undo no longer moves the active ROI.
3. **`ensure_editable` and the switch.** Inline path first, then the background path, then mesh and voxel entry.
4. **Delete the promotion API and UI.** Then update `tests/switch_guard.rs` to exercise the new path.

## Consequences

- Wrong-authority states become unrepresentable; roughly the `authority.rs` error enums and `NotXRoi` checks disappear.
- A lossless switch costs about 30 ms and invalidates nothing but the contour views.
- Lossy steps (mesh to contour, future contour simplification) are explicit, reported, and undoable.
- The voxel baseline costs one extra mask per voxel-sourced ROI that becomes contour-primary.
- Contour simplification (backlog 3.3) will make re-derivation lossy; the guard tests then need an explicit Dice bound, and a switch must bump `shape` instead of only `form`.

## Open questions

1. **Per-ROI vs global undo.** This ADR proposes per-ROI, acting on the active ROI. The alternative keeps one global stack in edit order, which can surprise users by changing the active ROI.
2. **The held gesture.** Replaying the first click after a background switch is smoother but adds state; ignoring clicks until the switch finishes is simpler. At measured speeds the wait is at most a few hundred milliseconds.
3. **Keeping the voxel baseline in memory.** It enables revert and compare; dropping it saves a mask per ROI. It can be made optional later.

## Not decided here

Oblique views stay derived, per-slice views. Multi-label import (one ROI per label) is separate and already implemented. Mesh rendering and SDF performance are Phase 4.
