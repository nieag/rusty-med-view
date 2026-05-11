# Subplan 8.1 Spatial Geometry Contract Handoff

This document is the source of truth for fixing ROI/image spatial alignment before continuing mesh deformation or additional mesh workflows.

## Problem

Manual verification exposed a structural mismatch:

- the green voxel label overlay appears in the expected image region
- mesh extraction from the same voxel ROI can be displaced or invisible
- the voxel overlay shader currently samples label textures through the main image display grid
- mesh extraction correctly uses ROI-owned voxel geometry and emits world-millimetre vertices

Those paths do not share one spatial contract. Local mesh fixes are therefore not sufficient.

## Goal

Create one central geometry path used by voxel overlays, contours, meshes, picking, and conversion helpers:

```text
representation-native index/point space
  -> representation geometry
  -> patient/world millimetres
  -> viewport projection
```

Loaded label geometry must remain authoritative. The image volume is the display reference for the viewport, not the owner of label geometry.

## Current Safety Gate

Status: resolved by the Subplan 8.1 implementation.

Before voxel overlay rendering became geometry-aware, mesh creation from voxel ROIs whose geometry differed from the main image geometry had to fail clearly.

Do not create:

- displaced meshes
- empty mesh ROIs from non-empty voxel sources
- silently resampled display-grid meshes that disagree with authoritative label geometry

The temporary geometry-mismatch gate may be removed only when the voxel overlay shader and CPU conversion tests share the central geometry contract. That condition is now met for Subplan 8.1: voxel overlay sampling maps through ROI-owned geometry, and mesh extraction remains native to source voxel geometry.

## Required Architecture

### 1. Centralize spatial transforms

Add or consolidate helpers in `src/convert/` for:

- `voxel_index_to_world_mm(index, geometry)`
- `world_mm_to_voxel_index(world_mm, geometry)`
- `volume_uv_to_world_mm(uvw, geometry)`
- `world_mm_to_volume_uv(world_mm, geometry)`
- main-image-to-ROI mapping helpers for CPU tests
- a GPU-friendly transform representation for label overlay sampling

The exact API names may differ, but the ownership must not:

- conversion math lives in `src/convert/`
- orientation composition remains in `src/util/orientation.rs`
- GUI code must not own spatial transforms
- renderers consume prepared transform/projection data rather than rebuilding ad hoc math

### 2. Make voxel overlay sampling geometry-aware

The voxel overlay shader must stop assuming that label texture coordinates equal main image texture coordinates.

Correct sampling model:

```text
main image uvw/sample position
  -> main image world mm
  -> ROI/label voxel index or uvw
  -> label textureLoad
```

This likely requires per-overlay uniform data carrying a `main_uv_to_label_index` or equivalent transform. Keep the initial implementation limited to the existing maximum overlay count.

### 3. Keep mesh extraction native

Voxel-to-mesh extraction should remain representation-native:

- input is the source `VoxelData`
- occupancy is read from source voxel indices
- vertices are emitted in patient/world millimetres via source `VoxelData.geometry`

Do not make mesh extraction depend on main image dimensions or display-grid resampling.

### 4. Keep contour projection consistent

Contour rendering already uses ROI-owned geometry/plane context. During this subplan, verify that contour placement still agrees with voxel overlay and mesh placement under the central transform path.

Do not rewrite contour editing unless a failing regression proves it is needed.

## Required Tests

Add CPU tests for the shared geometry contract:

- identity geometry maps voxel index to world and back without drift
- non-unit spacing and shifted origin roundtrip correctly
- non-identity orientation roundtrips correctly
- main-image sample world position maps into the expected ROI voxel index
- out-of-bounds ROI sampling is explicit and safe

Add integration-style regression tests:

- voxel overlay sampling transform and voxel-to-mesh extraction agree for the same ROI geometry
- a label with same dimensions but shifted origin is not treated as image-grid aligned
- a label with matching affine but different dimensions can still map through world space
- mesh vertex world bounds match source ROI geometry, not main image geometry
- geometry-mismatched voxel ROI mesh creation remains fail-fast until geometry-aware overlay sampling is implemented; after geometry-aware overlay sampling, mesh creation accepts mismatched geometry and remains aligned through world-space projection

## Manual Verification

Use real image + label data that previously exposed the bug.

Expected before removing the safety gate:

- loading image + label still shows the existing voxel overlay behavior
- creating mesh from geometry-mismatched voxel ROI shows a clear unsupported-geometry status
- no new empty/invisible mesh ROI is created

Expected after geometry-aware overlay sampling:

- voxel overlay remains visible in the correct image region
- extracted mesh overlaps the voxel overlay in 3D
- scrolling/rotating does not produce label/mesh drift
- labels with different origin/spacing/orientation either align through world geometry or fail clearly if outside the image field of view

Observed manual verification:

- image + label load without WGPU uniform-buffer errors after dynamic uniform stride fix
- label overlay is visible and aligned on real image + label data after NIfTI qform fallback fixed main-image origin
- mesh created from loaded voxel label aligns with the green voxel overlay in 3D
- drawn contours can be converted to mesh and remain coherent in the viewer

## Non-Goals

Do not implement:

- mesh deformation tools
- mesh-to-voxel regeneration
- mesh-to-contour regeneration
- smoothing, decimation, topology repair, margins, booleans
- SDF/TSDF
- full registration UI
- broad renderer abstraction rewrite

This phase is only about making spatial alignment central and testable.

## Completion Criteria

- central geometry helpers exist and are tested
- voxel overlay sampling uses ROI geometry rather than main-grid texture coordinates
- mesh extraction continues to use source ROI geometry
- mesh rendering projects world-mm vertices through the shared display projection path
- fail-fast geometry mismatch tests are replaced or narrowed only when the geometry-aware overlay path is implemented
- `cargo test -q`, `cargo check --target wasm32-unknown-unknown -q`, and `cargo fmt --all` pass

## Implementer Prompt

Continue the segmentation reimplementation using `docs/subplan-8-1-spatial-geometry-contract-handoff.md` as the source of truth. Implement only the central spatial geometry contract and geometry-aware voxel overlay sampling needed to make voxel overlay, contour, and mesh placement use the same ROI-native-to-world-to-viewport path. Do not start mesh deformation, mesh-to-voxel regeneration, mesh-to-contour regeneration, smoothing, SDF/TSDF, import/export, registration UI, or broad renderer abstraction work. Preserve current image loading, voxel label loading, contour editing, contour extraction, contour-to-voxel rebuild, 2D navigation, and 3D viewer behavior except where voxel overlay sampling must change to respect ROI geometry. Update `docs/segmentation-reimplementation-plan.md` with the completed checkpoint, run `cargo test -q`, `cargo check --target wasm32-unknown-unknown -q`, and `cargo fmt --all`, and include concrete manual verification notes before summarizing.
