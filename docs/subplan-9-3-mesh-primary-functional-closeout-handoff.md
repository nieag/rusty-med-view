# Subplan 9.3 Mesh-Primary Functional Closeout

Purpose: make mesh-primary ROI workflow functional enough before performance/GPU/chunking work begins.

Subplan 9.2 closes voxel/contour workflow. Subplan 9.3 completes first mesh-primary roundtrip so mesh can be an authoritative representation, not display-only.

## Decisions

- First mesh roundtrip route:
  - `Mesh -> Voxel` by CPU mesh voxelization into target voxel grid.
  - `Mesh -> Contour` by `Mesh -> Voxel -> Contour`.
- Future higher-quality route may use SDF/TSDF. Do not implement SDF/TSDF in 9.3.
- Mesh deformation scope is minimal:
  - one simple edit operation enough to prove authoritative mesh mutation + derived rebuild.
  - full polished deformation UX deferred.
- 3D display remains mesh cache/surface.
- No 3D contour-polyline fallback.
- QA remains state/log based.

## Required User Workflow

1. Load image + voxel label.
2. Create mesh ROI from voxel/contour-derived voxel cache.
3. Make mesh ROI primary/active.
4. Apply minimal mesh edit.
5. Commit edit to authoritative mesh.
6. Request/rebuild voxel cache from mesh.
7. Request/rebuild contour view cache from voxel.
8. Inspect MPR + 3D; states are current/stale/rebuilding/blocked, never silent.

## Implementation Steps

### Step 9.3A: Mesh -> Voxel Contract and Target Grid

Files:

- `src/app/roi_runtime.rs`
- `src/convert/`
- `src/app/components.rs`

Deliver:

- choose target voxel grid:
  - prefer main volume geometry
  - fallback only if explicit and tested
- define mesh voxelization semantics:
  - closed mesh expected
  - boundary handling documented
  - invalid/open mesh returns structured error/blocker
- replace `MeshDerivedRebuildError::NotImplemented` for voxel rebuild path where possible

Tests:

- missing ROI
- non-mesh ROI
- missing target grid
- simple tetra/cube-like mesh voxelizes non-empty result
- invalid/open mesh fails safely

### Step 9.3B: CPU Mesh Voxelization V1

Files:

- new `src/convert/mesh_voxelize.rs` or equivalent
- `src/convert/mod.rs`

Deliver:

- deterministic CPU mesh-to-voxel conversion
- conservative/simple V1 acceptable if documented
- returns `VoxelData` with target geometry
- no GPU dependency

Tests:

- simple closed mesh fills expected approximate voxels
- empty mesh -> error or empty result per documented semantics
- output geometry preserved
- deterministic across runs

### Step 9.3C: Mesh -> Voxel Runtime Rebuild

Files:

- `src/app/roi_runtime.rs`
- `src/io/volume.rs` if GPU upload helper reuse needed

Deliver:

- process queued mesh-primary `RebuildVoxelCache`
- store CPU voxel cache
- upload GPU voxel cache when GPU resources available
- update bind groups
- request states transition stale/rebuilding/current

Tests:

- mesh edit dirties voxel cache
- rebuild creates current voxel cache
- GPU-missing path reports no-GPU blocker or CPU-current/GPU-missing state clearly

### Step 9.3D: Mesh -> Contour Through Voxel

Files:

- `src/app/roi_runtime.rs`
- existing `src/convert/voxel_contour_extract.rs`

Deliver:

- contour view cache requests for mesh-primary use current voxel cache
- if voxel cache missing/stale, report rebuilding/stale/blocker
- no direct mesh-plane slicing in 9.3

Tests:

- mesh-primary with current voxel cache can request orthogonal contour view cache
- stale/missing voxel cache blocks contour view with reason

### Step 9.3E: Minimal Mesh Edit Operation

Files:

- `src/app/roi_runtime.rs`
- `src/gui/` or `src/systems/` minimal UI/tool path

Deliver:

- one minimal mesh mutation operation:
  - e.g. translate selected mesh ROI vertices by small world-mm offset, or simple control-handle deformation
- mutation calls existing mesh authoritative update path
- dirty/rebuild voxel + contour caches
- clear status message

Tests:

- mutation changes mesh authoritative data
- voxel cache dirty after mutation
- contour view caches dirty/stale after mutation

### Step 9.3F: QA and Manual Closeout

Files:

- `src/app/mod.rs`
- `src/app/qa.rs`
- `tests/qa1_viewerqa.spec.js` if needed
- docs

Deliver:

- QA state shows mesh-primary request chain:
  - 3D mesh current
  - voxel cache current/stale/rebuilding
  - contour view current/stale/rebuilding/blocked
- manual checklist complete

Validation commands:

```bash
rtk cargo fmt --all
rtk cargo test -q
rtk cargo check --target wasm32-unknown-unknown -q
rtk cargo clippy --all-targets --all-features -- -D warnings
```

Manual checks:

- create mesh from voxel label
- edit mesh minimally
- rebuild voxel cache
- inspect MPR overlay/stats
- inspect contour view state
- inspect 3D mesh

## Do Not Do

- Do not implement SDF/TSDF.
- Do not optimize/chunk/GPU-accelerate voxelization in 9.3.
- Do not add full polished mesh deformation UX.
- Do not add screenshot/pixel QA.
- Do not add direct mesh-plane slicing unless mesh voxelization path is complete and scope remains controlled.

## Completion Criteria

- Mesh-primary ROI can be edited minimally.
- Mesh edit becomes authoritative.
- Mesh-derived voxel cache rebuilds.
- Mesh-derived contour views route through voxel cache.
- 2D MPR + 3D state remains coherent.
- Unsupported/invalid mesh cases report blockers, not silent failure.
