pub mod components;
pub mod context;
pub mod events;

use crate::app::components::*;
use crate::app::context::RenderingContext;
use crate::app::events::AppEvent;
use crate::io::handlers;
use crate::render::pipeline;
use crate::render::protocols;
use crate::systems;
use std::sync::Arc;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;
use winit::{
    application::ApplicationHandler,
    event::*,
    event_loop::{ActiveEventLoop, EventLoopProxy},
    window::WindowAttributes,
};

/// Pixel-to-line scroll normalization factor for trackpad deltas.
const PIXEL_SCROLL_FACTOR: f64 = 0.05;

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

impl Default for App {
    fn default() -> Self {
        Self::new()
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
                systems::sys_update_mouse(
                    &mut ctx.scene.world,
                    &ctx.scene.entities,
                    position.x,
                    position.y,
                );
                systems::sys_handle_mouse_drag(&mut ctx.scene.world, &ctx.scene.entities);
                ctx.window.request_redraw();
            }
            WindowEvent::MouseInput { button, state, .. } => {
                systems::sys_handle_mouse_button(
                    &mut ctx.scene.world,
                    &ctx.scene.entities,
                    button,
                    state,
                );
                ctx.window.request_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let y_delta = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(pos) => (pos.y * PIXEL_SCROLL_FACTOR) as f32,
                };
                if y_delta != 0.0 {
                    systems::sys_handle_input_scroll(
                        &mut ctx.scene.world,
                        &ctx.scene.entities,
                        y_delta,
                    );
                    ctx.window.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                systems::sys_update_modifiers(
                    &mut ctx.scene.world,
                    &ctx.scene.entities,
                    modifiers.state(),
                );
                ctx.window.request_redraw();
            }
            WindowEvent::Resized(size) => {
                ctx.gpu.config.width = size.width;
                ctx.gpu.config.height = size.height;
                ctx.gpu.surface.configure(&ctx.gpu.device, &ctx.gpu.config);
                let mut query = ctx
                    .scene
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
                    &ctx.gpu,
                    &ctx.volume_resources,
                    &mut ctx.pipelines,
                    &mut ctx.scene,
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
                                    &ctx.gpu.device,
                                    &ctx.gpu.queue,
                                    &mut ctx.scene.world,
                                    &ctx.scene.entities,
                                    loaded,
                                );
                                handlers::set_status_message(
                                    &mut ctx.scene.world,
                                    &ctx.scene.entities,
                                    format!("Volume Loaded: {}x{}", dims[0], dims[1]),
                                );
                                dims
                            }
                            LoadResult::Label(ref loaded_label) => {
                                let (new_entity, dims) = handlers::handle_label_load(
                                    &ctx.gpu.device,
                                    &ctx.gpu.queue,
                                    &mut ctx.scene.world,
                                    loaded_label,
                                );
                                if let Ok(mut editor) = ctx
                                    .scene
                                    .world
                                    .get::<&mut EditorState>(ctx.scene.entities.editor)
                                {
                                    editor.active_layer = Some(new_entity);
                                }
                                handlers::set_status_message(
                                    &mut ctx.scene.world,
                                    &ctx.scene.entities,
                                    format!("Label Loaded: {}x{}", dims[0], dims[1]),
                                );

                                dims
                            }
                        };

                        let active_layer = ctx
                            .scene
                            .world
                            .query::<&EditorState>()
                            .iter()
                            .next()
                            .and_then(|(_, e)| e.active_layer);
                        handlers::recreate_bind_groups(
                            &ctx.gpu.device,
                            &mut ctx.scene.world,
                            &handlers::BindGroupResources {
                                layout: &ctx.volume_resources.texture_bind_group_layout,
                                uniform_buffer: &ctx.volume_resources.uniform_buffer,
                                dummy_view: &ctx.volume_resources.dummy_r8.1,
                                dummy_sampler: &ctx.volume_resources.dummy_r8.2,
                                default_lut_view: &ctx.volume_resources.default_lut.1,
                                overlay_buffer: &ctx.volume_resources.overlay_buffer,
                            },
                            active_layer,
                        );
                    }
                    Err(e) => {
                        handlers::set_status_message(
                            &mut ctx.scene.world,
                            &ctx.scene.entities,
                            format!("Error: {:?}", e),
                        );
                    }
                }
                ctx.window.request_redraw();
            }
            AppEvent::RebuildBindGroups => {
                let active_layer = ctx
                    .scene
                    .world
                    .query::<&EditorState>()
                    .iter()
                    .next()
                    .and_then(|(_, e)| e.active_layer);
                handlers::recreate_bind_groups(
                    &ctx.gpu.device,
                    &mut ctx.scene.world,
                    &handlers::BindGroupResources {
                        layout: &ctx.volume_resources.texture_bind_group_layout,
                        uniform_buffer: &ctx.volume_resources.uniform_buffer,
                        dummy_view: &ctx.volume_resources.dummy_r8.1,
                        dummy_sampler: &ctx.volume_resources.dummy_r8.2,
                        default_lut_view: &ctx.volume_resources.default_lut.1,
                        overlay_buffer: &ctx.volume_resources.overlay_buffer,
                    },
                    active_layer,
                );
                ctx.window.request_redraw();
            }
            AppEvent::SwitchProtocol(name) => {
                protocols::apply_protocol(&mut ctx.scene.world, &ctx.scene.entities, &name);
                ctx.window.request_redraw();
            }
            AppEvent::ToggleMaximize(entity) => {
                protocols::toggle_maximize(&mut ctx.scene.world, &ctx.scene.entities, entity);
                ctx.window.request_redraw();
            }
            AppEvent::SwapViewports(a, b) => {
                protocols::swap_viewports(&mut ctx.scene.world, &ctx.scene.entities, a, b);
                ctx.window.request_redraw();
            }
            AppEvent::FocusAnnotation(id) => {
                if let Ok(mut state) = ctx
                    .scene
                    .world
                    .get::<&mut AnnotationState>(ctx.scene.entities.annotations)
                {
                    state.focused_id = Some(id);
                    state.show_right_sidebar = true;
                }
                ctx.window.request_redraw();
            }
            AppEvent::AddComment(id, text) => {
                if let Ok(mut state) = ctx
                    .scene
                    .world
                    .get::<&mut AnnotationState>(ctx.scene.entities.annotations)
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
                    .scene
                    .world
                    .get::<&mut AnnotationState>(ctx.scene.entities.annotations)
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
