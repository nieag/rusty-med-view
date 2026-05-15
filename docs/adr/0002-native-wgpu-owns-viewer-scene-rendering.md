# Native WGPU Owns Viewer Scene Rendering

Viewport scene content is rendered through WGPU, while egui is limited to GUI controls, dialogs, status text, and temporary diagnostics. This keeps medical image content, ROI overlays, contours, meshes, projection, and viewport clipping in one native rendering path.

## Consequences

Any feature visible inside a viewer viewport needs render-owned data preparation, explicit GPU resources or draw paths, renderer-owned clipping, and shared coordinate conversion helpers.
