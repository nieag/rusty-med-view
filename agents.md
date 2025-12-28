# Project Overview: Rust WGPU Medical Viewer

This project is a high-performance 2D/3D medical imaging viewer built with **Rust**, **wgpu**, and **hecs** (ECS). It provides a synchronized 4-quadrant view for exploring volumetric data.

## Architecture

- **ECS Core (`hecs`)**: Manages state through components (`Transform`, `InputState`, `ViewState`, `VolumeData`).
- **Rendering (`wgpu`)**: Uses a single optimized pipeline and a unified WGSL shader.
- **Multiview Shader**: Renders different views (3D Raymarching or 2D Slices) based on a `view_mode` uniform.
- **Synchronized Navigation**: All viewports share a single 3D cursor position. Moving the cursor in one view updates the slices in all others.

## Current Interaction Model

| Feature | Control | Description |
| :--- | :--- | :--- |
| **Zoom** | `Ctrl + Scroll` | Zoom centered on the mouse cursor position. |
| **Pan** | `Middle-click` or `Alt+Drag` | Move the data within the viewport. |
| **3D Picking** | `Left-click` (3D View) | Uses a MIP (Maximum Intensity Projection) heuristic to snap to the densest anatomy. |
| **2D Picking** | `Left-click` (2D View) | Places the crosshair and shifts other views to that slice coordinate. |
| **Slice Move** | `Scroll` (no Ctrl) | Steps through the volume slices in the active viewport. |

## Key Technical Details

- **Uniform Alignment**: The `Uniforms` struct is 64-byte aligned (vec4 → vec2 → f32) to satisfy strict WGSL requirements without wasted padding.
- **CPU Volume Copy**: A 256KB copy of the volume density (alpha) is kept in the `VolumeData` component for instant CPU-side raymarching (picking).
- **Edge Handling**: Manual bounds checking in the shader ensures clean backgrounds instead of stretched edges when panning.

## Roadmap & Future Direction

1. **Real Data**: Replace the synthetic "Phantoma" generator with a DICOM/NIfTI loader.
2. **Transfer Functions**: Implement a GUI-controlled color/opacity ramp for better visualization of different tissue densities.
3. **Window/Level**: Add contrast and brightness adjustments (standard medical W/L).
4. **Volume Performance**: Implement empty-space skipping or octree acceleration for larger datasets.
5. **Measurements**: Add 3D distance and ROI (Region of Interest) measurement tools.

## Setup Instructions
- Requires a GPU supporting WebGPU/Vulkan/Metal/DX12.
- Run with `cargo run`.
- Shaders are located in `src/shaders/shader.wgsl` and are hot-reloaded on app restart.
