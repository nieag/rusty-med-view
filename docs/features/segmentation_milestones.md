# Segmentation System — Milestones

## Milestone 0: Coordinate & Orientation Consolidation [COMPLETE]
**Goal**: Native orientation support for non-RAS volumes  
**Deliverable**: Correct projection and anatomical labels for any NIfTI orientation

- [x] Update `SlicePlane` with dynamic anatomical flips
- [x] Pass flip masks/axis mapping to `shader.wgsl`
- [x] Align `picking.rs` and `overlays.rs` with orientation-aware logic
- [x] Unit tests for LAS/LPS/RAS orientation parity

---

## Milestone 1: Contour Data Structure [COMPLETE]
**Goal**: Store and display 2D contours  
**Deliverable**: Contours rendered in 2D views from labelmap

- [x] `Contour` struct (points, closed flag, segment_id)
- [x] `SliceContours` and `ContourSet` storage
- [x] Marching Squares: extract contour from labelmap slice
- [x] Render contours as line overlay in 2D views
- [x] Unit tests for Marching Squares

---

## Milestone 2: Mesh Data Structure [COMPLETE]
**Goal**: Display 3D mesh from labelmap  
**Deliverable**: Smooth mesh visible in 3D view

- [x] `SegmentationMesh` struct (vertices, normals, indices)
- [x] GPU buffer management (create/update)
- [x] Surface Nets algorithm
- [x] Mesh render pipeline (Phong shading)
- [x] `mesh.wgsl` fragment shader
- [x] Unit tests for Surface Nets

---

## Milestone 3: TSDF Infrastructure [COMPLETE]
**Goal**: Chunked TSDF for sync operations  
**Deliverable**: Convert labelmap ↔ TSDF

- [x] `Chunk` struct (32³ i8 values)
- [x] `ChunkedTSDF` with HashMap storage
- [x] Labelmap → TSDF conversion (distance transform)
- [x] TSDF → Labelmap export (threshold)
- [x] Unit tests for round-trip

---

## Milestone 4: TSDF Sync Pipeline [COMPLETE]
**Goal**: Sync TSDF to display representations  
**Deliverable**: 2D/3D views update reactively from TSDF

- [ ] Add `ChunkedTSDF` to `Segmentation` component
- [ ] Implement `TSDF → Surface Nets (Dirty Only)`
- [ ] Implement `TSDF → Slice → Contours`
- [ ] Reactive dirty tracking for chunks
- [ ] Sync system: watch TSDF for changes

---

## Milestone 5: TSDF Brush Tools [COMPLETE]
**Goal**: Modify TSDF directly via Brush  
**Deliverable**: 2D Brush and Eraser update TSDF chunks

- [ ] Port `paint.rs` to use TSDF instead of Labelmap
- [ ] Multi-label support in TSDF (optional/next)
- [ ] Sub-voxel brush falloff (soft edges)
- [ ] Live Mesh/Contour update during brush stroke

---

## Milestone 6: Advanced Contour Tools
**Goal**: Resolution-independent vector tools  
**Deliverable**: Spline/Polygon tools that commit to TSDF

- [ ] Tool selection UI (toolbar)
- [ ] Freehand draw tool
- [ ] Polygon/Spline tool
- [ ] Rasterize contour → TSDF commit logic

---

## Milestone 7: TSDF Sculpture (3D)
**Goal**: 3D sculpting on the mesh  
**Deliverable**: Push/pull tools in 3D view

- [ ] 3D sphere brush logic
- [ ] Push/Pull TSDF field in 3D
- [ ] Live re-mesh during sculpting

---

## Milestone 8: Polish & Advanced Tools
**Goal**: Production-ready with undo/export

- [ ] Undo/Redo (chunk deltas)
- [ ] Spline tool
- [ ] Threshold tool
- [ ] Island removal / fill holes
- [ ] NIfTI export
- [ ] STL/OBJ mesh export
- [ ] Worker thread for WASM

---

## Recommended Order

```
M1 (Contour) → M3 (TSDF) → M2 (Mesh) → M4 (Sync)
                                          ↓
                                    M5 (Basic Tools)
                                          ↓
                                    M6 (Contour Drag)
                                          ↓
                                    M7 (Contour Brush)
                                          ↓
                                    M8 (Polish)
```

## Related Documents

- [Architecture](./segmentation_architecture.md) — System design overview
- [PolySeg Original](./polyseg.md) — Initial WASM-first concept
