# 04: ECS Singleton Refactor

Improve singleton component access patterns.

## Problem

Current code spawns singleton entities and queries them with loops:

```rust
// Wasteful - iterates all entities to find one
for (_, input) in world.query::<&InputState>().iter() {
    viewport = input.active_viewport;
}
```

## Options

### Option A: Store Entity IDs

```rust
pub struct AppEntities {
    pub input: hecs::Entity,
    pub view: hecs::Entity,
    pub editor: hecs::Entity,
    pub cursor: hecs::Entity,
    pub gui_state: hecs::Entity,
    pub windowing: hecs::Entity,
    pub annotations: hecs::Entity,
    pub overlay: hecs::Entity,
}

// Usage
let input = world.get::<&InputState>(entities.input).unwrap();
```

**Pros:** Minimal code change, ECS-compatible
**Cons:** Still ECS overhead, need to pass `AppEntities` around

### Option B: Move to RenderingContext

```rust
pub struct RenderingContext {
    // ... GPU resources ...
    
    // State (not in ECS)
    pub input: InputState,
    pub view: ViewState,
    pub editor: EditorState,
    pub windowing: VolumeWindowing,
    pub annotations: AnnotationState,
    pub overlay: OverlayManager,
}
```

**Pros:** Direct access, no queries
**Cons:** Larger refactor, breaks ECS pattern

### Recommended: Option A (Entity IDs)

Less disruptive, maintains ECS for actual entities (volumes, layers).

## Affected Singletons

| Component | Current | Proposed |
|-----------|---------|----------|
| `InputState` | Queried in loops | `entities.input` |
| `ViewState` | Queried in loops | `entities.view` |
| `EditorState` | Queried in loops | `entities.editor` |
| `GuiState` | Queried in loops | `entities.gui` |
| `VolumeWindowing` | Queried in loops | `entities.windowing` |
| `AnnotationState` | Queried in loops | `entities.annotations` |
| `OverlayState` | Queried in loops | `entities.overlay` |
| `CameraRig` | Queried in loops | `entities.camera` |
| `VolumeLoadingState` | Queried in loops | `entities.loading` |

## Migration Steps

1. Add `AppEntities` struct to `components.rs`
2. Store in `RenderingContext`
3. Update `create_rendering_context` to populate entity IDs
4. Update systems to use `world.get::<T>(entity)` instead of queries
5. Consider helper methods like `get_input(world, entities)`

## Example Refactor

Before:
```rust
pub fn sys_update_modifiers(world: &mut World, mods: ModifiersState) {
    for (_, input) in world.query::<&mut InputState>().iter() {
        input.modifiers = mods;
    }
}
```

After:
```rust
pub fn sys_update_modifiers(world: &mut World, entities: &AppEntities, mods: ModifiersState) {
    if let Ok(mut input) = world.get::<&mut InputState>(entities.input) {
        input.modifiers = mods;
    }
}
```
