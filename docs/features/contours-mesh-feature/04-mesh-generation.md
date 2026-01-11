# 04 - Mesh Generation

Conversion algorithms for mesh representation.

## Labelmap → Mesh (Marching Cubes)

Extract isosurface as triangle mesh from 3D labelmap.

### Algorithm

Examine each 2×2×2 cell (8 vertices), classify into 1 of 256 configurations.

### API

```rust
// src/convert/labelmap_to_mesh.rs

pub fn marching_cubes(
    labelmap: &[u8],
    dims: [u32; 3],
    spacing: [f32; 3],
    label_id: u8,
) -> MeshData
```

### Subtasks

- [ ] Implement edge table (256 entries)
- [ ] Implement triangle table (256 × 16)
- [ ] Core marching cubes loop
- [ ] Vertex deduplication (optional)
- [ ] Compute vertex normals
- [ ] Unit tests

---

## Contours → Mesh (Slice Extrusion)

Build mesh by triangulating between adjacent slices. **More efficient** when contours are already available.

### Algorithm

1. For each pair of adjacent slices, find corresponding contours
2. Triangulate between them (lofting)
3. Cap top and bottom slices

### API

```rust
// src/convert/contour_to_mesh.rs

pub fn extrude_contours(
    contours: &ContourData,
    dims: [u32; 3],
    spacing: [f32; 3],
) -> MeshData
```

### Subtasks

- [ ] Implement contour correspondence matching
- [ ] Implement slice-to-slice triangulation
- [ ] Handle topology changes (split/merge)
- [ ] Cap end slices

---

## Mesh → Contours (Plane Intersection)

Slice mesh with planes to extract contours. Used when mesh is source.

### API

```rust
pub fn slice_mesh(mesh: &MeshData, axis: u8, position: f32) -> Vec<ContourPolyline>
```
