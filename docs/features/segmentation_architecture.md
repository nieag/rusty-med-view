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
│                    Core Source of Truth: TSDF                    │
│                    (Chunked, sparse i8 field)                   │
├─────────────────────────────┬───────────────────────────────────┤
│   Sync: TSDF → Slicing      │   Sync: TSDF → Surface Nets       │
│   Result: 2D Contours       │   Result: 3D Mesh                 │
└─────────────────────────────┴───────────────────────────────────┤
│             Export: Labelmap (NIfTI) / Mesh (STL)               │
└─────────────────────────────────────────────────────────────────┘
```

## Key Design Decisions

1. **TSDF is the volumetric source of truth** — Both 2D Brush tools and 3D sculpting modify TSDF chunks.
2. **Resolution-independent 2D fallback** — Higher-fidelity Bezier/Spline tools can still exist as overlay Contours, which rasterize *into* the TSDF on commit.
3. **Reactive Re-meshing** — Surface Nets only re-calculates triangles for TSDF chunks marked as dirty.
4. **Zero-Lag Slicing** — 2D views "slice" the TSDF at the current crosshair plane to generate visual contours.

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
| 2D Edit | Brush → TSDF → (Sync All) | Fast, sparse chunk updates |
| 3D Edit | Sculpt → TSDF → (Sync All) | Consistent with 2D |
| Sync Mesh | TSDF → Surface Nets | Only dirty chunks |
| Sync Contours | TSDF → 2D Slicing | Generated on-the-fly |
| Export | TSDF → Threshold → NIfTI | Standard grid export |

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
