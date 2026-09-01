# ROI Multi-Representation Closeout Plan

## Goal

Finish ROI work as one coherent editing system:

- draw or deform authoritative contours in 2D
- see the active contour update immediately
- see other visible 2D planes and the 3D mesh update during interaction
- deform an authoritative mesh in 3D
- see intersecting 2D contours and voxel overlay update during interaction
- converge to generation-consistent voxel, contour, and mesh representations after commit
- keep representation loss, rebuild state, and unsupported geometry explicit

"Realtime" is a perceived-interaction requirement, not a requirement to rebuild every representation at 60 Hz:

- input, camera, active edit preview, and rendering sustain 60 FPS
- the active representation tracks pointer movement every frame
- visible derived previews start quickly enough to feel immediate, targeting 100 ms
- derived preview rebuilds may be throttled, coalesced, or skipped when superseded
- exact committed caches may converge asynchronously, but must not freeze input or display stale data as current

## Current State

Working now:

- one authoritative representation per ROI: voxel, contour, or mesh
- derived voxel, contour-view, and mesh cache slots with generation/dirty state
- ROI-owned geometry and main-display projection contract
- geometry-aware voxel overlays
- voxel-to-contour extraction, including requested orthogonal slices
- contour editing and contour-to-voxel rebuild
- voxel/contour-to-mesh creation
- native WGPU contour and mesh rendering
- explicit current/stale/rebuilding/blocked/unsupported request states
- mesh-to-voxel conversion, mesh-derived contours, and live mesh deformation previews
- dependency-aware preview jobs, dirty-region rebuilds, and bounded caches
- direct mesh-plane previews and incremental contour-to-mesh updates
- single-pass compositing for up to eight geometry-aware voxel ROI overlays

Missing for the target workflow:

- measured oblique edit latency and incremental rasterization if the full-contour fallback misses the interaction budget
- end-to-end visual and latency QA for bidirectional workflows

## Consistency Contract

Keep ADR 0001: exactly one authoritative primary representation per ROI.

Rules:

1. Pointer movement mutates editor preview state, not authoritative data.
2. Preview outputs carry the authoritative generation plus a preview revision.
3. Pointer release commits once and increments authoritative generation once.
4. Every derived result records source generation and optional preview revision.
5. Superseded results are discarded before cache commit or GPU upload.
6. Derived data may render as `Preview` or `Stale`, but only `Current` data is editable/promotable.
7. Primary switches remain explicit and may be lossy.
8. Failed conversion preserves last valid display data plus an explicit blocker.

Required cache states:

- `Current`
- `Preview`
- `Stale`
- `Queued`
- `Rebuilding`
- `Blocked { reason }`
- `Unsupported { reason }`

## Runtime Direction

Replace the single queued/running job slot with a dependency-aware work queue.

Each job needs:

- ROI entity/id
- source authoritative generation
- preview revision or committed marker
- input representation and output representation
- dirty region or requested view keys
- priority: interactive preview, visible committed view, background cache
- cancellation/supersession token

Dependency examples:

- contour edit -> voxel dirty region -> affected contour views + mesh chunks
- mesh deform -> direct visible plane intersections for preview
- mesh commit -> voxel dirty region/full voxelization -> contour views + overlay/stats
- voxel edit/import -> requested contour views + visible mesh chunks

Native builds may use worker threads. WASM must use Web Workers when available or time-sliced jobs with a strict per-frame budget. Conversion work must never run unbounded in render passes.

## Phase R1: Runtime and Preview Foundation

Deliver:

- `Preview` cache state and preview revision tracking
- multi-job queue with dependency ordering and supersession
- visible-view priority scheduling
- dirty voxel AABB, dirty contour slice keys, and dirty mesh chunk keys
- bounded contour-view cache with eviction
- native/WASM executor abstraction
- structured timing metrics for queue delay, conversion time, and commit latency

Acceptance:

- two requested derived jobs cannot overwrite each other
- stale generation and stale preview results never commit
- pointer input remains responsive while jobs run
- cache growth remains bounded while scrolling all slices

## Phase R2: Mesh-Primary Correctness Roundtrip

This supersedes the narrow Subplan 9.3 implementation sequence.

Deliver:

- closed-mesh validation and structured open/non-manifold blockers
- deterministic mesh-to-voxel conversion into explicit target geometry
- mesh voxel rebuild with CPU cache, GPU upload, overlay/stats update, and generation checks
- mesh-to-contour through current voxel cache for exact committed state
- minimal tested mesh deformation operation

Implementation rule:

- prefer a proven Rust geometry/voxelization implementation with native and WASM support
- isolate library types behind `src/convert/`
- do not place voxelization in render or GUI code

Acceptance:

- mesh edit -> voxel overlay/stats -> orthogonal contour views reaches `Current`
- invalid mesh fails without corrupting previous caches
- mismatched target geometry converts through world space correctly

## Phase R3: Live Mesh-to-2D Preview

Do not wait for full voxelization to update visible 2D planes.

Deliver:

- tested mesh/plane intersection producing ordered 2D contour loops
- intersection support for axial, coronal, sagittal, and oblique displayed planes
- WGPU rendering of preview loops through existing contour batches/scissors
- throttled/coalesced visible-plane recomputation during mesh drag; it need not run every frame
- exact voxel-mediated contours replace preview loops after commit

Acceptance:

- mesh vertices render every drag frame while derived plane intersections update at a frame-safe cadence
- visible 2D intersection previews update within 100 ms on QA sample
- preview loops are visibly/statefully distinguished from committed current loops
- exact post-commit contours replace previews without jumping to another world plane

## Phase R4: Live Contour-to-Mesh and Cross-Plane Preview

Deliver:

- contour edits identify affected voxel slabs/AABB
- incremental contour rasterization for changed slices only
- affected orthogonal/oblique contour-view invalidation only
- chunked voxel-to-mesh extraction with halo/seam rules
- rebuild only mesh chunks intersecting dirty voxel bounds
- stable GPU mesh-buffer update for changed chunks

Use current deterministic surface extraction for correctness first. Replace it with a proven smooth isosurface implementation only behind the same chunk contract after latency and geometry tests pass.

Acceptance:

- active contour follows drag every frame while cross-plane/mesh work updates at a frame-safe cadence
- cross-plane previews and mesh begin updating within 100 ms on QA sample
- unaffected mesh chunks and contour views keep their current generation
- commit produces the same result as a clean full rebuild

## Phase R5: Editing UX and Representation Promotion

Deliver:

- explicit edit mode: contour, mesh, navigation
- clear active authoritative representation and derived-state indicators
- one-command promotion of a current derived contour view or mesh to authority
- disabled edit actions with concrete stale/rebuilding/blocked reasons
- cancel/revert interaction restoring last committed authority
- undo/redo records authoritative commits, not intermediate preview frames
- visibility, color, opacity, lock, and active selection work across all representations

Acceptance:

- user cannot accidentally create two authoritative representations
- undo restores all derived state through normal invalidation/rebuild
- hidden ROIs produce no voxel, contour, or mesh scene geometry

## Phase R6: Multi-ROI Rendering and Oblique Completion

Deliver:

- replace two-overlay bind limit with scalable per-ROI compositing or texture-array strategy
- active-first ordering remains deterministic
- multiple visible contours and meshes render; only active ROI is editable
- geometry-aware oblique voxel overlay sampling
- oblique contour preview/cache behavior uses same state contract

Acceptance:

- at least eight visible QA ROIs render with stable colors/opacity and no silent truncation
- every viewport uses renderer-owned clipping
- orthogonal and oblique image/voxel/contour projections agree in world space

## Phase R7: QA and Closeout

Automated scenarios:

1. Voxel-primary import -> contour views -> mesh.
2. Contour draw/drag -> cross-plane preview -> mesh preview -> exact commit.
3. Mesh deform -> 2D intersection preview -> voxel/contour exact commit.
4. Promotion between representations with generation invalidation.
5. Rapid repeated edits proving superseded jobs cannot commit.
6. Geometry-mismatched main volume and ROI.
7. Hole/nested-loop topology and disconnected components.
8. Multi-ROI visibility, locking, selection, and compositing.
9. Native and WASM parity.

Performance gates on standard QA sample:

- active edit rendering: sustain 60 FPS (16.7 ms frame budget); no conversion task blocks the frame for more than 4 ms
- first derived preview: <= 100 ms target, <= 200 ms hard gate
- subsequent derived previews may be coalesced or throttled; latest preview must replace superseded work
- exact committed convergence: <= 1 s target, <= 3 s hard gate
- bounded cache/job memory during full-volume scrolling and repeated edits

Validation:

- unit/property tests for conversion and generation rules
- native integration tests for job dependencies and cancellation
- WASM compile and browser QA
- structured QA state/log assertions
- Playwright screenshots for representative contour-edit, mesh-edit, oblique, and multi-ROI states
- canvas pixel checks for nonblank/clipped WGPU output; screenshots remain supplementary to state assertions

## Checkpoints

Each phase lands as a focused checkpoint with:

- code and regression tests
- implementation-status update in this document and the canonical plan
- native tests, WASM check, clippy
- browser QA for rendering/UI phases
- screenshots or short clips for visible behavior changes

Recommended order:

1. R1 runtime foundation
2. R2 mesh correctness roundtrip
3. R3 live mesh-to-2D preview
4. R4 live contour-to-mesh/cross-plane preview
5. R5 editing UX
6. R6 multi-ROI/oblique completion
7. R7 performance and closeout QA

Do not call ROI work complete before both contour-authoritative and mesh-authoritative live workflows pass R7 gates.

## Implementation Status

State: in progress.

Current checkpoint: manual R6 oblique acceptance and R7 measured performance/final QA.

Implementation checkpoints:

- `90d94ae` R6: implement bidirectional ROI editing closeout

Completed prerequisites:

- Subplans 1 through 9.2
- post-9.2 spatial, topology, cache-state, visibility, and clipping fixes
- R1 preview/job/cache runtime foundation:
  - monotonic preview revisions and explicit preview state
  - priority/dependency-aware multi-job queue
  - generation/preview supersession and dirty-region metadata
  - bounded contour-view caches and structured job metrics
  - queue state exposed through runtime/QA
- R2 mesh-primary correctness roundtrip:
  - closed/topology-validated mesh-to-voxel conversion through target-grid index space
  - CPU/GPU mesh-derived voxel rebuild and overlay/stat refresh
  - mesh-derived contour views through current voxel cache
  - minimal committed mesh translation operation
- R3 mesh-to-2D preview engine:
  - non-authoritative mesh edit preview with commit/cancel
  - immediate WGPU mesh preview
  - direct mesh-plane intersections for visible 2D contour previews
  - exact voxel-mediated reconciliation after commit
  - mesh commit accepts closed, orientation-balanced voxel shells that touch along an edge
  - real liver-sample preview, cancel, and translated commit verified in hardware WGPU browser QA
- editing/UI robustness:
  - physical cursor coordinates always reach scene input even when egui consumes pointer motion, preventing stale click placement
  - contour point selection no longer starts or commits a move before a two-pixel drag threshold
  - committed contour moves are regression-tested through current voxel-cache convergence
  - toolbar status messages render on a separate row instead of overlapping tool controls
- automatic voxel/contour-primary derived mesh rebuild for 3D
- R4 correctness preview path:
  - contour point drag schedules coalesced preview revisions without mutating authority
  - orthogonal preview rasterization updates only the changed contour slice
  - committed Add Loop, point move, and point insertion reuse the retained voxel cache and rasterize only the edited axial, coronal, or sagittal slab
  - preview voxel cache drives visible cross-plane contour caches
  - preview mesh cache renders in 3D and is discarded on commit/cancel
  - committed and preview mesh caches retain deterministic voxel-chunk provenance
  - old and preview contour bounds merge into a tight max-exclusive dirty voxel AABB
  - dirty mesh rebuilds replace only intersecting chunks with one-voxel neighbor ownership halo
  - incremental chunk output is regression-tested against a clean full-volume rebuild
  - ROI job timing uses the WASM-safe `web-time` clock
  - preview mesh extraction persists as resumable ECS work under a four-millisecond frame budget
  - committed voxel-to-mesh extraction also persists as resumable work under the frame budget and reuses retained chunks for slab-local contour commits
  - superseded preview revisions cancel before additional chunk work or cache commit
  - unchanged mesh chunks share immutable payloads instead of cloning vertex/face arrays
  - WGPU mesh buffers persist by ROI, viewport, and chunk key; unchanged projected chunks skip upload
  - QA render state exposes per-frame mesh chunk upload and reuse counts
- R5 bidirectional editing core:
  - explicit navigation, contour edit/draw, and mesh edit tools enforce the active authoritative representation
  - 3D mesh vertex picking uses the shared display projection context
  - radius/strength brush deformation derives every drag preview from committed authority
  - coincident face-local surface vertices receive identical deformation, preserving indexed surface seams
  - selected mesh vertices render as native WGPU handles inside the viewport scissor
  - mesh previews update direct visible-plane intersections before commit
  - pointer release commits authority once and queues exact voxel/contour convergence
  - cancel, tool switch, and ROI switch discard previews and pending commits
  - voxel-derived axial, coronal, or sagittal contours promote to authority on the same ROI
  - current derived meshes promote to authority on the same ROI; missing or stale caches are rejected
  - promotion preserves entity identity, metadata, layer settings, and compatible current caches
  - promotion advances authority generation once and activates the corresponding edit tool
  - authority can roundtrip voxel-to-contour-to-voxel and voxel-to-mesh-to-voxel on the same ROI
  - stale or missing target caches disable authority switches instead of silently promoting old data
  - Edit Points moves existing contour vertices; Add Loop creates additional closed loops
  - Add Loop performs planar boolean union, merges overlapping boundaries, preserves existing holes, and auto-closes an additive correction after it exits and re-enters the existing ROI
  - contour addition, point movement, and point insertion reproject through world space when the display and authoritative slice use different coplanar local frames
  - contour edit rebuild jobs carry the authoritative slice key, preventing cache updates from being filtered out by a display-frame key
  - liver QA manual acceptance confirms Add Loop closure, boolean addition, voxel fill, and click/result alignment
  - bounded contour/mesh undo and redo snapshots authoritative commits only; intermediate preview revisions and no-op commits do not create history
  - undo/redo restores authority as a new generation, clears transient editor previews/selections, and reuses normal derived-cache invalidation and rebuild paths
  - contour history retains slice-local dirty scope, so existing-slice undo/redo rerasterizes one slab and rebuilds affected mesh chunks instead of forcing a full-volume conversion
  - history falls back to a full rebuild when creating or removing an entire contour slice because no restored plane record exists for safe slab clearing
  - new commits clear redo history, while authority promotion clears incompatible history for that ROI
  - toolbar actions and standard `Cmd/Ctrl+Z`, `Cmd/Ctrl+Shift+Z`, and `Cmd/Ctrl+Y` shortcuts expose ROI undo/redo
  - shortcut polling avoids re-entering the locked egui context, preventing the WASM `parking_lot` runtime panic
- R6 multi-ROI voxel compositing:
  - main volume, eight label textures, and one shared LUT fit within the guaranteed sampled-texture budget and render in one viewport pass
  - each overlay slot retains independent dimensions, opacity, and main-index-to-ROI-index geometry transform
  - active ROI stays first; remaining overlays sort by stable ROI id
  - eight visible voxel ROIs are selected, while a ninth remains explicitly reported as truncated
  - Rust/WGSL uniform layout and the complete WGPU shader/bind-group contract are regression-tested headlessly
  - all visible contour ROIs render through native WGPU batches in deterministic active-first order
  - inactive contours retain per-ROI color and layer opacity but never receive edit handles, selection highlighting, drafts, or move previews
- R6 oblique spatial/rendering path:
  - oblique image and voxel sampling consume the shared CPU-derived `PlaneDefinition` basis instead of reconstructing independent WGSL geometry
  - samples outside the oriented display volume render black instead of clamping to edge voxels
  - requested oblique voxel contours extract into plane-local millimetres and use the normal current/stale/rebuilding cache contract
  - contour render selection finds any nonempty usable derived view, so an earlier empty orthogonal cache cannot suppress a valid oblique contour
  - current displayed oblique caches promote through the exact view key rather than unsupported full-family extraction
  - promoted oblique contours rebuild current voxels and retain all authoritative oblique planes
  - oblique dirty-slice jobs conservatively rebuild the full contour set until incremental arbitrary-plane rasterization exists
  - the ROI QA preset exposes axial, coronal, sagittal, RMB-rotatable oblique, and 3D viewports simultaneously
  - navigation and editing picks use each active viewport's aspect ratio, matching render projection in mixed-size layouts
- post-R6 cache stability: 3D viewport sync preserves a current derived mesh instead of immediately dirtying and requeueing it before render preparation

Pending:

- R4 measured 60 FPS and preview-latency gates on representative volumes
- R6 manual image/voxel/contour world-space alignment acceptance
- oblique incremental rasterization if measured full-contour rebuild latency misses the interaction gate
- R7 performance and closeout QA

Follow-on architecture cleanup and segmentation-tool transition planning is tracked in `docs/roi-authoring-runtime-cleanup-plan.md`.
