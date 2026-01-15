# GEMINI.md - Project Context & Setup

This document provides a technical overview of the **Rust Medical Imaging Viewer** to assist future development sessions.

## 🚀 Project Overview
A high-performance 2D/3D medical volume viewer built with **Rust** and **WGPU**. It supports orthogonal slicing, volumetric X-ray rendering, and interactive crosshair picking.

### 📈 Recent Progress
- **Orientation Consolidation**: Centralized all 3D transforms in `src/orientation.rs`. Separated **Data Orientation** (from NIfTI) from **User Rotation** (interactive) to allow independent view manipulation and clean resets.
- **Unified 3D API**: Implemented `screen_to_ray_3d` and `volume_to_screen_3d` as the single source of truth for picking and projection across CPU and GPU.
- **Radiological Convention**: Native support for flipped X-axis (Right-on-Left) across picking, 2D slicing, and 3D volume rendering.
- **Parity Testing**: Expanded test suite to 30+ tests, including "Shader Parity" tests that verify Rust math exactly matches WGSL logic for raymarching and projection.

## 🛠 Tech Stack
- **Graphics**: [wgpu](https://github.com/gfx-rs/wgpu) (using Metal, WebGPU/WebGL2)
- **ECS (Entity Component System)**: [hecs](https://github.com/Ralith/hecs)
- **UI**: [egui](https://github.com/emelk/egui)
- **Windowing**: [winit](https://github.com/rust-windowing/winit)
- **Formats**: [nifti-rs](https://github.com/pka/nifti-rs) for `.nii` and `.nii.gz` volumes.

## 📂 Project Structure
- `src/main.rs`: Simple entry point calling `lib::run()`.
- `src/lib.rs`: Winit event loop, GPU context initialization, and the main rendering loop.
- `src/orientation.rs`: **The single source of truth** for all coordinate systems and radiological mappings.
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

### Orientation & Coordinate Systems
The app uses a strict **Radiological convention**:
- **2D Views**: Axial (XY), Coronal (XZ), and Sagittal (YZ) planes.
- **Signage**: Patient Right maps to Screen Left (flipped X).
- **Rotation Composition**: Final view rotation is composed as `user_rotation * BASE_ROTATION * data_orientation`. 
    - `data_orientation` is the intrinsic NIfTI orientation.
    - `BASE_ROTATION` aligns "Superior" to "Up".
    - `user_rotation` handles interactive drags.
- **Parity**: Parity between CPU (Rust) and GPU (WGSL) is maintained via "Shader Emulation" unit tests in `orientation.rs`.

### Aligned 3D Crosshairs
Crosshairs in the 3D view are projected from world-space local axes:
- **Projection**: Calculated in `shader.wgsl` using the volume's rotation matrix and perspective derivatives.
- **Anatomical Colors**: Red (X), Green (Y), Blue (Z) represent the primary anatomical axes.

### ECS Singleton Registry (`AppEntities`)
To avoid O(N) query overhead, global components are accessed via a centralized registry:
- **Pattern**: Singleton entity IDs are stored in `AppEntities` during initialization.
- **Access**: Systems use `world.get::<T>(entities.input)` for O(1) performance.

### Reactive Architecture & Event-Driven Rendering
The application uses a purely reactive model to minimize CPU usage while maintaining high performance:
- **`AppEvent`**: A custom enum (VolumeLoaded, RebuildBindGroups, etc.) that centralizes all asynchronous and UI-triggered wake-ups.
- **`EventLoopProxy`**: Used by background loading threads and GUI interactions to signal the main thread without polling.
- **Explicit Redraws**: Redraws are only requested (`window.request_redraw()`) when a state change occurs (input, loading complete, UI interaction).
- **Zero-Lag Interaction Loop**: When a redraw is requested, the system preserves the optimal execution order:
    1. **GUI Prepare**: `gui.prepare` runs first to handle interactions and UI state.
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
- **Centralized Logic**: Uses `orientation::screen_to_ray_3d` on the CPU to ensure picking rays perfectly match the visual raymarching in the shader.
- **Continuous Update**: Navigation mode supports live crosshair updates during Left-click drags.
- **Safety**: Tools (Brush/Eraser) are automatically inhibited during Zoom/Pan/Rotate operations.

### Labelmap Overlays
The system supports raw **Hounsfield Unit (HU)** based windowing and translucent overlays.
- **Data Format**: `R32Float` for raw intensities; `R8Uint` for label textures.
- **2D vs 3D**: 2D slices use alpha-blended translucency; 3D view treats labels as solid, ray-terminating structures.

### Annotation Discussion Workflow
Implemented a collaborative clinical note-taking system:
- **Stable IDs**: Annotations use `uuid::Uuid` for stable cross-viewport and discussion mapping.
- **Threaded UI**: A reactive right-sidebar handles multi-line notes and threaded user comments.
- **Slice-Awareness**: To minimize clutter, 2D viewports filter annotations based on their proximity (depth) to the active crosshair slice using the centralized orientation API.
- **Global landmarking**: Annotations remain always visible in the 3D perspective view for spatial reference.

### Startup State & Empty Viewports
To prevent shader artifacts (like NaN aspect ratios) when no data is loaded:
- **Guard**: `shader.wgsl` implements an early-exit if `volume_dims` are zero, returning solid black.
- **Initialization**: The app starts with a dummy 1x1x1 volume but dimensions are explicitly set to `[0, 0, 0]` in the `VolumeData` component.
- **Overlays**: Crosshairs and primitives are inhibited until a valid volume is loaded to prevent visual clutter.

### Reliability & Testing
The codebase uses a "modular math" approach to ensure critical logic is unit-testable without GPU or ECS dependencies:
- **Modular Refactoring**: Complex systems (like `paint.rs` and `nifti_loader.rs`) have been refactored to expose pure functions (e.g., `calculate_orientation_from_rows`, `get_stroke_points`).
- **Test Suite**: Covers AABB intersections, quaternion ↔ matrix conversions, anatomical orientations, and voxel painting interpolation.
- **Validation**: All tests reside in `#[cfg(test)]` modules within the respective source files.

## 🛠 How to Run
- **Native**: `cargo run`
- **Web (WASM)**: `trunk serve` (requires `wasm32-unknown-unknown` target)

## 🔄 CI/CD
The project uses GitHub Actions for automated testing and deployment.
- **CI (`ci.yml`)**: Runs on every push and pull request. Performs linting (`rustfmt`, `clippy`), runs unit tests, and verifies compilation for both native and WASM targets.
- **Deploy (`deploy.yml`)**: Runs on every push to the `main` branch. Builds the WASM version using `wasm-bindgen-cli` and deploys it to GitHub Pages.

### Live Demo
The latest version is automatically hosted on GitHub Pages:
`https://<github-username>.github.io/rust_starter_app/`
*(Note: You must enable GitHub Pages in repo settings: Settings → Pages → Build and deployment → Source: GitHub Actions)*

## 💡 Future Session Tips
- When modifying interaction logic, check the submodules in `src/systems/`.
- **Coordinate Systems**: Both the shader and CPU picking use object-space raymarching via the volume's inverse rotation.
- **Annotations**: Using the "Locate" feature (🎯) centers the 2D viewports on the world position by modifying `ViewState.pan`.
- **Uniforms**: Ensure `Uniforms` struct alignment in `components.rs` (16-byte boundaries) matches `shader.wgsl` exactly.
- **Testing**: Run `cargo test` to verify math and coordinate transforms before deploying.
