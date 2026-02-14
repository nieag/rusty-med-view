# Performance Plan Re-Evaluation + Continuation (P4-next)

**Target**: 60+ fps in WASM on web (~16ms frame budget, single-threaded).

## Summary

Current state is improved but still not at "snappy" target because the core live
pipeline is still contours -> SDF -> Marching Cubes on whole active ROI per edit.
What is done:

- Incremental SDF ROI updates landed.
- Persistent mesh GPU cache landed (no per-frame reupload when mesh unchanged).
- Live mesh always-on, live path uses cheaper normals.
- Finalize defers briefly after edits to reduce immediate stalls.

Main gap:

- No chunked TSDF store.
- No per-chunk mesh cache/invalidation.
- No Surface Nets extractor (replacing Marching Cubes).
- No compute-shader acceleration yet.

Chosen continuation path:

- Chunked CPU first, then GPU compute offload.
- Surface Nets replaces Marching Cubes everywhere (see Algorithm Decision below).

## Implementation Status (February 14, 2026)

- Latest plan commits:
  - `fb2e8dd` Phase A chunk runtime dirty-queue wiring from contour edits.
  - `d8603f5` deferred finalize defaults + chunk meshing hooks retained.
  - `3436f43` Phase A queue-driven derivative updates + Phase B TSDF chunk foundation.

- Phase A status: complete.
  - [x] Chunk primitives module added (`src/convert/chunk_grid.rs`).
  - [x] `ChunkKey` and `ChunkBounds` available.
  - [x] Segment runtime cache added (`SegmentRuntimeCache` in `src/app/segment.rs`).
  - [x] Dirty TSDF + dirty mesh chunk queues added.
  - [x] Dirty marking wired in `finish_drawing(...)` ROI -> chunk enqueue.
  - [x] Queue-driven derivative execution is now wired as primary trigger path.

- Phase B status: started.
  - [x] TSDF chunk data model and quantization helpers added (`src/convert/contour_to_tsdf_chunks.rs`).
  - [x] `regenerate_live_chunk_meshes` + `merge_chunk_meshes` implemented (dead code, needs activation).
  - [ ] Chunk recompute/invalidation pipeline not yet integrated (queues cleared without processing).
- Phase C status: not started (Surface Nets implementation pending).
- Phase D status: not started.
- Phase E status: not started.
- Phase F status: partial (deferred finalize behavior exists, but not chunk/final queue architecture).

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
