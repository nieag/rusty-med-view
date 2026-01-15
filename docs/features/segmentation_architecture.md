# Dual-Representation Segmentation System

A professional segmentation system with **Contour** as 2D source of truth and **Mesh** as 3D source of truth, synchronized via TSDF intermediary.

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                    User-Facing Representations                  │
├─────────────────────────────┬───────────────────────────────────┤
│   2D: Contour Polylines     │   3D: Triangle Mesh               │
│   (Resolution-Independent)  │   (Resolution-Independent)        │
│   Primary for: Draw, Fill   │   Primary for: Sculpt, Deform     │
├─────────────────────────────┴───────────────────────────────────┤
│                    Synchronization Layer                         │
│   • Contour → TSDF → Mesh (when 2D edits)                       │
│   • Mesh → Slice → Contour (when 3D edits, no TSDF)             │
├─────────────────────────────────────────────────────────────────┤
│                    Hidden Intermediary: TSDF                     │
│                    (Chunked, one-way for meshing only)           │
├─────────────────────────────────────────────────────────────────┤
│                    Export: Labelmap (NIfTI) / Mesh (STL)        │
└─────────────────────────────────────────────────────────────────┘
```

## Key Design Decisions

1. **TSDF is one-way** — Contours never round-trip through TSDF to avoid resolution loss
2. **Mesh→Contour via plane intersection** — Direct slicing, no grid degradation
3. **Resolution-independent storage** — Both contours (float polylines) and mesh (float vertices) are exact

## Data Representations

### Contour (2D Source of Truth)
```rust
pub struct Contour {
    pub points: Vec<Vec2>,
    pub is_closed: bool,
    pub segment_id: Uuid,
}

pub struct ContourSet {
    pub slices: HashMap<(ViewMode, i32), Vec<Contour>>,
}
```

### Mesh (3D Source of Truth)
```rust
pub struct SegmentationMesh {
    pub vertices: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub indices: Vec<u32>,
}
```

### TSDF (Sync Intermediary)
```rust
pub struct ChunkedTSDF {
    pub chunks: HashMap<(i16, i16, i16), Chunk>,  // 32³ i8 values each
}
```

## Synchronization

| Direction | Path | Notes |
|-----------|------|-------|
| 2D → 3D | Contour → TSDF → Mesh | TSDF handles topology |
| 3D → 2D | Mesh → Plane Intersection → Contour | Exact, no grid |
| Export | Contours → Rasterize → NIfTI | Skip TSDF |

## Tool Categories

| Tool | Modifies | Sync |
|------|----------|------|
| Freehand, Polygon, Spline | Contour | → TSDF → Mesh |
| Contour Drag | Contour | → TSDF → Mesh |
| Contour Brush | Contour | → TSDF → Mesh |
| Sculpt (3D) | Mesh | → Slice → Contour |
| Threshold | Contour | → TSDF → Mesh |

## File Structure

```
src/segmentation/
├── mod.rs
├── contour.rs          # Contour, ContourSet
├── mesh.rs             # SegmentationMesh
├── tsdf.rs             # Chunk, ChunkedTSDF
├── sync.rs             # Sync pipelines
├── algorithms/
│   ├── marching_squares.rs
│   ├── surface_nets.rs
│   ├── rasterize.rs
│   └── mesh_slice.rs
└── tools/
    ├── freehand.rs
    ├── polygon.rs
    ├── spline.rs
    ├── contour_drag.rs
    ├── contour_brush.rs
    └── threshold.rs
```

## Related Documents

- [Milestones](./segmentation_milestones.md) — Detailed task breakdown
- [PolySeg Original](./polyseg.md) — Initial WASM-first TSDF concept
