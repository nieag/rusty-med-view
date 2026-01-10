# 03: Split systems.rs

Break 1294-line `systems.rs` into focused submodules.

## Current Structure

| Function | Lines | Purpose |
|----------|-------|---------|
| `sys_update_modifiers` | 13-18 | Input |
| `get_hu_at_mouse` | 20-47 | Picking |
| `sys_prepare_render_data` | 49-187 | Render prep |
| `sys_sync_annotations_to_overlay` | 189-218 | Overlay |
| `get_overlay_render_data` | 220-238 | Overlay |
| `sys_handle_input_scroll` | 240-348 | Input |
| `sys_update_mouse` | 350-408 | Input |
| `intersect_aabb` | 410-434 | Picking |
| `get_voxel_at_mouse` | 442-671 | Picking |
| `sys_handle_mouse_button` | 673-760 | Input |
| `sys_handle_mouse_drag` | 762-876 | Input |
| `sys_paint` | 878-1233 | Paint (350+ lines) |
| `apply_window` | 1237-1255 | Windowing (test-only) |

## Proposed Structure

```
src/systems/
├── mod.rs           # Re-exports only
├── input.rs         # Mouse, scroll, modifiers, drag
├── picking.rs       # Voxel picking, raymarching, AABB
├── paint.rs         # Brush painting system
└── render_prep.rs   # Uniforms, overlay sync
```

## Module Contents

### input.rs (~250 lines)
- `sys_update_modifiers`
- `sys_update_mouse`
- `sys_handle_input_scroll`
- `sys_handle_mouse_button`
- `sys_handle_mouse_drag`

### picking.rs (~280 lines)
- `get_voxel_at_mouse`
- `get_hu_at_mouse`
- `intersect_aabb`

### paint.rs (~360 lines)
- `sys_paint` (entire function)

### render_prep.rs (~200 lines)
- `sys_prepare_render_data`
- `sys_sync_annotations_to_overlay`
- `get_overlay_render_data`

### mod.rs
```rust
mod input;
mod paint;
mod picking;
mod render_prep;

pub use input::*;
pub use paint::*;
pub use picking::*;
pub use render_prep::*;
```

## Migration Steps

1. Create `src/systems/` directory
2. Create `mod.rs` with re-exports
3. Move functions one module at a time
4. Update imports in each new module
5. Test after each module migration

## Shared Dependencies

All modules need:
```rust
use crate::components::*;
use hecs::World;
```

`picking.rs` additionally needs:
```rust
use glam::{Mat3, Quat, Vec3};
```

`input.rs` additionally needs:
```rust
use winit::event::{ElementState, MouseButton};
use winit::keyboard::ModifiersState;
```
