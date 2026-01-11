# 03 - Contour Rendering

Render contour polylines in 2D viewports.

## Approach

Use **SDF-based line rendering** in the existing fragment shader (matches overlay primitive pattern).

## Data Flow

```
ContourData → ContourSegment[] → Storage Buffer → Shader
```

## Shader Addition

```wgsl
struct ContourSegment {
    start: vec2<f32>,     // Volume UV (0-1)
    end: vec2<f32>,
    color: vec4<f32>,
    thickness: f32,
    viewport_mask: u32,
    _pad: vec2<f32>,
}

@group(0) @binding(8) var<storage, read> contour_segments: array<ContourSegment>;

fn line_sdf(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}
```

## Subtasks

- [ ] Add `ContourSegment` struct to `components.rs`
- [ ] Create storage buffer in `render.rs`
- [ ] Add binding slot 8 to bind group layout
- [ ] Implement line SDF rendering in shader
- [ ] Add contour segment count to uniforms
- [ ] Wire up in `render_prep.rs` based on display mode

## Files

| File | Change |
|------|--------|
| `src/components.rs` | Add `ContourSegment` |
| `src/render.rs` | Create buffer, update bind group |
| `src/shaders/shader.wgsl` | Add line rendering loop |
| `src/systems/render_prep.rs` | Populate segment buffer |
