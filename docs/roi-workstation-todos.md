# ROI Workstation TODOs

Ordered backlog. Complete top-to-bottom unless a measured regression changes the order. Every step leaves `cargo test`, `cargo clippy --all-targets --all-features -D warnings`, `cargo check --target wasm32-unknown-unknown`, and `cargo fmt --check` green, and lands as its own commit.

## Product constraints

These drive the structure below.

- Each representation (voxel, contour, mesh) can be authoritative. Voxel authority is a read-only source, for example a deep-learning prediction. Users edit contours and meshes, not voxels.
- Switching between representations must be smooth and automatic. No manual "promote" step, no visible geometry swap.
- Exactly one contour plane family is primary and editable at a time. All other views are derived and stay consistent with it. The primary family follows the view the user edits in.
- Oblique views are derived, per-slice views only. They never become the primary contour set.
- Switching primary view without editing is lossless and instant. A switch is an undo step, never a history wipe.

## Done

- [x] 1. Dead code: duplicate plane-compatibility check, test-only delegates, deprecated `SlicePlane::from_viewport`.
- [x] 2. Docs: archive pruned to the three documents still cited (`docs-archive-full` tag holds the rest).
- [x] 3. Inline test modules moved to sibling `tests.rs` files.
- [x] 9. QA snapshot and sample bootstrap moved out of `app/mod.rs` into `app/qa/`.

## Next

- [ ] **A. Guard tests** on the liver sample, before structural changes:
  - time to re-derive all slices in a new plane family (decides background job vs. inline within the 100 ms preview / 200 ms hard budget);
  - overlap (Dice) and time across a contour to voxel to contour round trip;
  - exact restore of the original loops after a no-edit switch away and back.
  - Run the six ignored milestone QA tests once as a baseline.
- [ ] **B. Geometry migration.** One immutable IJK-to-world affine as the only ROI geometry type, per `docs/adr/`. Delete `VoxelGeometry` decomposition and `from_legacy_parts`. Reject invalid transforms at import instead of falling back to identity. Decide `f32` vs `f64` for world coordinates.
- [ ] **C. Generic `Derived<T>` cache.** One type for value plus source revision plus geometry identity, replacing the hand-rolled voxel, contour-view, mesh, and preview cache logic. The previous family's derived contours stay cached until an edit invalidates them.
- [ ] **D. `RoiBody` and automatic switching.**
  - `enum RoiBody { Voxel, Contour, Mesh }` with per-authority edit state, preview, and history. Editing code takes the concrete type, so wrong-authority errors disappear.
  - First edit gesture in a non-primary view makes that view's family primary. The clicked view's derived loops start the gesture immediately; the full re-derivation runs as a background job.
  - Voxel and mesh ROIs become contour-primary on the first contour gesture; the voxel source stays as an immutable baseline.
  - Conversions report loss (volume delta or Dice) so it is observable.
  - Delete the family dropdown, "Make displayed view editable", `RequiresConversion`, the promotion error enums, and history clearing on switch.
- [ ] **E. Split large files** by concern once D has shrunk them: `app/roi_runtime.rs` (coordinator, per-representation rebuild, import, view building) and `app/components.rs` (viewport, ROI state, editing).
- [ ] **F. Scope pruning.**
  - Remove voxel-to-authority promotion (`promote_current_voxel_cache_to_authority`, `VoxelAuthorityPromotionError`) if confirmed unneeded; voxel stays a derived export form.
  - Decide what a contour tool does to a mesh-primary ROI (convert with loss, undoable, or refuse).
  - Move the remaining `cfg(test)` wrappers in `roi_runtime.rs` into a test-support module.

## Carried over from the previous backlog

Product work, not yet scheduled relative to A to F:

- Persistent active-ROI status area (name, authority, tool, lock state, derived-work state) and concise processing, ready, and failure messages.
- ROI catalog: select, visibility, opacity, rename, lock/unlock, visible eight-overlay limit.
- Contour authoring polish: hover target, close-loop and cancel guidance, shortcut hints.
- Export a selected ROI as a NIfTI labelmap from its owned reference geometry; refuse stale-cache export; reload and verify.
- Show selected ROI voxel count and volume in mm³.
- Sidebar overlay-cap message derived from the shared renderer cap; regression test for eight accepted and a ninth rejected.
- Decide annotation scope: native WGPU or provisional.

## Later, only with evidence

- Profile representative large and multi-ROI workloads before optimizing further.
- GPU compute only if a measured conversion path misses its interaction budget.
- Session persistence, registration/resampling, DICOM, or collaboration after the author-to-export loop is useful.

## Not tasks now

- A general ECS rewrite or generic job framework. Step D is a targeted change to how ROIs are typed and accessed.
- New contour, voxel, or meshing algorithms. Steps A to F change how they are called, not what they compute. Meshing work is tracked in `docs/mesh-authority-and-meshing-plan.md`.
- GPU compute migration without a measured bottleneck.
