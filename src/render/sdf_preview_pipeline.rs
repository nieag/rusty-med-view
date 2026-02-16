//! SDF preview rendering pipeline.
//!
//! Renders a 2D heatmap of the active segment's SDF in slice viewports.

use crate::app::segment::{SdfVolume, Segment};
use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct SdfPreviewUniforms {
    pub zoom: f32,
    pub _pad0: f32,
    pub pan: [f32; 2],
    pub pivot: [f32; 2],
    pub view_mode: u32,
    pub show_zero_isoline: u32,
    pub resolution: [f32; 2],
    pub _pad_align0: [u32; 2],
    pub volume_dims: [u32; 3],
    pub current_slice: i32,
    pub volume_spacing: [f32; 3],
    pub opacity: f32,
    pub sdf_dims: [u32; 3],
    pub _pad1: u32,
    pub value_window_mm: f32,
    pub _pad_align1: [f32; 3],
    pub _pad2: [f32; 4],
}

impl SdfPreviewUniforms {
    pub fn empty() -> Self {
        Self {
            zoom: 1.0,
            _pad0: 0.0,
            pan: [0.0, 0.0],
            pivot: [0.5, 0.5],
            view_mode: 1,
            show_zero_isoline: 0,
            resolution: [1.0, 1.0],
            _pad_align0: [0; 2],
            volume_dims: [1, 1, 1],
            current_slice: 0,
            volume_spacing: [1.0, 1.0, 1.0],
            opacity: 0.35,
            sdf_dims: [1, 1, 1],
            _pad1: 0,
            value_window_mm: 8.0,
            _pad_align1: [0.0; 3],
            _pad2: [0.0; 4],
        }
    }
}

pub struct SdfPreviewPipeline {
    pipeline: wgpu::RenderPipeline,
    uniform_bind_groups: [wgpu::BindGroup; 4],
    uniform_buffers: [wgpu::Buffer; 4],
    texture_bind_group_layout: wgpu::BindGroupLayout,
    texture_bind_group: Option<wgpu::BindGroup>,
    sdf_texture: Option<wgpu::Texture>,
    sdf_texture_view: Option<wgpu::TextureView>,
    sdf_sampler: wgpu::Sampler,
    cached_segment_id: Option<uuid::Uuid>,
    cached_sdf_revision: u64,
    sdf_dims: [u32; 3],
}

impl SdfPreviewPipeline {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("SDF Preview Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/sdf_preview.wgsl").into()),
        });

        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SDF Preview Uniform Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("SDF Preview Texture Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D3,
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                        count: None,
                    },
                ],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("SDF Preview Pipeline Layout"),
            bind_group_layouts: &[&uniform_layout, &texture_bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("SDF Preview Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
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
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let mut uniform_buffers = Vec::with_capacity(4);
        let mut uniform_bind_groups = Vec::with_capacity(4);
        for i in 0..4 {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("SDF Preview Uniform Buffer {i}")),
                size: std::mem::size_of::<SdfPreviewUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("SDF Preview Uniform BG {i}")),
                layout: &uniform_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            });
            uniform_buffers.push(buffer);
            uniform_bind_groups.push(bind_group);
        }

        let sdf_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("SDF Preview Sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        Self {
            pipeline,
            uniform_bind_groups: uniform_bind_groups
                .try_into()
                .unwrap_or_else(|_| panic!("Expected 4 SDF preview bind groups")),
            uniform_buffers: uniform_buffers
                .try_into()
                .unwrap_or_else(|_| panic!("Expected 4 SDF preview uniform buffers")),
            texture_bind_group_layout,
            texture_bind_group: None,
            sdf_texture: None,
            sdf_texture_view: None,
            sdf_sampler,
            cached_segment_id: None,
            cached_sdf_revision: 0,
            sdf_dims: [0, 0, 0],
        }
    }

    pub fn clear_preview(&mut self) {
        self.texture_bind_group = None;
        self.sdf_texture = None;
        self.sdf_texture_view = None;
        self.cached_segment_id = None;
        self.cached_sdf_revision = 0;
        self.sdf_dims = [0, 0, 0];
    }

    pub fn sdf_dims(&self) -> [u32; 3] {
        self.sdf_dims
    }

    pub fn update_from_active_segment(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        active_segment: Option<&Segment>,
    ) {
        let Some(segment) = active_segment else {
            self.clear_preview();
            return;
        };

        let Some(sdf) = &segment.sdf else {
            self.clear_preview();
            return;
        };

        let needs_upload = self.cached_segment_id != Some(segment.id)
            || self.cached_sdf_revision != segment.sdf_revision
            || self.sdf_dims != sdf.dimensions;

        if !needs_upload {
            return;
        }

        self.upload_sdf(device, queue, segment.id, segment.sdf_revision, sdf);
    }

    fn upload_sdf(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        segment_id: uuid::Uuid,
        sdf_revision: u64,
        sdf: &SdfVolume,
    ) {
        let dims = sdf.dimensions;
        if dims[0] == 0 || dims[1] == 0 || dims[2] == 0 || sdf.data.is_empty() {
            self.clear_preview();
            return;
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("SDF Preview Texture"),
            size: wgpu::Extent3d {
                width: dims[0],
                height: dims[1],
                depth_or_array_layers: dims[2],
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&sdf.data),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(dims[0] * std::mem::size_of::<f32>() as u32),
                rows_per_image: Some(dims[1]),
            },
            wgpu::Extent3d {
                width: dims[0],
                height: dims[1],
                depth_or_array_layers: dims[2],
            },
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("SDF Preview Texture BG"),
            layout: &self.texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sdf_sampler),
                },
            ],
        });

        self.sdf_texture = Some(texture);
        self.sdf_texture_view = Some(view);
        self.texture_bind_group = Some(texture_bind_group);
        self.cached_segment_id = Some(segment_id);
        self.cached_sdf_revision = sdf_revision;
        self.sdf_dims = dims;
    }

    pub fn update_uniforms(
        &self,
        queue: &wgpu::Queue,
        uniforms: &SdfPreviewUniforms,
        view_mode: u32,
    ) {
        let idx = view_mode as usize;
        if idx < self.uniform_buffers.len() {
            queue.write_buffer(&self.uniform_buffers[idx], 0, bytemuck::bytes_of(uniforms));
        }
    }

    pub fn render<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, view_mode: u32) {
        let idx = view_mode as usize;
        let Some(texture_bg) = self.texture_bind_group.as_ref() else {
            return;
        };
        if idx >= self.uniform_bind_groups.len() {
            return;
        }

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.uniform_bind_groups[idx], &[]);
        pass.set_bind_group(1, texture_bg, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sdf_preview_uniform_size() {
        let size = std::mem::size_of::<SdfPreviewUniforms>();
        assert_eq!(size, 128);
    }
}
