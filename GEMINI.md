# GEMINI.md - Project Context & Setup

This document provides a technical overview of the **Rust Medical Imaging Viewer** to assist future development sessions.

## 🚀 Project Overview
A high-performance 2D/3D medical volume viewer built with **Rust** and **WGPU**. It supports orthogonal slicing, volumetric X-ray rendering, and interactive crosshair picking.

## 🛠 Tech Stack
- **Graphics**: [wgpu](https://github.com/gfx-rs/wgpu) (using Metal, WebGPU/WebGL2)
- **ECS (Entity Component System)**: [hecs](https://github.com/Ralith/hecs)
- **UI**: [egui](https://github.com/emelk/egui)
- **Windowing**: [winit](https://github.com/rust-windowing/winit)
- **Formats**: [nifti-rs](https://github.com/pka/nifti-rs) for `.nii` and `.nii.gz` volumes.

## 📂 Project Structure
- `src/main.rs`: Simple entry point calling `lib::run()`.
- `src/lib.rs`: Winit event loop, GPU context initialization, and the main rendering loop.
- `src/components.rs`: ECS components (`ViewState`, `InputState`, `Uniforms`).
- `src/systems.rs`: Core logic for input handling (zoom, pan, scroll) and GPU data preparation.
- `src/shaders/shader.wgsl`: Unified vertex and fragment shader for all viewports.
- `src/nifti_loader.rs` / `src/volume.rs`: Volume data structures and NIfTI parsing logic.

## 📐 Key Implementation Details

### Viewport Layout
The app renders 4 viewports in a grid:
0. **Top-Left**: 3D X-Ray / MIP View.
1. **Top-Right**: Axial Slice.
2. **Bottom-Left**: Coronal Slice.
3. **Bottom-Right**: Sagittal Slice.

### Stable Mouse-Centered Zoom
Uses a decoupled pivot system to ensure the image stays stationary under the cursor.
- **Formula**: `VolumeUV = (ScreenUV - Pivot) / Zoom + Pivot + Pan`
- **Pivot**: Stored in `ViewState.pivot` and updated only during scroll events.
- **Pan Compensation**: When shifting the pivot to a new mouse position, `Pan` is adjusted: `Pan' = Pan + (P' - P) * (1/Zoom - 1)`.

### Uniform Alignment
The `Uniforms` struct is strictly aligned to 16-byte boundaries (grouping `vec4` types first) to satisfy WGSL requirements. The GPU buffer uses an offset of **256 bytes** per viewport (WGPU's `MIN_BINDING_OFFSET_ALIGNMENT`).

### 3D Picking
Implements high-intensity raymarching in `sys_handle_mouse_button` to accurately place the crosshair on anatomical structures in the 3D view.

## 🛠 How to Run
- **Native**: `cargo run`
- **Web (WASM)**: `trunk serve` (requires `wasm32-unknown-unknown` target)

## 💡 Future Session Tips
- When modifying interaction logic, check `src/systems.rs` first.
- Always ensure `Uniforms` struct alignment in `components.rs` matches `shader.wgsl` exactly.
- The 3D camera is currently fixed at a radius of `3.5`, with `zoom` acting as a view-plane scale multiplier.
