# Implementation Plan - Phase 1: Legacy Import & Authority Handoff

This phase focuses on the "Inbound" pipeline: taking existing voxel-based labelmaps (NIfTI) and promoting them into the hybrid vector/SDF system. This is the foundation for all future vector edits.

## User Review Required

> [!IMPORTANT]
> This phase establishes the "Authority Handoff". Once a labelmap is imported and converted to vectors, the raw voxel data will no longer be the primary source of truth for edits.

## Proposed Changes

### Segmentation Logic

#### [NEW] [tsdf_import.rs](file:///Users/nieage/dev/git/rust_starter_app/src/segmentation/algorithms/tsdf_import.rs)
- Implement `voxel_to_tsdf(labelmap: &LabelmapData) -> ChunkedTSDF`.
- Uses a 3D distance transform in physical space.
- Implements sub-voxel smoothing (Gaussian or similar) to strip staircase artifacts.

#### [NEW] [handoff.rs](file:///Users/nieage/dev/git/rust_starter_app/src/segmentation/algorithms/handoff.rs)
- Implement `tsdf_to_spatial_contours(tsdf: &ChunkedTSDF) -> VectorContourSet`.
- Slices the TSDF at key axial/coronal/sagittal intervals.
- Extracts polylines via Marching Squares.
- Simplifies polylines (Ramer-Douglas-Peucker) to maintain reasonable control point counts.

### Integration

#### [MODIFY] [load_handlers.rs](file:///Users/nieage/dev/git/rust_starter_app/src/load_handlers.rs)
- Update `handle_label_load` to trigger the Promotion Pipeline instead of just updating a raw label texture.

## Verification Plan

### Automated Tests
- `test_voxel_to_tsdf_distance`: Verify that a sphere in voxel space produces correct distance values in the TSDF.
- `test_handoff_reconstruction`: Verify that slicing the generated TSDF and extracting contours results in polylines that enclose the original voxel region.

### Manual Verification
1. Load a `.nii.gz` labelmap.
2. Confirm that the 3D mesh (derived from TSDF) looks smooth (staircase artifacts removed).
3. Confirm that 2D contours appear correctly on slices.
