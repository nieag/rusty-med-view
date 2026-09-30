# Code Map

This is a navigation guide to the current repository, not a second architecture plan.

## Start here

| Need | Entry point |
| --- | --- |
| Application startup and event loop | `src/lib.rs`, `src/app/mod.rs` |
| ECS state and stable entity registry | `src/app/components.rs` |
| ROI domain model and mutations | `src/app/roi/` |
| Derived ROI work and GPU cache synchronization | `src/app/roi_runtime.rs` |
| User input and authoring | `src/systems/` |
| Coordinate/conversion algorithms | `src/convert/` |
| WGPU rendering | `src/render/`, `src/shaders/` |
| UI panels and controls | `src/gui/` |
| NIfTI and GPU texture loading | `src/io/` |
| Browser QA contract | `src/app/qa.rs`, `src/lib.rs` |

## Module ownership

Layer order, enforced for the lower layers by `tests/layering.rs`: `model` <- `convert` <- `util`,
`io` <- everything else. A file in a lower layer may only name modules at or below its own layer.
The upper layers (`app`, `render`, `systems`, `gui`) still depend on each other in both
directions; untangling that is backlog item 2b.9 and 2b.7.

```text
src/model/         Pure data shared by every layer: VoxelGeometry, VoxelData, PlaneFamily and
                   PlaneDefinition, contour and mesh data, VolumeData, LoadedLabel, ViewMode.
                   Imports nothing from the crate.
src/app/handlers.rs
                   Loading a volume or labelmap into the scene, and status messages
src/app/roi/       ROI identity, authority, cache state, history, previews, requests, scheduling
src/app/roi_runtime.rs
                   Concrete conversion executor, cache installation, GPU-resource synchronization
src/convert/       Pure geometry/rasterization/extraction/meshing functions; depends on model only
src/systems/       Input, picking, contour editing, mesh editing, render-data preparation
src/render/         Render protocols, WGPU pipelines, contour/mesh view preparation and drawing
src/gui/            egui controls only
src/io/             NIfTI parsing and volume/texture upload (depends on model, convert, util)
src/util/           Shared orientation helpers
src/overlay/        Annotation-overlay primitives and management
```

`roi_runtime.rs` is deliberately concrete: it is the only frame-work coordinator today, not a general job framework. Its eventual cleanup target is a narrower executor module, but no split is needed until a real ownership boundary emerges.

## Frame and data flow

```text
winit event
  -> GUI/input systems mutate ECS state
  -> RedrawRequested
  -> render::pipeline::render_frame
       -> roi_runtime::advance_roi_work
          -> CPU conversion jobs + validated cache install + GPU mirror sync
       -> systems::sys_prepare_render_data
       -> volume / voxel-overlay / contour / mesh WGPU passes
  -> request another redraw while ROI work remains
```

`App` in `src/app/mod.rs` owns the winit lifecycle. `RenderingContext` in `src/app/context.rs` owns the WGPU device, surface, pipelines, ECS world, GUI, and render resources.

## ROI flow

```text
authoritative edit (voxel | contour | mesh)
  -> source generation/revision changes; dirty bounds recorded
  -> requested visible/edit/export representation becomes demand
  -> advance_roi_work runs conversion on CPU
  -> cache is accepted only when source generation and RoiGeometry identity match
  -> current voxel/contour/mesh cache is uploaded/prepared for WGPU rendering
```

Useful files by concern:

| Concern | Files |
| --- | --- |
| ROI body types and authoritative edits | `app/roi/model.rs`, `app/roi/authority.rs` |
| Automatic representation switching (`ensure_editable`) | `app/roi/switch.rs` |
| Cache state and validation | `app/roi/cache.rs`, `app/roi/requests.rs` |
| Undo/redo and interaction previews | `app/roi/history.rs`, `app/roi/preview.rs` |
| Work demand and job ordering | `app/roi/scheduler.rs`, `app/roi_runtime.rs` |
| Contour ↔ voxel | `convert/contour_raster.rs`, `convert/voxel_contour_extract.rs` |
| Voxel ↔ mesh | `convert/voxel_mesh_extract.rs`, `convert/mesh_voxelize.rs` |
| Mesh ↔ displayed plane | `convert/mesh_plane_intersect.rs` |

## Coordinate rule

Patient/world millimetres are the interchange space. Each ROI owns immutable `RoiGeometry`; each voxel cache carries geometry identity. Conversion helpers live in `src/convert/geometry.rs` and `src/convert/coord_mapping.rs`; orientation helpers live in `src/util/orientation.rs`.

Do not add coordinate or projection math in `src/gui/`. For viewport rendering, use the shared projection/context path described in [rendering architecture](rendering-architecture.md).

## Rendering rule

`egui` provides controls; native WGPU renders all scene content. The main volume and voxel overlays use `src/shaders/shader.wgsl`; contours and meshes use dedicated renderers and shaders under `src/render/` and `src/shaders/`.

## Tests and QA

Pure behavior tests live beside their module in `#[cfg(test)]`. The browser QA surface is installed from `src/lib.rs` on WASM and reports state/metrics from `src/app/qa.rs`. Use it for live cache and timing evidence; headless fallback remains a contract check when WebGPU is unavailable.

## Read alongside this map

- [Current state](current-state.md)
- [Rendering architecture](rendering-architecture.md)
- [ROI-core record](roi-core-restructure-plan.md)
- [ADRs](adr/)
