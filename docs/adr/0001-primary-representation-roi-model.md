# Primary Representation ROI Model

ROIs use exactly one authoritative primary representation at a time, with other representations treated as derived session caches. This avoids pretending voxel, contour, and mesh forms can remain losslessly synchronized while still allowing explicit conversion workflows between them.

## Consequences

Primary switches must be explicit, conversions may be lossy, and cache invalidation must be observable rather than hidden inside rendering or UI code.
