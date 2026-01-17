# Decoupled, World-Space Vector Rendering Pipeline

## Goal

Introduce a **retained, world-space, depth-aware vector rendering pass** for medical geometry (segmentations, annotations) that:

* Is independent of `egui`
* Uses cached GPU meshes
* Renders in millimeter (mm) world coordinates
* Shares the depth buffer with the volume renderer
* Keeps `egui` strictly as an interaction/UI layer

This system is **not** a general-purpose 2D UI framework. It is a **domain-specific graphics pipeline** for medical vectors.

---

## Non-Goals (Explicitly Out of Scope)

To prevent scope creep, this implementation will **not**:

* Render text
* Perform layout or widget composition
* Implement clipping stacks
* Replace `egui`
* Support full SVG-style paths (initially)

---

## High-Level Architecture

```
┌────────────────────┐
│   UI Layer (egui)  │
│────────────────────│
│ - Mouse input      │
│ - Editing tools    │
│ - Panels / text    │
└─────────┬──────────┘
          │ edits
┌─────────▼──────────┐
│  Data Layer        │
│────────────────────│
│ SpatialContour     │  (mm space)
│ AnnotationAnchor   │
└─────────┬──────────┘
          │ dirty flag
┌─────────▼──────────┐
│ Vector Renderer    │
│ (wgpu)             │
│────────────────────│
│ - Tessellation     │
│ - GPU mesh cache   │
│ - Depth-aware draw │
└────────────────────┘
```

---

## Phase 0 — Foundations (No Rendering Yet)

### 0.1 Shared Geometry Utilities

Create a `render::geometry` module containing **pure math utilities**:

* World → Clip → Screen projection
* Camera and view-projection matrices
* Line/point distance tests for picking

Requirements:

* UI-agnostic
* Renderer-agnostic
* Used by both `egui` (interaction) and `wgpu` (rendering)

**Outcome:** Single source of truth for all spatial math.

---

## Phase 1 — Retained Vector Scene

### 1.1 `VectorScene`

Introduce a retained scene graph for medical vectors:

```rust
pub struct VectorScene {
    polylines: Vec<Polyline>,
    points: Vec<PointMarker>,
}
```

```rust
pub struct Polyline {
    world_points: Vec<Vec3>, // millimeters
    width_mm: f32,
    color: Color,
    dirty: bool,
    gpu_mesh: Option<GpuMesh>,
}
```

Rules:

* This replaces all `egui` drawing for contours and markers
* `egui` may **modify** this data, but never draw it

**Invariant:** Medical geometry is authoritative here, not in the UI.

---

## Phase 2 — Minimal Tessellation Engine (CPU)

### 2.1 Initial Tessellation Strategy

Implement a **homegrown CPU tessellator**:

* Each polyline segment → quad (2 triangles)
* Width specified in **world millimeters**
* Perpendicular computed in world space

At joins:

* Emit a small round cap (triangle fan)
* No miter/bevel logic initially

Rationale:

* Fast to implement
* Easy to debug
* Zero dependencies
* Sufficient for first clinical-quality pass

> Complex tessellation libraries (e.g., `lyon`) are explicitly deferred.

---

### 2.2 Tessellation API

```rust
fn tessellate_polyline(polyline: &Polyline) -> MeshData;
```

Outputs:

* `Vec<Vertex>` (world-space positions)
* `Vec<u32>` indices

---

## Phase 3 — GPU Mesh Cache

### 3.1 Mesh Lifecycle

Each polyline owns its GPU representation:

```rust
if polyline.dirty {
    polyline.gpu_mesh = Some(upload_to_gpu(mesh_data));
    polyline.dirty = false;
}
```

Rules:

* Meshes rebuild **only when geometry changes**
* Camera movement never triggers tessellation
* GPU buffers are reused frame-to-frame

This is the primary performance win over egui’s immediate-mode approach.

---

## Phase 4 — Vector Render Pass (wgpu)

### 4.1 Renderer Module

Create `render/vector_renderer.rs` responsible for:

* Pipeline creation
* Bind group management
* Issuing draw calls for vector meshes

Pipeline configuration:

* Primitive: triangle list
* Depth test: enabled
* Depth write: disabled (configurable)
* Alpha blending: enabled

---

### 4.2 Shader Responsibilities

**Vertex Shader**

* Accept world-space positions
* Apply view-projection matrix
* Output clip-space coordinates

**Fragment Shader**

* Solid color output (initially)
* Optional depth bias hook for future refinement

---

## Phase 5 — Annotation Rendering (Hybrid Model)

### 5.1 Annotation Decomposition

| Component | Responsibility                          |
| --------- | --------------------------------------- |
| Anchor    | World position (mm, authoritative data) |
| Icon      | GPU-rendered billboard                  |
| UI        | egui panels, text, comments             |

---

### 5.2 Billboard Rendering

* Annotation icons rendered as camera-facing quads
* Fully depth-tested against the volume
* Size defined in world or screen units (configurable)

Text and complex UI remain entirely in egui.

---

## Phase 6 — Interaction & Picking

### 6.1 Initial Picking Strategy (CPU)

1. Project polyline points to screen using `render::geometry`
2. Compute distance to mouse in pixel space
3. Compare against stroke width threshold

Advantages:

* Simple
* Deterministic
* No GPU readback

Future upgrades:

* GPU ID buffer
* Spatial acceleration structures

---

## Phase 7 — Decommission egui Drawing

### 7.1 Cleanup

* Remove all visual drawing from `overlays.rs`
* Retain only:

  * input handling
  * tool logic
  * selection state

After this phase, egui acts purely as a **controller**, not a renderer.

---

## Phase 8 — Deferred / Future Work

Not part of the initial implementation:

* Lyon integration for complex joins and fills
* Dashed or patterned strokes
* GPU-expanded line segments
* Depth bias tuning and layering
* Headless rendering backend
* Vector export (SVG / PDF)

---

## Deliverables

* [ ] World-space vector renderer
* [ ] Depth-aware segmentation contours
* [ ] Cached GPU mesh pipeline
* [ ] Hybrid annotation rendering
* [ ] Full decoupling from egui drawing

---

## Summary

This plan establishes a **clean separation of concerns**:

* **egui**: interaction and UI
* **Data layer**: authoritative medical geometry
* **wgpu**: high-performance, depth-aware rendering

It minimizes risk, avoids overengineering, and creates a scalable foundation for future clinical and visualization features.
