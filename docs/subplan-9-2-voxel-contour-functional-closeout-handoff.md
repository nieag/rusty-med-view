# Subplan 9.2 Voxel/Contour Functional Closeout

Purpose: make voxel-primary and contour-primary workflows usable end-to-end before performance/GPU/chunking work begins.

This subplan follows Subplan 9.1. It uses the representation request/cache-state contract from 9.1 and turns contour view caches into functional viewport behavior for voxel/contour workflows.

## Decisions

- Scope is voxel/contour functionality only. Mesh-primary full roundtrip is Subplan 9.3.
- Functional enough means:
  - load voxel ROI
  - extract contours
  - edit contours
  - rebuild voxel cache
  - volume/overlay updates
  - MPR/oblique contour behavior is clear
- Oblique contour editing/display is in scope.
- Oblique voxel overlay remains blocked unless existing renderer support is already correct. Do not force oblique voxel overlay in 9.2.
- Derived contour view caches should be real where algorithms exist.
- Orthogonal derived contour views should be built from current voxel cache.
- Oblique derived contour views may remain blocked if arbitrary-plane extraction is not ready.
- Primary-switch UI should be explicit:
  - make editable contour from active ROI/view
  - commit contour to voxel / rebuild voxel
  - promote this contour view
- Limited implicit promotion is allowed only when requested contour view cache is `current`.
- Contour-primary voxel overlay may show stale data while rebuild runs, but QA/UI must expose `stale`/`rebuilding`.
- QA remains state/log based. No screenshots/crops/pixel matching.

## Required User Workflows

### Voxel -> Contour -> Voxel

1. Load image + voxel label.
2. Select voxel-primary ROI.
3. Extract contour ROI for axial/coronal/sagittal.
4. Edit contour loop.
5. Runtime marks voxel cache stale/rebuilding.
6. Voxel cache rebuild completes.
7. 2D overlay, stats, and QA state reflect rebuilt voxel cache.

### Contour View Promotion

1. Contour-primary ROI has one editable active family.
2. Other orthogonal views request derived contour view cache.
3. Current derived view can be promoted.
4. Promotion makes target family authoritative.
5. Previous active family becomes stale derived cache.
6. Voxel + mesh + contour-view caches dirty/rebuild/block as documented.

### Oblique Contour Path

1. Oblique viewport can create/edit active oblique contour.
2. Oblique contour view request reports `current`, `stale`, `rebuilding`, `blocked`, or `unsupported`.
3. Oblique voxel overlay remains blocked with explicit reason unless implemented correctly.

## Implementation Steps

### Step 9.2A: Audit Current Runtime/UI Paths

Files:

- `src/app/roi_runtime.rs`
- `src/gui/sidebar.rs`
- `src/systems/contour_editing.rs`
- `src/render/roi_views.rs`
- `src/app/mod.rs`

Deliver:

- short code comments or doc notes only where state transitions are ambiguous
- identify current explicit/implicit primary switches
- verify no hidden primary switch occurs without runtime API

Tests:

- no code behavior required unless audit exposes bug

### Step 9.2B: Build Orthogonal Contour View Caches From Voxel Cache

Files:

- `src/app/roi_runtime.rs`
- `src/convert/voxel_contour_extract.rs` if needed
- `src/app/components.rs`

Deliver:

- runtime API to request/build contour view cache for current orthogonal viewport slice
- uses current voxel cache as source
- stores `ContourViewCache` with correct `source_generation`
- sets state `Current`, `Stale`, `Rebuilding`, or `Blocked`
- does not run conversion inside render passes

Tests:

- voxel-primary ROI can create current axial/coronal/sagittal contour view cache from voxel data
- dirty voxel cache -> contour view request reports stale/rebuilding/blocker
- cache key uses logical plane key, not viewport entity

### Step 9.2C: Wire Contour View Promotion UI

Files:

- `src/gui/sidebar.rs` or focused tool UI file
- `src/app/roi_runtime.rs`
- `src/systems/contour_editing.rs` if implicit edit promotion is added

Deliver:

- explicit action to promote current contour view to authoritative editable family
- clear status messages for:
  - promoted
  - cache stale
  - cache rebuilding
  - cache missing/blocked
- optional implicit promotion on edit click only when cache is current/promotable

Tests:

- current derived view promotes
- stale/rebuilding/blocked view rejects with status
- old active family becomes stale derived cache

### Step 9.2D: Contour Edit Rebuild State

Files:

- `src/app/roi_runtime.rs`
- `src/app/mod.rs`
- `src/app/qa.rs`

Deliver:

- contour edit commit marks voxel overlay state stale/rebuilding
- derived contour view caches become stale
- mesh request state becomes stale/blocked where applicable
- QA reflects states without relying on screenshots

Tests:

- `replace_contour_data(...)` stales derived contour views
- queued voxel rebuild state visible through request status
- completed rebuild clears stale state

### Step 9.2E: Oblique Contour Behavior

Files:

- `src/systems/contour_editing.rs`
- `src/render/contours.rs`
- `src/render/roi_views.rs`
- `src/app/mod.rs`

Deliver:

- active oblique contour edit/render remains functional
- oblique derived contour view request returns explicit state
- oblique voxel overlay remains blocked with reason if not implemented

Tests:

- oblique active contour is editable/renderable
- oblique non-active contour view request is blocked/unsupported with reason unless cache supported
- no silent false-ready QA state

### Step 9.2F: QA and Manual Closeout

Files:

- `tests/qa1_viewerqa.spec.js` if state assertions need extension
- `docs/segmentation-reimplementation-plan.md`
- this handoff

Deliver:

- QA state asserts voxel/contour workflow facts
- manual checklist completed on sample data
- implementation status updated

Validation commands:

```bash
rtk cargo fmt --all
rtk cargo test -q
rtk cargo check --target wasm32-unknown-unknown -q
rtk cargo clippy --all-targets --all-features -- -D warnings
```

Browser QA outside sandbox:

```bash
trunk serve
npx playwright test tests/qa1_viewerqa.spec.js --reporter=line
```

Manual checks:

- load image + voxel label
- extract contour from voxel
- edit contour
- observe voxel overlay/stat rebuild
- promote derived contour view
- edit oblique contour
- verify unsupported oblique voxel overlay reports blocker

## Do Not Do

- Do not implement mesh-primary full roundtrip here.
- Do not implement SDF/TSDF.
- Do not add screenshot/pixel QA.
- Do not move conversion into render passes.
- Do not implement performance chunking/eviction/windowing except tiny correctness scaffolding.
- Do not make multiple contour families authoritative at once.

## Completion Criteria

- Voxel/contour workflow usable on sample data.
- Orthogonal contour view caches are real, not just state placeholders.
- Active oblique contour editing/display works or fails with explicit blocker.
- Derived contour view promotion works through runtime API.
- Contour edits update voxel-derived overlay/stats through explicit stale/rebuild/current states.
- QA can explain every viewport representation request.
