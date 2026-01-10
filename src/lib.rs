// src/lib.rs

mod components;
mod file_dialog;
mod geometry;
mod gizmo;
mod gui;
mod load_handlers;
mod nifti_loader;
mod render;
mod systems;
mod volume;

use components::*;
use hecs::World;
use std::sync::Arc;
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
        std::sync::mpsc::Receiver<Result<components::LoadResult, nifti_loader::LoadError>>,
    volume_sender: std::sync::mpsc::Sender<Result<components::LoadResult, nifti_loader::LoadError>>,

    // Shared Resources
    dummy_r8: (wgpu::Texture, wgpu::TextureView, wgpu::Sampler),
    default_lut: (wgpu::Texture, wgpu::TextureView),
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
        let (volume_texture, volume_view, volume_sampler, volume_data) =
            volume::create_demo_voxel_texture(&device, &queue);

        // Create shared resources
        let dummy_r8 = volume::create_dummy_r8_texture(&device, &queue);
        let default_lut = volume::create_default_colormap(&device, &queue);

        // Create Demo Labelmap
        let (label_tex, label_view, label_sampler) = volume::create_demo_labelmap(&device, &queue);

        // Create async channel for volume loading
        let (volume_sender, volume_receiver) =
            std::sync::mpsc::channel::<Result<components::LoadResult, nifti_loader::LoadError>>();

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
            viewport_rect: [0.0, 0.0, config.width as f32, config.height as f32],
        },));

        // Initialize GUI State
        world.spawn((GuiState {
            load_requested: false,
            load_label_requested: false,
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
            is_rotating: false,
            rotation_start_pos: [0.0, 0.0],
            rotation_start_val: [0.0, 0.0, 0.0, 1.0], // Identity quaternion
        },));
        world.spawn((ViewState {
            zoom: [1.0, 1.0, 1.0, 1.0],
            pan: [[0.0, 0.0]; 4],
            pivot: [[0.5, 0.5]; 4],
            rotation: [[0.0, 0.0, 0.0, 1.0]; 4], // Identity quaternions
        },));

        // --- Pipeline Setup (using render module) ---
        let uniform_buffer = render::create_uniform_buffer(&device);
        let texture_bind_group_layout = render::create_bind_group_layout(&device);

        let diffuse_bind_group = render::create_scene_bind_group(
            &device,
            &texture_bind_group_layout,
            &volume_view,
            &volume_sampler,
            &uniform_buffer,
            &label_view,
            &default_lut.1,
            &dummy_r8.1,
            &default_lut.1,
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

        // Spawn Demo Labelmap Entity
        // NOTE: We need to also store the CPU data for editing!
        // create_demo_labelmap doesn't return data, so we need to recreate the data or call it differently.
        // Let's modify behavior slightly: Just create a "Blank" one of same size for simplicity OR
        // we can fetch the data back from GPU? No, unnecessary complexity.
        // Let's just create a blank one for now that is editable, OR update create_demo_labelmap to return data.
        // For minimal changes: Let's assume the demo map is STATIC or we just create a NEW blank editable one.
        // The user request "Add basic label editing".
        // Let's spawn a NEW Blank Editable Layer instead of the hardcoded demo one?
        // OR better: Update Components to hold LabelmapData for the demo layer.
        // For speed, let's just make a new editable layer.

        // Actually, to make the Demo layer editable, we need its data.
        // Let's just create a Blank 64x64x64 layer for "New Layer" testing.
        // But let's spawn the demo layer as non-editable for now (since we lack CPU data easily here without copy paste).
        // Wait, I can just copy the logic from `create_demo_labelmap` here to generate data.
        // Or I can update `volume.rs` to return data.
        // I will adhere to "minimal changes" principle and just create a NEW blank layer logic in the Paint system or here.

        // Let's create an EDITABLE blank layer at start so the user has something to draw on.
        let (blank_tex, blank_view, blank_sampler, blank_data) =
            volume::create_blank_labelmap(&device, &queue, [64, 64, 64]);
        let blank_entity = world.spawn((
            Segmentation {
                name: "Layer 1".to_string(),
                is_visible: true,
            },
            LayerSettings {
                opacity: 0.7,
                active_representation: 0,
            },
            LabelmapData {
                dimensions: [64, 64, 64],
                spacing: [1.0, 1.0, 1.0],
                raw_data: blank_data,
            },
            Representation::Voxel(GpuVolumeResources {
                texture: blank_tex,
                view: blank_view,
                sampler: blank_sampler,
                bind_group: diffuse_bind_group.clone(), // Re-use until recreation
            }),
            SegmentationTag,
        ));

        // Initialize EditorState with active layer
        world.spawn((EditorState {
            active_layer: Some(blank_entity),
            ..Default::default()
        },));

        // Initialize VolumeWindowing with default values
        world.spawn((VolumeWindowing::default(),));

        // Initialize Annotations
        world.spawn((AnnotationState {
            annotations: vec![
                Annotation {
                    world_pos: glam::Vec3::new(0.5, 0.5, 0.5),
                    label: "Target".to_string(),
                },
                Annotation {
                    world_pos: glam::Vec3::new(0.2, 0.2, 0.2),
                    label: "Tumor A".to_string(),
                },
            ],
        },));

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
            gui,
            settings_entity,
            volume_receiver,
            volume_sender,
            dummy_r8,
            default_lut,
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
                    &mut ctx.world,
                    &mut ctx.gui,
                    &ctx.window,
                    ctx.volume_sender.clone(),
                );
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        let mut state = self.state.lock().unwrap();
        if let Some(ctx) = &mut state.context {
            systems::sys_handle_mouse_drag(&mut ctx.world);
            systems::sys_paint(&mut ctx.world, &ctx.queue); // Execute Paint System

            // Handle "Create New Layer" request from GUI?
            // Cleanest way: Check GuiState for a request flag
            let mut create_layer = false;
            for (_, gui_state) in ctx.world.query::<&mut GuiState>().iter() {
                if gui_state.load_label_requested {
                    gui_state.load_label_requested = false;
                    create_layer = true;
                }
            }

            if create_layer {
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
                for (_, res) in ctx.world.query::<&GpuVolumeResources>().iter() {
                    placeholder_bg = Some(res.bind_group.clone());
                    break;
                }
                // Fallback for initial startup if no volume exists yet (should not happen if we load default volume first)
                // But create_rendering_context creates volume FIRST.
                let placeholder_bg =
                    placeholder_bg.expect("Main volume should exist and have a bindRef");

                let entity = ctx.world.spawn((
                    Segmentation {
                        name,
                        is_visible: true,
                    },
                    LayerSettings {
                        opacity: 0.7,
                        active_representation: 0,
                    },
                    LabelmapData {
                        dimensions: dims,
                        spacing: [1.0, 1.0, 1.0],
                        raw_data: data,
                    },
                    Representation::Voxel(GpuVolumeResources {
                        texture: tex,
                        view: view,
                        sampler: sampler,
                        bind_group: placeholder_bg,
                    }),
                    SegmentationTag,
                ));

                // Now globally update the bind groups (this updates everyone's bind_group to the new correct one)
                load_handlers::recreate_bind_groups(
                    &ctx.device,
                    &mut ctx.world,
                    &ctx.texture_bind_group_layout,
                    &ctx.uniform_buffer,
                    &ctx.dummy_r8.1,
                    &ctx.dummy_r8.2,
                    &ctx.default_lut.1,
                );

                // Select the new layer
                for (_, editor) in ctx.world.query_mut::<&mut EditorState>() {
                    editor.active_layer = Some(entity);
                }
            }

            // File dialogs are now spawned directly from GUI button clicks (gui.rs)
            // to maintain browser user gesture chain for WASM compatibility.

            // Check for loaded data from async task
            if let Ok(result) = ctx.volume_receiver.try_recv() {
                match result {
                    Ok(load_res) => {
                        let dims = match load_res {
                            components::LoadResult::Volume(ref loaded) => {
                                let dims = load_handlers::handle_volume_load(
                                    &ctx.device,
                                    &ctx.queue,
                                    &mut ctx.world,
                                    loaded,
                                );
                                load_handlers::set_status_message(
                                    &mut ctx.world,
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

                        // Recreate bind groups with updated textures
                        load_handlers::recreate_bind_groups(
                            &ctx.device,
                            &mut ctx.world,
                            &ctx.texture_bind_group_layout,
                            &ctx.uniform_buffer,
                            &ctx.dummy_r8.1,
                            &ctx.dummy_r8.2,
                            &ctx.default_lut.1,
                        );
                    }
                    Err(e) => {
                        log::error!("Failed to load NIfTI: {:?}", e);
                        load_handlers::set_status_message(
                            &mut ctx.world,
                            format!("Error: {:?}", e),
                        );
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
