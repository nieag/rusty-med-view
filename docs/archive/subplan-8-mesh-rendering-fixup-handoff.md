# Subplan 8 Mesh Rendering Fixup Handoff

This document is the source of truth for fixing the first mesh workflow implementation before it is committed or used as the base for deformation work.

Repository-wide rendering policy:

- [docs/rendering-architecture.md](/Users/nieage/dev/git/rust_starter_app/docs/rendering-architecture.md:1) is authoritative for the egui/native-rendering boundary
- `egui` is GUI only; viewport scene geometry must use native wgpu rendering

Follow-up display-geometry fix:

- remaining mesh-vs-voxel placement mismatch was resolved through [docs/subplan-8-1-spatial-geometry-contract-handoff.md](/Users/nieage/dev/git/rust_starter_app/docs/subplan-8-1-spatial-geometry-contract-handoff.md:1)
- Subplan 8 acceptance depends on voxel overlay, mesh extraction, and mesh rendering agreeing through the shared ROI-native-to-world-to-viewport geometry contract

The current Subplan 8 implementation is useful as a first extraction/runtime spike, but manual verification exposed rendering failures that must be fixed before proceeding:

- extracted meshes can appear spatially displaced from the voxel label they came from
- mesh drawing can bleed outside the 3D viewport bounds
- close inspection can show unexpected disconnected components when the source labelmap contains multiple non-zero label values
- the mesh is currently drawn through an egui overlay path, which is not acceptable for the mesh renderer baseline

## Goal

Make mesh visualization correct, native, and explicit enough to trust as the baseline for later mesh deformation.

The fix is not to tune offsets until the screenshot looks better. The fix is to make the coordinate-space contract explicit and testable:

- mesh vertices remain stored in patient/world millimetres
- rendering projects those world positions into the current display volume coordinate system
- viewport clipping/scissoring is owned by the renderer, not by egui painter behavior
- volume, contour, mesh, picking, and overlay projection consume one shared display projection context rather than rebuilding viewport math independently
- binary voxel-to-mesh extraction semantics are documented and tested

## Non-Goals

Do not implement:

- mesh deformation/editing tools
- mesh-to-voxel regeneration
- mesh-to-contour regeneration
- marching cubes smoothing, decimation, booleans, or margin tools
- SDF/TSDF integration
- import/export
- registration/resampling
- broad renderer architecture rewrites beyond what is necessary for correct mesh rendering

## Diagnosis

### 1. Mesh display currently mixes coordinate spaces

`MeshData` vertices are stored in patient/world millimetres. That is correct.

The current render path then converts each mesh world point back to volume UV using the mesh ROI's own cached voxel geometry:

```rust
world_mm_to_volume_uv(mesh_vertex.world_mm, mesh_roi_projection_geometry)
```

That is not sufficient for displaying the mesh over the main image volume. The 3D viewport renders the image in the main volume's normalized display box and aspect ratios. A mesh created from a label ROI must therefore be projected into the main display volume's UV space for visualization:

```rust
world_mm_to_volume_uv(mesh_vertex.world_mm, main_display_volume_geometry)
```

Using the source label geometry for this projection only works when the label geometry exactly matches the image geometry. Different origin, spacing, dimensions, or orientation will place the mesh in the wrong normalized display box.

### 2. The mesh renderer must not use egui

The current path draws mesh wireframe segments from `src/gui/overlays.rs` through egui painting. That should be removed for mesh rendering.

Mesh visualization must be a native wgpu render pass, similar in ownership to the existing contour overlay renderer:

- render-prep converts ROI mesh data into GPU-facing vertices
- upload updates a wgpu vertex/index buffer
- render executes a wgpu pipeline after the volume pass and before egui
- egui remains UI only, not mesh drawing

### 3. Viewport bounds must be enforced by the render path

The mesh should never draw into:

- the toolbar
- neighboring viewports
- side panels
- outside the active 3D viewport rectangle

The renderer must use viewport/scissor control or equivalent GPU clipping. Do not rely on egui clip rects.

### 4. Multi-label voxel data is currently treated as binary occupancy

`extract_mesh_from_voxel_data` currently treats every non-zero voxel as occupied. That means a labelmap with values `1`, `2`, `3`, etc. becomes one binary occupancy field. If the loaded labelmap contains disconnected label classes, the generated mesh will also contain disconnected components.

This is acceptable only if it is explicit in code, UI text, tests, and manual verification. If the app later adds per-label ROI selection, mesh extraction should filter by selected label value instead of all non-zero voxels.

For this fixup, do one of these:

- keep binary all-non-zero extraction, but rename/status/document the action as "Create mesh from all non-zero voxels"
- or add a real label-value selection/filter if that already exists in the app

Do not silently imply that the generated mesh is a single anatomical ROI if the source is a multi-label map.

### 5. Projection state must have a single sync point

Manual verification after the first native mesh-rendering fix still showed mesh/voxel misalignment. The likely cause is that mesh render prep rebuilt its own `ViewProjection` using raw `viewport_state.user_rotation`, while the volume renderer uses:

```rust
compose_view_rotation(main_volume.orientation, viewport_state.user_rotation)
```

That mismatch means the volume and mesh can use different 3D transforms even when both are otherwise using the main display volume geometry.

The fix must not be a mesh-only one-line patch that happens to add `compose_view_rotation(...)` locally. Add or reuse a shared display projection context builder so future render paths cannot drift again.

Suggested shape:

```rust
pub struct DisplayProjectionContext {
    pub geometry: VoxelGeometry,
    pub aspect_ratios: [f32; 3],
    pub cursor_pos: [f32; 3],
    pub rotation: [f32; 4],
    pub zoom: f32,
    pub pan: [f32; 2],
    pub pivot: [f32; 2],
    pub screen_aspect: f32,
    pub viewport_rect: [f32; 4],
    pub window_size: [f32; 2],
}

pub fn display_projection_context_for_viewport(
    world: &World,
    entities: &AppEntities,
    viewport: &Viewport,
    viewport_state: &ViewportState,
) -> Option<DisplayProjectionContext>
```

The exact type name/location may differ, but the contract is non-negotiable:

- one helper owns main display volume geometry lookup
- one helper owns 3D composed rotation
- one helper owns screen aspect, viewport rect, and window size
- mesh render prep consumes this helper
- later render paths should consume the same helper instead of reconstructing projection state ad hoc

## Required Implementation

### Step 8F.1: Add an explicit display geometry helper

Add a helper that returns the geometry used to display the main image volume.

Suggested shape:

```rust
pub fn main_volume_geometry(world: &World) -> Option<VoxelGeometry>
```

It should build `VoxelGeometry` from the `MainVolumeTag` `VolumeData`:

- `dimensions`
- `spacing`
- `origin`
- `orientation`

Use this helper for mesh rendering projection. The mesh ROI's source voxel geometry may still be retained for provenance/cache ownership, but it is not the projection geometry when drawing over the main image.

Acceptance tests:

- helper returns `None` when no main volume exists
- helper returns exact dimensions, spacing, origin, and orientation from the main volume

### Step 8F.2: Replace egui mesh drawing with native wgpu rendering

Remove or disable the mesh drawing path in `src/gui/overlays.rs`:

- remove `draw_mesh_wireframe_overlay(...)`
- remove calls to `draw_mesh_wireframe_overlay(...)`
- remove egui mesh projection logic from GUI overlay code

Add native mesh rendering in `src/render/meshes.rs`.

Minimum acceptable V1:

- a `MeshRenderer` owned by `Pipelines`
- a mesh shader, or reuse a small screen-space overlay shader if the vertex format is compatible
- render-prep that emits GPU-facing vertices after projecting mesh world points into the displayed 3D viewport
- upload logic with growable buffers
- render logic called from `render_frame(...)` after the volume pass and before egui rendering

Preferred V1:

- alpha-blended filled triangles using `MeshFace` indices
- optional wireframe overlay only if it is native wgpu, not egui

If filled triangles are too large for the immediate fix, native wireframe is acceptable only as a temporary debug mode and must be documented as such.

### Step 8F.3: Project mesh vertices through patient/world mm into main display volume UV

For every mesh vertex:

1. start with `MeshVertex.world_mm`
2. convert to main display volume UV using `world_mm_to_volume_uv(world_mm, main_volume_geometry)`
3. project that display UV into the 3D viewport using the same `ViewProjection` / `world_to_ndc` path as the volume view
4. convert viewport-relative coordinates to full-screen clip/NDC coordinates using the viewport rect and window size

Do not project mesh vertices through the source label geometry unless there is no main volume and the renderer is in a standalone mesh inspection mode. The app's current image+label verification path should use main volume geometry.

Projection state requirement:

- mesh rendering must use the same display projection context as the volume pass
- for `ViewMode::ThreeD`, the context must use `compose_view_rotation(main_volume.orientation, viewport_state.user_rotation)`
- do not use raw `viewport_state.user_rotation` for 3D mesh projection
- do not duplicate the volume projection setup inside `src/render/meshes.rs`

Acceptance tests:

- a mesh created from shifted label geometry projects to the same screen position as the equivalent world point in the main volume
- a mesh created from label geometry with different spacing projects by physical world location, not by label-local normalized UV
- non-identity orientation is handled by `world_mm_to_volume_uv(..., main_geometry)` without special cases
- non-identity main volume orientation uses the same composed 3D rotation as the volume pass
- mesh render prep fails or tests fail if it uses raw user rotation instead of the shared composed display rotation

### Step 8F.4: Enforce viewport bounds in the native renderer

The mesh renderer must not draw outside the viewport it belongs to.

Implement one of:

- per-viewport render batches with `set_scissor_rect(...)`
- a render target / viewport strategy that clips to the viewport rectangle
- CPU clipping before upload, if the implementation is simpler and tested

Required behavior:

- only render mesh batches for `ViewMode::ThreeD`
- skip unsupported viewport modes cleanly
- skip triangles/segments when projection returns `None`
- handle vertices behind the camera without panics
- no drawing outside the 3D viewport rectangle, including toolbar and neighboring viewports

Acceptance tests:

- render-prep creates no mesh vertices for non-3D viewports
- render-prep creates batches associated with the correct 3D viewport rect
- projected vertices use full-window NDC coordinates derived from the viewport rect
- vertices/triangles with failed projection are dropped safely

Manual verification must explicitly check viewport leakage.

### Step 8F.5: Make binary extraction semantics explicit

Current extraction is binary:

```rust
occupied = raw_value != 0
```

Keep this only if it is explicit.

Required changes:

- document the extraction as "all non-zero voxels"
- update the sidebar hover/status text to avoid implying single-label extraction
- add a unit test proving values `1` and `2` are both occupied under current V1 semantics
- add a note in the plan that per-label mesh extraction is a future workflow unless implemented now

If implementing label-value filtering now:

- add an explicit extraction option/type
- do not infer active label value from display color or LUT
- add tests for selected label only, all non-zero, and empty selected label

### Step 8F.6: Add topology and geometry regressions

Add focused tests around the actual failure modes.

Required extraction tests:

- single occupied voxel produces exactly 6 exterior quads / 12 triangles under the current blocky extraction algorithm
- a solid `2x2x2` occupied block has no internal faces
- invalid raw-data length returns `InvalidRawDataLength`
- all-non-zero label values are occupied, if keeping binary semantics
- geometry with shifted origin and non-unit spacing produces expected world-space mesh bounds

Required render-prep tests:

- source label geometry differs from main volume geometry but projection uses main volume geometry
- mesh render prep uses the shared display projection context rather than local ad hoc projection setup
- non-identity main volume orientation projects with `compose_view_rotation(main_volume.orientation, viewport_state.user_rotation)`
- mesh render-prep is empty when there is no main volume, unless a standalone fallback is intentionally implemented and documented
- hidden mesh ROIs do not render
- invalid face indices are skipped safely
- non-3D viewports produce no mesh render batches

### Step 8F.7: Update documentation and status honestly

Update `docs/segmentation-reimplementation-plan.md` so Subplan 8 is not marked as fully complete until this fixup passes.

Required wording:

- Subplan 8 extraction/runtime creation exists
- Subplan 8 native rendering is implemented but under review due to remaining manual placement failure
- deformation work is blocked until mesh placement, clipping, native renderer ownership, and shared projection-context synchronization are correct

Update `docs/subplan-8-mesh-workflow-handoff.md` to reference this fixup doc.

## Manual Verification Checklist

Run these checks before summarizing or committing.

### Load and Create

- Action: load the usual image NIfTI.
- Expected: image displays as before in 2D and 3D.
- Observed: record pass/fail.

- Action: load the usual voxel labelmap.
- Expected: voxel label overlay displays as before.
- Observed: record pass/fail.

- Action: create mesh from active voxel ROI.
- Expected: status message says whether the mesh was created from all non-zero voxels or a selected label value.
- Observed: record exact status behavior.

### Placement

- Action: compare mesh against the voxel label in 3D.
- Expected: mesh surface overlays the same physical label region, not shifted into a different normalized box.
- Observed: record pass/fail.

- Action: rotate the 3D view.
- Expected: image, voxel label, and mesh remain locked together through rotation, including data with non-identity orientation.
- Observed: record pass/fail.

- Action: pan and zoom the 3D view.
- Expected: mesh moves with the image/label and does not lag or drift.
- Observed: record pass/fail.

### Viewport Bounds

- Action: zoom/pan until the mesh reaches the edge of the 3D viewport.
- Expected: mesh is clipped to the 3D viewport and never draws over the toolbar, sidebar, or neighboring viewports.
- Observed: record pass/fail.

- Action: switch between Standard 2x2 and 3D-only protocols.
- Expected: mesh remains clipped to the active 3D viewport bounds in both protocols.
- Observed: record pass/fail.

### Binary Semantics

- Action: if using a multi-label labelmap, create a mesh from the voxel ROI.
- Expected: if V1 is all-non-zero, all non-zero connected components may appear and the UI/status text makes that clear.
- Observed: record whether any "extra" components are expected by this rule.

### Regressions

- Action: inspect axial/coronal/sagittal image + voxel label views.
- Expected: existing 2D image/label behavior is unchanged.
- Observed: record pass/fail.

- Action: inspect contour editing and contour draw tools.
- Expected: existing contour behavior is unchanged.
- Observed: record pass/fail.

## Validation Commands

Run:

```bash
cargo test -q
cargo check --target wasm32-unknown-unknown -q
cargo fmt --all
```

If native mesh renderer changes touch shader/pipeline setup, also run:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

## Continuation Prompt

Use this prompt for the implementer:

```text
Continue the segmentation reimplementation using docs/subplan-8-mesh-rendering-fixup-handoff.md as the source of truth. Fix only the remaining Subplan 8 mesh rendering/projection issues described there. Do not start mesh deformation, mesh-to-voxel regeneration, mesh-to-contour regeneration, SDF/TSDF work, smoothing/decimation/booleans, import/export, registration/resampling, or later deformation-focused phases. Preserve current image, voxel label, contour extraction, contour editing, contour-to-voxel rebuild, 2D navigation, and 3D viewer behavior. The fix must keep mesh drawing native wgpu-only, introduce or reuse a shared display projection context instead of reconstructing projection state inside mesh rendering, project mesh world-mm vertices through main display volume geometry, use the same composed 3D rotation as the volume pass (`compose_view_rotation(main_volume.orientation, viewport_state.user_rotation)`), enforce viewport bounds in native wgpu rendering, keep all-non-zero voxel extraction semantics explicit unless a real selected-label filter is implemented, update docs/segmentation-reimplementation-plan.md honestly, and run cargo test -q, cargo check --target wasm32-unknown-unknown -q, and cargo fmt --all before summarizing. Include the manual verification checklist with expected vs observed behavior.
```
