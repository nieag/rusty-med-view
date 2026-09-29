# ROI-Owned Spatial Metadata

Voxel-authoritative ROI data owns its own spatial metadata instead of borrowing dimensions, spacing, origin, or orientation from the current main volume. Imported labelmaps may differ from the displayed image geometry, so ROI conversion, volume measurement, mesh extraction, and overlay projection must use an explicit ROI-native-to-world geometry contract.

## Current import convention

Until the affine migration is complete, NIfTI import uses the first usable spatial transform in this order:

1. non-degenerate finite `sform` rows;
2. a valid coded `qform` reconstructed from NIfTI quaternion fields;
3. legacy fallback: `pixdim` spacing, zero origin, and identity orientation.

The current runtime then stores a decomposed spacing, origin, and quaternion approximation. It preserves rigid orientation and translation but cannot preserve shear and silently normalizes invalid quaternions to identity. This is compatibility behavior, not the target contract.

## Target coordinate contract

- A validated, immutable ROI-owned IJK-to-world affine is the source of truth.
- Integer IJK coordinates denote voxel centres; continuous grid bounds are `[-0.5, dimension - 0.5]`.
- ROI and displayed-volume coordinates meet only through patient/world millimetres.
- Derived caches record the geometry identity used to create them.
- Invalid or singular spatial transforms are rejected at construction/import boundaries; they never become identity implicitly.

## Normalized volume UV

Normalized volume UV is cell-centred and edge-to-edge: `uv = (index + 0.5) / dimension`, where `index` is the continuous IJK coordinate with voxel centres at integers. UV 0 and 1 are the outer voxel faces, voxel `k` is centred at `(k + 0.5) / dimension`, and the voxel containing a UV is `floor(uv * dimension)`.

This is the GPU texture-coordinate convention, so image sampling needs no correction, and it is resolution independent: a physical point keeps its UV when a volume is resampled to another resolution over the same field of view. Node-based UV (`index / (dimension - 1)`) is not used anywhere. All CPU conversions between index space and UV live in `src/convert/coord_mapping.rs`, and the overlay path in `src/shaders/shader.wgsl` mirrors the same arithmetic.

## Consequences

Display alignment requires deliberate conversion through patient/world space, and renderers must not assume label texture coordinates are interchangeable with main volume texture coordinates.
