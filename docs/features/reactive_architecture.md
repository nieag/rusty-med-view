# Reactive Architecture Design

This document describes the transition from a polling-based "Game Loop" architecture to a reactive, event-driven model for the Medical Imaging Viewer.

## ⚠️ Current Polling Architecture

Currently, the application uses the `about_to_wait` hook in `winit` to perform background polling.

```rust
// Current lib.rs approach
fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
    // 1. Check for async file loads (Polling)
    if let Ok(result) = ctx.volume_receiver.try_recv() { ... }
    
    // 2. Run continuous systems
    systems::sys_paint(&mut ctx.world, &ctx.entities, &ctx.queue);
    
    // 3. Force a redraw
    ctx.window.request_redraw();
}
```

### Disadvantages
- **High Resource Usage**: The app consumes CPU cycles even when idle.
- **Winit Best Practices**: Winit documentation explicitly advises against using `about_to_wait` for typical apps.
- **Battery Impact**: Constant polling prevents the CPU from entering low-power states.

---

## ✅ Proposed Reactive Architecture

The new architecture follows a "Wake-on-Interaction" model. The application only performs work when an event (user input, window resize, or async completion) occurs.

### 1. `UserEvent` Enum
We define a custom event type to allow background threads to communicate with the main event loop.

```rust
pub enum AppEvent {
    VolumeLoaded(Result<LoadedVolume, LoadError>),
    LabelLoaded(Result<LoadedLabel, LoadError>),
    RequestBindGroupRebuild,
}
```

### 2. `EventLoopProxy`
A thread-safe proxy used to "wake up" the event loop from anywhere.

- **Async Loading**: Instead of polling a receiver, the loading thread will call `proxy.send_event(AppEvent::VolumeLoaded(data))`.
- **UI Interaction**: When a button in `egui` is clicked, it can send a signal to the backend to perform heavy lifting.

### 3. Reactive Redraws
Redraws are no longer forced every loop. They are requested explicitly:
- **Navigation**: `WindowEvent::CursorMoved` or `MouseWheel` calls `window.request_redraw()`.
- **Painting**: The `paint` system only runs when a mouse-down event is active.
- **Load Complete**: Handling a `UserEvent` triggers a redraw.

### 4. Simplified `RedrawRequested`
Logic that must happen "just-in-time" for rendering (like updating uniforms or running the paint head) is moved directly into the redraw handler.

---

## 🛠 Implementation Details

### Viewport Synchronization
Systems in `src/systems/` will be updated to return a boolean indicating if state changed. If `true`, the event loop will call `window.request_redraw()`.

### Handling Async Drops
By using `UserEvent` instead of `try_recv`, we ensure that volume loading is handled as a high-priority interrupt, reducing the latency between "File Loaded" and "Image Displayed."

### Resource Cleanup
The `about_to_wait` function will be completely removed, leaving an idle loop that consumes 0% CPU when no interaction is happening.
