# Subplan 9.1 Representation Orchestration and Contour View Caches

Purpose: tie the primary-representation ROI model, derived session caches, viewport render requests, and QA facts into one explicit runtime contract.

Subplan 9 creates render-view adapters. Subplan 9.1 defines how those adapters request voxel, contour, and mesh representations per viewport and how cache state is reported while representations rebuild.

## Decisions

- 3D ROI display uses mesh cache/surface. Voxel-primary and contour-primary ROIs request derived mesh cache for 3D.
- Contour-primary ROIs keep one editable active contour plane family.
- Non-active contour families can exist as read-only derived contour view caches.
- Starting edit in another contour view promotes that view to the authoritative active contour family.
- Near-term promotion/rebuild path is voxel-mediated:
  - active contour edit updates authoritative contour data
  - authoritative contour rebuilds voxel cache
  - other contour view caches rebuild from voxel
  - mesh cache rebuilds from voxel for 3D
- Future goal: higher-quality mesh/SDF/TSDF-mediated synchronization may replace or augment voxel-mediated promotion. Do not implement that in 9.1.
- Derived views may show stale data while rebuilds run, but must expose stale/loading state.
- Stale derived views are display-only; editing blocks until current or promotion succeeds.
- Cache keys are logical view requests, not viewport entity IDs.
- Orthogonal contour view cache granularity for 9.1 is current displayed slice only.
- QA remains state/log based. No screenshots, crops, or pixel matching.

## Required Model

Add a real contour view-cache concept.

Suggested shape:

```rust
pub struct ContourViewCache {
    pub key: ContourViewKey,
    pub data: ContourData,
    pub source_generation: u64,
    pub state: CacheViewState,
}

pub struct ContourViewKey {
    pub family: PlaneFamily,
    pub plane: PlaneDefinition,
    pub slice_key: ContourSliceKey,
}

pub enum CacheViewState {
    Current,
    Stale,
    Rebuilding,
    Blocked { reason: String },
}
```

Exact Rust names can differ, but semantics must remain.

Cache key requirements:

- axial/coronal/sagittal keys include enough plane identity to represent current displayed slice
- oblique keys include concrete `PlaneDefinition`
- key must not depend on viewport entity ID because protocols recreate viewport entities

## Runtime Contract

Viewport asks for representation data through render-view adapter/request APIs.

Requests:

- `VoxelOverlay`: needs current or stale voxel cache plus GPU resources
- `ContourView`: needs active authoritative contour data or derived contour view cache for requested plane
- `Mesh3d`: needs current or stale mesh cache/surface data

States:

- `current`: safe to render and edit if authoritative/active
- `stale`: safe to display with stale marker; not safe to edit as derived view
- `rebuilding`: show placeholder/status; may keep stale display if available
- `blocked`: explicit reason, no silent absence
- `unsupported`: explicit reason for view/representation combinations not implemented

Promotion flow when user edits non-active contour view:

1. User clicks edit in a non-active contour view or uses explicit “make this view editable” action.
2. Runtime checks requested contour view cache state.
3. If current, promote that contour view data to authoritative `ContourData` and set active plane family.
4. Previous authoritative contour family becomes derived display cache only, stale until rebuilt from voxel.
5. Mark voxel cache dirty/rebuild.
6. Mark other contour view caches dirty/stale.
7. Mark mesh cache dirty/rebuild.
8. If requested view cache is stale/rebuilding/blocked, reject edit with status reason.

Promotion must not create multiple authoritative contour families.

## View-Mode Representation Policy

### Orthogonal 2D

- image volume is base layer
- voxel overlay may render when voxel cache/GPU resources exist
- contour view cache may render for requested slice/family
- active editable contour renders above derived contours
- derived contour views are read-only unless promoted

### Oblique

- image volume is base layer
- active editable oblique contour renders when active family is oblique
- derived oblique contour view cache may render for concrete oblique plane
- voxel overlay renders only if geometry-aware oblique sampling exists; otherwise explicit blocker

### 3D

- ROI geometry display uses mesh cache/surface
- voxel-primary and contour-primary request mesh cache for 3D
- no contour-polyline 3D hack
- voxel volume overlay in 3D remains unsupported unless a future renderer strategy is explicitly added

## Cache Rebuild Order

After contour edit:

1. update authoritative contour data immediately
2. mark voxel cache dirty and request rebuild
3. when voxel cache current, rebuild requested contour view caches from voxel
4. when voxel cache current, rebuild/request mesh cache from voxel for 3D
5. update QA/view adapter states throughout

After voxel-primary edit/import:

1. authoritative voxel data changes
2. mark contour view caches dirty/stale
3. mark mesh cache dirty/stale
4. rebuild requested contour views and mesh cache on demand

After mesh-primary edit:

1. authoritative mesh data changes
2. mark voxel cache dirty/stale
3. mark contour view caches dirty/stale
4. mesh-to-voxel and mesh-to-contour rebuilds may be blocked until those conversions exist
5. blockers must be explicit in UI/QA

## QA Contract

Extend QA state only as needed. Keep existing fields stable.

Recommended per-viewport/ROI facts:

- `representation_requests`
- `voxel_cache_state`
- `contour_view_cache_state`
- `mesh_cache_state`
- `contour_editable`
- `contour_promotable`
- `stale`
- `render_blockers`

State values should be stable snake_case strings:

- `current`
- `stale`
- `rebuilding`
- `blocked`
- `unsupported`

Recommended QA presets:

- voxel-primary MPR: voxel overlay current, contour views requested/available or explicit blocker, mesh cache requested for 3D
- contour-primary active view: active contour editable, voxel/mesh derived caches requested
- contour-primary switched view: non-active contour view promotable; after promotion old active family is stale derived view
- 3D mesh-cache request: 3D viewport requests mesh cache for voxel/contour primary ROI

Browser QA must assert facts from `window.__viewerQa.state()`, `logs()`, `metrics()`, and `lastError()`.

Do not add screenshots or pixel matching.

## Implementation Steps

### Step 9.1A: Add Contour View Cache Types

Files:

- `src/app/components.rs`
- likely `src/app/roi_runtime.rs`
- unit tests near cache model

Implement:

- contour view cache data/key/state
- storage in ROI session caches or a runtime-owned cache map
- dirty/stale helpers
- no rendering behavior change yet

Tests:

- contour-primary keeps one authoritative active family
- derived contour view cache can exist for another family
- viewport entity identity is not part of cache key
- edit invalidation marks derived contour view caches stale

### Step 9.1B: Add Representation Request Runtime APIs

Files:

- `src/app/roi_runtime.rs`
- `src/render/roi_views.rs`

Implement:

- request voxel overlay state
- request contour view state for a logical plane/view
- request 3D mesh state
- stable blocker/reason strings
- no conversion in render passes

Tests:

- orthogonal 2D requests voxel + contour view
- oblique requests contour view and reports voxel blocker if unsupported
- 3D requests mesh cache
- missing dirty caches return rebuilding/stale/blocked, not silent false

### Step 9.1C: Promotion Flow

Files:

- `src/app/roi_runtime.rs`
- UI trigger location in `src/gui/` or input system, minimal if practical
- tests in runtime

Implement:

- explicit promotion API: derived contour view -> authoritative contour data
- optional implicit edit-trigger path if cache current
- reject stale/rebuilding/blocked derived view edits with status reason
- invalidate voxel, contour view, and mesh caches after promotion

Tests:

- current derived view promotes to authoritative active family
- previous authoritative family becomes derived stale view
- stale derived view cannot be edited/promoted silently
- voxel/mesh/other contour caches dirty after promotion

### Step 9.1D: Adapter and QA Facts

Files:

- `src/render/roi_views.rs`
- `src/app/qa.rs`
- `src/app/mod.rs`
- `tests/qa1_viewerqa.spec.js`

Implement:

- adapter exposes representation request/cache states
- QA snapshot reports cache/request facts
- add state-only Playwright assertions for new presets when available

Tests:

- existing QA-1/QA-2/QA-3 still pass
- new preset facts pass in GPU-capable browser
- no-GPU branch remains structured WGPU failure only

### Step 9.1E: Closeout

Update:

- `docs/segmentation-reimplementation-plan.md`
- this handoff implementation status

Run:

```bash
cargo fmt --all
cargo test -q
cargo check --target wasm32-unknown-unknown -q
cargo clippy --all-targets --all-features -- -D warnings
```

Browser QA outside sandbox:

```bash
trunk serve
npx playwright test tests/qa1_viewerqa.spec.js --reporter=line
```

Real render QA requires GPU-capable Chrome/WebGPU. Headless/no-GPU validates only structured WGPU failure.

## Do Not Do

- Do not make multiple contour plane families authoritative at once.
- Do not implement mesh/SDF/TSDF-mediated sync in 9.1.
- Do not add 3D contour polyline rendering as substitute for mesh cache.
- Do not add screenshot/crop/pixel QA.
- Do not move conversion work into render passes.
- Do not make cache keys depend on viewport entities.
- Do not turn Subplan 10 performance/eviction/windowing into 9.1 scope.

## Completion Criteria

- Runtime can explain what representation each viewport requests.
- Runtime can report current/stale/rebuilding/blocked/unsupported cache state.
- Contour-primary ROI remains single-authority while supporting derived contour views.
- Editing another contour view promotes it through explicit runtime flow.
- 3D requests mesh cache for ROI display.
- QA can prove representation orchestration from structured state.
