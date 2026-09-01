# Current Repository State

Status: stable-v0 ROI core accepted.

## What works

- NIfTI image and labelmap loading, orthogonal/oblique/3D viewing, picking, pan/zoom/rotation, windowing, annotations, and notes.
- A single ROI identity with voxel, contour, or mesh authority. The other forms are derived, revision- and geometry-checked caches.
- Contour authoring and editing in orthogonal and active oblique planes; voxel/contour/mesh conversion; native WGPU rendering for contours, voxel overlays, and meshes.
- ROI-owned immutable reference geometry. ROIs on different voxel grids remain comparable in patient/world millimetres; voxel-wise comparison requires an explicit target grid and resampling policy.
- One ROI work coordinator advances demanded derived work, rejects stale results, uploads current GPU mirrors, and requests follow-up frames until the queue is empty.

## Runtime boundary

```text
input + GUI intent
  -> authoritative ROI edit
  -> generation/revision + dirty bounds
  -> demanded conversion work on CPU
  -> geometry-checked cache install
  -> WGPU texture/buffer upload
  -> native viewport rendering
```

`src/convert/` owns pure geometry, rasterization, contour extraction, and meshing algorithms. `src/app/roi/` owns authority, caches, history, scheduling, and requests. `src/render/` owns WGPU preparation and drawing. `egui` owns controls only, never viewer-scene drawing.

The CPU performs representation conversion. The GPU renders already-prepared textures and buffers; it does not run ROI compute shaders.

## Stable-v0 evidence

Manual GPU-browser QA with the liver sample accepted the full contour loop:

| Commit path | Contour raster | Convergence | Queue delay | GPU upload |
| --- | ---: | ---: | ---: | ---: |
| Fresh orthogonal contour | 3 ms | 5 ms | 1 ms | 1 ms |
| Fresh oblique contour | 3 ms | 26 ms | 1 ms | 2 ms |

Both paths finished with current voxel, contour, and mesh caches and no failed or discarded jobs. The retained ROI-core record contains the exact acceptance contract and measurements.

## Living documentation

- [Rendering architecture](rendering-architecture.md) — mandatory viewport-rendering boundary.
- [ROI core restructure](roi-core-restructure-plan.md) — accepted stable-v0 model, coordinate contract, and performance evidence.
- [Architecture decisions](adr/) — primary representation, WGPU ownership, and ROI spatial metadata.
- [Domain glossary](../CONTEXT.md) — shared terminology.

Completed plans and handoffs are under [archive/](archive/); they are context, not instructions.

## Next planning decision

There is no active implementation plan. The core loop is stable enough to choose product value rather than further infrastructure work.
