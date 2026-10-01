# Rendering Architecture

This document records repository-wide rendering rules. It applies to all viewer, ROI, contour, mesh, annotation, and future segmentation work.

This document is about what draws what. How positions are defined and converted is in `spatial-model.md`.

## Egui Boundary

`egui` is for GUI only.

Allowed egui responsibilities:

- menus, toolbars, sidebars, dialogs, sliders, buttons, and status text
- UI layout and interaction widgets
- debug-only diagnostics when explicitly marked as temporary and not part of the viewer rendering path

Disallowed egui responsibilities:

- drawing medical image content
- drawing ROI geometry
- drawing contour loops, contour points, mesh surfaces, mesh wireframes, or segmentation overlays
- implementing viewport clipping for clinical/viewer content
- owning projection math for image-space, patient-space, or viewport-space primitives

If a feature appears inside a viewport as part of the viewer scene, it must be rendered through the native rendering stack, not through egui painting.

Known exceptions, until they get native renderers: annotation markers and labels, and the orientation gizmo, are drawn by egui from projections computed by the shared helpers.

## Native Rendering Ownership

Viewer-scene rendering belongs under `src/render/` and `src/shaders/`.

Required pattern for new viewport-rendered geometry:

- prepare render-facing data in a renderer-specific prep path
- keep conversion/extraction algorithms out of render passes
- upload to wgpu buffers/textures explicitly
- draw through a wgpu pipeline/pass
- enforce viewport bounds in the renderer through viewport/scissor control or equivalent GPU clipping
- use shared coordinate/geometry helpers from `src/convert/` and `src/util/orientation.rs`

The renderer may consume ECS state, ROI caches, and prepared view models, but it must not mutate authoritative ROI data or run conversion algorithms inline.

## Coordinate-Space Rule

Renderable geometry may be stored in representation-native space, but projection must be explicit.

Examples:

- contours store loop points in plane-local millimetres and project through their `PlaneDefinition`
- mesh vertices store patient/world millimetres and project through the active display volume geometry when overlaying the image
- voxel overlays use ROI-owned voxel geometry for conversion/cache ownership and the renderer's display-space contract for visualization
- voxel overlays and geometry-derived renderables created from those overlays must share a display-compatible voxel source grid when used in the same visual workflow

Do not add ad hoc coordinate transforms inside GUI code. Shared transforms belong in `src/convert/` or `src/util/orientation.rs`, with tests.

## Display-Compatible Voxel Sources

Authoritative voxel ROI geometry and displayed voxel overlay geometry are separate concepts.

Rules:

- preserve authoritative voxel geometry from import/conversion
- voxel overlay sampling must use ROI-owned voxel geometry rather than assuming label texture coordinates equal main image texture coordinates
- mesh extraction from voxel ROIs remains native to the source `VoxelData.geometry` and emits patient/world millimetre vertices
- mesh-from-voxel workflows and the green voxel overlay must agree through the same ROI-native-to-world-to-viewport geometry contract
- do not silently compare or convert between a displayed texture-aligned voxel overlay and mesh geometry extracted from a different native label grid; texture-aligned overlay sampling was the legacy shortcut that Subplan 8.1 replaced

This is especially important when a loaded labelmap has dimensions, spacing, origin, or orientation that differs from the main image volume.

## Projection Sync Point

Viewport-rendered features must not independently rebuild display projection state.

The repository should maintain one shared display projection context path that owns:

- main display volume geometry lookup
- viewport rect and window size
- screen aspect
- zoom, pan, and pivot
- cursor position
- display aspect ratios
- 3D composed rotation

For 3D viewports, the composed rotation must match the volume pass:

```rust
compose_view_rotation(main_volume.orientation, viewport_state.user_rotation)
```

Renderers for contours, meshes, overlays, picking helpers, and future ROI view adapters should consume this shared context or a clearly equivalent helper. Do not locally reconstruct projection state in each renderer.

## Review Checklist

Before accepting rendering-related work, verify:

- no non-debug viewport scene geometry is drawn with egui
- render prep and render draw paths are separated
- viewport clipping is handled by the native render path
- renderers use the shared display projection context rather than local ad hoc projection setup
- coordinate-space conversions are explicit and tested
- current image, voxel label, contour, 2D navigation, and 3D viewer behavior are preserved
