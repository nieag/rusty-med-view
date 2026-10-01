# Current Repository State

Status: stable-v0 ROI core accepted; foundation hardening (backlog Phase 2b) and exact conversions (chunk F, ADR 0006) done. Drawing features (Phase 3) are next.

## What works

- NIfTI image and labelmap loading, orthogonal/oblique/3D viewing, picking, pan/zoom/rotation, windowing, and notes and comment threads anchored in world space (annotations are entities).
- A single ROI identity with voxel, contour, or mesh authority. The other forms are derived, revision- and geometry-checked caches, and a frame never draws a derived form that disagrees with the ROI's revision. Switching authority happens automatically when an edit needs it, as one undo step.
- Contour authoring and editing in orthogonal and active oblique planes; native WGPU rendering for contours, voxel overlays, and meshes.
- ROI-owned immutable reference geometry. ROIs on different grids stay comparable in patient/world millimetres. A ROI holds its voxel forms only in a snug box of its reference grid (a cropped label never allocates the whole volume), so 150 labels import within a stated memory ceiling.
- A contour is a plane and loops in world space, never snapped to the image voxel grid. How grids, planes, and views are defined is in [spatial-model.md](spatial-model.md).
- Exact conversions ([ADR 0006](adr/0006-exact-conversions-and-soft-voxel-hub.md)): a mesh becomes contours by cutting it with the layer planes (4 ms on the liver, no voxels); a contour ROI's mesh is the zero level of the signed distance field of its loops, updated per slice and re-meshed per changed chunk (one slice edit about 9 ms, a full build about 190 ms, time-sliced across frames); the contours of a contour ROI in the other plane families are that mesh cut with the plane. ROI volume comes from the authoritative form (contour area times layer thickness, mesh divergence theorem, voxel count).
- One ROI work coordinator advances demanded derived work in time-sliced steps, rejects stale results, uploads current GPU mirrors, and requests follow-up frames until the queue is empty.

## Runtime boundary

```text
input + GUI intent
  -> authoritative ROI edit
  -> generation/revision + dirty bounds
  -> demanded conversion work on CPU (time-sliced)
  -> geometry-checked cache install
  -> WGPU texture/buffer upload
  -> native viewport rendering
```

`src/model/` is pure shared data and imports nothing from the crate. `src/convert/` owns pure geometry, rasterization, contour extraction, the distance field, and meshing algorithms. `src/app/roi/` owns authority, caches, history, scheduling, and requests. `src/app/roi_runtime/` runs the conversion jobs. `src/render/` owns WGPU preparation and drawing. `egui` owns controls only, never viewer-scene drawing. The layer order is enforced by `tests/layering.rs`.

The CPU performs representation conversion. The GPU renders already-prepared textures and buffers; it does not run ROI compute shaders.

## Evidence

- Unit and lifecycle tests (about 400), plus release guards that assert timings and exactness on the liver sample: `tests/switch_guard.rs` (conversion budgets), `tests/scale_guard.rs` (150 labels), `tests/conversion_accuracy.rs` (chains against analytic shapes; the numbers are the record).
- The browser QA spec `tests/qa1_viewerqa.spec.js` (7 tests) runs the app with WebGPU and checks readiness, geometry and orientation, cached 3D view, every visible ROI showing its derived forms, and a 150-label case.
- Stable-v0 manual GPU-browser QA on the liver sample (the retained ROI-core record has the exact contract): fresh orthogonal contour commit converged in 5 ms, oblique in 26 ms, with current voxel, contour, and mesh caches and no failed jobs. Later chunks only made these paths faster or exact; their budgets are in the guard tests.

## Known gaps

The open items, in order, are in the [backlog](roi-workstation-todos.md). The ones that shape the next work: the contour ROI's voxel cache is still rasterized from the loops (deriving it from the field, built on demand, is the next structural item); the drag preview is still voxel-based; undo and the cached per-slice field data are not yet counted in the memory budget (4.5); ROI algebra will work on the signed distance field (3.9); interpolation between slices will be an explicit, visibly marked tool (3.8).

## Living documentation

- [Spatial model](spatial-model.md) — grids, planes, slices, and views.
- [Rendering architecture](rendering-architecture.md) — mandatory viewport-rendering boundary.
- [Code map](code-map.md) — repository navigation and runtime ownership.
- [ROI workstation TODOs](roi-workstation-todos.md) — ordered execution backlog.
- [ROI core restructure](roi-core-restructure-plan.md) — accepted stable-v0 model, coordinate contract, and performance evidence.
- [Architecture decisions](adr/) — primary representation (0001), WGPU ownership (0002), ROI spatial metadata (0003), typed body and automatic switching (0004), scene entity model (0005), exact conversions (0006).
- [Domain glossary](../CONTEXT.md) — shared terminology.

Completed plans and handoffs are under [archive/](archive/); they are context, not instructions.

## Next planning decision

The proposed direction is the [ROI workstation plan](roi-workstation-plan.md): polish one complete local authoring-to-export workflow before adding new algorithms or infrastructure.
