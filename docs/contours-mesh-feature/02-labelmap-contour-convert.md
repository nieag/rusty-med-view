# 02 - Labelmap ↔ Contour Conversion

Conversion algorithms between labelmap and contour representations.

## Labelmap → Contours (Marching Squares)

Extract 2D contour polylines from a labelmap slice.

### Algorithm

For each 2×2 cell, classify into 1 of 16 configurations based on corner values:

```
 0---1      Config 5 (0101):    1---0
 |   |      diagonal edges      |   |
 3---2                          0---1
```

### API

```rust
// src/convert/labelmap_to_contour.rs

pub fn extract_slice_contours(
    labelmap: &[u8],
    dims: [u32; 3],
    axis: u8,        // 0=YZ, 1=XZ, 2=XY
    slice_idx: u32,
    label_id: u8,
) -> Vec<ContourPolyline>
```

### Subtasks

- [ ] Implement `marching_squares_2d()` core algorithm
- [ ] Implement `chain_segments()` to link edges into polylines
- [ ] Add unit tests for all 16 configurations
- [ ] Benchmark: target < 5ms for 512×512 slice

---

## Contours → Labelmap (Rasterization)

Convert contour polylines back to voxels.

### Algorithm

1. Rasterize contour edges using Bresenham's line algorithm
2. Flood fill interior for closed contours
3. Handle edge cases: self-intersecting, nested contours

### API

```rust
// src/convert/contour_to_labelmap.rs

pub fn rasterize_contour_to_slice(
    contour: &ContourPolyline,
    slice: &mut [u8],
    width: u32,
    height: u32,
)
```

### Subtasks

- [ ] Implement `rasterize_polyline()` 
- [ ] Implement `flood_fill_interior()`
- [ ] Handle multiple contours per slice
- [ ] Add unit tests
