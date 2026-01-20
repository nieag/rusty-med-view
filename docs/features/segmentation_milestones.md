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

- [x] Add `ChunkedTSDF` to `Segmentation` component
- [x] Implement `TSDF → Surface Nets (Dirty Only)` *(IncrementalMesher)*
- [x] Implement `TSDF → Slice → Contours`
- [x] Reactive dirty tracking for chunks
- [x] Sync system: watch TSDF for changes

**Implementation Notes:**
- `IncrementalMesher` in `src/segmentation/algorithms/incremental_mesher.rs`
- Per-chunk meshing with `mesh_chunk()`, only dirty chunks updated via `update_dirty()`
- `flatten()` combines all chunk meshes into single GPU buffer
- Performance: O(dirty_chunks × 32³) vs O(volume³) — ~50-500x speedup

---

## Milestone 5: TSDF Brush Tools [COMPLETE]
**Goal**: Modify TSDF directly via Brush  
**Deliverable**: 2D Brush and Eraser update TSDF chunks

- [x] Port `paint.rs` to use TSDF instead of Labelmap
- [ ] Multi-label support in TSDF (optional/next)
- [ ] Sub-voxel brush falloff (soft edges)
- [x] Live Mesh/Contour update during brush stroke

---

## Milestone 5.5: Legacy Import Pipeline (Promote to Vector) [COMPLETE]
**Goal**: Convert voxel labelmaps into the vector-authoritative system  
**Deliverable**: Importing a .nii.gz labelmap generates initial 3D Vector Contours

- [x] Voxel-to-TSDF promotion (Physical space distance transform)
- [x] TSDF smoothing to remove staircase artifacts
- [x] Slice TSDF to generate initial `SpatialContour` set (Authority Handoff)
- [x] RDP Simplification (0.0001 tolerance) for sub-voxel precision
- [x] Multi-label handling (Separate TSDFs/Contour sets per label)

---

## Milestone 6: Vector-Authoritative Layer [COMPLETE]
**Goal**: Transition from slice-based to 3D-spatial contours  
**Deliverable**: 3D vector contours rendered dynamically in all views

- [x] `SpatialContour` struct (3D plane, origin/axes, 2D polyline, influence)
- [x] `VectorContourSet` storage in `Segmentation` component
- [x] Dynamic 2D projection: Project 3D contours onto active axial/coronal/sagittal planes
- [x] High-fidelity intersection solver (Point-box rendering for perpendicular planes)
- [x] Render projected vectors directly (bypass Marching Squares for these)
- [x] Unit tests for projection math (19+ algorithm tests passing)

---

## Milestone 7: Constraint-Driven TSDF Baking [COMPLETE]
**Goal**: Generate TSDF from vector constraints  
**Deliverable**: Vector contours influence the volumetric segmentation

- [x] Implement TSDF Baking: Project 3D vector contours into `ChunkedTSDF`
- [x] Sparse baking: Update only chunks intersecting contour influence AABBs
- [x] Conservative distance aggregation for overlapping constraints
- [x] Integrate with Brush tools (Via system sync)

---

## Milestone 8: Hybrid Reconstruction (Snapping)
**Goal**: Exact agreement between 3D mesh and vector contours  
**Deliverable**: Mesh vertices "snap" to authoritative vector boundaries

- [ ] Update `IncrementalMesher` with vertex snapping logic
- [ ] Snap mesh vertices to nearest vector constraint within influence radius
- [ ] SDF-gradient based normals for non-snapped regions
- [ ] Smooth interpolation between sparse vector constraints

---

## Milestone 9: Advanced Contour Tools
**Goal**: Production-grade vector editing  
**Deliverable**: Spline/Polygon tools with multi-view editing

- [ ] Spline/Bezier contour support
- [ ] Multi-view editing (edit contour in one view, see update in others)
- [ ] 3D Sculpting (Push/Pull) integrated with vector constraints
- [ ] Toolbar UI for tool selection

---

## Milestone 10: Production Polish
**Goal**: Workflow completeness and reliability

- [ ] Undo/Redo for `VectorContourSet` (Geometry-based deltas)
- [ ] NIfTI export (Rasterize TSDF to Labelmap)
- [ ] STL/OBJ mesh export
- [ ] Performance: Move TSDF baking and snapping to Compute Shaders (GPU)

---

## Recommended Order

```
M0-M5 (Foundation) → M5.5 (Import) → M6 (Vector Layer) → M7 (TSDF Bake) → M8 (Snapping)
                                                              ↓
                                                      M9 (Advanced Tools)
                                                              ↓
                                                        M10 (Polish & GPU)
```

## Related Documents

- [Architecture](./segmentation_architecture.md) — System design overview
- [PolySeg Original](./polyseg.md) — Initial project context
