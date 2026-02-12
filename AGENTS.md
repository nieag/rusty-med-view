# Repository Guidelines

## Project Structure & Module Organization
- `src/` contains the application code.
- Core areas:
  - `src/app/`: ECS components, app context, segmentation data model.
  - `src/systems/`: runtime systems (input, contouring, segmentation updates, rendering prep).
  - `src/render/`: WGPU pipelines and frame rendering.
  - `src/convert/`: representation conversions (contours, SDF, meshing, chunk utilities).
  - `src/gui/`: egui panels and tooling UI.
  - `src/shaders/`: WGSL shader sources.
- `docs/features/` stores implementation plans and feature design notes.
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
  - descriptive system names like `sys_update_segment_derivatives`.
- Keep public APIs explicit in `mod.rs`; avoid broad re-exports unless needed.

## Testing Guidelines
- Add unit tests with each non-trivial behavior change.
- Test names should describe outcome, e.g. `test_incremental_update_preserves_prior_active_bounds`.
- For rendering/segmentation changes, include:
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
- If an active plan document exists (for example `docs/features/.../performance-plan.md`), update it as implementation progresses.
- Maintain an **Implementation Status** log with:
  - current phase/state,
  - concise completed/pending checklist items,
  - commit hashes for each plan-relevant checkpoint.
- When a code change advances the plan, include the plan-doc update in the same commit whenever practical; do not leave status tracking behind code changes.
