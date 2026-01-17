# Implementation Plan - Phase 4: Hybrid Reconstruction & Snapping

The final architectural step: ensuring the 3D mesh (derived from TSDF) and the 2D vector contours are in perfect agreement via **Vertex Snapping**.

## Proposed Changes

### Meshing Logic

#### [MODIFY] [incremental_mesher.rs](file:///Users/nieage/dev/git/rust_starter_app/src/segmentation/algorithms/incremental_mesher.rs)
- Integrate a "Snapping Pass" into the Surface Nets loop.
- For each mesh vertex:
    - Query the `VectorContourSet` for constraints within its influence radius.
    - If found, snap the vertex position to the authoritative vector boundary.
- Calculate normals using the SDF gradient, but project them to stay tangential to the snapped surface.

## Verification Plan

### Automated Tests
- `test_mesh_vector_alignment`: Quantitatively verify that snapped mesh vertices are within a 0.001mm tolerance of the authoritative vector geometry.

### Manual Verification
1. Zoom in extremely close to a 2D vector contour in a 2D view.
2. Enable 3D mesh visibility.
3. Verify that the mesh silhouette and the 2D vector line are perfectly coincident with zero jitter.
