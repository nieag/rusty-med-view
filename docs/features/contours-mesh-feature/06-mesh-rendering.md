# Phase 6: Mesh Rendering Pipeline

## Goal

Render triangle meshes with proper lighting in the 3D viewport.

## Files to Create

### [NEW] `src/shaders/mesh.wgsl`
### [NEW] `src/render/mesh_pipeline.rs`
### [MODIFY] `src/render/mod.rs`

## Shader

### `src/shaders/mesh.wgsl`

```wgsl
// Mesh uniforms
struct MeshUniforms {
    model: mat4x4<f32>,
    view: mat4x4<f32>,
    proj: mat4x4<f32>,
    color: vec4<f32>,
    light_dir: vec3<f32>,
    _padding: f32,
    camera_pos: vec3<f32>,
    _padding2: f32,
};

@group(0) @binding(0)
var<uniform> uniforms: MeshUniforms;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    
    let world_pos = uniforms.model * vec4<f32>(in.position, 1.0);
    out.world_position = world_pos.xyz;
    
    // Transform normal (use inverse transpose for non-uniform scale)
    out.world_normal = normalize((uniforms.model * vec4<f32>(in.normal, 0.0)).xyz);
    
    out.clip_position = uniforms.proj * uniforms.view * world_pos;
    
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let N = normalize(in.world_normal);
    let L = normalize(uniforms.light_dir);
    let V = normalize(uniforms.camera_pos - in.world_position);
    let H = normalize(L + V);
    
    // Blinn-Phong lighting
    let ambient = 0.2;
    let diffuse = max(dot(N, L), 0.0) * 0.6;
    let specular = pow(max(dot(N, H), 0.0), 32.0) * 0.3;
    
    // Two-sided lighting
    let back_diffuse = max(dot(-N, L), 0.0) * 0.3;
    
    let lighting = ambient + diffuse + specular + back_diffuse;
    
    return vec4<f32>(uniforms.color.rgb * lighting, uniforms.color.a);
}
```

## Pipeline

### `src/render/mesh_pipeline.rs`

```rust
use wgpu::util::DeviceExt;
use crate::app::segment::MeshData;

/// GPU-side mesh uniforms
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MeshUniformsGpu {
    pub model: [[f32; 4]; 4],
    pub view: [[f32; 4]; 4],
    pub proj: [[f32; 4]; 4],
    pub color: [f32; 4],
    pub light_dir: [f32; 3],
    pub _padding: f32,
    pub camera_pos: [f32; 3],
    pub _padding2: f32,
}

impl Default for MeshUniformsGpu {
    fn default() -> Self {
        Self {
            model: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], 
                    [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]],
            view: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], 
                   [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]],
            proj: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], 
                   [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]],
            color: [0.8, 0.2, 0.2, 1.0],
            light_dir: [0.5, 0.5, 1.0],
            _padding: 0.0,
            camera_pos: [0.0, 0.0, 5.0],
            _padding2: 0.0,
        }
    }
}

pub struct MeshPipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
}

impl MeshPipeline {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Mesh Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/mesh.wgsl").into()),
        });
        
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Mesh Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Mesh Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Mesh Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[
                    // Position
                    wgpu::VertexBufferLayout {
                        array_stride: 12,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x3,
                        }],
                    },
                    // Normal
                    wgpu::VertexBufferLayout {
                        array_stride: 12,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x3,
                        }],
                    },
                ],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,  // Two-sided
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        
        Self { pipeline, bind_group_layout }
    }
}

pub struct MeshResources {
    pub vertex_buffer: wgpu::Buffer,
    pub normal_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub uniform_buffer: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
    pub index_count: u32,
}

impl MeshResources {
    pub fn from_mesh(
        device: &wgpu::Device,
        mesh: &MeshData,
        pipeline: &MeshPipeline,
    ) -> Self {
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Mesh Vertex Buffer"),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        
        let normal_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Mesh Normal Buffer"),
            contents: bytemuck::cast_slice(&mesh.normals),
            usage: wgpu::BufferUsages::VERTEX,
        });
        
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Mesh Index Buffer"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Mesh Uniform Buffer"),
            contents: bytemuck::cast_slice(&[MeshUniformsGpu::default()]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Mesh Bind Group"),
            layout: &pipeline.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });
        
        Self {
            vertex_buffer,
            normal_buffer,
            index_buffer,
            uniform_buffer,
            bind_group,
            index_count: mesh.indices.len() as u32,
        }
    }
    
    pub fn update_uniforms(&self, queue: &wgpu::Queue, uniforms: &MeshUniformsGpu) {
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[*uniforms]));
    }
}
```

## Render Integration

Add to `src/render/mod.rs`:

```rust
pub fn render_mesh(
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    depth_view: &wgpu::TextureView,
    pipeline: &MeshPipeline,
    resources: &MeshResources,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Mesh Render Pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,  // Preserve volume rendering
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth_view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Load,  // Preserve volume depth
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        ..Default::default()
    });
    
    pass.set_pipeline(&pipeline.pipeline);
    pass.set_bind_group(0, &resources.bind_group, &[]);
    pass.set_vertex_buffer(0, resources.vertex_buffer.slice(..));
    pass.set_vertex_buffer(1, resources.normal_buffer.slice(..));
    pass.set_index_buffer(resources.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
    pass.draw_indexed(0..resources.index_count, 0, 0..1);
}
```

## Depth Buffer

Ensure depth buffer exists for 3D view:

```rust
pub fn create_depth_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Depth Texture"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}
```

## Verification

### Visual Test

1. Run: `cargo run`
2. Load volume
3. Draw contours on several axial slices
4. Switch to 3D view
5. **Verify:** Lit, shaded mesh appears

### Check List

- [ ] Mesh renders with smooth shading
- [ ] Lighting responds to view rotation
- [ ] Mesh composites correctly with volume (depth test)
- [ ] Two-sided lighting works (back faces visible)

## Acceptance Criteria

- [ ] Mesh pipeline compiles shader correctly
- [ ] Uniform matrices transform mesh properly
- [ ] Blinn-Phong lighting looks correct
- [ ] Depth compositing with volume works
- [ ] Visual verification passes
