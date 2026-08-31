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

## Consequences

Display alignment requires deliberate conversion through patient/world space, and renderers must not assume label texture coordinates are interchangeable with main volume texture coordinates.
