//! Contour rendering pipeline.
//!
//! A dedicated GPU pipeline for rendering contour line segments efficiently.
//! Uses instanced rendering - each contour segment is a line instance drawn as a quad.

use bytemuck::{Pod, Zeroable};

/// GPU-compatible contour line segment.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct ContourLineGpu {
    /// Start point in volume UV coordinates (0-1)
    pub p0: [f32; 3],
    pub _pad0: f32,
    /// End point in volume UV coordinates (0-1)
    pub p1: [f32; 3],
    pub _pad1: f32,
    /// Color RGBA
    pub color: [f32; 4],
    /// Plane info: [view_mode (1=axial,2=coronal,3=sagittal,4=oblique), slice_index, _pad, _pad]
    pub plane_info: [f32; 4],
}

impl ContourLineGpu {
    pub fn new(p0: [f32; 3], p1: [f32; 3], color: [f32; 4], view_mode: u32, slice: i32) -> Self {
        Self {
            p0,
            _pad0: 0.0,
            p1,
            _pad1: 0.0,
            color,
            plane_info: [view_mode as f32, slice as f32, 0.0, 0.0],
        }
    }
}

/// GPU-compatible uniform buffer for contour rendering.
/// Layout MUST match contour.wgsl exactly!
/// WGSL vec2 has 8-byte alignment, vec3 has 16-byte alignment.
/// Total size: 128 bytes
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct ContourUniforms {
    /// Zoom level (offset 0)
    pub zoom: f32,
    pub _pad0: f32, // offset 4: pad so pan starts at offset 8 (vec2 alignment)
    /// Pan offset (offset 8)
    pub pan: [f32; 2],
    /// Zoom pivot (offset 16)
    pub pivot: [f32; 2],
    /// View mode (0=3D, 1=Axial, 2=Coronal, 3=Sagittal) (offset 24)
    pub view_mode: u32,
    pub _pad1: u32, // align to 32 bytes
    /// Resolution (offset 32)
    pub resolution: [f32; 2],
    pub _pad2: [f32; 2], // align to 48 bytes
    /// Volume dimensions (offset 48, vec3<u32> aligned to 16 bytes)
    pub volume_dims: [u32; 3],
    pub _pad3: u32,
    /// Volume spacing (offset 64, vec3<f32> aligned to 16 bytes)
    pub volume_spacing: [f32; 3],
    /// Current slice index (offset 76)
    pub current_slice: i32,
    /// Cursor position in volume UV.
    pub cursor_pos: [f32; 3],
    pub _pad4: f32,
    /// User rotation quaternion used by oblique 2D view.
    pub rotation: [f32; 4],
    /// Padding to 128 bytes.
    pub _padding: [f32; 4],
}

const MAX_CONTOUR_LINES: usize = 131072; // Increased to 8MB to handle complex multi-viewport segmentations

/// Resources for contour rendering.
/// Supports up to 4 viewport slots (matching hanging protocol viewport count),
/// each with its own uniform buffer to avoid collisions.
pub struct ContourPipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
    /// One bind group per view mode
    pub bind_groups: [wgpu::BindGroup; 4],
    /// One uniform buffer per view mode
    pub uniform_buffers: [wgpu::Buffer; 4],
    pub line_buffer: wgpu::Buffer,
    pub line_count: u32,
}

impl ContourPipeline {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        // Create shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Contour Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "../shaders/contour.wgsl"
            ))),
        });

        // Create 4 uniform buffers, one for each viewport slot.
        let uniform_buffers: [wgpu::Buffer; 4] = std::array::from_fn(|i| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("Contour Uniforms {i}")),
                size: std::mem::size_of::<ContourUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });

        // Create line storage buffer
        let line_buffer_size = (std::mem::size_of::<ContourLineGpu>() * MAX_CONTOUR_LINES) as u64;
        let line_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Contour Lines Buffer"),
            size: line_buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Create bind group layout
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Contour Bind Group Layout"),
            entries: &[
                // Uniforms
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Line storage
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        // Create 4 bind groups
        let bind_groups: [wgpu::BindGroup; 4] = std::array::from_fn(|i| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("Contour Bind Group {i}")),
                layout: &bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform_buffers[i].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: line_buffer.as_entire_binding(),
                    },
                ],
            })
        });

        // Create pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Contour Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // Create render pipeline
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Contour Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[], // No vertex buffers - using instancing
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None, // Draw both sides of lines
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Self {
            pipeline,
            bind_group_layout,
            bind_groups,
            uniform_buffers,
            line_buffer,
            line_count: 0,
        }
    }

    /// Update the line buffer with new contour segments.
    pub fn update_lines(&mut self, queue: &wgpu::Queue, lines: &[ContourLineGpu]) {
        self.line_count = lines.len().min(MAX_CONTOUR_LINES) as u32;
        if self.line_count > 0 {
            queue.write_buffer(
                &self.line_buffer,
                0,
                bytemuck::cast_slice(&lines[..self.line_count as usize]),
            );
        }
    }

    /// Update uniforms for a specific viewport.
    pub fn update_uniforms(
        &self,
        queue: &wgpu::Queue,
        uniforms: &ContourUniforms,
        slot_index: usize,
    ) {
        let index = slot_index;
        if index < self.uniform_buffers.len() {
            queue.write_buffer(
                &self.uniform_buffers[index],
                0,
                bytemuck::cast_slice(&[*uniforms]),
            );
        }
    }

    /// Render contour lines for the given viewport slot.
    pub fn render<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>, slot_index: usize) {
        if self.line_count == 0 {
            return;
        }

        let index = slot_index;
        if index >= self.bind_groups.len() {
            return;
        }

        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.bind_groups[index], &[]);
        // 4 vertices per line (triangle strip quad), line_count instances
        render_pass.draw(0..4, 0..self.line_count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contour_line_gpu_size() {
        // Ensure proper alignment for GPU
        assert_eq!(std::mem::size_of::<ContourLineGpu>(), 64);
    }

    #[test]
    fn test_contour_uniforms_size() {
        // Check uniform size
        let size = std::mem::size_of::<ContourUniforms>();
        assert!(size <= 256, "Uniforms too large: {}", size);
    }

    #[test]
    fn test_contour_line_creation() {
        let line = ContourLineGpu::new(
            [0.1, 0.2, 0.3],
            [0.4, 0.5, 0.6],
            [1.0, 0.0, 0.0, 1.0],
            1, // Axial
            50,
        );
        assert_eq!(line.p0, [0.1, 0.2, 0.3]);
        assert_eq!(line.p1, [0.4, 0.5, 0.6]);
        assert_eq!(line.plane_info[0], 1.0);
        assert_eq!(line.plane_info[1], 50.0);
    }
}
