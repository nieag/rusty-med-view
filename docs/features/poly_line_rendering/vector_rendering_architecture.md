# Feature: Decoupled Vector Rendering Architecture

## The Issue: UI-Layer Coupling
Current segmentation contours are rendered using the `egui` (UI) painter in `src/gui/overlays.rs`. While convenient for early development, this creates several architectural and performance bottlenecks:

1.  **Strict Layer Coupling**: The medical "Truth" (segmentation vectors) is directly dependent on a specific UI library (`egui`). Moving to a different UI framework would require a total rewrite of the rendering logic.
2.  **Lack of 3D Depth Integration**: `egui` is an "overlay" layer draw on top of the 3D scene. Vectors cannot share the GPU Depth Buffer, meaning they cannot be correctly occluded by the 3D volume (e.g., a liver contour appearing *behind* a rib).
3.  **Coordinate Complexity**: Sub-pixel alignment requires an intermediate "UV to Screen" transformation on the CPU every frame, which is prone to rounding errors and "lag" (as observed in earlier versions).
4.  **CPU Bottleneck**: Every line segment must be tessellated (turned into triangles) on the CPU by `egui` every frame.

## The Solution: Standalone Vector Pipeline

We propose transitioning to a custom **`VectorPipeline`** built directly on `wgpu`. This moves the "Authority of Drawing" from the UI layer to the Graphics layer.

### Architectural Shift

| Feature | Current Implementation (`egui`) | Proposed Implementation (`wgpu`) |
| :--- | :--- | :--- |
| **Logic Location** | `src/gui/overlays.rs` | `src/render/vector_renderer.rs` |
| **Tessellation** | CPU (every frame via `epaint`) | GPU/CPU (cached vertex buffers) |
| **Coordinate Space** | Screen-space Logical Pixels | World-space Millimeters (mm) |
| **Depth Casting** | Disabled (always on top) | **Enabled** (fully depth-aware) |

### Implementation Components

#### 1. Neutral Tessellator
Instead of `epaint`, we should use a library like **`lyon`**. `lyon` is a library-agnostic path tessellator that turns paths into raw triangle streams. This allows us to generate geometry that can move between any rendering backend.

#### 2. GPU Vertex Buffer
Instead of sending lines to `egui` every frame:
*   Convert `SpatialContour` (mm) $\to$ Triangles (mm) once on the CPU.
*   Upload the result to a static `wgpu::Buffer`.
*   Update only when the contour is edited (e.g., via a brush stroke).

#### 3. Vertex Shader Projection
The expensive projection math (Millimeters $\to$ Perspective UV $\to$ Screen) should move into a **Vertex Shader**. This provides zero-latency navigation (zooming/panning) because the CPU no longer has to recalculate points during a scroll event.

```wgsl
// Hypothetical Vertex Shader
@vertex
fn vs_main(model: VertexInput) -> VertexOutput {
    let world_pos = uniforms.model_matrix * vec4<f32>(model.position, 1.0);
    out.clip_position = uniforms.view_proj * world_pos;
    return out;
}
```

## Benefits
*   **Performance**: Supports millions of points with nearly zero CPU overhead.
*   **Fidelity**: Sub-pixel antialiasing and correct 3D occlusion for clinical use.
*   **Portability**: The renderer becomes independent of the UI kit, allowing for headless rendering (automated reports) and cross-platform UI migration.

## Hybrid Entities (Annotations)

Annotations (Markers, Discussions, Rulers) present a unique "Hybrid" challenge. They are geographically authoritative but interactively complex.

### Classification

1.  **The Anchor (Data Layer)**: The `world_pos` (mm) of an annotation is authoritative medical data. Like segmentations, this must move perfectly with the volume and spacing.
2.  **The Representation (Graphics Layer)**: The visual icon (e.g., a 🎯 or 📍) should ideally be rendered by the `VectorPipeline` to ensure correct 3D depth and occlusion.
3.  **The Interaction (UI Layer)**: The text input, comment threads, and sidebar logic are purely UI concerns.

### Proposed Hybrid Model

To resolve the "UI Coupling" for annotations while keeping them interactive:

*   **Anchor Sync**: Use the same `render::geometry` utility to calculate screen positions for both segmentations and annotation anchors.
*   **Depth-Aware Sprites**: Render the annotation icons as **Billboards** in the GPU pipeline. This ensures they are depth-tested against the volume but still face the camera.
*   **Active Overlays**: Only use the UI layer (`egui`) for the "high-level" interaction (multi-line text boxes, comment bubbles) that appears once an anchor is clicked.

## Implementation Strategy: Custom vs. Library

We have two distinct paths for the "Tessellation Engine":

### 1. The Homegrown Path (Simplified Custom Engine)
We can recreate the essentials of line-drawing without a heavy dependency.
*   **Technique**: Convert each line segment into a 2-triangle quad. Use a "Vertex Circle" (billboard) at every join to hide the gaps between segments.
*   **Pros**: Zero dependencies, 100% control, very small binary footprint.
*   **Cons**: No complex fills, potentially visible artifacts at extremely sharp angles, no easy "Dashed Lines."

### 2. The Library Path (Lyon / Epaint)
*   **Technique**: Use a specialized crate to handle complex joins (Miter/Round/Bevel) and path offsets.
*   **Pros**: Professional "Clinical" look. Handles self-intersections and complex SVG-style paths automatically.
*   **Cons**: Adds a large library dependency; slightly higher learning curve for the API.

## Implementation Path
1.  **Research**: Prototype a "Simple Quad + Point Join" shader for `wgpu` (The Homegrown Path).
2.  **Extraction**: Move `draw_contours_2d` and `draw_annotations` math into a shared `render::geometry` utility.
3.  **Pipeline**: Create `render/vector_renderer.rs` to handle both polylines (segmentations) and points (annotations).
4.  **Cleanup**: Remove visual drawing calls from `overlays.rs`, leaving only interaction handlers (click detection).
