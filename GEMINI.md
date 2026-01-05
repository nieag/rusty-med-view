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
- `src/components.rs`: ECS components (`ViewState`, `InputState`, `Uniforms`, `VolumeData`, etc.).
- `src/systems.rs`: Core logic for input handling (zoom, pan, scroll, rotation) and GPU data preparation.
- `src/render.rs`: Rendering infrastructure (pipeline setup, bind group creation, frame rendering).
- `src/load_handlers.rs`: Handlers for async volume/labelmap loading results and bind group recreation.
- `src/gui.rs`: egui-based GUI implementation for file loading and layer controls.
- `src/file_dialog.rs`: Cross-platform file dialog supporting both native (rfd) and WASM (web-sys).
- `src/geometry.rs`: Vertex struct definition and fullscreen quad geometry.
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

### Labelmap Overlays
The system supports up to 2 simultaneous labelmap overlays.
- **Data Format**: Labels are stored as `R8Uint` textures (`t_label1`, `t_label2`).
- **Color Lookup**: An `R8` index is mapped to an RGBA color via a 1D LUT texture (`t_lut1`, `t_lut2`).
- **Visibility**: Controlled via `uniforms.overlay_opacities` and `uniforms.overlay_flags`.

### 2D vs 3D Label Rendering
- **2D Slices**: Labels are rendered with their LUT-defined alpha multiplied by the layer's opacity, allowing the underlying grayscale volume to be visible (translucent).
- **3D X-Ray/MIP**: Labels are rendered as **solid** structures. During raymarching, if a label is encountered, it consumes the remaining ray budget and sets the final color to the label color, making it appear opaque while staying "inside" the volume context.

## 🛠 How to Run
- **Native**: `cargo run`
- **Web (WASM)**: `trunk serve` (requires `wasm32-unknown-unknown` target)

## 💡 Future Session Tips
- When modifying interaction logic, check `src/systems.rs` first.
- Always ensure `Uniforms` struct alignment in `components.rs` matches `shader.wgsl` exactly.
- The 3D view uses **volume-based rotation** (not camera orbiting). The camera is fixed at `(0, 0, -3.5)`, and the quaternion in `ViewState.rotation[0]` rotates the volume in object space.

### Volume Orientation & 3D Gizmo
- **NIfTI Orientation**: The `sform` affine matrix is extracted from NIfTI headers and converted to a quaternion (stored in `VolumeData.orientation`). On load, this initializes the 3D view rotation.
- **3D Gizmo**: A tri-axis gizmo (R/A/S labels for Right, Anterior, Superior) is rendered in the top-left corner of the 3D viewport. Uses depth-based fading to show which axes point toward/away from the camera.
- **Object-Space Raymarching**: Both the shader and CPU-side picking (`get_voxel_at_mouse`) transform rays into object space using the inverse of the volume rotation matrix.
