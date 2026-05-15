pub mod components;
pub mod context;
pub mod events;
pub mod qa;
pub mod roi_runtime;

use crate::app::components::*;
use crate::app::context::RenderingContext;
use crate::app::events::AppEvent;
use crate::io::handlers;
use crate::render::pipeline;
use crate::render::protocols;
use crate::systems;
use std::collections::BTreeMap;
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
    pub qa: qa::QaRuntime,
}

pub struct App {
    pub instance: wgpu::Instance,
    pub state: Arc<std::sync::Mutex<AppState>>,
    pub event_proxy: Option<EventLoopProxy<AppEvent>>,
}

impl App {
    pub fn new(
        qa_enabled: bool,
        requested_sample: Option<String>,
        requested_preset: Option<String>,
    ) -> Self {
        let mut qa = qa::QaRuntime::new(qa_enabled, requested_sample, requested_preset);
        if qa_enabled {
            qa.log(
                0,
                qa::QaLevel::Info,
                "qa",
                "qa mode enabled",
                BTreeMap::new(),
            );
        }
        Self {
            instance: wgpu::Instance::default(),
            state: Arc::new(std::sync::Mutex::new(AppState { context: None, qa })),
            event_proxy: None,
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new(false, None, None)
    }
}

impl AppState {
    pub fn qa_state_snapshot(&self) -> qa::QaSnapshot {
        let qa_info = qa::QaSnapshotQa {
            enabled: self.qa.enabled,
            version: qa::QA_API_VERSION,
            requested_sample: self.qa.requested_sample.clone(),
            requested_preset: self.qa.requested_preset.clone(),
            ready: self.qa.ready(),
            last_error: self.qa.last_error.clone(),
        };

        if let Some(ctx) = &self.context {
            let status = ctx
                .scene
                .world
                .get::<&GuiState>(ctx.scene.entities.gui_state)
                .ok()
                .and_then(|state| state.status_message.clone());

            let (active_roi_entity, active_tool) = ctx
                .scene
                .world
                .get::<&EditorState>(ctx.scene.entities.editor)
                .map(|editor| {
                    (
                        editor.active_roi,
                        Some(match editor.active_tool {
                            EditorTool::Navigation => "navigation".to_string(),
                            EditorTool::ContourSelect => "contour_select".to_string(),
                            EditorTool::ContourDraw => "contour_draw".to_string(),
                        }),
                    )
                })
                .unwrap_or((None, None));

            let mut active_roi_id = None;
            let mut active_roi_name = None;
            let mut rois = Vec::new();
            for (entity, roi) in ctx.scene.world.query::<&Roi>().iter() {
                if Some(entity) == active_roi_entity {
                    active_roi_id = Some(roi.metadata.roi_id.0);
                    active_roi_name = Some(roi.metadata.name.clone());
                }
                rois.push(qa::QaSnapshotRoi {
                    id: roi.metadata.roi_id.0,
                    name: roi.metadata.name.clone(),
                    visible: roi.metadata.is_visible,
                    active: Some(entity) == active_roi_entity,
                });
            }

            let volume = ctx
                .scene
                .world
                .query::<&VolumeData>()
                .with::<&MainVolumeTag>()
                .iter()
                .next()
                .map(|(_, v)| {
                    let world_bounds = if v.dimensions.iter().all(|d| *d > 0) {
                        let max = [
                            v.origin[0] + v.spacing[0] * (v.dimensions[0] as f32 - 1.0),
                            v.origin[1] + v.spacing[1] * (v.dimensions[1] as f32 - 1.0),
                            v.origin[2] + v.spacing[2] * (v.dimensions[2] as f32 - 1.0),
                        ];
                        Some([v.origin, max])
                    } else {
                        None
                    };
                    qa::QaSnapshotVolume {
                        loaded: v.dimensions != [0, 0, 0],
                        dimensions: v.dimensions,
                        spacing: v.spacing,
                        origin: v.origin,
                        orientation: v.orientation,
                        world_bounds,
                    }
                })
                .unwrap_or(qa::QaSnapshotVolume {
                    loaded: false,
                    dimensions: [0, 0, 0],
                    spacing: [0.0, 0.0, 0.0],
                    origin: [0.0, 0.0, 0.0],
                    orientation: [0.0, 0.0, 0.0, 1.0],
                    world_bounds: None,
                });

            let mut viewports = Vec::new();
            for (_, vp) in ctx.scene.world.query::<&Viewport>().iter() {
                let mode = match vp.mode {
                    ViewMode::ThreeD => "three_d",
                    ViewMode::Axial => "axial",
                    ViewMode::Coronal => "coronal",
                    ViewMode::Sagittal => "sagittal",
                    ViewMode::Oblique => "oblique",
                };
                let valid_rect = vp.rect[2] > 0.0 && vp.rect[3] > 0.0;
                viewports.push(qa::QaSnapshotViewport {
                    mode: mode.to_string(),
                    rect: vp.rect,
                    ready: valid_rect,
                    overlay_renderable: false,
                });
            }

            let app = qa::QaSnapshotApp {
                status,
                active_tool,
                active_roi_id,
                active_roi_name,
                frame_counter: self.qa.frame_counter,
            };

            let render = qa::QaSnapshotRender {
                overlay_slots_used: 0,
                overlay_slots_max: 2,
            };

            qa::QaSnapshot {
                qa: qa_info,
                app,
                volume,
                rois,
                viewports,
                render,
            }
        } else {
            qa::QaSnapshot {
                qa: qa_info,
                app: qa::QaSnapshotApp {
                    status: None,
                    active_tool: None,
                    active_roi_id: None,
                    active_roi_name: None,
                    frame_counter: self.qa.frame_counter,
                },
                volume: qa::QaSnapshotVolume {
                    loaded: false,
                    dimensions: [0, 0, 0],
                    spacing: [0.0, 0.0, 0.0],
                    origin: [0.0, 0.0, 0.0],
                    orientation: [0.0, 0.0, 0.0, 1.0],
                    world_bounds: None,
                },
                rois: vec![],
                viewports: vec![],
                render: qa::QaSnapshotRender {
                    overlay_slots_used: 0,
                    overlay_slots_max: 2,
                },
            }
        }
    }

    pub fn qa_metrics_snapshot(&self) -> qa::QaMetricsSnapshot {
        let visible_rois = self.context.as_ref().map_or(0, |ctx| {
            ctx.scene
                .world
                .query::<&Roi>()
                .iter()
                .filter(|(_, roi)| roi.metadata.is_visible)
                .count()
        });
        let warning_count = self
            .qa
            .log_buffer
            .snapshot()
            .events
            .iter()
            .filter(|event| event.level == qa::QaLevel::Warn)
            .count() as u64;
        let error_count = self
            .qa
            .log_buffer
            .snapshot()
            .events
            .iter()
            .filter(|event| event.level == qa::QaLevel::Error)
            .count() as u64;
        qa::QaMetricsSnapshot {
            frame_counter: self.qa.frame_counter,
            visible_rois,
            overlay_slots_used: 0,
            overlay_slots_max: 2,
            warning_count,
            error_count,
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
        let app_state = &mut *state;
        let (context, qa_runtime) = (&mut app_state.context, &mut app_state.qa);
        let ctx = if let Some(ctx) = context {
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
                qa_runtime.frame_counter = qa_runtime.frame_counter.saturating_add(1);
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
        let app_state = &mut *state;
        let (context, qa_runtime) = (&mut app_state.context, &mut app_state.qa);
        let ctx = if let Some(ctx) = context {
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
                                match handlers::handle_label_load(
                                    &ctx.gpu.device,
                                    &ctx.gpu.queue,
                                    &mut ctx.scene.world,
                                    loaded_label,
                                ) {
                                    Ok(outcome) => {
                                        if let Ok(mut editor) = ctx
                                            .scene
                                            .world
                                            .get::<&mut EditorState>(ctx.scene.entities.editor)
                                        {
                                            editor.active_roi = Some(outcome.entity);
                                        }
                                        let dims = outcome.dimensions;
                                        handlers::set_status_message(
                                            &mut ctx.scene.world,
                                            &ctx.scene.entities,
                                            format!("Label Loaded: {}x{}", dims[0], dims[1]),
                                        );

                                        dims
                                    }
                                    Err(err) => {
                                        let mut fields = BTreeMap::new();
                                        fields.insert("error".to_string(), err.clone());
                                        let _event = qa_runtime.log(
                                            qa_runtime.frame_counter,
                                            qa::QaLevel::Error,
                                            "load.label",
                                            "label load failed",
                                            fields.clone(),
                                        );
                                        qa_runtime.set_error("load.label", err.clone(), fields);
                                        #[cfg(target_arch = "wasm32")]
                                        if qa_runtime.enabled {
                                            let json = qa::to_json(&_event);
                                            web_sys::console::error_1(
                                                &wasm_bindgen::JsValue::from_str(&json),
                                            );
                                        }
                                        handlers::set_status_message(
                                            &mut ctx.scene.world,
                                            &ctx.scene.entities,
                                            err,
                                        );
                                        [0, 0, 0]
                                    }
                                }
                            }
                        };

                        let active_roi = ctx
                            .scene
                            .world
                            .query::<&EditorState>()
                            .iter()
                            .next()
                            .and_then(|(_, e)| e.active_roi);
                        roi_runtime::recreate_scene_bind_groups(
                            &ctx.gpu.device,
                            &mut ctx.scene.world,
                            &roi_runtime::BindGroupResources {
                                layout: &ctx.volume_resources.texture_bind_group_layout,
                                uniform_buffer: &ctx.volume_resources.uniform_buffer,
                                dummy_view: &ctx.volume_resources.dummy_r8.1,
                                dummy_sampler: &ctx.volume_resources.dummy_r8.2,
                                default_lut_view: &ctx.volume_resources.default_lut.1,
                                overlay_buffer: &ctx.volume_resources.overlay_buffer,
                            },
                            active_roi,
                        );
                    }
                    Err(e) => {
                        let mut fields = BTreeMap::new();
                        fields.insert("error".to_string(), format!("{e:?}"));
                        let _event = qa_runtime.log(
                            qa_runtime.frame_counter,
                            qa::QaLevel::Error,
                            "load.volume",
                            "volume or label load event failed",
                            fields.clone(),
                        );
                        qa_runtime.set_error("load.volume", format!("{e:?}"), fields);
                        #[cfg(target_arch = "wasm32")]
                        if qa_runtime.enabled {
                            let json = qa::to_json(&_event);
                            web_sys::console::error_1(&wasm_bindgen::JsValue::from_str(&json));
                        }
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
                let active_roi = ctx
                    .scene
                    .world
                    .query::<&EditorState>()
                    .iter()
                    .next()
                    .and_then(|(_, e)| e.active_roi);
                roi_runtime::recreate_scene_bind_groups(
                    &ctx.gpu.device,
                    &mut ctx.scene.world,
                    &roi_runtime::BindGroupResources {
                        layout: &ctx.volume_resources.texture_bind_group_layout,
                        uniform_buffer: &ctx.volume_resources.uniform_buffer,
                        dummy_view: &ctx.volume_resources.dummy_r8.1,
                        dummy_sampler: &ctx.volume_resources.dummy_r8.2,
                        default_lut_view: &ctx.volume_resources.default_lut.1,
                        overlay_buffer: &ctx.volume_resources.overlay_buffer,
                    },
                    active_roi,
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
