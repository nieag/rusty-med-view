use crate::app::components::*;
use crate::app::events::AppEvent;
use crate::app::roi_runtime;
use crate::gui::Gui;
use crate::io::volume;
use crate::render::pipeline;
use crate::render::protocols;
use hecs::World;
use std::sync::Arc;
use winit::event_loop::EventLoopProxy;
use winit::window::Window;

pub struct GpuState {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
    /// Errors wgpu reported outside any error scope, for the frame loop to surface.
    pub errors: crate::render::gpu_errors::GpuErrorSink,
}

pub struct Pipelines {
    pub render: wgpu::RenderPipeline,
    /// 3D view image without crosshair, marched into `view3d_cache` (see `render::view3d_cache`).
    pub march_3d: wgpu::RenderPipeline,
    /// Crosshair and primitives of the 3D view over the cached image.
    pub overlay_3d: wgpu::RenderPipeline,
    /// Draws the cached 3D image into the window.
    pub blit_3d: crate::render::view3d_cache::Blit3d,
    /// `None` until the first frame.
    pub view3d_cache: Option<crate::render::view3d_cache::View3dCache>,
    pub contour_overlay: crate::render::contours::ContourRenderer,
    pub mesh_overlay: crate::render::meshes::MeshRenderer,
}

pub struct VolumeResources {
    pub texture_bind_group_layout: wgpu::BindGroupLayout,
    pub uniform_buffer: wgpu::Buffer,
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub num_indices: u32,
    pub dummy_r8: (wgpu::Texture, wgpu::TextureView, wgpu::Sampler),
    pub default_lut: (wgpu::Texture, wgpu::TextureView),
}

pub struct SceneState {
    pub world: World,
    pub session: Session,
    /// Tracks camera movement so the 3D raymarch can drop quality while it moves.
    pub camera_motion: crate::render::pipeline::CameraMotion,
}

pub struct RenderingContext {
    pub window: Arc<Window>,
    pub gpu: GpuState,
    pub pipelines: Pipelines,
    pub volume_resources: VolumeResources,
    pub scene: SceneState,
    pub gui: Gui,
    pub event_proxy: EventLoopProxy<AppEvent>,
}

#[derive(Debug, Clone)]
pub struct RenderingInitError {
    pub category: &'static str,
    pub message: String,
}

impl RenderingContext {
    pub async fn new(
        instance: &wgpu::Instance,
        window: Arc<Window>,
        event_proxy: EventLoopProxy<AppEvent>,
    ) -> Result<Self, RenderingInitError> {
        log::info!("Initializing Rendering Context...");
        let size = window.inner_size();

        let surface =
            instance
                .create_surface(window.clone())
                .map_err(|err| RenderingInitError {
                    category: "wgpu.surface",
                    message: format!("Failed to create surface: {err}"),
                })?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .map_err(|err| RenderingInitError {
                category: "wgpu.adapter",
                message: format!("Failed to find an appropriate adapter: {err:?}"),
            })?;

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_limits: crate::render::gpu_errors::required_limits(&adapter),
                ..wgpu::DeviceDescriptor::default()
            })
            .await
            .map_err(|err| RenderingInitError {
                category: "wgpu.device",
                message: format!("Failed to create device: {err}"),
            })?;

        let gpu_errors = crate::render::gpu_errors::GpuErrorSink::install(&device);

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps.formats[0];

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 1,
        };
        surface.configure(&device, &config);

        let mut world = World::new();
        let (volume_texture, volume_view, volume_sampler) =
            volume::create_dummy_r32_texture(&device, &queue);
        let volume_data = VolumeData {
            dimensions: [0, 0, 0],
            geometry: None,
            intensities: vec![],
            intensity_range: [0.0, 0.0],
        };

        let dummy_r8 = volume::create_dummy_r8_texture(&device, &queue);
        let default_lut = volume::create_default_colormap(&device, &queue);

        let uniform_buffer = pipeline::create_uniform_buffer(&device);
        let texture_bind_group_layout = pipeline::create_bind_group_layout(&device);

        let diffuse_bind_group = pipeline::create_scene_bind_group(
            &device,
            &texture_bind_group_layout,
            &pipeline::SceneTextureViews {
                volume_view: &volume_view,
                volume_sampler: &volume_sampler,
                uniform_buffer: &uniform_buffer,
                overlay_views: [&dummy_r8.1; MAX_VOXEL_OVERLAY_SLOTS],
                overlay_lut: &default_lut.1,
            },
        );

        world.spawn((
            volume_data,
            GpuVolumeResources {
                texture: volume_texture,
                view: volume_view,
                sampler: volume_sampler,
                bind_group: diffuse_bind_group.clone(),
            },
            MainVolumeTag,
        ));

        let mut session = Session::new(config.width, config.height);
        protocols::apply_protocol(&mut world, &mut session, "Standard 2x2");

        let render_pipeline =
            pipeline::create_render_pipeline(&device, &texture_bind_group_layout, config.format);
        let march_3d = pipeline::create_view3d_pipeline(
            &device,
            &texture_bind_group_layout,
            config.format,
            pipeline::View3dPass::March,
        );
        let blit_3d = crate::render::view3d_cache::Blit3d::new(&device, config.format);
        let overlay_3d = pipeline::create_view3d_pipeline(
            &device,
            &texture_bind_group_layout,
            config.format,
            pipeline::View3dPass::Overlay,
        );
        let contour_overlay =
            crate::render::contours::create_contour_renderer(&device, config.format);
        let mesh_overlay = crate::render::meshes::create_mesh_renderer(&device, config.format);
        let (vertex_buffer, index_buffer, num_indices) = pipeline::create_geometry_buffers(&device);

        let gui = Gui::new(&device, config.format, &window);

        roi_runtime::recreate_scene_bind_groups(
            &device,
            &mut world,
            &roi_runtime::BindGroupResources {
                layout: &texture_bind_group_layout,
                uniform_buffer: &uniform_buffer,
                dummy_view: &dummy_r8.1,
                dummy_sampler: &dummy_r8.2,
                default_lut_view: &default_lut.1,
            },
            None,
        );

        Ok(RenderingContext {
            window,
            gpu: GpuState {
                device,
                queue,
                surface,
                config,
                errors: gpu_errors,
            },
            pipelines: Pipelines {
                render: render_pipeline,
                march_3d,
                overlay_3d,
                blit_3d,
                view3d_cache: None,
                contour_overlay,
                mesh_overlay,
            },
            volume_resources: VolumeResources {
                texture_bind_group_layout,
                uniform_buffer,
                vertex_buffer,
                index_buffer,
                num_indices,
                dummy_r8,
                default_lut,
            },
            scene: SceneState {
                world,
                session,
                camera_motion: Default::default(),
            },
            gui,
            event_proxy,
        })
    }
}
