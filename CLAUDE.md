# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

A high-performance 2D/3D medical volume viewer built with Rust + WGPU. Supports orthogonal slicing (Axial/Coronal/Sagittal), volumetric X-ray rendering, interactive crosshair picking, and contour-based segmentation with a chunked TSDF → Surface Nets meshing pipeline. Targets both native desktop (Metal) and WebAssembly (WebGPU/WebGL2).

## Commands

```bash
cargo run                                                   # Run native desktop app
cargo test -q                                               # Run all 129 unit tests
cargo test <module_path>::<test_name>                       # Run a single test
cargo fmt --all                                             # Format code
cargo fmt --all -- --check                                  # Check formatting (CI)
cargo clippy -- -D warnings -A clippy::too-many-arguments  # Lint (CI)
cargo build --release                                       # Verify native build (CI)
cargo check --target wasm32-unknown-unknown -q              # Verify WASM compilation (CI)
trunk serve                                                 # Run WASM locally (requires trunk + wasm32 target)
```

CI runs fmt check, clippy, tests, and both native/WASM builds on every push to main and every PR.

## Architecture

**Entry:** `main.rs` → `lib::run()` creates a `winit` event loop, initializes the `RenderingContext` (GPU device/queue, all pipelines, ECS world), and drives the frame loop.

**ECS:** Uses `hecs`. Global singletons (input state, volume data, GUI state) are stored as entities and looked up via `AppEntities` — a registry of entity IDs enabling O(1) access instead of O(N) queries. Systems call `world.get::<T>(entities.foo)`.

**Frame loop order (order matters):**
1. `gui.prepare()` — process UI interactions, update state
2. `sys_prepare_render_data()` — build uniforms from updated state
3. Render — GPU commands with latest data

This zero-lag order means interactions appear in the same frame they occur.

**Key module areas:**
- `src/app/` — ECS components (`components.rs`), app event enum (`events.rs`), segmentation data model (`segment.rs`), rendering context init (`context.rs`)
- `src/systems/` — input, picking (3D raymarching), paint (voxel labelmap editing), segment_system (contour→SDF→TSDF→Surface Nets), contour_draw, render_prep
- `src/convert/` — pure algorithmic code: `contour_to_sdf`, `contour_to_tsdf_chunks`, `surface_nets` (active mesher), `marching_cubes` (retained, not active), `chunk_grid`, `slice_isolines`, `labelmap_to_contours`, `coord_mapping`
- `src/render/` — WGPU pipeline setup and frame rendering
- `src/gui/` — egui panels (toolbar, sidebar, annotations, segments)
- `src/io/` — async NIfTI loading, texture creation, bind group recreation
- `src/util/orientation.rs` — **single source of truth** for all coordinate transforms and radiological mappings
- `src/shaders/` — WGSL shader sources (`shader.wgsl` is the main volume raymarcher)

## Coordinate Systems & Orientation

All coordinate work goes through `src/util/orientation.rs`. The app uses **radiological convention** (patient Right → screen Left, flipped X).

View rotation composition: `final = user_rotation * BASE_ROTATION * data_orientation`
- `data_orientation` — intrinsic quaternion from NIfTI header
- `BASE_ROTATION` — aligns "Superior" to "Up" (90° X-axis)
- `user_rotation` — interactive drag

CPU picking (`screen_to_ray_3d`) and GPU raymarching use identical math — verified by "shader parity" unit tests in `orientation.rs`.

## Segmentation Pipeline

`Contours → SDF (incremental ROI update) → TSDF chunks (i16 quantized) → Surface Nets → merged mesh`

- `SegmentRuntimeCache.tsdf_chunks: HashMap<ChunkKey, TsdfChunk>` is the persistent authoritative representation
- Each chunk is padded +1 voxel on all edges so Surface Nets can connect vertices across boundaries
- Live editing processes only dirty chunks within a configurable **frame budget**
- "Finalize" path uses `surface_nets_from_sdf` for full-volume extraction at higher resolution

## Testing Approach

Tests live in `#[cfg(test)]` modules within each source file. The "modular math" pattern extracts pure functions (no GPU/ECS dependencies) so geometry and coordinate logic can be unit tested directly. Key coverage: AABB intersections, quaternion↔matrix conversions, voxel painting, Surface Nets parity (CPU vs expected), pipeline budget/locality verification, contour discretization.

For rendering/segmentation changes include: a correctness test (geometry/bounds), a regression test (prior bug), and a WASM compile check.

## Key Invariants

- `Uniforms` struct layout in `components.rs` must match `shader.wgsl` exactly (16-byte alignment boundaries)
- `shader.wgsl` early-exits when `volume_dims == [0,0,0]` (startup guard against NaN aspect ratios)
- Overlays and crosshairs are inhibited until a valid volume is loaded
- Marching Cubes is retained in `convert/marching_cubes.rs` but is not in the active pipeline

## Coding Conventions

- Rust 2021 edition, 4-space indentation, standard `rustfmt`
- `snake_case` for functions/variables/modules, `CamelCase` for types
- Descriptive system names: `sys_update_segment_derivatives`
- Commit style: short imperative with scope prefix — `Perf: incremental SDF ROI updates`, `Fix: crosshair projection in sagittal view`
- Keep commits focused; don't mix refactors with behavior changes
- If an active plan doc exists in `docs/features/`, update its Implementation Status log in the same commit as the code change

## Sample Data

`liver_0.nii` and `liver_0_label.nii` in the repo root are sample NIfTI files for local testing.
