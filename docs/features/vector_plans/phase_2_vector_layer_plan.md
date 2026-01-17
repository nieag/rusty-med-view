# Implementation Plan - Phase 2: Vector Layer & Dynamic Projection

This phase establishes the **Vector Layer** as the primary storage for geometry and implements dynamic projection into 2D viewports.

## Proposed Changes

### Data Structures

#### [MODIFY] [contour.rs](file:///Users/nieage/dev/git/rust_starter_app/src/segmentation/contour.rs)
- Replace `ContourSet` (slice-based) with `VectorContourSet` (3D spatial).
- Define `SpatialContour` with plane-agnostic polyline data.

### Rendering

#### [NEW] [projection.rs](file:///Users/nieage/dev/git/rust_starter_app/src/segmentation/algorithms/projection.rs)
- Implement `project_to_plane(contour: &SpatialContour, view_plane: Plane3) -> Option<ProjectedPolyline>`.
- Handles intersection of the spatial contour's 3D influence volume with the current slice.

#### [MODIFY] [render_prep.rs](file:///Users/nieage/dev/git/rust_starter_app/src/systems/render_prep.rs)
- Update contour synchronization to project from the `VectorContourSet` instead of fetching from slice indices.

## Verification Plan

### Automated Tests
- `test_oblique_projection`: Verify that a 3D contour drawn on a SAGITTAL plane correctly projects as a "line" (or intersection) when viewed AXIALly.

### Manual Verification
1. Draw a contour in the Axial view.
2. Verify it is visible in 3D and its intersections are visible in Coronal/Sagittal views.
3. Rotate the view to an oblique angle (if supported) and verify vector rendering stability.
