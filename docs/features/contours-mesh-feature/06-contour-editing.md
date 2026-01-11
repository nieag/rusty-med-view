# 06 - Contour Editing Tools

Interactive contour editing in 2D viewports.

## Tools

| Tool | Action | Result |
|------|--------|--------|
| ContourEdit | Click point | Select vertex |
| ContourEdit | Drag point | Move vertex |
| ContourEdit | Double-click edge | Insert vertex |
| ContourEdit | Delete key | Remove vertex |
| ContourDraw | Draw stroke | Create new contour |

## State

```rust
pub struct ContourEditState {
    pub selected: Option<(usize, usize)>,  // (polyline, point)
    pub hover: Option<(usize, usize)>,
    pub dragging: bool,
    pub drag_start: Option<[f32; 2]>,
}
```

## Source Switching

When ContourEdit tool activates:
1. Ensure contours representation exists
2. Set `segment.source = SourceKind::Contours`
3. Invalidate labelmap + mesh (they're now derived)

When switching back to Brush:
1. Ensure labelmap exists (convert if needed)
2. Set `segment.source = SourceKind::Labelmap`
3. Invalidate contours + mesh

## Real-Time Sync

During drag:
- Update contour point position directly
- If 3D viewport visible: update mesh preview (regional)

On release:
- Full mesh reconversion for affected slices
- Mark labelmap dirty

## Subtasks

- [ ] Add `ContourEdit` to `EditorTool` enum
- [ ] Add `ContourEditState` component
- [ ] Implement point hit-testing in `systems/input.rs`
- [ ] Implement drag handling
- [ ] Add visual feedback (hover/selected highlights)
- [ ] Implement source switching logic
- [ ] Add vertex insert/delete

## Files

| File | Change |
|------|--------|
| `src/components.rs` | Add tool + state |
| `src/systems/input.rs` | Handle edits |
| `src/gui.rs` | Tool selector |
| `src/segment.rs` | Source switching |
