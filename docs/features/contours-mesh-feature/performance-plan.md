# Performance Plan Re-Evaluation + Continuation (P4-next)

**Target**: 60+ fps in WASM on web (~16ms frame budget, single-threaded).

## Summary

The core CPU pipeline is now chunked and running. The live edit path uses:
contours → SDF (incremental ROI) → TSDF chunk bake → Surface Nets → merge.

What is done:

- Incremental SDF ROI updates.
- Persistent mesh GPU cache (no per-frame reupload when mesh unchanged).
- Chunked TSDF store (`tsdf_chunks` in `SegmentRuntimeCache`).
- Surface Nets replaces Marching Cubes in live and finalize paths.
- Per-chunk mesh cache with frame-budget-limited processing.
- TSDF chunks baked with +1 voxel overlap for seamless Surface Nets meshing.

Remaining gaps:

- Per-chunk GPU mesh resources (currently merged + re-uploaded).
- Compute-shader acceleration.
- Chunk-aware finalize queue.

Chosen continuation path:

- Chunked CPU first (✅ done), then GPU resource model, then compute offload.

## Implementation Status (February 15, 2026)

### Commit Log

| Commit | Phase | Description |
|--------|-------|-------------|
| `fb2e8dd` | A | Chunk runtime dirty-queue wiring from contour edits |
| `d8603f5` | A | Deferred finalize defaults + chunk meshing hooks |
| `3436f43` | A+B | Queue-driven derivative updates + TSDF chunk foundation |
| `faa8653` | C | Surface Nets isosurface extractor |
| `2576d23` | B-2 | TSDF-authoritative chunked pipeline with Surface Nets |

### Phase A — Chunk Runtime Model: ✅ Complete

- [x] Chunk primitives module (`src/convert/chunk_grid.rs`)
- [x] `SegmentRuntimeCache` with dirty queues (`src/app/segment.rs`)
- [x] Dirty marking wired in `finish_drawing` ROI → chunk enqueue
- [x] Queue-driven derivative execution as primary trigger path

### Phase B — TSDF-Authoritative Chunk Store: ✅ Complete

- [x] TSDF chunk data model and quantization (`src/convert/contour_to_tsdf_chunks.rs`)
- [x] `tsdf_chunks: HashMap<ChunkKey, TsdfChunk>` added to `SegmentRuntimeCache`
- [x] TSDF baking from SDF with +1 voxel overlap padding
- [x] Chunk recompute/invalidation pipeline integrated
- [ ] `Segment.sdf` retained for SDF preview pipeline (deferred removal)

### Phase C — Surface Nets Meshing: ✅ Complete

- [x] `surface_nets_chunk(tsdf_chunk)` reads quantized i16 directly
- [x] `surface_nets_from_sdf(sdf, iso_level, bounds)` full-volume wrapper
- [x] 5 unit tests (empty, all-inside, sphere, boundary ownership, parity)
- [x] Live path: Surface Nets on chunks with frame budget + merge
- [x] Finalize path: `surface_nets_from_sdf` full-volume extraction
- [x] MC removed from active pipeline (retained in codebase)

### Verification: ✅ Complete

- [x] Budget test: SDF <16ms/frame, mesh <3ms (release-gated)
- [x] Locality test: small edit ≤8 chunks
- [x] Partial processing test: frame budget respected mid-queue
- [x] Full test suite: 112 tests pass (debug + release)
- [x] WASM check: `cargo check --target wasm32-unknown-unknown` clean

### Phase D — GPU Resource Model: Not started
### Phase E — Compute Offload: Not started
### Phase F — Finalize/Export Path: Partial (deferred finalize exists, not yet chunk-aware)

### Phase G — Spatial Contours + TSDF Slice-Authoritative Rendering: In progress

Goal: keep contour editing high-fidelity while using derived SDF/TSDF geometry for
cross-view contour display consistency.

#### Decision (locked)

- Source-of-truth for editing remains authored contours.
- Source-of-truth for cross-view 2D contour display is derived field slicing
  (SDF/TSDF iso-contours), not raw contour projection.
- Raw authored contours render as overlays; derived isolines render as the
  anatomy-consistent contour in each slice viewport.

#### Implementation Status (February 15, 2026)

- [x] Add `SpatialPlane` / `SpatialContour` model primitives in `src/app/segment.rs`.
- [x] Enable oblique authored contour participation in 2D cross-plane render path
  (remove CPU skip in `src/render/pipeline.rs`).
- [x] Add active-segment SDF marching-squares isolines per viewport slice in
  `src/render/pipeline.rs` (performance bounded by per-view segment cap).
- [ ] Migrate derived contour extraction from monolithic `Segment.sdf` to chunked
  TSDF sampling path.
- [ ] Add compute-shader slice extraction path (WebGPU) with CPU fallback.
- [ ] Add oblique drawing-plane authoring path.

#### Performance Gates for Phase G

- Maintain interactive frame pacing under live edits (target <= 16.6ms p95 on
  representative WASM scenes).
- Avoid full-volume recompute in interactive loop; rely on bounded per-slice
  extraction cost and existing chunked derivative updates.
- Keep line buffer writes bounded (`MAX_CONTOUR_LINES`) and avoid invalid GPU
  writes on overflow.

#### Alignment Tightening Checkpoint (current branch)

- [x] Shared coordinate mapping helpers added (`src/convert/coord_mapping.rs`)
  and consumed by contour draw + contour render paths.
- [x] Slice/depth conversions deduplicated through shared helpers in:
  - `src/render/pipeline.rs`
  - `src/systems/contour_draw.rs`
  - `src/systems/input.rs`
- [x] 2D contour display locked to strict slice-following with fixed behavior
  (no toggle): authored contours on their exact source slice, derived SDF
  isolines for views/slices without authored contours.
- [x] Neighbor-slice SDF slab bridging is disabled by default
  (`SdfBuildConfig.neighbor_slice_bridging = false`) to avoid keyframe-style
  interpolation in 2D contour behavior.
- [x] Live derivative build enables neighbor bridging for volumetric continuity
  in mesh/cross-view derived contours (`src/systems/segment_system.rs`), while
  authored-slice overlays remain strict.
- [x] Derived-by-default display model wired with explicit per-slice overrides:
  edited slices are marked override and render authored contours; untouched
  slices render derived isolines.
- [x] Explicit slice override state added to `Segment` (`edited_slices`) instead
  of implicit authored-presence checks, so render behavior follows a stable
  data contract.
- [x] 2D derived isoline extraction moved from monolithic `Segment.sdf` sampling
  to chunked TSDF sampling (`Segment.chunk_runtime.tsdf_chunks` + TSDF grid
  metadata), aligning display with the authoritative volumetric runtime store.
- [x] Mapping invariants covered by new unit tests in
  `src/convert/coord_mapping.rs`.

Notes:
- Derived isolines now use TSDF chunk sampling for axis-aligned slice views.
- Monolithic `Segment.sdf` remains in runtime for compatibility with the SDF
  preview pipeline and CPU build/update orchestration.
- Frame budget target remains hard gate (`<= 16.6ms p95`) for interactive mode.

## Algorithm Decision: Surface Nets Everywhere

Medical segmentation produces organic shapes (organs, tumors) — no sharp features.
Compared three isosurface extractors for the 16ms WASM budget:

| | Marching Cubes | Surface Nets | Dual Contouring |
|---|---|---|---|
| Triangle count | High (staircase) | ~40-60% fewer | Fewest |
| Quality | Staircase artifacts | Smooth | Sharp feature preservation |
| Per-cell cost | Low | Low | 3-5× (QEF solve) |
| Best for | General | Organic shapes ✅ | CAD/hard edges |

**Decision**: Surface Nets everywhere. MC stays in codebase but is removed from the
active pipeline. DC is overkill for organic medical shapes.

## Current Baseline (facts from code)

- Mesh generation method (being replaced): marching_cubes_with_options(...) in
  src/convert/marching_cubes.rs.
- SDF store (being replaced): monolithic SdfVolume (Vec<f32>, full volume) in
  src/app/segment.rs. Will be superseded by chunked TsdfChunk (Vec<i16>) as the
  persistent authoritative store.
- Incremental SDF function exists:
  update_sdf_region_from_contours_with_config(...) in src/convert/
  contour_to_sdf.rs. Will be scoped to chunk-sized temp buffers.
- Live/final orchestration in src/systems/segment_system.rs.
- Render-side mesh caching exists in src/render/pipeline.rs + src/app/context.rs.
- Perf settings still exist in SegPerfConfig, though UI now hard-forces live
  behavior.

## Phase A — Introduce Chunk Runtime Model (no algorithm swap yet)

1. Add chunk primitives:

- New module: src/convert/chunk_grid.rs
- Types:
  - ChunkKey { x: i32, y: i32, z: i32 }
  - ChunkBounds { min: [u32;3], max: [u32;3] }
  - ChunkSet utilities for ROI->chunk mapping.

2. Extend Segment runtime:

- Add runtime: SegmentRuntimeCache in src/app/segment.rs.
- SegmentRuntimeCache:
  - dirty_tsdf_chunks: VecDeque<ChunkKey>
  - dirty_mesh_chunks: VecDeque<ChunkKey>
  - mesh_chunks_cpu: HashMap<ChunkKey, MeshData>
  - mesh_chunks_gpu_revision: HashMap<ChunkKey, u64>
  - chunk_size: u32 (fixed default, no UI knob).

3. Dirty marking:

- In finish_drawing flow (src/systems/segment_system.rs), convert stroke world
  AABB to chunk keys and enqueue only intersected chunks.

## Phase B — TSDF-Authoritative Chunk Store (CPU)

TsdfChunk replaces the monolithic SdfVolume as the persistent authoritative
representation. SdfVolume becomes a transient per-chunk compute buffer.

1. Existing module: src/convert/contour_to_tsdf_chunks.rs.
2. Data type (already implemented):

- TsdfChunk { dims:[u32;3], voxel_size:[f32;3], values: Vec<i16>, weight:
  Vec<u8>, revision:u64 }.

3. Architecture change:

- Remove Segment.sdf: Option<SdfVolume> as persistent field.
- Add tsdf_chunks: HashMap<ChunkKey, TsdfChunk> to SegmentRuntimeCache.
- Per dirty chunk: allocate small temp SdfVolume (32³ = 128KB), compute contour
  distances into it, quantize via build_tsdf_chunk_from_sdf, drop temp buffer.
- GPU-forward: i16 maps directly to R16Sint textures for compute offload.

4. Integration:

- sys_update_segment_derivatives becomes queue-driven:
  - per frame: limited TSDF chunks processed under budget (≤3ms).
  - enqueue corresponding mesh chunks when TSDF chunk changed.

## Phase C — Surface Nets Meshing (CPU, replaces Marching Cubes)

1. New module: src/convert/surface_nets.rs.
2. Implement full Surface Nets extractor:

- surface_nets_chunk(tsdf_chunk) -> MeshData.
- surface_nets_from_sdf(sdf, iso_level) -> MeshData (full-volume convenience wrapper).
- Deterministic boundary ownership rule to prevent cracks.
- Dual vertex at weighted average; central-difference gradient normals.
- Primary path reads from TsdfChunk (dequantize i16 → f32 on the fly).

3. Mesh assembly:

- Do not rebuild monolithic mesh every update.
- Keep per-chunk mesh cache and only rebuild touched chunks.
- merge_chunk_meshes assembles per-chunk caches into segment.mesh.

4. Pipeline swap:

- Replace marching_cubes_with_options calls in segment_system.rs.
- Both live and finalize use Surface Nets (differ by resolution/budget, not algorithm).
- MC code retained in codebase but removed from active pipeline.

## Phase D — GPU Resource Model for Chunk Meshes

1. Add ChunkMeshResources in src/render/mesh_pipeline.rs:

- persistent vertex/index buffers per chunk.
- revision-gated upload only when chunk mesh changed.

2. Render list:

- SegmentGpuCache becomes per-segment + per-chunk draw records.

3. Culling:

- Basic frustum + viewport visibility culling at chunk granularity.

## Phase E — Compute Offload (after CPU chunk pipeline is stable)

1. New compute module(s): src/render/compute/tsdf_compute.rs and optionally
   surface_nets_compute.rs.
2. Offload order:

- First TSDF update kernel (highest predictable win).
- Then optional Surface Nets assist (classification/compaction), keep CPU
  fallback.

3. Runtime capability:

- Detect WebGPU compute/storage support and branch cleanly.
- Maintain single-thread WASM architecture (no workers required).

## Phase F — Finalize/Export Path

1. Finalize uses high-resolution Surface Nets from converged TSDF.
   (Same algorithm as live path, higher resolution multiplier.)

2. Finalize runs as background queue slices under frame budget; never blocks
   interaction loop.

## Public API / Interface Changes

- src/app/segment.rs:
  - add SegmentRuntimeCache, chunk queues, chunk mesh caches.
- src/convert/mod.rs:
  - export contour_to_tsdf_chunks, surface_nets.
- src/systems/segment_system.rs:
  - replace global dirty booleans as primary driver with chunk queues.
  - add queue-budget processing helpers.
- src/render/mesh_pipeline.rs / src/render/pipeline.rs:
  - add chunk mesh GPU resource APIs and per-chunk draw submission.

## Test Cases and Scenarios

1. Unit:

- ROI->chunk mapping correctness.
- Chunk seam continuity (TSDF values across borders).
- Surface Nets chunk edge ownership (no cracks for adjacent chunks).

2. Integration:

- Small sculpt edit updates only nearby chunks (assert number of processed
  chunks).
- Continuous drawing does not trigger full-volume remesh.
- Mesh remains visible and stable while editing.

3. Performance regression:

- Timed scripted stroke benchmark:
  - p50 and p95 edit-to-visible latency.
  - per-frame max processed chunks bounded by budget.

4. WASM:

- cargo check --target wasm32-unknown-unknown.
- runtime smoke on WebGPU path + CPU fallback path.

## Acceptance Criteria

- No multi-second UI freeze on small contour edits.
- Live edit path updates only local chunks.
- GPU uploads for unchanged chunks are zero.
- Perceived interaction remains fluid during drawing and sculpting.
- Finalize quality remains available without blocking live edits.

## Assumptions and Defaults

- Primary target: modern desktop WebGPU in WASM.
- Priority: speed/responsiveness over live-path perfect shading.
- No new user-facing performance settings in this phase.
- Single-threaded WASM model retained.
