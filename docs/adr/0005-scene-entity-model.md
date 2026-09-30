# Scene Entity Model

Status: Accepted (2026-09-30), with the open questions answered below. Implements backlog items 2b.9 and 3.6 and prepares 3.7.

## Context

The scene holds two kinds of thing, and the code treats them alike:

- **State that exists once:** the editor (active ROI, tool, selection), input, GUI state, the cursor, windowing, the protocol, window settings. Nine of them are entities in the `hecs` world, reached through an `AppEntities` handle list. About 110 lookups of them are fallible, so error enums carry variants (`MissingEditorState`, `MissingCursor`, ...) that cannot occur in a running app, and nearly every function takes `(world, entities)`, which hides what it touches.
- **Things that are many and different:** ROIs, viewports, and annotations. ROIs are one large `Roi` component; annotations are a `Vec<Annotation>` inside one singleton component; overlay markers are a third structure (`OverlayManager`) drawn through a shader array capped at 64 primitives.

The target changes what is needed. One case holds 100 to 200 ROIs plus hundreds of notes, measurements, points of interest, and comments. A case is reviewed by several people, and comments are how they talk to each other, so an annotation must identify its author and time, stay where it was put in the anatomy, and survive being sent to someone else. The owner decided (2026-09-30) that comments follow a 3D point into every view, and that nothing shown may be stale.

An ECS fits the second kind of thing at this scale and does not fit the first. See `docs/foundation-review-2026-09-30.md`, section 5.

## Decision

### 1. Two kinds of state, two homes

- **Singletons become plain typed fields** of one `Scene` struct (`scene.editor`, `scene.input`, `scene.gui`, `scene.cursor`, `scene.windowing`, `scene.protocol`, `scene.window_settings`). A function takes `&mut` or `&` of exactly the parts it needs. The fallible lookups and their impossible error variants disappear. `AppEntities` goes away.
- **Everything that is many stays an entity** in the `hecs` world: ROIs, viewports, image layers, and annotations. Transient in-flight work components on ROI entities (the mesh and contour rebuild work) stay as they are; attaching a component to an entity while a job runs is a fair use of an ECS.

### 2. Entity kinds and components

Each kind is a set of small components. Systems select by component, not by a kind tag.

| Kind | Components |
| --- | --- |
| ROI | `Id`, `RoiMetadata` (name, colour, visibility, lock), `RoiBody` (the authoritative form), `RoiCaches`, `RoiJobs`, `RoiHistory`, `LayerOpacity` |
| Viewport | `Viewport`, `ViewportState` |
| Image layer | `VolumeData`, `GpuVolume`, `MainVolume` marker |
| Annotation (note, point of interest) | `Id`, `Anchor`, `Label`, `Text`, `Colour`, `Visibility`, `Provenance` |
| Measurement | `Id`, `Anchor` (first point), `Measurement { kind, points }`, `Label`, `Visibility`, `Provenance` |
| Comment | `Id`, `Thread(Id of the annotation it belongs to)`, `Text`, `Provenance`, optional `Replaces(Id)` |

`Roi` stops being one struct. The split of its current fields into components follows what is read together: body, caches, jobs, history, metadata. Existing behaviour does not change; the lifecycle tests are the check.

### 3. Identity

Every entity that can be referred to from outside the running app has a stable `Id` (a UUID). An entity handle (`hecs::Entity`) is an in-memory detail and is never stored or sent. `RoiId` (a counter) is replaced by the same UUID. The existing `Annotation::id` already is one.

### 4. Anchors follow the anatomy

An `Anchor` is a point in **world millimetres** (the ROI and image affine's world, not volume UV). World millimetres mean the same thing in every view, for every viewer, and for a differently resampled copy of the same scan; volume UV does not. The current annotation position is volume UV (the overlay shader's space) and is converted on load. A measurement stores its points the same way. A comment has no anchor of its own: it belongs to a thread, and the thread's annotation anchors it.

The renderer projects anchors into each view, so a marker shows in every view whose slice is near it (2D) or at its position (3D). Drawing is batched with no fixed cap.

### 5. Shared review: provenance and append-only comments

Because several people view a case:

- `Provenance { author, created_at }` is on every annotation, measurement, and comment. The author is a display name for now; there are no accounts (decision 1).
- **Comments are append-only.** Posting creates a new comment entity with its own `Id`. Editing creates a new comment with `Replaces(old Id)`; retracting creates a tombstone that replaces it with no text. Nothing is changed in place, so two people's comments on the same thread merge by set union with no conflict, and the history of a discussion is kept.
- Annotation fields (label, text, position) are last-writer-wins per annotation, with `Provenance` updated.
- ROI bodies are **not** merged. A ROI is a whole shape with its own undo history; two people editing the same ROI is a conflict the tool reports instead of resolving (decision 3).

### 6. Undo

ROI edits keep their per-ROI history (ADR 0004). Annotation create, move, and delete use a scene-level command stack cleared on session load. Posting a comment is not undoable (it is immutable, and possibly already shared); retracting it is the undo.

### 7. A case as a file

A saved case (backlog 3.7) is a versioned document of entity records keyed by `Id`, with the large payloads kept out of it: ROI bodies as binary parts (a voxel body as a labelmap, contours as a compact list, a mesh in binary), referenced by `Id` and a content hash. The image is referenced by name and hash, not embedded. Loading rebuilds entities from records; the layering test keeps the format code in a layer that depends only on `model`. How a case travels between people (a file they exchange, or a server) is open (decision 2) and does not change the model above, only where the file is stored.

## Staging

Each stage is a commit series on chunk B's branch and leaves the lifecycle tests, the QA spec, clippy, fmt, and the wasm check green.

1. **Singletons to fields. Done.** The nine are plain fields of `Session` (`src/app/components.rs`; named `Session` rather than `Scene` because the hecs world is the other half of the scene); `AppEntities` and the impossible `Missing*` variants are gone. Functions take `&mut EditorState` or `&Session` instead of `(world, entities)`. `advance_roi_work` takes a `ViewFocus` (active ROI and cursor) instead of reading them from the world.
2. **Split `Roi` into components. Done in a smaller form than first planned.** Measured on the code: `Roi`'s ten fields are touched about 340 times in production code and 280 in tests, and every job processor reads and writes nearly all of them together, so splitting all of them would touch about 600 sites, make each processor borrow every component at once, and change nothing in behaviour or speed. What was done instead: `RoiMetadata` (id, name, colour, visibility, lock) is its own component, because the GUI and the render code read it without the rest; `Roi::new_*` return `(Roi, RoiMetadata)` and `spawn_roi_layer` spawns the usual bundle. Job scheduling already is "work as a component" (the in-flight rebuild state is inserted on the ROI entity and resumed each frame); the shared endings (discard, fail, park the work) moved to `roi_runtime/jobs.rs`, and `roi_runtime.rs` was split by concern (`contour_voxel`, `mesh_voxel`, `voxel_mesh`, `views`, `create`, `jobs`), which also does backlog item 2b.7. `hecs` has no scheduler, so `advance_roi_work` stays the one ordered list of steps per frame.
3. **Annotations as entities** with `Anchor` in world millimetres and `Provenance`; migrate `AnnotationState` and the marker path; remove the 64-primitive cap by batching.
4. **The file split** (backlog 2b.7): `roi_runtime.rs` and `components.rs` by concern, following the boundaries stage 1 and 2 expose.

Nothing in the staging waits on the open questions any more.

## Consequences

- Function signatures state what they read and change; tests build only what they need.
- Annotations, measurements, and comments share systems (draw, list, select, save) instead of each having its own structure.
- Comments cannot lose each other's edits when cases are exchanged, at the price of never deleting one (a tombstone remains).
- The anchor change touches the annotation renderer and the overlay shader path once.
- Measurements and points of interest are new features (3.6); this ADR only fixes their shape.

## Decisions on the open questions (owner, 2026-09-30)

1. **Identity: a mocked display name for now.** There are no accounts and no server yet. The author is a display name held in the scene (`scene.user`), defaulting to a placeholder such as "Reviewer", editable in the interface. `Provenance.author` stores that string, so the model does not change when real identities arrive.
2. **Transport: no server for now.** Cases will open from a server down the line. Until then a case is a local file in the format of section 7. The format keeps every record addressable by `Id` and every large payload a separate part, so a server can later serve or store the same records without a redesign.
3. **Two people editing the same ROI:** the tool reports the conflict by version and lets the person pick a version; it does not merge shapes.

## Not decided here

The measurement kinds beyond distance and angle, the comment thread UI, and live collaboration (several people in one session at once) are out of scope; the record-per-entity model does not prevent them.
