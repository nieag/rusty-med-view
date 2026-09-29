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
   `last_completed_job`, and failed/discarded counts.
4. Repeat once with an anisotropic or rotated ROI geometry fixture. The
   headless browser contract is not performance evidence because it has no
   WebGPU adapter.

Measured liver mesh-authority resample before deferral: `969 ms` mesh
voxelization, `1 ms` GPU upload, and `972 ms` accepted-work convergence, with
no failed/discarded jobs. The resample scans the mesh voxel bounds
in one render frame at this checkpoint. The explicit rebuild now scans under
the ROI work scheduler's frame budget; mesh commits still do not request it.

Decision: keep mesh-authority voxelization as an explicitly provisional,
reference-grid resample for v0. It samples a closed world-mm mesh at the
ROI-owned voxel centres for overlays and downstream voxel workflows; it is not
lossless mesh export and must never replace mesh-authority contour display.
Mesh commits therefore leave that cache stale and do not enqueue a rebuild.
The sidebar's explicit **Build voxels** action requests the potentially slow
resample when the voxel overlay or voxel authority is actually needed. Mesh
deformation commits now require a valid closed, non-self-intersecting surface;
invalid previews are discarded without changing authority or undo history.
**Build voxels** independently validates its input before sampling.

## Non-goals

- Replacing the authority model with voxel-only or SDF-only editing.
- GPU compute migration before a measured conversion bottleneck.
- Adaptive dual contouring, octrees, global remeshing, or a general sculpting
  framework.
- Changing the shared coordinate contract or ECS architecture.

## Implementation status

Current phase: Phase 4 GPU QA and live usability verification. The milestone
is not complete: a liver deform/build test found no tears, but scattered voxel
output and the browser impact of the rebuild-pause fix still need confirmation.

Completed:

- [x] Confirmed the authority-following-tool model.
- [x] Identified cube-face extraction as the source of blocky derived meshes.
- [x] Identified the mesh-preview/direct-intersection versus post-commit
  voxel-contour switch as the likely 2D deformation discontinuity.
- [x] Added 3D orientation, orthographic projection, overlay alignment, and
  zoom-redraw coalescing fixes before this milestone.
- [x] Phase 1: mesh-authority contour views stay direct mesh-plane
  intersections while voxelization is queued and after it completes.
- [x] Phase 1: the direct-contour regression covers both axial and genuinely
  oblique planes before and after mesh voxel resampling.
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
- [x] Phase 3: overlapping projected faces select the frontmost containing
  surface; the interaction regression clicks inside a face rather than a
  vertex.
- [x] Phase 3: explicit voxel-cache rebuilding validates mesh topology and
  intersections. Mesh-preview commits now use the same validator and reject
  invalid edits before changing the authoritative mesh or history.
- [x] Phase 4: live Trunk browser QA contract passes (3/3); it verifies the
  QA state schema and the honest no-WebGPU fallback in this environment.
- [x] Phase 4: automated smooth-mesh to voxel resampling round-trip preserves
  a rotated, anisotropic ROI grid exactly.
- [x] Phase 4: mesh-authority voxelization remains an explicit reference-grid
  resample for v0, not a lossless mesh export or display source.
- [x] Phase 4: liver GPU-browser QA measured `969 ms` for the eager mesh
  resample; mesh commits now defer it until an explicit **Build voxels**
  request, while direct contours and the authoritative mesh remain live.
- [x] Phase 4: QA identifies the completed job kind, so a generic duration is
  no longer mistaken for mesh-to-voxel resampling.

Pending:

- [ ] Finish QA liver deformation and explicit **Build voxels** verification:
  one live run found no tears, but scattered occupancy was not reported.
- [ ] Recheck the immediate **Build voxels** pause in the browser after
  validation reuse; native release timing is not browser acceptance.
- [ ] Complete GPU QA across axial/oblique views and rotated/anisotropic grids.
- [ ] Decide whether the measured ~101 ms synchronous liver SDF setup is
  acceptable for v0 editing, or split it across frames.
- [ ] Audit remaining extraction/topology and deformation validity guarantees
  before accepting the smooth-meshing milestone.

Explicit rebuild pause follow-up:

- The live liver workflow reported an immediate 0.5–1 s freeze on clicking
  **Build voxels**, while deformation produced no visible tears in that run.
- A release-mode liver test at the actual request/scheduler seam reproduced
  two synchronous stages: about 69 ms in the request and 82 ms in the first
  frame. Full mesh validation ran in both stages; the first frame also built
  the voxelizer's triangle acceleration structure.
- A successfully committed mesh preview now records that its exact authority
  generation passed full validation. The explicit request and voxelizer reuse
  it; other mesh entry paths still validate, and authority changes clear the
  record. The same native test measured about 0 ms request and 22 ms first
  frame after the change. Resampling remains frame-budgeted. Browser impact is
  pending live confirmation.

Correctness follow-up:

- Reproduced a closed chunked surface becoming open after a small brush drag.
  Chunk merge retained duplicate seam vertices, disconnecting brush adjacency.
- Merge now shares exact-position vertex indices across chunks. No spatial
  tolerance is used to merge nearby surfaces.
- A curved anisotropic fixture also exposed opposite edge-interpolation order
  producing unequal coordinates at chunk seams. Extraction now uses a
  canonical edge order.
- Regression coverage checks closure after deformation, local and broad brush
  voxel resampling on a rotated anisotropic grid, and seam identity across
  chunk sizes. These fixtures do not replace live liver QA or prove arbitrary
  deformations are free from self-intersection.
- Validation: `cargo test -q` (322 passed), WASM target check, Clippy with
  warnings denied, formatting, and diff whitespace check passed.

Representative CPU QA follow-up:

- Added an explicit, ignored-by-default liver regression using the checked-in
  NIfTI label, production chunk size, surface brush, and voxelizer. Run with
  `cargo test -q test_liver_deformation_preserves_closed_surface_and_local_voxel_changes -- --ignored --nocapture`.
- Passed twice: 23,848 vertices / 47,692 faces; undeformed mesh closed and
  voxel round-trip exact. A 20 mm brush with `[2, 1, -1]` mm displacement stayed
  closed and changed 39 voxels, all within the brush neighborhood.
- Native debug timings from the second run: extraction 11.06 s, baseline
  resample 7.10 s, deformed resample 7.68 s. These are diagnostic timings, not
  browser or release performance acceptance measurements.
- All 255 nonempty binary 2×2×2 patterns pass closure validation and exact
  voxel round-trip on isotropic and anisotropic grids (510 cases).
- Remaining: live GPU interaction, larger ambiguous topology configurations,
  and validity under aggressive/repeated deformation. Neither local closure
  tests nor a single liver brush gesture prove absence of self-intersections.

Validation follow-up:

- Added 256 fixed-seed 3×3×3 anisotropic masks to exercise neighboring ambiguous
  cells; all pass closure and exact voxel round-trip. This is bounded coverage,
  not a proof of general topology correctness.
- Reproduced the shared validator accepting a closed indexed mesh with a
  collinear, zero-area triangle. It now rejects such faces before voxelization,
  using f64 area arithmetic without a size tolerance. The regression covers
  both the validation API and actual voxel conversion.
- `cargo test -q`: 325 passed, one explicit liver QA test ignored by default;
  WASM check and Clippy with warnings denied passed.
- Explicit liver QA also passed with the zero-area validator: exact baseline
  round-trip and the same 39 local voxel changes after deformation.
- Live browser check: Trunk listens on localhost:8080, but browser connection
  discovery returns no available browsers. GPU acceptance remains unverified.
- At this checkpoint, general self-intersection detection and
  aggressive/repeated-deformation behavior remained open; zero-area rejection
  alone did not solve those cases.

Self-intersection follow-up:

- Reproduced a closed octahedron folding through its opposite surface after
  repeated surface-brush edits while the old validator still accepted it.
- Shared voxelization validation now rejects intersecting triangle regions
  beyond legitimate shared vertices/edges, including coplanar overlaps and
  duplicate faces. It uses the existing Parry BVH for candidate pairs and
  local f64 separating-axis checks; no new dependency or per-frame validation.
- Regressions cover repeated brush edits, valid shared boundaries, shared-vertex
  crossings, coplanar containment, and close but separate parallel surfaces.
- Liver QA with intersection checks passed: exact undeformed round-trip and
  39 local changed voxels after deformation. Native debug resampling increased
  to roughly 9–10 s in this run; browser/release cost still needs measurement.
- This is a rejection boundary for **Build voxels**, not collision-constrained
  sculpting: a self-intersecting mesh may still be edited/displayed, but cannot
  be converted to voxels. Live GPU QA and usability of rejection/undo remain
  pending. Numerical predicates use floating point, not exact arithmetic.
- Validation: 327 tests passed, one liver test ignored by default (passed
  explicitly); WASM check, Clippy, formatting and diff whitespace checks passed.

Commit safety follow-up:

- Live user QA reported `SelfIntersection { faces: [13458, 14368] }` after
  deformation, with long stretched triangles visible. Rebuild rejection alone
  was not an adequate editing workflow.
- Added validation before mesh-preview commit. Invalid previews are cleared,
  the previous authoritative mesh is retained, and no undo entry is added.
  The status message explains rejection and suggests a shorter drag/larger brush.
  This supersedes the earlier decision to allow invalid/open preview commits.
- This protects committed data; it does not prevent spikes during the preview
  or provide collision-constrained dragging. Smooth large-drag behavior and
  live QA of rejection/recovery remain pending.
- Native optimized liver run: extraction 235 ms, baseline resample 1.18 s,
  deformed resample 1.16 s. A second instrumented run measured validation at
  125–176 ms and resampling at 1.9–2.0 s. Both passed; timings vary with host
  load and are not browser performance evidence. Full validation now also runs
  once at pointer-release commit, not on each drag frame.

Large-drag interaction decision (adaptive area selected):

- Traced preview updates: every frame reads authoritative geometry and applies
  total pointer displacement from drag start. Repeated identical mouse events
  do not compound movement; the projected-drag regression now checks this.
- Current geodesic influence radius remains fixed while displacement is
  unlimited. Long drags can therefore stretch a small neighborhood into spikes
  even with correct seam connectivity and coordinate mapping.
- User selected adaptive influence area, not bounded displacement.
- Implemented `effective_radius = max(minimum_radius, 1.5 * |strength * drag|)`
  in the connected-surface brush. The same radius drives neighborhood search
  and smoothstep falloff; the requested displacement is not clamped. This
  halves the adaptive radius compared with the initial 3× setting, following
  user feedback that too much of the mesh moved. It is not a guarantee against
  collisions with other nearby surfaces; commit validation remains required.
- Slider now reads **Minimum radius mm** and explains automatic growth.
- Regressions cover growth beyond the minimum radius, strength scaling,
  zero motion, disconnected-surface isolation, and closed geometry after a
  long drag. With the user-requested tighter 1.5× influence, the repeated-drag
  octahedron fixture can fold again; validation detects and rejects it.
- Commit-time validation remains enabled. Live GPU usability/performance QA
  is still required, particularly when large drags reach much of the mesh.
- Representative long-drag limitation reproduced: at the existing liver anchor,
  `[24, 12, -12]` mm displacement with 12 mm minimum radius still intersects
  faces `[23134, 23139]` despite adaptive influence. The explicit liver test
  now verifies rejection of that case as well as successful small-edit
  resampling. This is not evidence that arbitrary large-drag sculpting is
  solved; smoother collision handling remains unfinished.
- Adaptive-area validation: 328 regular tests passed; explicit liver test
  passed both small-edit resampling and large-edit rejection. WASM compilation,
  Clippy and formatting checks passed. Live interaction QA remains pending.

3D zoom regression (on hold at user request):

- User reports the app slows immediately after zooming into a freshly loaded
  liver, before deformation, and other views slow while it remains zoomed.
  Zoom changes the viewport transform and requests redraw; it does not run
  mesh validation or ROI representation conversion.
- User confirmed the lag remains with the liver ROI hidden. That makes mesh
  projection and ROI overlay sampling insufficient explanations for the
  regression. The 3D volume pass remains the leading suspect, but CPU versus
  GPU cost is not yet measured.
- No cause or performance fix has been confirmed. Temporary frame timing and
  shader experiments from the interrupted investigation were removed. Resume
  with a live CPU/GPU frame capture if this becomes the priority again.

Incremental mesh-voxel rebuild follow-up:

- **Build voxels** now retains its validated mesh and partial occupancy across
  frames, processing bounded 512-voxel batches within the existing 4 ms ROI
  work budget. A completed job still installs one generation-checked cache and
  one GPU upload. A stale job is discarded without silently resampling the new
  mesh; the user must request **Build voxels** again.
- The shared synchronous voxelizer drains the same incremental kernel, so
  offline conversion and runtime rebuild use identical sampling. A regression
  checks one-voxel steps against the expected anisotropic block occupancy;
  a runtime regression checks that the cache stays stale while work is pending
  and that an intervening mesh edit cancels the old request.
- The axial/oblique direct-contour regression now exercises a surface-brush
  preview commit, rather than only a mesh translation, and checks the same
  mesh-derived contours after the optional voxel rebuild.
- Visible inactive mesh ROIs now receive direct contour views for the displayed
  planes as well; current mesh-plane intersections are reused until the mesh or
  preview revision changes. Rebuilding optional voxels no longer marks those
  mesh-derived contours stale. The regression checks axial/oblique renderability
  for an inactive mesh ROI and retains current contours across resampling.
- Reproduced a canceled mesh preview still being returned to the contour
  renderer for one frame. Mesh-authority render lookup now rejects ended
  previews and stale generations; the next viewport sync restores the direct
  authoritative intersection. Commit and cancel regressions cover this boundary.
- Topology audit found the old one-voxel dirty halo was not conservative for
  the global Euclidean SDF: a single voxel edit changed mesh interpolation in
  a supposedly untouched chunk. Dirty mesh jobs now rebuild every chunk using
  the existing frame-budgeted scheduler. This preserves geometry correctness
  but gives up local chunk reuse until old/new SDF fields can identify exact
  changed cells. A fixed-seed rotated, anisotropic stress regression compares
  incremental and clean rebuilds by exact vertex bits, and the failing sparse
  configuration is retained as a regular regression.
- Native release liver QA after this change: three repeated single-voxel dirty
  rebuilds took 112–115 ms for synchronous SDF setup and 21–22 ms for all
  chunk extraction. Reusing distance-transform scratch arrays cut setup to
  101–102 ms in three subsequent release runs; chunk extraction stayed near
  22 ms. Each result matched a clean full rebuild exactly. Only chunk
  extraction is frame-budgeted today; SDF setup can still hitch one frame.
  These CPU timings are not browser/GPU acceptance evidence.
- An end-to-end rotated, anisotropic ROI regression now drives voxel authority
  through scheduled smooth-mesh rebuild, mesh promotion, a committed surface
  brush, direct axial/oblique contours, and explicit voxel resampling. It keeps
  mesh authority and both direct contour caches current after resampling.
- Exhaustive adjacent-cell QA covers all 4,095 nonempty binary 3×2×2 masks on
  an anisotropic grid; every extracted surface passed closed-mesh validation
  and exact voxel round-trip. The stress test is ignored by default.
- Mesh validation, Parry mesh construction, and final GPU upload remain
  synchronous stages. Live liver frame-time QA is needed before claiming the
  explicit operation is fully hitch-free. `last_mesh_voxelization_ms` records
  active setup and sampling CPU time; `last_job_duration_ms` includes waits
  between frames.
- Validation: 332 regular tests passed, five QA tests ignored by default.
  The two chunk stress tests and dirty liver timing test passed explicitly;
  liver deformation and exhaustive adjacent-cell masks passed at the prior
  checkpoint. WASM check, Clippy, and formatting passed.

Checkpoint commits:

- `b9a177e` Perf: coalesce rapid 3D zoom redraws
- `67a2037` Fix: preserve direct contours for mesh authority
- `fab1421` Feat: add ROI signed distance conversion
- `d7cd04e` Feat: add local SDF marching cubes extractor
- `b1e7ab0` Feat: use smooth meshes in chunked ROI rebuilds
- `d28f022` Feat: make mesh brush surface-aware
- `af658ad` Test: cover rotated mesh voxel roundtrip
- `96e64c5` Test: cover oblique mesh authority contours
- `c5dac9b` Fix: pick frontmost mesh surface
- `05312e8` Perf: defer mesh voxel resampling
- `be31f42` QA: identify completed ROI job kind
- `fcb9487` Fix: keep open meshes out of voxel resampling
- `0c54f0a` Fix: stabilize mesh authority and SDF rebuilds
