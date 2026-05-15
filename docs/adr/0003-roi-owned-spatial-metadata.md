# ROI-Owned Spatial Metadata

Voxel-authoritative ROI data owns its own spatial metadata instead of borrowing dimensions, spacing, origin, or orientation from the current main volume. Imported labelmaps may differ from the displayed image geometry, so ROI conversion, volume measurement, mesh extraction, and overlay projection must use an explicit ROI-native-to-world geometry contract.

## Consequences

Display alignment requires deliberate conversion through patient/world space, and renderers must not assume label texture coordinates are interchangeable with main volume texture coordinates.
