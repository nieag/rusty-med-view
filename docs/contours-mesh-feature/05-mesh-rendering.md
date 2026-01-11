# 05 - Mesh Rendering

Render triangle mesh in 3D viewport with proper lighting.

## Architecture

Separate render pass from volume raymarching:

```
Frame
├── Pass 1: Volume raymarching (existing shader)
└── Pass 2: Mesh rendering (new pipeline) ◄── depth composited
```

## Mesh Pipeline

```rust
pub struct MeshPipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
}

pub struct MeshResources {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
    pub bind_group: wgpu::BindGroup,
}
```

## Shader

```wgsl
// src/shaders/mesh.wgsl

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Lambert diffuse + ambient
    let N = normalize(in.normal);
    let L = normalize(vec3(0.5, 1.0, -0.5));
    let diffuse = max(dot(N, L), 0.0);
    return vec4(color.rgb * (0.2 + 0.8 * diffuse), color.a);
}
```

## Subtasks

- [ ] Create `src/shaders/mesh.wgsl`
- [ ] Create `src/mesh_render.rs` with pipeline setup
- [ ] Add vertex buffer creation from `MeshData`
- [ ] Integrate into render loop (after volume pass)
- [ ] Apply same camera/rotation as 3D viewport
- [ ] Add depth buffer sharing

## Files

| File | Change |
|------|--------|
| `src/shaders/mesh.wgsl` | NEW |
| `src/mesh_render.rs` | NEW |
| `src/lib.rs` | Integrate mesh pass |
