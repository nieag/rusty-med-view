# 01: Quick Cleanup

Remove dead code, fix Clippy warnings, eliminate LLM artifacts.

## Files to Modify

### [components.rs](file:///Users/nieage/dev/git/rust_starter_app/src/components.rs)

**Remove unused imports (line 1-4):**
```diff
-use glam::{Mat4, Vec3, Vec4};
-use std::sync::Arc;
+use glam::Vec3;
 use web_time::Instant;
-use wgpu::BindGroup;
 use winit::keyboard::ModifiersState;
```

**Remove `CameraRig.radius`** - Field is never read.

---

### [lib.rs](file:///Users/nieage/dev/git/rust_starter_app/src/lib.rs)

**Fix unused variables (line 125):**
```diff
-let (label_tex, label_view, label_sampler) = volume::create_demo_labelmap(&device, &queue);
+let (_label_tex, label_view, _label_sampler) = volume::create_demo_labelmap(&device, &queue);
```

**Fix redundant field names (lines 497-498):**
```diff
-view: view,
-sampler: sampler,
+view,
+sampler,
```

---

### [load_handlers.rs](file:///Users/nieage/dev/git/rust_starter_app/src/load_handlers.rs)

**Fix irrefutable pattern (line 133):**
```diff
-if let Representation::Voxel(res) = r {
-    overlay_views.push(res.view.clone());
-}
+let Representation::Voxel(res) = r;
+overlay_views.push(res.view.clone());
```

---

### [systems.rs](file:///Users/nieage/dev/git/rust_starter_app/src/systems.rs)

**Fix irrefutable pattern (line 929)** - Same as above.

**Move `apply_window` to `#[cfg(test)]`** - Only used in tests.

---

### [geometry.rs](file:///Users/nieage/dev/git/rust_starter_app/src/geometry.rs)

**Remove redundant import (line 1):**
```diff
-use wgpu;
```

---

### [gui.rs](file:///Users/nieage/dev/git/rust_starter_app/src/gui.rs)

**Remove unnecessary `mut` (line 487):**
```diff
-Some((_, mut state)),
+Some((_, state)),
```

---

## Verification

```bash
cargo clippy --all-targets 2>&1 | grep -E "^warning:" | wc -l  # Target: 0
cargo build
cargo test
```
