# 02: Overlay System Refactor

Extract and abstract the overlay rendering system.

## Problem

Current implementation mixes:
- Annotation markers
- Brush preview
- Future: measurements, slice intersections

All share one flat `Vec<OverlayPrimitive>` with no source tracking.

## Proposed Structure

```
src/
├── overlay/
│   ├── mod.rs           # OverlayManager + re-exports
│   ├── primitives.rs    # GPU-facing OverlayPrimitive struct
│   ├── annotations.rs   # Annotation-specific logic
│   └── brush.rs         # Brush preview logic
```

## New Types

### OverlayManager (replaces OverlayState)

```rust
pub struct OverlayManager {
    annotations: Vec<AnnotationMarker>,
    brush_preview: Option<BrushPreview>,
    // Future: measurements, slice_lines
    
    // Interaction state
    hovered_idx: Option<usize>,
    dragging_idx: Option<usize>,
}

impl OverlayManager {
    pub fn collect_primitives(&self) -> Vec<OverlayPrimitive> { ... }
    pub fn hit_test(&self, screen_uv: [f32; 2], viewport: u32) -> Option<HitResult> { ... }
}
```

### Source-Specific Types

```rust
pub struct AnnotationMarker {
    pub world_pos: Vec3,
    pub annotation_idx: usize,  // Back-reference to AnnotationState
}

pub struct BrushPreview {
    pub center_voxel: [f32; 3],
    pub size: f32,
    pub viewport: u32,
}
```

## Migration Steps

1. Create `src/overlay/` directory
2. Move `OverlayPrimitive`, `OverlayPrimitiveKind` → `overlay/primitives.rs`
3. Create `OverlayManager` in `overlay/mod.rs`
4. Update `lib.rs` to spawn `OverlayManager` instead of `OverlayState`
5. Update `systems.rs` to use new API
6. Update `gui.rs` annotation dragging

## Files Changed

| File | Change |
|------|--------|
| `src/overlay/mod.rs` | NEW |
| `src/overlay/primitives.rs` | NEW - from components.rs |
| `src/components.rs` | Remove overlay types |
| `src/lib.rs` | Spawn OverlayManager |
| `src/systems.rs` | Use OverlayManager API |
| `src/gui.rs` | Update drag handling |
