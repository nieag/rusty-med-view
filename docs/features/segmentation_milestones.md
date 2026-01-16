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

## Milestone 3: TSDF Infrastructure
**Goal**: Chunked TSDF for sync operations  
**Deliverable**: Convert labelmap ↔ TSDF

- [ ] `Chunk` struct (32³ i8 values)
- [ ] `ChunkedTSDF` with HashMap storage
- [ ] Labelmap → TSDF conversion (distance transform)
- [ ] TSDF → Labelmap export (threshold)
- [ ] Unit tests for round-trip

---

## Milestone 4: Sync Pipeline
**Goal**: 2D edits update 3D mesh  
**Deliverable**: Modify contour → mesh updates

- [ ] Contour → rasterize → TSDF
- [ ] TSDF → Surface Nets → Mesh (dirty chunks only)
- [ ] Mesh → plane intersection → Contour
- [ ] Dirty region tracking
- [ ] Integration test: contour edit → mesh update

---

## Milestone 5: Basic Contour Edit Tools
**Goal**: Draw new contours in 2D  
**Deliverable**: Freehand and polygon tools work

- [ ] Tool selection UI (toolbar)
- [ ] Freehand draw tool
- [ ] Polygon tool
- [ ] Fill/rasterize on commit
- [ ] Contour hover/select feedback

---

## Milestone 6: Contour Drag Tool
**Goal**: Grab and drag contour boundaries  
**Deliverable**: Drag contour with 3D falloff visible in mesh

- [ ] Contour hit-testing (detect grab)
- [ ] Drag state tracking
- [ ] Update contour on drag
- [ ] 3D gaussian falloff via TSDF
- [ ] Re-mesh + re-slice adjacent contours
- [ ] Live update during drag

---

## Milestone 7: Contour Brush
**Goal**: Push/pull contour like sculpting  
**Deliverable**: Expand/contract/smooth modes

- [ ] Brush cursor preview
- [ ] Expand mode (push outward)
- [ ] Contract mode (push inward)
- [ ] Smooth mode (blur boundary)
- [ ] Brush size slider

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
