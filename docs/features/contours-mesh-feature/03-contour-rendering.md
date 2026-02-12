# Phase 3: 2D Contour Rendering

## Goal

Render contour outlines for interactive 2D editing views.

## Current Implementation Status

- Implemented in `src/render/contour_pipeline.rs` and integrated in `src/render/pipeline.rs`.
- Renders contour lines for axis-aligned slice views:
  - Axial
  - Coronal
  - Sagittal
- Live drawing preview is rendered while drawing.

## Scope Boundary (Current)

- In scope:
  - 2D contour overlays for axis-aligned views.
  - Per-segment color rendering.
  - Current-slice filtering.
- Deferred:
  - Oblique contour overlay rendering in 2D.

## Notes

- Oblique contours are preserved in data model and processing pipeline, but are skipped in the
  current 2D overlay pass.
- This is intentional for Phase 3 closure and should be revisited in a later phase.
