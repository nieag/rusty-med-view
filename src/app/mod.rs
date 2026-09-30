pub mod annotations;
pub mod components;
pub mod context;
pub mod events;
pub mod handlers;
pub mod qa;
pub mod roi;
pub mod roi_runtime;

use crate::app::components::*;
use crate::app::context::RenderingContext;
use crate::app::events::AppEvent;
use crate::app::qa::sample::{
    apply_image_label_mpr_basic_preset, QA_PRESET_IMAGE_LABEL_MPR_BASIC, QA_SAMPLE_LIVER_0,
};
#[cfg(target_arch = "wasm32")]
use crate::app::qa::sample::{spawn_qa_fetch_label, spawn_qa_fetch_volume};
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
    zoom_redraw_pending: bool,
}

impl App {
    pub fn new(
        qa_enabled: bool,
        requested_sample: Option<String>,
        requested_preset: Option<String>,
    ) -> Self {
        let mut qa = qa::QaRuntime::new(qa_enabled, requested_sample, requested_preset);
        if qa_enabled {
            if qa.requested_sample.as_deref() == Some(QA_SAMPLE_LIVER_0) {
                qa.sample_phase = qa::QaSamplePhase::NotRequested;
            }
            if qa.requested_preset.as_deref() == Some(QA_PRESET_IMAGE_LABEL_MPR_BASIC) {
                qa.preset_phase = qa::QaPresetPhase::PendingSample;
            }
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
            zoom_redraw_pending: false,
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new(false, None, None)
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
                match pollster::block_on(RenderingContext::new(&self.instance, window, proxy)) {
                    Ok(context) => self.state.lock().unwrap().context = Some(context),
                    Err(err) => {
                        log::error!("{}", err.message);
                        let mut guard = self.state.lock().unwrap();
                        if guard.qa.enabled {
                            let mut fields = BTreeMap::new();
                            fields.insert("error".to_string(), err.message.clone());
                            let frame = guard.qa.frame_counter;
                            let _event = guard.qa.log(
                                frame,
                                qa::QaLevel::Error,
                                err.category,
                                "rendering context initialization failed",
                                fields.clone(),
                            );
                            guard
                                .qa
                                .set_error(err.category, err.message.clone(), fields);
                            #[cfg(target_arch = "wasm32")]
                            {
                                let json = qa::to_json(&event);
                                web_sys::console::error_1(&wasm_bindgen::JsValue::from_str(&json));
                            }
                        }
                    }
                }
            }

            #[cfg(target_arch = "wasm32")]
            {
                let state_clone = self.state.clone();
                let instance_clone = self.instance.clone();
                let proxy_clone = proxy.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    match RenderingContext::new(&instance_clone, window, proxy).await {
                        Ok(context) => {
                            state_clone.lock().unwrap().context = Some(context);
                            let _ = proxy_clone.send_event(AppEvent::QaStartSampleLoad);
                        }
                        Err(err) => {
                            log::error!("{}", err.message);
                            let mut guard = state_clone.lock().unwrap();
                            if guard.qa.enabled {
                                let mut fields = BTreeMap::new();
                                fields.insert("error".to_string(), err.message.clone());
                                let frame = guard.qa.frame_counter;
                                let event = guard.qa.log(
                                    frame,
                                    qa::QaLevel::Error,
                                    err.category,
                                    "rendering context initialization failed",
                                    fields.clone(),
                                );
                                guard
                                    .qa
                                    .set_error(err.category, err.message.clone(), fields);
                                let json = qa::to_json(&event);
                                web_sys::console::error_1(&wasm_bindgen::JsValue::from_str(&json));
                            }
                        }
                    }
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

        let egui_consumed = ctx.gui.handle_event(&ctx.window, &event);
        if let WindowEvent::CursorMoved { position, .. } = &event {
            systems::sys_update_mouse(
                &mut ctx.scene.world,
                &mut ctx.scene.session,
                position.x,
                position.y,
            );
            systems::sys_handle_mouse_drag(&mut ctx.scene.world, &mut ctx.scene.session);
            ctx.window.request_redraw();
            return;
        }
        if egui_consumed {
            ctx.window.request_redraw();
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::CursorMoved { .. } => unreachable!("cursor events return above"),
            WindowEvent::MouseInput { button, state, .. } => {
                systems::sys_handle_mouse_button(
                    &mut ctx.scene.world,
                    &mut ctx.scene.session,
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
                    let changed_3d_zoom = systems::sys_handle_input_scroll(
                        &mut ctx.scene.world,
                        &mut ctx.scene.session,
                        y_delta,
                    );
                    if changed_3d_zoom {
                        self.zoom_redraw_pending = true;
                    } else {
                        ctx.window.request_redraw();
                    }
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                systems::sys_update_modifiers(&mut ctx.scene.session, modifiers.state());
                ctx.window.request_redraw();
            }
            WindowEvent::Resized(size) => {
                apply_surface_size(ctx, size);
                ctx.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                // The window is the truth. On the web the initial resize can arrive before the
                // context exists; without this the surface and the ROI overlays stay 1x1.
                let size = ctx.window.inner_size();
                if size.width > 0
                    && size.height > 0
                    && (size.width != ctx.gpu.config.width || size.height != ctx.gpu.config.height)
                {
                    apply_surface_size(ctx, size);
                }
                qa_runtime.frame_counter = qa_runtime.frame_counter.saturating_add(1);
                qa_runtime.last_presented_frame = Some(qa_runtime.frame_counter);
                let (repaint_after, frame_stats) = pipeline::render_frame(
                    &ctx.gpu,
                    &ctx.volume_resources,
                    &mut ctx.pipelines,
                    &mut ctx.scene,
                    &mut ctx.gui,
                    &ctx.window,
                    ctx.event_proxy.clone(),
                );
                qa_runtime.viewport_uniform_count = frame_stats.viewport_uniform_count;
                qa_runtime.contour_batch_count = frame_stats.contour_batch_count;
                qa_runtime.mesh_batch_count = frame_stats.mesh_batch_count;
                qa_runtime.mesh_chunks_uploaded = frame_stats.mesh_chunks_uploaded;
                qa_runtime.mesh_chunks_reused = frame_stats.mesh_chunks_reused;
                if frame_stats.view3d_marched {
                    qa_runtime.view3d_march_count = qa_runtime.view3d_march_count.saturating_add(1);
                }
                if let Some(category) = frame_stats.last_warning {
                    qa_runtime.last_render_warning = Some(category.to_string());
                }
                for message in ctx.gpu.errors.take() {
                    handlers::set_status_message(&mut ctx.scene.session, message.clone());
                    qa_runtime.last_render_error = Some(message.clone());
                    qa_runtime.set_error("wgpu.runtime", message, Default::default());
                }
                if let Some(category) = frame_stats.last_error {
                    qa_runtime.last_render_error = Some(category.to_string());
                }

                if repaint_after.is_zero()
                    || frame_stats.roi_work_pending
                    || frame_stats.camera_settling
                {
                    ctx.window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if !self.zoom_redraw_pending {
            return;
        }

        let state = self.state.lock().unwrap();
        if let Some(ctx) = &state.context {
            ctx.window.request_redraw();
            self.zoom_redraw_pending = false;
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
            AppEvent::QaStartSampleLoad => {
                #[cfg(target_arch = "wasm32")]
                if qa_runtime.enabled
                    && !qa_runtime.sample_bootstrap_started
                    && qa_runtime.requested_sample.as_deref() == Some(QA_SAMPLE_LIVER_0)
                {
                    qa_runtime.sample_bootstrap_started = true;
                    qa_runtime.sample_phase = qa::QaSamplePhase::FetchingVolume;
                    if qa_runtime.requested_preset.as_deref()
                        == Some(QA_PRESET_IMAGE_LABEL_MPR_BASIC)
                    {
                        qa_runtime.preset_phase = qa::QaPresetPhase::PendingSample;
                    }
                    let mut fields = BTreeMap::new();
                    fields.insert("sample".to_string(), QA_SAMPLE_LIVER_0.to_string());
                    qa_runtime.log(
                        qa_runtime.frame_counter,
                        qa::QaLevel::Info,
                        "qa.sample",
                        "fetching sample volume",
                        fields,
                    );
                    spawn_qa_fetch_volume(ctx.event_proxy.clone());
                }
            }
            AppEvent::VolumeLoaded(result) => {
                match result {
                    Ok(load_res) => {
                        if qa_runtime.enabled {
                            qa_runtime.sample_phase = match load_res {
                                LoadResult::Volume(_) => qa::QaSamplePhase::LoadingVolume,
                                LoadResult::Label(_) => qa::QaSamplePhase::LoadingLabel,
                            };
                        }
                        #[cfg(target_arch = "wasm32")]
                        let is_volume = matches!(load_res, LoadResult::Volume(_));
                        let is_label = matches!(load_res, LoadResult::Label(_));
                        let _dims = match load_res {
                            LoadResult::Volume(ref loaded) => {
                                let dims = handlers::handle_volume_load(
                                    &ctx.gpu.device,
                                    &ctx.gpu.queue,
                                    &mut ctx.scene.world,
                                    &mut ctx.scene.session,
                                    loaded,
                                );
                                handlers::set_status_message(
                                    &mut ctx.scene.session,
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
                                        {
                                            let editor = &mut ctx.scene.session.editor;
                                            editor.active_roi = outcome.session.first().copied();
                                        }
                                        let dims = outcome.dimensions;
                                        let message = if outcome.session.len() > 1 {
                                            format!(
                                                "Label Loaded: {} labels as separate ROIs ({}x{})",
                                                outcome.session.len(),
                                                dims[0],
                                                dims[1]
                                            )
                                        } else {
                                            format!("Label Loaded: {}x{}", dims[0], dims[1])
                                        };
                                        handlers::set_status_message(
                                            &mut ctx.scene.session,
                                            message,
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
                                        handlers::set_status_message(&mut ctx.scene.session, err);
                                        [0, 0, 0]
                                    }
                                }
                            }
                        };

                        let active_roi = ctx.scene.session.editor.active_roi;
                        roi_runtime::recreate_scene_bind_groups(
                            &ctx.gpu.device,
                            &mut ctx.scene.world,
                            &roi_runtime::BindGroupResources {
                                layout: &ctx.volume_resources.texture_bind_group_layout,
                                uniform_buffer: &ctx.volume_resources.uniform_buffer,
                                dummy_view: &ctx.volume_resources.dummy_r8.1,
                                dummy_sampler: &ctx.volume_resources.dummy_r8.2,
                                default_lut_view: &ctx.volume_resources.default_lut.1,
                            },
                            active_roi,
                        );

                        #[cfg(target_arch = "wasm32")]
                        if qa_runtime.enabled
                            && is_volume
                            && qa_runtime.requested_sample.as_deref() == Some(QA_SAMPLE_LIVER_0)
                        {
                            qa_runtime.sample_phase = qa::QaSamplePhase::FetchingLabel;
                            let mut fields = BTreeMap::new();
                            fields.insert("sample".to_string(), QA_SAMPLE_LIVER_0.to_string());
                            qa_runtime.log(
                                qa_runtime.frame_counter,
                                qa::QaLevel::Info,
                                "qa.sample",
                                "volume loaded, fetching label",
                                fields,
                            );
                            spawn_qa_fetch_label(ctx.event_proxy.clone());
                        }

                        if qa_runtime.enabled
                            && is_label
                            && qa_runtime.requested_sample.as_deref() == Some(QA_SAMPLE_LIVER_0)
                        {
                            qa_runtime.sample_phase = qa::QaSamplePhase::Loaded;
                            if qa_runtime.requested_preset.as_deref()
                                == Some(QA_PRESET_IMAGE_LABEL_MPR_BASIC)
                            {
                                qa_runtime.preset_phase = qa::QaPresetPhase::Applying;
                                let active_roi = {
                                    let editor = &mut ctx.scene.session.editor;
                                    editor.active_roi
                                };
                                if let Some(active_roi) = active_roi {
                                    let ok = apply_image_label_mpr_basic_preset(
                                        &mut ctx.scene.world,
                                        &mut ctx.scene.session,
                                        active_roi,
                                    );
                                    if ok {
                                        qa_runtime.preset_phase = qa::QaPresetPhase::Applied;
                                        qa_runtime.preset_applied_frame =
                                            Some(qa_runtime.frame_counter);
                                        qa_runtime.active_qa_roi =
                                            Some(format!("{:?}", active_roi));
                                        let _ =
                                            ctx.event_proxy.send_event(AppEvent::RebuildBindGroups);
                                        qa_runtime.log(
                                            qa_runtime.frame_counter,
                                            qa::QaLevel::Info,
                                            "qa.preset",
                                            "image_label_mpr_basic applied",
                                            BTreeMap::new(),
                                        );
                                    } else {
                                        qa_runtime.preset_phase = qa::QaPresetPhase::Failed;
                                        let fields = BTreeMap::new();
                                        qa_runtime.set_error(
                                            "qa.preset",
                                            "failed to center cursor from non-empty label bounds",
                                            fields,
                                        );
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let is_qa_sample = qa_runtime.enabled
                            && qa_runtime.requested_sample.as_deref() == Some(QA_SAMPLE_LIVER_0);
                        let mut fields = BTreeMap::new();
                        fields.insert("error".to_string(), format!("{e:?}"));
                        let error_category = if is_qa_sample {
                            "qa.sample"
                        } else {
                            "load.volume"
                        };
                        let _event = qa_runtime.log(
                            qa_runtime.frame_counter,
                            qa::QaLevel::Error,
                            error_category,
                            "volume or label load event failed",
                            fields.clone(),
                        );
                        if is_qa_sample {
                            qa_runtime.set_error("qa.sample", format!("{e:?}"), fields);
                        } else {
                            qa_runtime.set_error("load.volume", format!("{e:?}"), fields);
                        }
                        if qa_runtime.enabled
                            && qa_runtime.requested_sample.as_deref() == Some(QA_SAMPLE_LIVER_0)
                        {
                            qa_runtime.sample_phase = qa::QaSamplePhase::Failed;
                            if qa_runtime.requested_preset.as_deref()
                                == Some(QA_PRESET_IMAGE_LABEL_MPR_BASIC)
                            {
                                qa_runtime.preset_phase = qa::QaPresetPhase::Failed;
                            }
                        }
                        #[cfg(target_arch = "wasm32")]
                        if qa_runtime.enabled {
                            let json = qa::to_json(&_event);
                            web_sys::console::error_1(&wasm_bindgen::JsValue::from_str(&json));
                        }
                        handlers::set_status_message(
                            &mut ctx.scene.session,
                            format!("Error: {:?}", e),
                        );
                    }
                }
                ctx.window.request_redraw();
            }
            AppEvent::RebuildBindGroups => {
                let active_roi = ctx.scene.session.editor.active_roi;
                roi_runtime::recreate_scene_bind_groups(
                    &ctx.gpu.device,
                    &mut ctx.scene.world,
                    &roi_runtime::BindGroupResources {
                        layout: &ctx.volume_resources.texture_bind_group_layout,
                        uniform_buffer: &ctx.volume_resources.uniform_buffer,
                        dummy_view: &ctx.volume_resources.dummy_r8.1,
                        dummy_sampler: &ctx.volume_resources.dummy_r8.2,
                        default_lut_view: &ctx.volume_resources.default_lut.1,
                    },
                    active_roi,
                );
                ctx.window.request_redraw();
            }
            AppEvent::SwitchProtocol(name) => {
                protocols::apply_protocol(&mut ctx.scene.world, &mut ctx.scene.session, &name);
                ctx.window.request_redraw();
            }
            AppEvent::ToggleMaximize(entity) => {
                protocols::toggle_maximize(&mut ctx.scene.world, &mut ctx.scene.session, entity);
                ctx.window.request_redraw();
            }
            AppEvent::SwapViewports(a, b) => {
                protocols::swap_viewports(&mut ctx.scene.world, &ctx.scene.session, a, b);
                ctx.window.request_redraw();
            }
            AppEvent::FocusAnnotation(id) => {
                {
                    let state = &mut ctx.scene.session.annotations;
                    state.focused_id = Some(id);
                    state.show_right_sidebar = true;
                }
                ctx.window.request_redraw();
            }
            AppEvent::AddComment(id, text) => {
                let author = ctx.scene.session.user.clone();
                annotations::add_comment(&mut ctx.scene.world, id, text, &author);
                ctx.window.request_redraw();
            }
            AppEvent::DeleteAnnotation(id) => {
                annotations::delete_annotation(&mut ctx.scene.world, id);
                let state = &mut ctx.scene.session.annotations;
                if state.focused_id == Some(id) {
                    state.focused_id = None;
                }
                ctx.window.request_redraw();
            }
        }
    }
}

/// Reconfigures the surface for a new window size and records it for the systems that clip
/// overlays to the window.
fn apply_surface_size(ctx: &mut RenderingContext, size: winit::dpi::PhysicalSize<u32>) {
    ctx.gpu.config.width = size.width;
    ctx.gpu.config.height = size.height;
    ctx.gpu.surface.configure(&ctx.gpu.device, &ctx.gpu.config);
    let settings = &mut ctx.scene.session.window_settings;
    settings.width = size.width;
    settings.height = size.height;
}
