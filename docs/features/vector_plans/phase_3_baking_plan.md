# Implementation Plan - Phase 3: Constraint-Driven TSDF Baking

This phase implements the "Back-end" logic that converts the high-fidelity **Vector Layer** back into a **TSDF** for topological resolution and mesh updates.

## Proposed Changes

### Baking Algorithms

#### [NEW] [tsdf_bake.rs](file:///Users/nieage/dev/git/rust_starter_app/src/segmentation/algorithms/tsdf_bake.rs)
- Implement `bake_vectors_to_tsdf(vectors: &VectorContourSet, dirty_chunks: Vec<Coord>)`.
- For each voxel, compute the signed distance to the nearest `SpatialContour`.
- Combine distances conservative (Min/Max operations) to handle overlapping labels.

### Tools Integration

#### [MODIFY] [paint.rs](file:///Users/nieage/dev/git/rust_starter_app/src/systems/paint.rs)
- Refactor paint tools to modify the `VectorContourSet` instead of direct voxel painting.
- Trigger dirty-chunk baking after each vector edit.

## Verification Plan

### Automated Tests
- `test_baking_convergence`: Verify that baking a simple spatial circle into a TSDF and then re-extracting contours results in a close match.

### Manual Verification
1. Use the Draw tool to create a new contour.
2. Observe the 3D mesh updating reactively as the TSDF is baked from the new vector geometry.
