# GEMINI.md - Project Context & Setup

This document provides a technical overview of the **Rust Medical Imaging Viewer** to assist future development sessions.

## 🚀 Project Overview
A high-performance 2D/3D medical volume viewer built with **Rust** and **WGPU**. It supports orthogonal slicing, volumetric X-ray rendering, and interactive crosshair picking.

### 📈 Recent Progress
- **Refactor**: Monolithic `systems.rs` architecture split into modular submodules.
- **Performance**: Integrated O(1) ECS singleton registry via `AppEntities`.
- **Latency**: Implemented zero-lag interaction loop (GUI state fed into current frame).
- **UX**: Added trackpad-friendly `Ctrl/Cmd + Left Click` panning and continuous crosshair updates.
- **Precision**: Fixed picking offsets and transitioned to clinical HU-based windowing.

## 🛠 Tech Stack
- **Graphics**: [wgpu](https://github.com/gfx-rs/wgpu) (using Metal, WebGPU/WebGL2)
- **ECS (Entity Component System)**: [hecs](https://github.com/Ralith/hecs)
- **UI**: [egui](https://github.com/emelk/egui)
- **Windowing**: [winit](https://github.com/rust-windowing/winit)
- **Formats**: [nifti-rs](https://github.com/pka/nifti-rs) for `.nii` and `.nii.gz` volumes.

## 📂 Project Structure
- `src/main.rs`: Simple entry point calling `lib::run()`.
- `src/lib.rs`: Winit event loop, GPU context initialization, and the main rendering loop.
- `src/components.rs`: ECS components and the **`AppEntities` registry** for singleton access.
- `src/systems/`: Modularized core logic:
    - `input.rs`: Mouse, trackpad, and navigation gestures.
    - `picking.rs`: 3D raymarching and viewport projection.
    - `paint.rs`: Discrete voxel labelmap editing.
    - `render_prep.rs`: Uniform preparation and overlay synchronization.
- `src/overlay/`: High-performance UI primitives (markers, crosshairs) managed outside standard ECS queries.
- `src/render.rs`: Rendering infrastructure (pipeline setup, bind group creation, frame rendering).
- `src/load_handlers.rs`: Handlers for async volume/labelmap loading and bind group recreation.
- `src/gui.rs`: egui-based GUI implementation and interactive annotation logic.
- `src/nifti_loader.rs` / `src/volume.rs`: Volume data structures and NIfTI parsing logic.

## 📐 Key Implementation Details

### ECS Singleton Registry (`AppEntities`)
To avoid O(N) query overhead, global components are accessed via a centralized registry:
- **Pattern**: Singleton entity IDs are stored in `AppEntities` during initialization.
- **Access**: Systems use `world.get::<T>(entities.input)` for O(1) performance.

### Zero-Lag Interaction Loop
The render loop is ordered to ensure the lowest possible latency between input and visual feedback:
1. **GUI Prepare**: `gui.prepare` runs first to capture dragging and tool state.
2. **System Prep**: `sys_prepare_render_data` builds uniforms using the *immediately* updated state.
3. **Render**: The frame is drawn using the latest interaction data in the *same* frame.

### Stable Mouse-Centered Zoom
Uses a decoupled pivot system to ensure the image stays stationary under the cursor.
- **Formula**: `VolumeUV = (ScreenUV - Pivot) / Zoom + Pivot + Pan`
- **Pivot**: Updated only during scroll events.
- **Pan Compensation**: `Pan' = Pan + (P' - P) * (1/Zoom - 1)`.

### Trackpad-Friendly Navigation
In addition to Middle-mouse panning:
- **Pan**: `Ctrl + Left Click` or `Cmd + Left Click` + Drag.
- **Rotate (3D)**: `Alt + Left Click` + Drag.

### 3D Picking & Crosshair
Implements high-intensity raymarching to accurately place the crosshair.
- **Continuous Update**: Navigation mode supports live crosshair updates during Left-click drags.
- **Safety**: Tools (Brush/Eraser) are automatically inhibited during Zoom/Pan/Rotate operations.

### Labelmap Overlays
The system supports raw **Hounsfield Unit (HU)** based windowing and translucent overlays.
- **Data Format**: `R32Float` for raw intensities; `R8Uint` for label textures.
- **2D vs 3D**: 2D slices use alpha-blended translucency; 3D view treats labels as solid, ray-terminating structures.

## 🛠 How to Run
- **Native**: `cargo run`
- **Web (WASM)**: `trunk serve` (requires `wasm32-unknown-unknown` target)

## 💡 Future Session Tips
- When modifying interaction logic, check the submodules in `src/systems/`.
- **Coordinate Systems**: Both the shader and CPU picking use object-space raymarching via the volume's inverse rotation.
- **Annotations**: Using the "Locate" feature (🎯) centers the 2D viewports on the world position by modifying `ViewState.pan`.
- **Uniforms**: Ensure `Uniforms` struct alignment in `components.rs` (16-byte boundaries) matches `shader.wgsl` exactly.
