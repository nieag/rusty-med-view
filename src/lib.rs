// src/lib.rs

mod components;
mod file_dialog;
mod geometry;
mod gizmo;
mod gui;
mod load_handlers;
mod nifti_loader;
mod orientation;
mod overlay;
mod render;
mod systems;
mod volume;

use components::*;
use hecs::World;
use overlay::OverlayManager;
use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    event::*,
    event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
    window::{Window, WindowAttributes},
};

#[derive(Debug)]
pub enum AppEvent {
    VolumeLoaded(Result<components::LoadResult, nifti_loader::LoadError>),
    RebuildBindGroups,
    CreateNewLayer,
}

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
    entities: AppEntities,
    gui: gui::Gui,
    settings_entity: hecs::Entity,

    // Shared Resources
    dummy_r8: (wgpu::Texture, wgpu::TextureView, wgpu::Sampler),
    default_lut: (wgpu::Texture, wgpu::TextureView),
    overlay_buffer: wgpu::Buffer,

    // Proxy for waking up the event loop from async tasks
    event_proxy: EventLoopProxy<AppEvent>,
}

pub struct AppState {
    pub context: Option<RenderingContext>,
}

pub struct App {
    instance: wgpu::Instance,
    state: Arc<std::sync::Mutex<AppState>>,
    event_proxy: Option<EventLoopProxy<AppEvent>>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        Self {
            instance: wgpu::Instance::default(),
            state: Arc::new(std::sync::Mutex::new(AppState { context: None })),
            event_proxy: None,
        }
    }

    async fn create_rendering_context(
        instance: &wgpu::Instance,
        window: Arc<Window>,
        event_proxy: EventLoopProxy<AppEvent>,
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
        let present_mode = if surface_caps
            .present_modes
            .contains(&wgpu::PresentMode::Immediate)
        {
            wgpu::PresentMode::Immediate
        } else if surface_caps
            .present_modes
            .contains(&wgpu::PresentMode::Mailbox)
        {
            wgpu::PresentMode::Mailbox
        } else if surface_caps
            .present_modes
            .contains(&wgpu::PresentMode::AutoVsync)
        {
            wgpu::PresentMode::AutoVsync
        } else {
            wgpu::PresentMode::Fifo
        };

        let width = size.width.max(1);
        let height = size.height.max(1);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width,
            height,
            present_mode,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 1,
        };
        surface.configure(&device, &config);

        let mut world = World::new();
        // Initialize world and camera
        let (volume_texture, volume_view, volume_sampler) =
            volume::create_dummy_r32_texture(&device, &queue);
        let volume_data = VolumeData {
            dimensions: [0, 0, 0],
            spacing: [1.0, 1.0, 1.0],
            intensities: vec![],
            intensity_range: [0.0, 0.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };

        // Create shared resources
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

        // Initialize GUI State
        let gui_state = world.spawn((GuiState {
            status_message: None,
        },));

        let input = world.spawn((InputState {
            last_mouse_pos: [0.0, 0.0],
            mouse_uv: [0.0, 0.0],
            active_viewport: 0,
            modifiers: winit::keyboard::ModifiersState::empty(),
            is_dragging: false,
            is_panning: false,
            drag_start_pos: [0.0, 0.0],
            drag_start_pan: [0.0, 0.0],
            is_rotating: false,
            rotation_start_pos: [0.0, 0.0],
            rotation_start_val: [0.0, 0.0, 0.0, 1.0], // Identity quaternion
            egui_wants_input: false,
            scroll_accumulator: [0.0; 4],
        },));
        let view = world.spawn((ViewState {
            zoom: [1.0, 1.0, 1.0, 1.0],
            pan: [[0.0, 0.0]; 4],
            pivot: [[0.5, 0.5]; 4],
            rotation: [[0.0, 0.0, 0.0, 1.0]; 4], // Identity quaternions
        },));

        // --- Pipeline Setup (using render module) ---
        let uniform_buffer = render::create_uniform_buffer(&device);
        let texture_bind_group_layout = render::create_bind_group_layout(&device);
        let overlay_buffer = render::create_overlay_buffer(&device);

        let diffuse_bind_group = render::create_scene_bind_group(
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

        // Store persistent WGPU volume resources in ECS (volume_data is already VolumeData)
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

        // Initialize EditorState with active layer
        let editor = world.spawn((EditorState {
            active_layer: None,
            ..Default::default()
        },));

        // Initialize VolumeWindowing with default values
        let windowing = world.spawn((VolumeWindowing::default(),));

        // Initialize Annotations
        let annotations = world.spawn((AnnotationState {
            annotations: vec![],
        },));

        // Initialize Overlay State for GPU-rendered primitives
        let overlay = world.spawn((OverlayManager::default(),));

        let entities = AppEntities {
            input,
            view,
            editor,
            gui_state,
            volume_windowing: windowing,
            annotations,
            overlay,
            cursor,
            window_settings: settings_entity,
        };

        let render_pipeline =
            render::create_render_pipeline(&device, &texture_bind_group_layout, config.format);
        let (vertex_buffer, index_buffer, num_indices) = render::create_geometry_buffers(&device);

        let gui = gui::Gui::new(&device, config.format, &window);

        // Trigger bind group update to actually bind the new blank layer?
        // The `diffuse_bind_group` created above uses `label_view` (the demo one).
        // We need to update it to use `blank_view`.
        // Since we have ECS now, `recreate_bind_groups` will do it.
        // We can just call it once here.
        load_handlers::recreate_bind_groups(
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
            event_proxy,
        }
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let has_context = self.state.lock().unwrap().context.is_some();
        if !has_context {
            let window_attributes =
                WindowAttributes::default().with_title("Medical Viewer - Reactive Architecture");

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
            let proxy = self.event_proxy.as_ref().unwrap().clone();

            #[cfg(not(target_arch = "wasm32"))]
            {
                let context = pollster::block_on(Self::create_rendering_context(
                    &self.instance,
                    window,
                    proxy,
                ));
                self.state.lock().unwrap().context = Some(context);
            }

            #[cfg(target_arch = "wasm32")]
            {
                let state_clone = self.state.clone();
                let instance_clone = self.instance.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let context =
                        Self::create_rendering_context(&instance_clone, window, proxy).await;
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
            ctx.window.request_redraw();
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::CursorMoved { position, .. } => {
                systems::sys_update_mouse(&mut ctx.world, &ctx.entities, position.x, position.y);
                ctx.window.request_redraw();
            }
            WindowEvent::MouseInput { button, state, .. } => {
                systems::sys_handle_mouse_button(&mut ctx.world, &ctx.entities, button, state);
                ctx.window.request_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let y_delta = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(pos) => (pos.y * 0.05) as f32,
                };
                if y_delta != 0.0 {
                    systems::sys_handle_input_scroll(&mut ctx.world, &ctx.entities, y_delta);
                    ctx.window.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                systems::sys_update_modifiers(&mut ctx.world, &ctx.entities, modifiers.state());
                ctx.window.request_redraw();
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
                ctx.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                render::render_frame(
                    &ctx.device,
                    &ctx.queue,
                    &ctx.surface,
                    &ctx.config,
                    &ctx.render_pipeline,
                    &ctx.uniform_buffer,
                    &ctx.vertex_buffer,
                    &ctx.index_buffer,
                    ctx.num_indices,
                    &ctx.overlay_buffer,
                    &mut ctx.world,
                    &ctx.entities,
                    ctx.settings_entity,
                    &mut ctx.gui,
                    &ctx.window,
                    ctx.event_proxy.clone(),
                );
            }
            _ => {}
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AppEvent) {
        let mut state = self.state.lock().unwrap();
        let ctx = if let Some(ctx) = &mut state.context {
            ctx
        } else {
            return;
        };

        match event {
            AppEvent::VolumeLoaded(result) => {
                match result {
                    Ok(load_res) => {
                        let dims = match load_res {
                            components::LoadResult::Volume(ref loaded) => {
                                let dims = load_handlers::handle_volume_load(
                                    &ctx.device,
                                    &ctx.queue,
                                    &mut ctx.world,
                                    &ctx.entities,
                                    loaded,
                                );
                                load_handlers::set_status_message(
                                    &mut ctx.world,
                                    &ctx.entities,
                                    format!("Volume Loaded: {}x{}", dims[0], dims[1]),
                                );
                                dims
                            }
                            components::LoadResult::Label(ref loaded_label) => {
                                let (new_entity, dims) = load_handlers::handle_label_load(
                                    &ctx.device,
                                    &ctx.queue,
                                    &mut ctx.world,
                                    loaded_label,
                                );
                                load_handlers::set_status_message(
                                    &mut ctx.world,
                                    &ctx.entities,
                                    format!("Label Loaded: {}x{}", dims[0], dims[1]),
                                );

                                // Set the newly loaded layer as active for editing
                                for (_, editor) in ctx.world.query_mut::<&mut EditorState>() {
                                    editor.active_layer = Some(new_entity);
                                }

                                dims
                            }
                        };
                        log::info!("Loaded data with dimensions: {:?}", dims);

                        let active_layer = ctx
                            .world
                            .query::<&EditorState>()
                            .iter()
                            .next()
                            .and_then(|(_, e)| e.active_layer);

                        // Recreate bind groups with updated textures
                        load_handlers::recreate_bind_groups(
                            &ctx.device,
                            &mut ctx.world,
                            &ctx.texture_bind_group_layout,
                            &ctx.uniform_buffer,
                            &ctx.dummy_r8.1,
                            &ctx.dummy_r8.2,
                            &ctx.default_lut.1,
                            &ctx.overlay_buffer,
                            active_layer,
                        );
                    }
                    Err(e) => {
                        log::error!("Failed to load NIfTI: {:?}", e);
                        load_handlers::set_status_message(
                            &mut ctx.world,
                            &ctx.entities,
                            format!("Error: {:?}", e),
                        );
                    }
                }
                ctx.window.request_redraw();
            }
            AppEvent::RebuildBindGroups => {
                let active_layer = ctx
                    .world
                    .query::<&EditorState>()
                    .iter()
                    .next()
                    .and_then(|(_, e)| e.active_layer);

                load_handlers::recreate_bind_groups(
                    &ctx.device,
                    &mut ctx.world,
                    &ctx.texture_bind_group_layout,
                    &ctx.uniform_buffer,
                    &ctx.dummy_r8.1,
                    &ctx.dummy_r8.2,
                    &ctx.default_lut.1,
                    &ctx.overlay_buffer,
                    active_layer,
                );
                ctx.window.request_redraw();
            }
            AppEvent::CreateNewLayer => {
                // Get dimensions from main volume
                let mut dims = [64, 64, 64];
                for (_, vol) in ctx.world.query::<&VolumeData>().iter() {
                    dims = vol.dimensions;
                }

                let (tex, view, sampler, data) =
                    volume::create_blank_labelmap(&ctx.device, &ctx.queue, dims);

                // Generate unique name
                let mut count = 0;
                for _ in ctx.world.query::<&Segmentation>().iter() {
                    count += 1;
                }
                let name = format!("Layer {}", count + 1);

                // Fetch an existing bind group to generic placeholder
                let mut placeholder_bg = None;
                if let Some((_, res)) = ctx.world.query::<&GpuVolumeResources>().iter().next() {
                    placeholder_bg = Some(res.bind_group.clone());
                }
                let placeholder_bg =
                    placeholder_bg.expect("Main volume should exist and have a bindRef");

                let entity = ctx.world.spawn((
                    Segmentation {
                        name,
                        is_visible: true,
                    },
                    LayerSettings { opacity: 0.7 },
                    LabelmapData {
                        dimensions: dims,
                        raw_data: data,
                    },
                    Representation::Voxel(GpuVolumeResources {
                        texture: tex,
                        view,
                        sampler,
                        bind_group: placeholder_bg,
                    }),
                    SegmentationTag,
                ));

                // Now globally update the bind groups
                let active_layer = ctx
                    .world
                    .query::<&EditorState>()
                    .iter()
                    .next()
                    .and_then(|(_, e)| e.active_layer);

                load_handlers::recreate_bind_groups(
                    &ctx.device,
                    &mut ctx.world,
                    &ctx.texture_bind_group_layout,
                    &ctx.uniform_buffer,
                    &ctx.dummy_r8.1,
                    &ctx.dummy_r8.2,
                    &ctx.default_lut.1,
                    &ctx.overlay_buffer,
                    active_layer,
                );

                // Select the new layer
                for (_, editor) in ctx.world.query_mut::<&mut EditorState>() {
                    editor.active_layer = Some(entity);
                }
                ctx.window.request_redraw();
            }
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

    let event_loop = EventLoop::<AppEvent>::with_user_event().build().unwrap();
    let mut app = App::new();
    app.event_proxy = Some(event_loop.create_proxy());
    let _ = event_loop.run_app(&mut app);
}
