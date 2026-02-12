# Performance Plan Re-Evaluation + Continuation (P4-next)

## Summary

Current state is improved but still not at “snappy” target because the core live
pipeline is still contours -> SDF -> Marching Cubes on whole active ROI per edit.
What is done:

- Incremental SDF ROI updates landed.
- Persistent mesh GPU cache landed (no per-frame reupload when mesh unchanged).
- Live mesh always-on, live path uses cheaper normals.
- Finalize defers briefly after edits to reduce immediate stalls.

Main gap:

- No chunked TSDF store.
- No per-chunk mesh cache/invalidation.
- No Surface Nets live extractor.
- No compute-shader acceleration yet.

Chosen continuation path:

- Chunked CPU first, then GPU compute offload.

## Implementation Status (February 12, 2026)

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
  - [ ] Chunk recompute/invalidation pipeline not yet integrated.
- Phase C status: not started.
- Phase D status: not started.
- Phase E status: not started.
- Phase F status: partial (deferred finalize behavior exists, but not chunk/final queue architecture).

## Current Baseline (facts from code)

- Mesh generation method: marching_cubes_with_options(...) in src/convert/
  marching_cubes.rs.
- Incremental SDF function exists:
  update_sdf_region_from_contours_with_config(...) in src/convert/
  contour_to_sdf.rs.
- Live/final orchestration in src/systems/segment_system.rs.
- Render-side mesh caching exists in src/render/pipeline.rs + src/app/context.rs.
- Perf settings still exist in SegPerfConfig, though UI now hard-forces live
  behavior.

## Phase A — Introduce Chunk Runtime Model (no algorithm swap yet)

1. Add chunk primitives:

- New module: src/seg/chunk_grid.rs
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

## Phase B — Chunked TSDF Cache (CPU)

1. New module: src/convert/contour_to_tsdf_chunks.rs.
2. Data type:

- TsdfChunk { dims:[u32;3], voxel_size:[f32;3], values: Vec<i16>, weight:
  Vec<u8>, revision:u64 }.

3. Behavior:

- Recompute only dirty chunks from contours (banded TSDF, truncated).
- Use overlap halo (1 voxel) at chunk borders for seam continuity.
- Keep compact quantized storage (i16 TSDF normalized by truncation distance).

4. Integration:

- sys_update_segment_derivatives becomes queue-driven:
  - per frame: limited TSDF chunks processed under budget.
  - enqueue corresponding mesh chunks when TSDF chunk changed.

## Phase C — Surface Nets Live Meshing (CPU)

1. New module: src/convert/surface_nets.rs.
2. Implement per-chunk extraction:

- surface_nets_chunk(tsdf_chunk, neighbors_halo) -> MeshData.
- Deterministic boundary ownership rule to prevent cracks.

3. Mesh assembly:

- Do not rebuild monolithic mesh every update.
- Keep per-chunk mesh cache and only rebuild touched chunks.

4. Render path:

- Add chunk mesh draw submission path in src/render/pipeline.rs.
- Keep existing monolithic path temporarily for fallback/finalize.

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

1. Keep finalize distinct:

- Option A (default): higher-quality Marching Cubes from converged TSDF.
- Option B: high-res Surface Nets.

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
