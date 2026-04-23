# Repository Guidelines

## Project Structure & Module Organization
- `src/` contains the application code.
- Core areas:
  - `src/app/`: ECS components, app context, and events.
  - `src/systems/`: runtime systems (input, picking, rendering prep).
  - `src/render/`: WGPU pipelines and frame rendering.
  - `src/convert/`: shared conversion and coordinate-mapping helpers.
  - `src/gui/`: egui panels and viewer UI.
  - `src/shaders/`: WGSL shader sources.
- `docs/` stores implementation plans and design notes.
- Tests are colocated in `#[cfg(test)]` modules in each Rust source file.

## Build, Test, and Development Commands
- `cargo run` : run native desktop app.
- `trunk serve` : run WebAssembly app locally (requires `wasm32-unknown-unknown` target).
- `cargo test -q` : run unit tests.
- `cargo check --target wasm32-unknown-unknown -q` : verify WASM compilation.
- `cargo fmt --all` : format code.
- `cargo clippy --all-targets --all-features -D warnings` : lint with warnings as errors.

## Coding Style & Naming Conventions
- Rust 2021 edition, 4-space indentation, standard `rustfmt`.
- Prefer small, testable functions for math/geometry logic.
- Naming:
  - `snake_case` for functions/modules/variables.
  - `CamelCase` for structs/enums/traits.
  - descriptive system names like `sys_prepare_render_data`.
- Keep public APIs explicit in `mod.rs`; avoid broad re-exports unless needed.

## Testing Guidelines
- Add unit tests with each non-trivial behavior change.
- Test names should describe outcome, e.g. `test_plane_distance_to_slice_index_matches_center_convention`.
- For rendering/viewer changes, include:
  - correctness test (geometry/bounds),
  - regression test (previous bug),
  - WASM compile check.

## Commit & Pull Request Guidelines
- Commit style in history is short, imperative, and scoped, e.g.:
  - `Perf: incremental SDF ROI updates...`
  - `Phase A: wire chunk runtime...`
  - `Docs: track performance plan...`
- Keep commits focused; avoid mixing refactors with behavior changes.
- PRs should include:
  - problem statement,
  - approach summary,
  - validation steps/commands run,
  - screenshots or short clips for UI/visual rendering changes.

## Plan Tracking Discipline
- If an active plan document exists, update it as implementation progresses.
- Maintain an **Implementation Status** log with:
  - current phase/state,
  - concise completed/pending checklist items,
  - commit hashes for each plan-relevant checkpoint.
- When a code change advances the plan, include the plan-doc update in the same commit whenever practical; do not leave status tracking behind code changes.
