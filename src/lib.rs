// src/lib.rs

mod components;
mod file_dialog;
mod geometry;
mod gui;
mod nifti_loader;
mod systems;
mod volume;

use components::*;
use hecs::World;
use std::sync::Arc;
use wgpu::util::DeviceExt;
use winit::{
    application::ApplicationHandler,
    event::*,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowAttributes},
};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

pub struct RenderingContext {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,

    // Pipelines and Buffers
    render_pipeline: wgpu::RenderPipeline,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    num_indices: u32,

    // ECS and GUI
    world: World,
    gui: gui::Gui,
    settings_entity: hecs::Entity,

    // Async file loading channel
    volume_receiver:
        std::sync::mpsc::Receiver<Result<nifti_loader::LoadedVolume, nifti_loader::LoadError>>,
    volume_sender:
        std::sync::mpsc::Sender<Result<nifti_loader::LoadedVolume, nifti_loader::LoadError>>,
}

pub struct AppState {
    pub context: Option<RenderingContext>,
}

pub struct App {
    instance: wgpu::Instance,
    state: Arc<std::sync::Mutex<AppState>>,
}

impl App {
    pub fn new() -> Self {
        Self {
            instance: wgpu::Instance::default(),
            state: Arc::new(std::sync::Mutex::new(AppState { context: None })),
        }
    }

    async fn create_rendering_context(
        instance: &wgpu::Instance,
        window: Arc<Window>,
    ) -> RenderingContext {
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
            .expect("Failed to find an appropriate adapter. Ensure WebGPU/WebGL is enabled.");

        log::info!("Adapter found: {:?}", adapter.get_info());

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .expect("Failed to create device");

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps.formats[0];
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        // --- Initialize Data & ECS ---
        let mut world = World::new();
        let (volume_texture, volume_view, volume_sampler, volume_info) =
            volume::create_demo_voxel_texture(&device, &queue);

        // Create async channel for volume loading
        let (volume_sender, volume_receiver) = std::sync::mpsc::channel();

        // Initialize world and camera
        world.spawn((CameraRig {
            radius: 3.5,
            speed: 1.0,
            start_time: web_time::Instant::now(),
        },));
        world.spawn((
            Transform {
                position: [0.5, 0.5, 0.5],
                rotation: [0.0, 0.0, 0.0],
            },
            CursorTag,
        ));
        let settings_entity = world.spawn((WindowSettings {
            width: config.width,
            height: config.height,
        },));

        // Initialize GUI State
        world.spawn((GuiState {
            load_requested: false,
            status_message: None,
        },));

        // Initial loading state
        world.spawn((VolumeLoadingState::Ready,));
        world.spawn((InputState {
            last_mouse_pos: [0.0, 0.0],
            mouse_uv: [0.0, 0.0],
            active_viewport: 0,
            modifiers: winit::keyboard::ModifiersState::empty(),
            is_dragging: false,
            drag_start_pos: [0.0, 0.0],
            drag_start_pan: [0.0, 0.0],
        },));
        world.spawn((ViewState {
            zoom: [1.0, 1.0, 1.0, 1.0],
            pan: [[0.0, 0.0]; 4],
            pivot: [[0.5, 0.5]; 4],
        },));

        // --- Pipeline Setup ---
        let uniform_alignment = 256;
        let uniform_buffer_size = (uniform_alignment * 4) as u64;
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Uniform Buffer"),
            size: uniform_buffer_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D3,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: true,
                            min_binding_size: Some(
                                std::num::NonZeroU64::new(std::mem::size_of::<Uniforms>() as u64)
                                    .unwrap(),
                            ),
                        },
                        count: None,
                    },
                ],
                label: Some("texture_bind_group_layout"),
            });

        let diffuse_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&volume_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&volume_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &uniform_buffer,
                        offset: 0,
                        size: Some(
                            std::num::NonZeroU64::new(std::mem::size_of::<Uniforms>() as u64)
                                .unwrap(),
                        ),
                    }),
                },
            ],
            label: Some("diffuse_bind_group"),
        });

        // Store persistent WGPU volume resources in ECS
        world.spawn((
            VolumeData {
                dimensions: volume_info.dimensions,
                spacing: volume_info.spacing,
                intensities: volume_info.intensities,
                intensity_range: volume_info.intensity_range,
            },
            GpuVolumeResources {
                texture: volume_texture,
                view: volume_view,
                sampler: volume_sampler,
                bind_group: diffuse_bind_group,
            },
            MainVolumeTag,
        ));

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/shader.wgsl").into()),
        });

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &[&texture_bind_group_layout],
                push_constant_ranges: &[],
            });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"), // Updated for wgpu 23
                buffers: &[geometry::Vertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"), // Updated for wgpu 23
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: bytemuck::cast_slice(geometry::QUAD_VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Index Buffer"),
            contents: bytemuck::cast_slice(geometry::QUAD_INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });
        let num_indices = geometry::QUAD_INDICES.len() as u32;

        let gui = gui::Gui::new(&device, config.format, &window);

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
            gui,
            settings_entity,
            volume_receiver,
            volume_sender,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let has_context = self.state.lock().unwrap().context.is_some();
        if !has_context {
            let window_attributes =
                WindowAttributes::default().with_title("Medical Viewer - Refactor (WASM)");

            // On Wasm, we need to find the canvas and append the window to it
            #[cfg(target_arch = "wasm32")]
            let window_attributes = {
                use winit::platform::web::WindowAttributesExtWebSys;
                let canvas = web_sys::window()
                    .and_then(|win| win.document())
                    .and_then(|doc| doc.get_element_by_id("canvas"))
                    .and_then(|canvas| canvas.dyn_into::<web_sys::HtmlCanvasElement>().ok());
                window_attributes.with_canvas(canvas)
            };

            let window = Arc::new(event_loop.create_window(window_attributes).unwrap());

            #[cfg(not(target_arch = "wasm32"))]
            {
                let context =
                    pollster::block_on(Self::create_rendering_context(&self.instance, window));
                self.state.lock().unwrap().context = Some(context);
            }

            #[cfg(target_arch = "wasm32")]
            {
                let state_clone = self.state.clone();
                let instance_clone = self.instance.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let context = Self::create_rendering_context(&instance_clone, window).await;
                    state_clone.lock().unwrap().context = Some(context);
                });
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let mut state = self.state.lock().unwrap();
        let ctx = if let Some(ctx) = &mut state.context {
            ctx
        } else {
            return;
        };

        if ctx.gui.handle_event(&ctx.window, &event) {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::CursorMoved { position, .. } => {
                systems::sys_update_mouse(&mut ctx.world, position.x, position.y);
            }
            WindowEvent::MouseInput { button, state, .. } => {
                systems::sys_handle_mouse_button(&mut ctx.world, button, state);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let y_delta = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(pos) => (pos.y * 0.001) as f32,
                };
                if y_delta != 0.0 {
                    systems::sys_handle_input_scroll(&mut ctx.world, y_delta);
                    ctx.window.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                systems::sys_update_modifiers(&mut ctx.world, modifiers.state());
            }
            WindowEvent::Resized(size) => {
                ctx.config.width = size.width;
                ctx.config.height = size.height;
                ctx.surface.configure(&ctx.device, &ctx.config);
                let mut query = ctx
                    .world
                    .query_one::<&mut WindowSettings>(ctx.settings_entity)
                    .unwrap();
                if let Some(settings) = query.get() {
                    settings.width = size.width;
                    settings.height = size.height;
                }
            }
            WindowEvent::RedrawRequested => {
                // Render Logic
                let mut all_uniforms_bytes = Vec::with_capacity(1024);
                for mode in 0..4 {
                    let u = systems::sys_prepare_render_data(&mut ctx.world, mode);
                    all_uniforms_bytes.extend_from_slice(bytemuck::cast_slice(&[u]));
                    while all_uniforms_bytes.len() % 256 != 0 {
                        all_uniforms_bytes.push(0);
                    }
                }
                ctx.queue
                    .write_buffer(&ctx.uniform_buffer, 0, &all_uniforms_bytes);
                ctx.gui.prepare(&ctx.window, &mut ctx.world);

                let frame = ctx.surface.get_current_texture().unwrap();
                let view = frame
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
                let mut encoder =
                    ctx.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("Render Encoder"),
                        });

                {
                    let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Render Pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: 0.1,
                                    g: 0.1,
                                    b: 0.1,
                                    a: 1.0,
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });

                    render_pass.set_pipeline(&ctx.render_pipeline);
                    render_pass.set_vertex_buffer(0, ctx.vertex_buffer.slice(..));
                    render_pass
                        .set_index_buffer(ctx.index_buffer.slice(..), wgpu::IndexFormat::Uint16);

                    let hw = ctx.config.width / 2;
                    let hh = ctx.config.height / 2;

                    // Query the bind group from ECS
                    // Query the bind group from ECS and render
                    {
                        let mut query = ctx
                            .world
                            .query::<&GpuVolumeResources>()
                            .with::<&MainVolumeTag>();
                        if let Some((_, res)) = query.iter().next() {
                            let bg = &res.bind_group;
                            render_pass.set_viewport(0.0, 0.0, hw as f32, hh as f32, 0.0, 1.0);
                            render_pass.set_bind_group(0, bg, &[0]);
                            render_pass.draw_indexed(0..ctx.num_indices, 0, 0..1);

                            render_pass
                                .set_viewport(hw as f32, 0.0, hw as f32, hh as f32, 0.0, 1.0);
                            render_pass.set_bind_group(0, bg, &[256]);
                            render_pass.draw_indexed(0..ctx.num_indices, 0, 0..1);

                            render_pass
                                .set_viewport(0.0, hh as f32, hw as f32, hh as f32, 0.0, 1.0);
                            render_pass.set_bind_group(0, bg, &[512]);
                            render_pass.draw_indexed(0..ctx.num_indices, 0, 0..1);

                            render_pass
                                .set_viewport(hw as f32, hh as f32, hw as f32, hh as f32, 0.0, 1.0);
                            render_pass.set_bind_group(0, bg, &[768]);
                            render_pass.draw_indexed(0..ctx.num_indices, 0, 0..1);
                        }
                    }
                }

                let screen_descriptor = egui_wgpu::ScreenDescriptor {
                    size_in_pixels: [ctx.config.width, ctx.config.height],
                    pixels_per_point: ctx.window.scale_factor() as f32,
                };
                ctx.gui.render(
                    &ctx.device,
                    &ctx.queue,
                    &mut encoder,
                    &view,
                    &screen_descriptor,
                );

                ctx.queue.submit(std::iter::once(encoder.finish()));
                frame.present();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        let mut state = self.state.lock().unwrap();
        if let Some(ctx) = &mut state.context {
            systems::sys_handle_mouse_drag(&mut ctx.world);

            // Check if user clicked the load button (read from GuiState)
            let mut load_requested = false;
            for (_, gui_state) in ctx.world.query_mut::<&mut GuiState>() {
                if gui_state.load_requested {
                    load_requested = true;
                    gui_state.load_requested = false; // Reset
                    gui_state.status_message = Some("Loading...".to_string());
                }
            }

            if load_requested {
                let sender = ctx.volume_sender.clone();
                file_dialog::spawn_file_picker(move |result| {
                    if let Some((_filename, data)) = result {
                        let load_result = nifti_loader::load_nifti_from_bytes(&data);
                        let _ = sender.send(load_result);
                    }
                });
            }

            // Check for loaded volumes from async task
            if let Ok(result) = ctx.volume_receiver.try_recv() {
                match result {
                    Ok(loaded) => {
                        log::info!("Volume loaded: {:?} dimensions", loaded.dimensions);

                        // Create new texture from loaded data
                        let (new_texture, new_view, new_sampler, volume_info) =
                            volume::create_texture_from_nifti(&ctx.device, &ctx.queue, &loaded);

                        // Recreate bind group with new texture
                        let new_bind_group =
                            ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                layout: &ctx.texture_bind_group_layout,
                                entries: &[
                                    wgpu::BindGroupEntry {
                                        binding: 0,
                                        resource: wgpu::BindingResource::TextureView(&new_view),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 1,
                                        resource: wgpu::BindingResource::Sampler(&new_sampler),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 2,
                                        resource: wgpu::BindingResource::Buffer(
                                            wgpu::BufferBinding {
                                                buffer: &ctx.uniform_buffer,
                                                offset: 0,
                                                size: Some(
                                                    std::num::NonZeroU64::new(std::mem::size_of::<
                                                        Uniforms,
                                                    >(
                                                    )
                                                        as u64)
                                                    .unwrap(),
                                                ),
                                            },
                                        ),
                                    },
                                ],
                                label: Some("diffuse_bind_group"),
                            });

                        // Update VolumeData and GpuVolumeResources in ECS
                        for (_, (vol, gpu_res)) in ctx
                            .world
                            .query_mut::<(&mut VolumeData, &mut GpuVolumeResources)>()
                        {
                            vol.dimensions = volume_info.dimensions;
                            vol.spacing = volume_info.spacing;
                            vol.intensities = volume_info.intensities.clone();
                            vol.intensity_range = volume_info.intensity_range;

                            gpu_res.texture = new_texture;
                            gpu_res.view = new_view;
                            gpu_res.sampler = new_sampler;
                            gpu_res.bind_group = new_bind_group;

                            // Break out after first match assuming one main volume entity
                            break;
                        }

                        for (_, gui_state) in ctx.world.query_mut::<&mut GuiState>() {
                            gui_state.status_message = Some(format!(
                                "Loaded: {}x{}x{}",
                                volume_info.dimensions[0],
                                volume_info.dimensions[1],
                                volume_info.dimensions[2]
                            ));
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to load NIfTI: {:?}", e);
                        for (_, gui_state) in ctx.world.query_mut::<&mut GuiState>() {
                            gui_state.status_message = Some(format!("Error: {:?}", e));
                        }
                    }
                }
            }

            ctx.window.request_redraw();
        }
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(start))]
pub fn run() {
    #[cfg(target_arch = "wasm32")]
    {
        std::panic::set_hook(Box::new(console_error_panic_hook::hook));
        console_log::init_with_level(log::Level::Info).expect("Could not initialize logger");
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Simple console logger for native
        let _ = env_logger::builder()
            .filter_level(log::LevelFilter::Info)
            .is_test(true)
            .try_init();
    }

    let event_loop = EventLoop::new().unwrap();
    let mut app = App::new();
    let _ = event_loop.run_app(&mut app);
}
