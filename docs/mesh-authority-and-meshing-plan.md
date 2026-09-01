# Mesh Authority and Smooth Meshing Plan

Status: Active

## Outcome

Keep the existing one-ROI/one-authority model while making mesh-authority
editing coherent and voxel/contour-authority meshes smooth enough for useful
3D inspection.

```text
ROI identity + immutable reference geometry
authority: contour | voxel | mesh
other forms: derived, generation-checked caches
```

Authority follows the committed editing tool. Conversion never silently
changes authority and is not assumed lossless.

## Representation contract

| Authority | Authoring/display source | Derived forms |
| --- | --- | --- |
| Contour | committed contour geometry | voxel cache; smooth mesh cache; contour views |
| Voxel | owned-grid binary label | contour views; smooth mesh cache |
| Mesh | committed world-mm triangle mesh | direct plane-intersection contour views; optional voxel cache/overlay/export |

For a mesh-authority ROI, 2D contours always come from direct mesh-plane
intersection. A rebuilt voxel cache is an explicitly quantized overlay/export
form and must not replace the displayed mesh contour.

An SDF, if introduced, is an internal derived helper for generating smooth
meshes from voxel data. It is not a fourth user-facing authority in this
milestone.

## Delivery order

### Phase 1: mesh-authority 2D consistency

- Preserve direct mesh-plane contour views after mesh-preview commit.
- Keep voxel-cache rebuilding asynchronous and independent of the displayed
  mesh contour.
- Add a regression scenario: deform mesh, commit, and verify the same plane
  remains direct mesh-derived rather than snapping to voxel extraction.

Acceptance: mesh deformation no longer changes the 2D contour representation
when the pointer is released.

### Phase 2: smooth derived mesh for voxel/contour authority

- Add a CPU signed-distance helper derived from the owned voxel grid in world
  millimetres.
- Replace the cube-face surface extractor used for the normal derived mesh
  path with a topology-aware marching-cubes implementation.
- Preserve ROI geometry, current cache generation rules, chunking/budgeting,
  and WGPU mesh rendering.
- Retain the current cubical extractor only while it supports focused
  regression comparisons; remove it once replacement coverage is sufficient.

Acceptance: a voxel/contour ROI produces a world-mm, watertight, visibly
smooth derived mesh; expected voxel round-trips remain stable on representative
fixtures.

### Phase 3: mesh deformation interaction

- Pick a triangle surface hit instead of only a nearest projected vertex.
- Apply the brush over a coherent connected surface neighbourhood.
- Validate/reject invalid mesh states before voxel resampling.

Acceptance: a brush gesture affects the visually selected local surface region
and mesh-authority 2D views remain coherent before and after commit.

### Phase 4: QA and decision

- Exercise contour, voxel, and mesh authority on axial, oblique, and 3D views.
- Include anisotropic and rotated ROI reference geometries.
- Measure mesh rebuild and voxel-resample timing on the QA liver sample.
- Decide whether mesh-authority voxelization is suitable for stable v0 export
  or remains an explicitly provisional/resampled operation.

Manual GPU-browser acceptance capture:

1. Open `/?qa=1&sample=liver_0&preset=image_label_mpr_basic`, create or
   promote a mesh-authority ROI, then deform and commit it in the 3D viewport.
2. Confirm axial, oblique, and 3D views remain coherent while the queue drains;
   in particular, 2D contours must stay direct mesh-plane intersections rather
   than snap to the resampled voxel cache.
3. After `pending_jobs` and `running_job` are empty, record the active ROI from
   `window.__viewerQa.state().rois`: `last_mesh_voxelization_ms`,
   `last_gpu_upload_ms`, `last_work_convergence_ms`, `last_job_duration_ms`,
   and failed/discarded counts.
4. Repeat once with an anisotropic or rotated ROI geometry fixture. The
   headless browser contract is not performance evidence because it has no
   WebGPU adapter.

Decision: keep mesh-authority voxelization as an explicitly provisional,
reference-grid resample for v0. It samples a closed world-mm mesh at the
ROI-owned voxel centres for overlays and downstream voxel workflows; it is not
lossless mesh export and must never replace mesh-authority contour display.

## Non-goals

- Replacing the authority model with voxel-only or SDF-only editing.
- GPU compute migration before a measured conversion bottleneck.
- Adaptive dual contouring, octrees, global remeshing, or a general sculpting
  framework.
- Changing the shared coordinate contract or ECS architecture.

## Implementation status

Current phase: Phase 4 QA and performance decision.

Completed:

- [x] Confirmed the authority-following-tool model.
- [x] Identified cube-face extraction as the source of blocky derived meshes.
- [x] Identified the mesh-preview/direct-intersection versus post-commit
  voxel-contour switch as the likely 2D deformation discontinuity.
- [x] Added 3D orientation, orthographic projection, overlay alignment, and
  zoom-redraw coalescing fixes before this milestone.
- [x] Phase 1: mesh-authority contour views stay direct mesh-plane
  intersections while voxelization is queued and after it completes.
- [x] Phase 2: CPU signed Euclidean distance field uses the ROI-owned grid and
  world-mm spacing.
- [x] Phase 2: keep the marching-cubes kernel local and testable; no meshing
  crate dependency.
- [x] Phase 2: local indexed marching-cubes extraction from a padded SDF,
  including an expected voxel round-trip fixture.
- [x] Phase 2: normal and preview mesh-cache rebuilds use the shared SDF with
  the existing chunked frame-budget scheduler; legacy cube-face extraction was
  removed from the normal path.
- [x] Phase 3: mesh deformation picks the clicked triangle surface and uses a
  connected, world-mm geodesic brush, so nearby disconnected surfaces are not
  deformed together.
- [x] Phase 3: committing a mesh preview validates closed-mesh topology before
  cache voxelization; an invalid preview remains available for correction.
- [x] Phase 4: live Trunk browser QA contract passes (3/3); it verifies the
  QA state schema and the honest no-WebGPU fallback in this environment.
- [x] Phase 4: automated smooth-mesh to voxel resampling round-trip preserves
  a rotated, anisotropic ROI grid exactly.
- [x] Phase 4: mesh-authority voxelization remains an explicit reference-grid
  resample for v0, not a lossless mesh export or display source.

Pending:

- [ ] Phase 4 QA/performance decision.

Checkpoint commits:

- `b9a177e` Perf: coalesce rapid 3D zoom redraws
- `67a2037` Fix: preserve direct contours for mesh authority
- `fab1421` Feat: add ROI signed distance conversion
- `d7cd04e` Feat: add local SDF marching cubes extractor
- `b1e7ab0` Feat: use smooth meshes in chunked ROI rebuilds
- `d28f022` Feat: make mesh brush surface-aware
- `af658ad` Test: cover rotated mesh voxel roundtrip
