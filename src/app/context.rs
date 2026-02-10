use crate::app::components::*;
use crate::app::events::AppEvent;
use crate::gui::Gui;
use crate::io::handlers;
use crate::io::volume;
use crate::overlay::OverlayManager;
use crate::render::contour_pipeline::ContourPipeline;
use crate::render::pipeline;
use crate::render::protocols;
use crate::systems::SegmentManager;
use hecs::World;
use std::sync::Arc;
use winit::event_loop::EventLoopProxy;
use winit::window::Window;

pub struct RenderingContext {
    pub window: Arc<Window>,
    pub surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,

    // Pipelines and Buffers
    pub render_pipeline: wgpu::RenderPipeline,
    pub texture_bind_group_layout: wgpu::BindGroupLayout,
    pub uniform_buffer: wgpu::Buffer,
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub num_indices: u32,

    // ECS and GUI
    pub world: World,
    pub entities: AppEntities,
    pub gui: Gui,
    pub settings_entity: hecs::Entity,

    // Shared Resources
    pub dummy_r8: (wgpu::Texture, wgpu::TextureView, wgpu::Sampler),
    pub default_lut: (wgpu::Texture, wgpu::TextureView),
    pub overlay_buffer: wgpu::Buffer,

    // Contour rendering pipeline
    pub contour_pipeline: ContourPipeline,

    // Proxy for waking up the event loop from async tasks
    pub event_proxy: EventLoopProxy<AppEvent>,
}

impl RenderingContext {
    pub async fn new(
        instance: &wgpu::Instance,
        window: Arc<Window>,
        event_proxy: EventLoopProxy<AppEvent>,
    ) -> Self {
        log::info!("Initializing Rendering Context...");
        let size = window.inner_size();

        let surface = instance
            .create_surface(window.clone())
            .expect("Failed to create surface");
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("Failed to find an appropriate adapter.");

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .expect("Failed to create device");

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
            spacing: [1.0, 1.0, 1.0],
            intensities: vec![],
            intensity_range: [0.0, 0.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };

        let dummy_r8 = volume::create_dummy_r8_texture(&device, &queue);
        let default_lut = volume::create_default_colormap(&device, &queue);

        let cursor = world.spawn((
            Transform {
                position: [0.5, 0.5, 0.5],
            },
            CursorTag,
        ));
        let settings_entity = world.spawn((WindowSettings {
            width: config.width,
            height: config.height,
            viewport_rect: [0.0, 0.0, config.width as f32, config.height as f32],
        },));

        let gui_state = world.spawn((GuiState {
            status_message: None,
        },));

        let input = world.spawn((InputState::default(),));
        let protocol = world.spawn((ProtocolState::default(),));

        let uniform_buffer = pipeline::create_uniform_buffer(&device);
        let texture_bind_group_layout = pipeline::create_bind_group_layout(&device);
        let overlay_buffer = pipeline::create_overlay_buffer(&device);

        let diffuse_bind_group = pipeline::create_scene_bind_group(
            &device,
            &texture_bind_group_layout,
            &volume_view,
            &volume_sampler,
            &uniform_buffer,
            &dummy_r8.1,
            &default_lut.1,
            &dummy_r8.1,
            &default_lut.1,
            &overlay_buffer,
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

        let editor = world.spawn((EditorState {
            active_layer: None,
            ..Default::default()
        },));

        let windowing = world.spawn((VolumeWindowing::default(),));
        let annotations = world.spawn((AnnotationState::default(),));
        let overlay = world.spawn((OverlayManager::default(),));
        let mut segment_manager = SegmentManager::new();
        segment_manager.add_segment("Segment 1", [1.0, 0.0, 0.0, 1.0]); // Red
        let segments = world.spawn((segment_manager,));

        let entities = AppEntities {
            input,
            editor,
            gui_state,
            volume_windowing: windowing,
            annotations,
            overlay,
            protocol,
            cursor,
            window_settings: settings_entity,
            segments,
        };

        protocols::apply_protocol(&mut world, &entities, "Standard 2x2");

        let render_pipeline =
            pipeline::create_render_pipeline(&device, &texture_bind_group_layout, config.format);
        let (vertex_buffer, index_buffer, num_indices) = pipeline::create_geometry_buffers(&device);

        let gui = Gui::new(&device, config.format, &window);

        handlers::recreate_bind_groups(
            &device,
            &mut world,
            &texture_bind_group_layout,
            &uniform_buffer,
            &dummy_r8.1,
            &dummy_r8.2,
            &default_lut.1,
            &overlay_buffer,
            None,
        );

        // Create contour rendering pipeline
        let contour_pipeline = ContourPipeline::new(&device, config.format);

        RenderingContext {
            window,
            surface,
            device,
            queue,
            config,
            render_pipeline,
            texture_bind_group_layout,
            uniform_buffer,
            vertex_buffer,
            index_buffer,
            num_indices,
            world,
            entities,
            gui,
            settings_entity,
            dummy_r8,
            default_lut,
            overlay_buffer,
            contour_pipeline,
            event_proxy,
        }
    }
}
