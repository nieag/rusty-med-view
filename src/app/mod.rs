pub mod components;
pub mod context;
pub mod events;
pub mod segment;

use crate::app::components::*;
use crate::app::context::RenderingContext;
use crate::app::events::AppEvent;
use crate::io::handlers;
use crate::render::pipeline;
use crate::render::protocols;
use crate::systems;
use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    event::*,
    event_loop::{ActiveEventLoop, EventLoopProxy},
    window::WindowAttributes,
};

pub struct AppState {
    pub context: Option<RenderingContext>,
}

pub struct App {
    pub instance: wgpu::Instance,
    pub state: Arc<std::sync::Mutex<AppState>>,
    pub event_proxy: Option<EventLoopProxy<AppEvent>>,
}

impl App {
    pub fn new() -> Self {
        Self {
            instance: wgpu::Instance::default(),
            state: Arc::new(std::sync::Mutex::new(AppState { context: None })),
            event_proxy: None,
        }
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let has_context = self.state.lock().unwrap().context.is_some();
        if !has_context {
            let window_attributes = WindowAttributes::default().with_title("Medical Viewer");

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
                let context =
                    pollster::block_on(RenderingContext::new(&self.instance, window, proxy));
                self.state.lock().unwrap().context = Some(context);
            }

            #[cfg(target_arch = "wasm32")]
            {
                let state_clone = self.state.clone();
                let instance_clone = self.instance.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let context = RenderingContext::new(&instance_clone, window, proxy).await;
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
                let repaint_after = pipeline::render_frame(
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
                    &mut ctx.gui,
                    &ctx.window,
                    ctx.event_proxy.clone(),
                );

                if repaint_after.is_zero() {
                    ctx.window.request_redraw();
                }
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
                        let _dims = match load_res {
                            LoadResult::Volume(ref loaded) => {
                                let dims = handlers::handle_volume_load(
                                    &ctx.device,
                                    &ctx.queue,
                                    &mut ctx.world,
                                    &ctx.entities,
                                    loaded,
                                );
                                handlers::set_status_message(
                                    &mut ctx.world,
                                    &ctx.entities,
                                    format!("Volume Loaded: {}x{}", dims[0], dims[1]),
                                );
                                dims
                            }
                            LoadResult::Label(ref loaded_label) => {
                                let (new_entity, dims) = handlers::handle_label_load(
                                    &ctx.device,
                                    &ctx.queue,
                                    &mut ctx.world,
                                    loaded_label,
                                );
                                handlers::set_status_message(
                                    &mut ctx.world,
                                    &ctx.entities,
                                    format!("Label Loaded: {}x{}", dims[0], dims[1]),
                                );

                                for (_, editor) in ctx.world.query_mut::<&mut EditorState>() {
                                    editor.active_layer = Some(new_entity);
                                }
                                dims
                            }
                        };

                        let active_layer = ctx
                            .world
                            .query::<&EditorState>()
                            .iter()
                            .next()
                            .and_then(|(_, e)| e.active_layer);
                        handlers::recreate_bind_groups(
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
                        handlers::set_status_message(
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
                handlers::recreate_bind_groups(
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
                let mut dims = [64, 64, 64];
                for (_, vol) in ctx.world.query::<&VolumeData>().iter() {
                    dims = vol.dimensions;
                }
                let (tex, view, sampler, data) =
                    crate::io::volume::create_blank_labelmap(&ctx.device, &ctx.queue, dims);
                let mut count = 0;
                for _ in ctx.world.query::<&Segmentation>().iter() {
                    count += 1;
                }
                let name = format!("Layer {}", count + 1);
                let mut placeholder_bg = None;
                if let Some((_, res)) = ctx.world.query::<&GpuVolumeResources>().iter().next() {
                    placeholder_bg = Some(res.bind_group.clone());
                }
                let placeholder_bg = placeholder_bg.expect("Main volume should exist");
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
                let active_layer = ctx
                    .world
                    .query::<&EditorState>()
                    .iter()
                    .next()
                    .and_then(|(_, e)| e.active_layer);
                handlers::recreate_bind_groups(
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
                for (_, editor) in ctx.world.query_mut::<&mut EditorState>() {
                    editor.active_layer = Some(entity);
                }
                ctx.window.request_redraw();
            }
            AppEvent::SwitchProtocol(name) => {
                protocols::apply_protocol(&mut ctx.world, &ctx.entities, &name);
                ctx.window.request_redraw();
            }
            AppEvent::ToggleMaximize(entity) => {
                protocols::toggle_maximize(&mut ctx.world, &ctx.entities, entity);
                ctx.window.request_redraw();
            }
            AppEvent::SwapViewports(a, b) => {
                protocols::swap_viewports(&mut ctx.world, &ctx.entities, a, b);
                ctx.window.request_redraw();
            }
            AppEvent::FocusAnnotation(id) => {
                if let Ok(mut state) = ctx
                    .world
                    .get::<&mut AnnotationState>(ctx.entities.annotations)
                {
                    state.focused_id = Some(id);
                    state.show_right_sidebar = true;
                }
                ctx.window.request_redraw();
            }
            AppEvent::AddComment(id, text) => {
                if let Ok(mut state) = ctx
                    .world
                    .get::<&mut AnnotationState>(ctx.entities.annotations)
                {
                    if let Some(ann) = state.annotations.iter_mut().find(|a| a.id == id) {
                        ann.comments.push(Comment {
                            author: "User".to_string(),
                            text,
                        });
                    }
                }
                ctx.window.request_redraw();
            }
            AppEvent::DeleteAnnotation(id) => {
                if let Ok(mut state) = ctx
                    .world
                    .get::<&mut AnnotationState>(ctx.entities.annotations)
                {
                    state.annotations.retain(|a| a.id != id);
                    if state.focused_id == Some(id) {
                        state.focused_id = None;
                    }
                }
                ctx.window.request_redraw();
            }
        }
    }
}
