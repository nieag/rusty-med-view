# Codebase Review: rusty-med-view

## Context

This review covers the `rusty-med-view` codebase — a medical image viewer and segmentation editor built in Rust with WebGPU (wgpu), winit, egui, and an ECS architecture (hecs). The project supports both native desktop and WASM targets, loading NIfTI medical images and providing contour-based segmentation with real-time SDF/mesh generation. The codebase is ~12,700 LOC across 44 Rust source files with 104 unit tests.

> **Progress:** 8/17 findings resolved. See individual items for commit references.

---

## Critical Priority

### 1. Integer overflow in volume dimension iteration (`src/io/nifti.rs:225-233`)

> ✅ **Fixed** — `43e9a81` (Fix: critical review findings — overflow, panics, deps)

**Issue:** Volume voxel iteration casts `depth`, `height`, `width` (u32) to `u16`:
```rust
for z in 0..depth as u16 {
    for y in 0..height as u16 {
        for x in 0..width as u16 {
```
Any volume dimension exceeding 65,535 silently wraps, producing corrupt data and wrong voxel count. Medical datasets (e.g., whole-body CT) can exceed this. Additionally, `width * height * depth` as `usize` on line 213 can overflow for very large volumes.

The identical issue exists in `load_label_from_bytes()` at lines 309-314.

**Impact:** Silent data corruption on large volumes; impossible-to-diagnose rendering artifacts. In a medical imaging context, silently producing truncated data is a patient-safety concern.

**Recommendation:** Validate dimensions fit in u16 at load time and return `LoadError::DimensionError` if they don't. For the multiplication, use checked arithmetic: `width.checked_mul(height).and_then(|wh| wh.checked_mul(depth))`.

**Effort:** Small (~30 min)

---

### 2. `panic!()` calls in rendering hot path (`src/render/pipeline.rs:354,367`)

> ✅ **Fixed** — `43e9a81` (Fix: critical review findings — overflow, panics, deps)

**Issue:** `panic!("Surface out of memory")` in `render_frame()` crashes the entire application on a recoverable GPU condition. On WASM, this kills the tab.

**Impact:** Unrecoverable crash on transient GPU memory pressure. Users lose unsaved segmentation work.

**Recommendation:** Log the error and skip the frame (matching the existing `Timeout` handling pattern already in the same function), or propagate as `Result`.

**Effort:** Small (~15 min)

---

### 3. Package name mismatch (`Cargo.toml:2`)

**Issue:** `name = "rust_starter_app"` — clearly a leftover from a project template. The binary, library crate, and WASM deploy URL all carry this name. `lib.rs` exports `rust_starter_app::run()`. CI references `rust_starter_app.wasm`.

**Impact:** Confusing for contributors; the WASM artifact is deployed as `rust_starter_app_bg.wasm`.

**Recommendation:** Rename to `rusty_med_view` for the crate name. Update `main.rs`, `index.html`, CI deploy config, and `Trunk.toml` accordingly.

**Effort:** Medium (~1 hour, touches multiple files and CI)

---

## High Priority

### 4. WASM-only dependencies compiled for native builds (`Cargo.toml:16-21`)

**Issue:** `wasm-bindgen`, `wasm-bindgen-futures`, `js-sys`, `web-sys`, `console_error_panic_hook`, and `console_log` are in the global `[dependencies]` section. Only `getrandom` is correctly gated under `[target.'cfg(target_arch = "wasm32")'.dependencies]`.

**Impact:** Slower native compile times; unnecessary dependency resolution.

**Recommendation:** Move all 6 WASM-specific crates under `[target.'cfg(target_arch = "wasm32")'.dependencies]`. Add `#[cfg(target_arch = "wasm32")]` guards on their `use` statements (most already exist).

**Effort:** Medium (~45 min, need to verify each import site)

---

### 5. Unused `cgmath` dependency (`Cargo.toml:11`)

> ✅ **Fixed** — `43e9a81` (Fix: critical review findings — overflow, panics, deps)

**Issue:** `cgmath = "0.18"` is listed but all math code uses `glam`. No `use cgmath` appears anywhere in the source.

**Impact:** Unnecessary compile-time cost and dependency bloat.

**Recommendation:** Remove `cgmath` from `Cargo.toml`.

**Effort:** Trivial (~2 min)

---

### 6. Duplicated NIfTI loading logic (`src/io/nifti.rs:161-261` vs `264-326`)

> ✅ **Fixed** — `2963228` (Refactor: extract shared NIfTI parse helper (#6))

**Issue:** `load_nifti_from_bytes()` and `load_label_from_bytes()` share ~60% identical code: gzip detection, decompression, header parsing, dimension validation, vox_offset checks, and volume reading. The code contains a comment: *"For brevity, I will re-implement the core loop or refactor later."*

**Impact:** Bug fixes (like the u16 overflow above) must be applied in two places.

**Recommendation:** Extract a shared `parse_nifti_header_and_volume(data: &[u8]) -> Result<(NiftiHeader, InMemNiftiVolume, [u32; 3]), LoadError>` function. Each loader calls this, then applies its own voxel interpretation (float scaling vs. u8 clamping).

**Effort:** Medium (~1 hour)

---

### 7. `render_frame()` god function (`src/render/pipeline.rs:275-859`)

> ✅ **Fixed** — `28c9199` (Refactor: decompose render_frame into per-pass functions (#7))

**Issue:** A single 585-line function that takes 15 parameters and handles: system ticks, GUI preparation, overlay sync, uniform updates, volume rendering, contour rendering, SDF preview, 3D mesh rendering, and GUI rendering. The CI globally suppresses `clippy::too-many-arguments` to accommodate this.

**Impact:** Extremely difficult to review, test, or modify individual rendering stages without risking side effects.

**Recommendation:** Break into named sub-functions per rendering pass:
- `prepare_frame_state(...)` — systems tick, GUI prep, overlay sync
- `render_volume_pass(...)` — main volume slices
- `render_contour_pass(...)` — contour overlay
- `render_sdf_preview_pass(...)` — SDF heatmap
- `render_mesh_pass(...)` — 3D surface mesh
- `render_gui_pass(...)` — egui final pass

Each takes only the parameters it needs. The orchestrator becomes ~50 lines.

**Effort:** Large (~3-4 hours, careful refactoring to preserve behavior)

---

### 8. `RenderingContext` mega-struct (`src/app/context.rs:19-54`)

> ✅ **Fixed** — `f2f35c2` (Refactor: introduce GpuState, Pipelines, VolumeResources, SceneState sub-structs (#8))

**Issue:** 20+ public fields holding the entire application state as a flat bag: GPU device, pipelines, buffers, ECS world, GUI, caches, event proxy. Every field is `pub`.

**Impact:** Any code with a `&mut RenderingContext` has unrestricted access to everything. No encapsulation, no borrowing granularity.

**Recommendation:** Group related fields into sub-structs:
- `GpuState { device, queue, surface, config }`
- `Pipelines { render, contour, sdf_preview, mesh }`
- `SceneState { world, entities }`

This improves readability and enables finer-grained borrowing.

**Effort:** Large (~3-4 hours)

---

## Medium Priority

### 9. Unguarded `unwrap()` on ECS entity lookups (multiple files)

**Issue:** ~20 non-test `unwrap()` / `expect()` calls on `world.get::<&Component>(entity)` that assume entities always exist:
- `src/gui/sidebar.rs:97` — `world.get::<&mut EditorState>(entities.editor).unwrap()`
- `src/render/protocols.rs:99,100,107,139-157` — 12 `.unwrap()` on world queries
- `src/systems/input.rs:268` — `active_vp.unwrap()`
- `src/app/mod.rs:294` — `placeholder_bg.expect("Main volume should exist")`

**Impact:** If an entity is despawned or a component removed, the application panics.

**Recommendation:** In GUI/systems code, prefer `if let Ok(component) = world.get::<&T>(entity)` or early-return patterns. Reserve `unwrap()` for truly invariant conditions (initialization, compile-time-known sizes).

**Effort:** Medium (~2 hours across multiple files)

---

### 10. `panic!()` in pipeline initialization (`src/render/contour_pipeline.rs:166,169` and `src/render/sdf_preview_pipeline.rs:180,183`)

> ✅ **Fixed** — `dafaef5` (Fix: remove Vec-to-array panics in pipeline constructors (#10))

**Issue:** `Vec::try_into::<[T; 4]>` with `unwrap_or_else(|_| panic!(...))`. These convert a Vec of exactly 4 elements into a fixed-size array.

**Impact:** Technically unreachable panics, but confusing and sets a bad precedent.

**Recommendation:** Use `std::array::from_fn(|i| ...)` to construct the arrays directly, avoiding the Vec-to-array conversion entirely.

**Effort:** Small (~30 min)

---

### 11. No integration tests or end-to-end test infrastructure

**Issue:** All 104 tests are unit tests embedded in source files. No `tests/` directory exists. No integration tests verify cross-module behavior.

**Impact:** Regressions in cross-module interactions go undetected until manual testing.

**Recommendation:** Add a `tests/` directory with at least:
- `tests/nifti_roundtrip.rs` — load a small synthetic NIfTI, verify dimensions/data
- `tests/contour_to_mesh_pipeline.rs` — draw contours -> generate SDF -> extract mesh -> verify vertex count

**Effort:** Medium (~2-3 hours)

---

### 12. Magic numbers without documentation

**Issue:** Several hardcoded constants lack explanation:
- `256` for uniform buffer alignment (`src/render/pipeline.rs:185,338`) — should query `device.limits().min_uniform_buffer_offset_alignment`
- `131072` for `MAX_CONTOUR_LINES` (`src/render/contour_pipeline.rs:68`)
- `64` for `MAX_OVERLAY_PRIMITIVES` (`src/render/pipeline.rs:116`)
- `0.05` for pixel scroll delta conversion (`src/app/mod.rs:115`)

**Impact:** Contributors cannot tell if these values are GPU requirements, performance tuning parameters, or arbitrary choices.

**Recommendation:** Add doc comments explaining the rationale. For GPU alignment, query from device limits at runtime.

**Effort:** Small (~30 min)

---

### 13. `LoadError` doesn't chain underlying errors (`src/io/nifti.rs:27-32`)

**Issue:** `LoadError` variants store `String` instead of the original error. The `std::error::Error` impl has no `source()` override.

**Impact:** Error context is lost at every boundary; debugging requires string matching.

**Recommendation:** Store the original error types (or use `Box<dyn Error>`) and implement `source()`. Alternatively, adopt `thiserror` for automatic derivation.

**Effort:** Small (~45 min)

---

## Low Priority

### 14. No benchmarks for performance-critical algorithms

**Issue:** The contour-to-SDF and marching cubes algorithms are performance-critical but have no benchmarks.

**Recommendation:** Add Criterion.rs benchmarks for `contour_to_sdf` and `marching_cubes` with representative volumes (64^3, 128^3, 256^3).

**Effort:** Medium (~2 hours)

---

### 15. No doctests or public API documentation examples

**Issue:** Public functions have narrative doc comments but no executable examples.

**Recommendation:** Add doctests to key public APIs: `load_nifti_from_bytes`, `VolumeData::aspect_ratios`, `SdfVolume::new`, `marching_cubes`.

**Effort:** Small (~1 hour)

---

### 16. `#[allow(dead_code)]` markers suggest incomplete features (`src/systems/segment_system.rs`)

> ✅ **Fixed** — `43e9a81` (Fix: critical review findings — overflow, panics, deps)

**Issue:** Two functions (`regenerate_live_chunk_meshes`, `merge_chunk_meshes`) are annotated with `#[allow(dead_code)]`, indicating they were written for a chunked mesh regeneration feature but never integrated.

**Recommendation:** Either integrate the functions or remove them. Dead code rots and creates false confidence in test coverage.

**Effort:** Trivial (~15 min per function, or medium if integrating)

---

### 17. No `rustfmt.toml` or `clippy.toml` for team conventions

**Issue:** While CI enforces `rustfmt` and `clippy`, there's no project-level configuration. The CI allows `clippy::too-many-arguments` globally, which masks the `render_frame` issue.

**Recommendation:** Add `rustfmt.toml` with explicit settings and `clippy.toml` to document intentional lint suppressions. Suppress `too-many-arguments` only on specific functions rather than globally.

**Effort:** Trivial (~15 min)

---

## Summary

| Priority | Total | Resolved | Remaining |
|----------|-------|----------|-----------|
| Critical | 3     | 2 ✅     | 1 (#3)    |
| High     | 5     | 4 ✅     | 1 (#4)    |
| Medium   | 5     | 0        | 5         |
| Low      | 4     | 0        | 4         |
| **Total**| **17**| **8 ✅** | **9**     |

## Recommended Implementation Order

**Phase 1 — Safety and correctness (1-2 days):**
1. Fix integer overflow and u16 truncation in nifti.rs (#1)
2. Replace rendering panics with graceful error handling (#2)
3. Extract shared NIfTI parsing logic (#6, fixes #1 in one place)
4. Rename package from `rust_starter_app` (#3)
5. Gate WASM dependencies to target arch (#4)
6. Remove unused `cgmath` (#5)

**Phase 2 — Robustness (2-3 days):**
7. Replace unwrap chains with `if let Ok(...)` pattern (#9)
8. Replace Vec-to-array panics with `std::array::from_fn` (#10)
9. Query uniform alignment from device limits (#12)
10. Define error handling convention (#13)

**Phase 3 — Architecture (3-5 days):**
11. Decompose `render_frame()` into pass functions (#7)
12. Restructure `RenderingContext` into sub-structs (#8)
13. Add integration tests (#11)
14. Decide on chunked mesh feature (#16)

---

## Strengths Worth Noting

- **Zero `unsafe` blocks** across the entire codebase — excellent for a GPU-heavy application
- **104 well-structured unit tests** with good coverage of algorithmic code (data structures, NIfTI parsing, quaternion math, contour operations, picking, drawing, segment management, coordinate system parity)
- **Clean ECS architecture** with singleton entity registry (`AppEntities`) for O(1) access
- **Cross-platform design** — single codebase for native + WASM with minimal conditional compilation
- **Thorough feature documentation** in `docs/features/` with phased implementation plans
- **CI/CD pipeline** with lint, test, native build, and WASM build jobs
- **Good shader/CPU parity testing** for coordinate system math in `util/orientation.rs`
- **Robust NIfTI error types** — `LoadError` enum with Display/Error impls follows Rust best practices
- **Well-designed segmentation pipeline** — contour-to-SDF-to-mesh with frame-budget-aware incremental updates
