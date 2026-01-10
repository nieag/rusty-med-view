# 05: Annotation System Fix

Fix annotation rendering, dragging, and offset issues.

## Current Issues

| Issue | Cause |
|-------|-------|
| **Lag when moving** | Position update happens in egui callback, GPU sees old position until next frame |
| **Not proper circles** | Shader radius calculation: `radius * zoom * 0.02` is approximate, doesn't account for aspect |
| **Offset from mouse** | Drag uses egui pointer pos → world pos → GPU projection, each with different coordinate systems |

## Architecture Problem

```
                      ┌─────────────────────┐
                      │   gui.rs            │
                      │ draw_annotations()  │
                      │ - egui drag detect  │
                      │ - updates world_pos │
                      │ - draws text labels │
                      └─────────┬───────────┘
                                │
            updates Annotation.world_pos
                                │
                                ▼
┌─────────────────────────────────────────────────────────┐
│              sys_sync_annotations_to_overlay()          │
│    Copies Annotation positions → OverlayPrimitive vec   │
│              (Called AFTER gui.prepare())               │
└─────────────────────────────────────────────────────────┘
                                │
                     Too late - GUI already rendered!
                                ▼
┌─────────────────────────────────────────────────────────┐
│                    Shader                               │
│  Reads overlay_primitives storage buffer                │
│  Projects world_pos → screen using DIFFERENT formula    │
└─────────────────────────────────────────────────────────┘
```

**Root cause**: GUI interaction and GPU rendering use different projection formulas.

## Specific Bugs

### 1. Non-interactive Area containing interactive widgets

```rust
// gui.rs:477-479
egui::Area::new("annotations_layer".into())
    .fixed_pos(central_rect.min)
    .interactable(false)  // ← But draw_annotations creates interactive widgets!
```

### 2. Drag position mismatch

In `gui.rs:640-682`, drag converts screen → world coords using:
```rust
let world_u = ((ndc_x - pivot[0]) * k / zoom) + pivot[0] + pan[0];
```

But shader uses different formula at line 473-474:
```wgsl
let rel_pos = (uv - pan - pivot) * zoom;
screen_pos = (rel_pos / vec2<f32>(k, 1.0)) + pivot;
```

The inverse formulas don't match!

### 3. sys_sync called AFTER gui.prepare

In `render.rs:283-284`:
```rust
systems::sys_sync_annotations_to_overlay(world);  // Sync here
// ...
gui.prepare(window, world, volume_sender);  // GUI already drew with OLD positions!
```

### 4. Circle size is wrong

Shader line 526:
```wgsl
let screen_radius = radius * uniforms.zoom * 0.02; // Magic number, ignores aspect
```

This produces ellipses on non-square viewports.

## Proposed Fixes

### Fix 1: Move interaction to systems (not egui)

- Remove egui-based drag detection from `draw_annotations()`
- Add `sys_handle_annotation_interaction(world)` in systems
- Update `InputState` with hit-test results before GUI runs

### Fix 2: Render circles in egui instead of shader

- Simpler approach: let egui draw circles with `painter.circle()`
- Remove annotation rendering from shader entirely
- Only use shader overlays for things that need GPU precision (brush preview)

### Fix 3: If keeping shader circles, fix projection

**Ensure matching formulas:**

Egui (screen → world):
```rust
world_u = ((ndc_x - 0.5) * k / zoom) + 0.5 + pan[0]
world_v = ((ndc_y - 0.5) / zoom) + 0.5 + pan[1]
```

Shader (world → screen):
```wgsl
screen_x = ((world_u - 0.5 - pan.x) * zoom / k) + 0.5
screen_y = ((world_v - 0.5 - pan.y) * zoom) + 0.5
```

### Fix 4: Sync annotations BEFORE gui.prepare

```rust
// render.rs
systems::sys_sync_annotations_to_overlay(world);
gui.prepare(window, world, volume_sender);  // Now has updated positions
```
Already in this order - actual issue is GUI renders text at new pos, shader renders circle at old.

### Fix 5: For low-lag drag, use CPU-based rendering

Just like brush preview uses `brush_center_voxel` uniform, annotations could pass `dragged_annotation_pos` directly without going through storage buffer.

## Recommended Approach

**Option A: Pure egui rendering (simpler)**
- Draw circles via `painter.circle()` in egui
- Remove shader overlay primitive loop for annotations
- Keep shader for brush preview only

**Option B: Fix shader pipeline (more complex)**
- Fix projection formulas to match exactly
- Pass dragged annotation position as uniform (like brush)
- Fix circle aspect ratio in shader

## Files to Modify

| File | Change |
|------|--------|
| `gui.rs` | Either: add circle drawing, or remove interaction |
| `systems.rs` | Add annotation interaction system if keeping in systems |
| `shader.wgsl` | Fix circle projection or remove annotation rendering |
| `render.rs` | Adjust sync order / add dragged pos uniform |
| `components.rs` | Add `dragged_annotation_uniform` to Uniforms if needed |
