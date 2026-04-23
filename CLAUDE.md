# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

A 2D/3D medical volume viewer built with Rust + WGPU. The current codebase supports orthogonal slicing (Axial/Coronal/Sagittal), volumetric rendering, interactive crosshair picking, labelmap overlays, annotations, and both native desktop and WebAssembly targets.

## Commands

```bash
cargo run                                                   # Run native desktop app
cargo test -q                                               # Run all unit tests
cargo test <module_path>::<test_name>                       # Run a single test
cargo fmt --all                                             # Format code
cargo fmt --all -- --check                                  # Check formatting (CI)
cargo clippy -- -D warnings                                 # Lint (CI)
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
- `src/app/` — ECS components (`components.rs`), app event enum (`events.rs`), rendering context init (`context.rs`)
- `src/systems/` — input, picking, render preparation
- `src/convert/` — shared coordinate-mapping helpers
- `src/render/` — WGPU pipeline setup and frame rendering
- `src/gui/` — egui panels (toolbar, sidebar, annotations, overlays)
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

## Current Editing Scope

The current application supports viewing a main volume, loading labelmaps as overlay layers, adjusting viewport/windowing state, and working with annotations. The older contour/SDF/mesh segmentation pipeline has been removed from the active codebase and is being redesigned separately.

## Testing Approach

Tests live in `#[cfg(test)]` modules within each source file. The "modular math" pattern extracts pure functions (no GPU/ECS dependencies) so geometry and coordinate logic can be unit tested directly. Key coverage includes coordinate transforms, picking, orientation math, and rendering-related helpers.

For rendering or overlay changes include: a correctness test where practical, a regression test for prior bugs, and a WASM compile check.

## Key Invariants

- `Uniforms` struct layout in `components.rs` must match `shader.wgsl` exactly (16-byte alignment boundaries)
- `shader.wgsl` early-exits when `volume_dims == [0,0,0]` (startup guard against NaN aspect ratios)
- Overlays and crosshairs are inhibited until a valid volume is loaded

## Coding Conventions

- Rust 2021 edition, 4-space indentation, standard `rustfmt`
- `snake_case` for functions/variables/modules, `CamelCase` for types
- Descriptive system names like `sys_handle_input_scroll`
- Commit style: short imperative with scope prefix — `Fix: crosshair projection in sagittal view`, `Docs: update current repo status`
- Keep commits focused; don't mix refactors with behavior changes
- If an active plan doc exists in `docs/`, update its Implementation Status log in the same commit as the code change

## Sample Data

`liver_0.nii` and `liver_0_label.nii` in the repo root are sample NIfTI files for local testing.
