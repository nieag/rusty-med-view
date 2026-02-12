# Contour-Based Segmentation Feature

**Status:** In Progress (Phases 1-3 implemented, P2/P3 stabilization pass active)

## Overview

This feature implements contour-based segmentation with sub-voxel precision, multi-axis editing, and high-quality 3D mesh rendering.

**Architecture:**
```
Contours (ground truth) → SDF (intermediate) → Mesh (3D display)
                       ↘ Contour outlines (2D display)
```

## Key Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Ground truth | Contours | Sub-voxel precision, natural drawing interface |
| Intermediate | SDF | Enables smooth mesh generation, handles multi-axis blending |
| 3D Display | Triangle mesh | Proper lighting, industry standard |
| 2D Display | Contour outlines | Lightweight, precision preserved |

## Document Index

| Document | Description |
|----------|-------------|
| [01-data-model.md](01-data-model.md) | Core data structures and APIs |
| [02-contour-drawing.md](02-contour-drawing.md) | Freehand contour drawing tool |
| [03-contour-rendering.md](03-contour-rendering.md) | 2D contour outline display |
| [04-contour-to-sdf.md](04-contour-to-sdf.md) | SDF conversion with multi-axis support |
| [05-marching-cubes.md](05-marching-cubes.md) | Mesh generation from SDF |
| [06-mesh-rendering.md](06-mesh-rendering.md) | 3D mesh rendering pipeline |
| [07-integration.md](07-integration.md) | End-to-end wiring and GUI |

## Current Scope Notes

- Phase 3 contour rendering is currently axis-aligned (axial/coronal/sagittal) in 2D.
- Oblique contour overlay rendering in 2D is intentionally deferred.

## Implementation Order

```
Phase 1 (Data Model) → Phase 2 (Drawing) → Phase 3 (2D Render)
                    ↘ Phase 4 (SDF) → Phase 5 (Mesh Gen) → Phase 6 (3D Render)
                                                                    ↓
                                                           Phase 7 (Integration)
```

## Estimated Effort

| Phase | Days |
|-------|------|
| 1. Data Model | 2 |
| 2. Contour Drawing | 3 |
| 3. 2D Rendering | 2 |
| 4. Contour → SDF | 3 |
| 5. Marching Cubes | 2 |
| 6. Mesh Pipeline | 3 |
| 7. Integration | 2 |
| **Total** | **~17 days** |
