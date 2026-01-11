# Viewport Architecture: Tiled vs. Floating Layouts

This document outlines the architectural shift from hardcoded quadrants to a flexible, UI-driven viewport system.

## Tiled Architecture (Primary Strategy)

In the **Tiled Architecture**, the layout is driven by the GUI but remains structured. This is the recommended approach for professional medical workstations.

### Core Concepts

1. **Viewports as First-Class Entities**: Every viewport on screen is represented as an entity in the ECS with a `Viewport` component storing view mode, screen-space rect, and uniform offsets.
2. **The "Layer Cake"**: WGPU handles high-speed medical rendering in the background, while `egui` draws interactive widgets and tool palettes on top.
3. **Direct-to-Screen Strategy**: We avoid the overhead of Render-to-Texture (RTT) by using `egui` for layout math and `set_viewport` for rendering, maintaining a single high-performance `RenderPass`.

### Scalability: Hanging Protocols
This generic design allows for dynamic "Hanging Protocols" (e.g., 2x2, 1x3, or 9x9 grids) without modification to the rendering engine.

---

## Floating Window Architecture (Comparison)

As the application matures, it may be necessary to support a "Floating" or "Virtual Desktop" experience.

### How it Differs
In a **Floating Architecture**, viewports are not confined to a grid. They exist as independent `egui::Window` widgets that can:
- **Overlap**: One viewport can be dragged on top of another.
- **Minimize/Maximize**: Windows can be minimized to a taskbar or maximized to fill the tab.
- **Pop-out**: If supported by the browser context, these could potentially be moved to second monitors.

### Implementation: Render-to-Texture (RTT)
To support overlapping and transparency between windows, the application would transition to **Render-to-Texture**:
1. Each viewport renders its medical data to a private `wgpu::Texture`.
2. `egui` consumes this texture via `ui.image()`.
3. This allows `egui` to handle all Z-index layering and clipping automatically.

### Performance Trade-off
While the Tiled Architecture is the most performant due to zero texture copies, the Floating Architecture provides maximum user freedom at the cost of slight GPU memory overhead and increased complexity in managing texture lifecycles.
