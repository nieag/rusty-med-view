//! Cached image of the 3D view.
//!
//! The 3D view is a raymarch with one ray per pixel, the most expensive thing drawn. Its image
//! depends only on the camera, windowing, overlays, and volume, not on the cursor, so it is
//! marched into an offscreen texture when one of those changes and drawn into the window on every
//! other frame. The crosshair and overlay primitives, which do depend on the cursor, are drawn
//! over the copy by a cheap second pass. Interacting with a 2D view therefore never re-marches
//! the 3D view.
//!
//! Invariant: the cache is keyed on the uniforms and on the identity of the scene's bind group, so
//! it is only correct while volume and label textures are immutable once created (a change is a
//! new texture and a new bind group). Writing into a displayed texture in place would leave this
//! cache stale.

use std::hash::{Hash, Hasher};

/// What the 3D view needs this frame, computed with the frame's uniforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct View3dPlan {
    /// Dynamic-offset slot of the 3D viewport's uniforms.
    pub uniform_index: u32,
    /// The viewport clamped to the window, in pixels: `[x, y, width, height]`.
    pub rect: [u32; 4],
    /// Hash of everything that changes the marched image (not the cursor).
    pub image_key: u64,
    /// Raymarch steps wanted now: fewer while the camera moves.
    pub steps: u32,
}

/// The full-screen quad and bind group one 3D pass draws with.
pub struct QuadDraw<'a> {
    pub bind_group: &'a wgpu::BindGroup,
    pub vertex_buffer: &'a wgpu::Buffer,
    pub index_buffer: &'a wgpu::Buffer,
    pub index_count: u32,
    /// Dynamic offset of the 3D viewport's uniforms.
    pub uniform_offset: u32,
}

impl QuadDraw<'_> {
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        pass.set_bind_group(0, self.bind_group, &[self.uniform_offset]);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}

/// Pipeline that draws the cached image into the window.
pub struct Blit3d {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

impl Blit3d {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("3D View Blit Shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "../shaders/blit_3d.wgsl"
            ))),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("3D View Blit Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("3D View Blit Pipeline Layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("3D View Blit Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[crate::render::geometry::Vertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        Self { pipeline, layout }
    }
}

pub struct View3dCache {
    view: wgpu::TextureView,
    blit_bind_group: wgpu::BindGroup,
    size: [u32; 2],
    image_key: u64,
    rect: [u32; 4],
    steps: u32,
}

impl View3dCache {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        size: [u32; 2],
        blit: &Blit3d,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("3D View Cache"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let blit_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("3D View Blit Bind Group"),
            layout: &blit.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        Self {
            view,
            blit_bind_group,
            size,
            image_key: 0,
            rect: [0; 4],
            steps: 0,
        }
    }

    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    /// Whether the cached image is what `plan` would draw. It is not when the image changed or
    /// when it was marched at a lower quality than is wanted now (the camera stopped moving).
    pub fn is_current(&self, plan: &View3dPlan) -> bool {
        self.image_key == plan.image_key && self.rect == plan.rect && self.steps == plan.steps
    }

    /// Marches the 3D view into the cache with the march pipeline.
    pub fn march(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        quad: &QuadDraw<'_>,
        plan: &View3dPlan,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("3D View March Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        let [x, y, w, h] = plan.rect.map(|value| value as f32);
        pass.set_pipeline(pipeline);
        pass.set_viewport(x, y, w, h, 0.0, 1.0);
        quad.draw(&mut pass);
        drop(pass);
        self.image_key = plan.image_key;
        self.rect = plan.rect;
        self.steps = plan.steps;
    }

    /// Draws the cached 3D view into the window, then the crosshair and primitives over it.
    pub fn composite(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        surface_view: &wgpu::TextureView,
        pipelines: (&Blit3d, &wgpu::RenderPipeline),
        quad: &QuadDraw<'_>,
        plan: &View3dPlan,
    ) {
        let (blit, overlay_pipeline) = pipelines;
        let [x, y, w, h] = plan.rect;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("3D View Composite Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: surface_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
        pass.set_vertex_buffer(0, quad.vertex_buffer.slice(..));
        pass.set_index_buffer(quad.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        pass.set_pipeline(&blit.pipeline);
        pass.set_bind_group(0, &self.blit_bind_group, &[]);
        pass.draw_indexed(0..quad.index_count, 0, 0..1);
        pass.set_pipeline(overlay_pipeline);
        pass.set_bind_group(0, quad.bind_group, &[quad.uniform_offset]);
        pass.draw_indexed(0..quad.index_count, 0, 0..1);
    }
}

/// The viewport rectangle clamped to the window, or `None` when nothing of it is visible.
pub fn clamp_rect(rect: [f32; 4], window: [u32; 2]) -> Option<[u32; 4]> {
    let x0 = rect[0].max(0.0).floor();
    let y0 = rect[1].max(0.0).floor();
    let x1 = (rect[0] + rect[2]).min(window[0] as f32).floor();
    let y1 = (rect[1] + rect[3]).min(window[1] as f32).floor();
    if !(x1 > x0 && y1 > y0) {
        return None;
    }
    Some([x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32])
}

/// Hash of the bytes of `value` with the fields that do not affect the marched image cleared by
/// the caller, combined with the identity of the scene's textures.
pub fn hash_image(uniform_bytes: &[u8], scene_identity: impl Hash) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    uniform_bytes.hash(&mut hasher);
    scene_identity.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clamp_rect_keeps_the_visible_part_and_rejects_empty() {
        assert_eq!(
            clamp_rect([10.0, 20.0, 100.0, 50.0], [800, 600]),
            Some([10, 20, 100, 50])
        );
        assert_eq!(
            clamp_rect([-5.0, 550.0, 100.0, 100.0], [800, 600]),
            Some([0, 550, 95, 50])
        );
        assert_eq!(clamp_rect([900.0, 0.0, 10.0, 10.0], [800, 600]), None);
        assert_eq!(clamp_rect([0.0, 0.0, 0.0, 10.0], [800, 600]), None);
    }

    #[test]
    fn test_hash_image_depends_on_bytes_and_scene_identity() {
        let a = hash_image(&[1, 2, 3], 7_u64);
        assert_eq!(a, hash_image(&[1, 2, 3], 7_u64));
        assert_ne!(a, hash_image(&[1, 2, 4], 7_u64));
        assert_ne!(a, hash_image(&[1, 2, 3], 8_u64));
    }
}
