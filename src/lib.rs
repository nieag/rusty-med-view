// src/lib.rs

pub mod app;
pub mod convert;
pub mod gui;
pub mod io;
pub mod model;
pub mod render;
pub mod systems;
pub mod util;

pub mod file_dialog;
pub mod overlay;

pub use crate::app::handlers as load_handlers;
pub use crate::app::App;
pub use crate::io::nifti as nifti_loader;
pub use crate::io::volume;
pub use crate::util::orientation;
pub use app::components;
pub use app::components::*;
pub use app::events::AppEvent;

use winit::event_loop::EventLoop;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::closure::Closure;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

#[cfg(target_arch = "wasm32")]
thread_local! {
    static QA_APP_STATE: std::cell::RefCell<Option<std::sync::Arc<std::sync::Mutex<app::AppState>>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(target_arch = "wasm32")]
fn qa_param_value(search: &str, key: &str) -> Option<String> {
    let trimmed = search.strip_prefix('?').unwrap_or(search);
    trimmed.split('&').find_map(|pair| {
        let mut it = pair.splitn(2, '=');
        let k = it.next()?;
        let v = it.next().unwrap_or("");
        if k == key && !v.is_empty() {
            Some(v.to_string())
        } else {
            None
        }
    })
}

#[cfg(target_arch = "wasm32")]
fn qa_enabled_from_url() -> (bool, Option<String>, Option<String>) {
    let search = web_sys::window()
        .and_then(|win| win.location().search().ok())
        .unwrap_or_default();
    let qa_enabled = qa_param_value(&search, "qa").as_deref() == Some("1");
    let sample = qa_param_value(&search, "sample");
    let preset = qa_param_value(&search, "preset");
    (qa_enabled, sample, preset)
}

#[cfg(target_arch = "wasm32")]
fn qa_state_json() -> String {
    QA_APP_STATE.with(|cell| {
        let Some(state) = cell.borrow().as_ref().cloned() else {
            return "{}".to_string();
        };
        let out = match state.lock() {
            Ok(guard) => app::qa::to_json(&guard.qa_state_snapshot()),
            Err(_) => "{}".to_string(),
        };
        out
    })
}

#[cfg(target_arch = "wasm32")]
fn qa_metrics_json() -> String {
    QA_APP_STATE.with(|cell| {
        let Some(state) = cell.borrow().as_ref().cloned() else {
            return "{}".to_string();
        };
        let out = match state.lock() {
            Ok(guard) => app::qa::to_json(&guard.qa_metrics_snapshot()),
            Err(_) => "{}".to_string(),
        };
        out
    })
}

#[cfg(target_arch = "wasm32")]
fn qa_logs_json() -> String {
    QA_APP_STATE.with(|cell| {
        let Some(state) = cell.borrow().as_ref().cloned() else {
            return "{}".to_string();
        };
        let out = match state.lock() {
            Ok(guard) => app::qa::to_json(&guard.qa.logs_snapshot()),
            Err(_) => "{}".to_string(),
        };
        out
    })
}

#[cfg(target_arch = "wasm32")]
fn qa_last_error_json() -> String {
    QA_APP_STATE.with(|cell| {
        let Some(state) = cell.borrow().as_ref().cloned() else {
            return "null".to_string();
        };
        let out = match state.lock() {
            Ok(guard) => app::qa::to_json(&guard.qa.last_error),
            Err(_) => "null".to_string(),
        };
        out
    })
}

#[cfg(target_arch = "wasm32")]
fn parse_json_or_null(json: String) -> wasm_bindgen::JsValue {
    js_sys::JSON::parse(&json).unwrap_or(wasm_bindgen::JsValue::NULL)
}

#[cfg(target_arch = "wasm32")]
fn install_viewer_qa() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let obj = js_sys::Object::new();

    let version = Closure::wrap(Box::new(move || {
        wasm_bindgen::JsValue::from_f64(app::qa::QA_API_VERSION as f64)
    }) as Box<dyn FnMut() -> wasm_bindgen::JsValue>);
    let _ = js_sys::Reflect::set(&obj, &"version".into(), version.as_ref().unchecked_ref());
    version.forget();

    let state = Closure::wrap(Box::new(move || parse_json_or_null(qa_state_json()))
        as Box<dyn FnMut() -> wasm_bindgen::JsValue>);
    let _ = js_sys::Reflect::set(&obj, &"state".into(), state.as_ref().unchecked_ref());
    state.forget();

    let metrics = Closure::wrap(Box::new(move || parse_json_or_null(qa_metrics_json()))
        as Box<dyn FnMut() -> wasm_bindgen::JsValue>);
    let _ = js_sys::Reflect::set(&obj, &"metrics".into(), metrics.as_ref().unchecked_ref());
    metrics.forget();

    let logs = Closure::wrap(Box::new(move || parse_json_or_null(qa_logs_json()))
        as Box<dyn FnMut() -> wasm_bindgen::JsValue>);
    let _ = js_sys::Reflect::set(&obj, &"logs".into(), logs.as_ref().unchecked_ref());
    logs.forget();

    let last_error = Closure::wrap(Box::new(move || parse_json_or_null(qa_last_error_json()))
        as Box<dyn FnMut() -> wasm_bindgen::JsValue>);
    let _ = js_sys::Reflect::set(
        &obj,
        &"lastError".into(),
        last_error.as_ref().unchecked_ref(),
    );
    last_error.forget();

    let wait_for_ready = js_sys::Function::new_with_args(
        "options",
        "const timeoutMs = options?.timeoutMs ?? 10000;
         const pollMs = options?.pollMs ?? 50;
         return new Promise((resolve, reject) => {
           const start = performance.now();
           const tick = () => {
             const state = window.__viewerQa.state();
             if (state?.qa?.ready) { resolve(state); return; }
             if (performance.now() - start >= timeoutMs) {
               reject({ message: 'wait_for_ready timeout', state, last_error: window.__viewerQa.lastError() });
               return;
             }
             setTimeout(tick, pollMs);
           };
           tick();
         });",
    );
    let _ = js_sys::Reflect::set(&obj, &"waitForReady".into(), &wait_for_ready.into());

    let _ = js_sys::Reflect::set(&window, &"__viewerQa".into(), &obj);
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
        let _ = env_logger::builder()
            .filter_level(log::LevelFilter::Info)
            .is_test(true)
            .try_init();
    }

    let event_loop = EventLoop::<app::events::AppEvent>::with_user_event()
        .build()
        .unwrap();
    #[cfg(target_arch = "wasm32")]
    let (qa_enabled, requested_sample, requested_preset) = qa_enabled_from_url();
    #[cfg(not(target_arch = "wasm32"))]
    let (qa_enabled, requested_sample, requested_preset) = (false, None, None);

    let mut app = app::App::new(qa_enabled, requested_sample, requested_preset);
    #[cfg(target_arch = "wasm32")]
    {
        QA_APP_STATE.with(|cell| {
            *cell.borrow_mut() = Some(app.state.clone());
        });
        if qa_enabled {
            install_viewer_qa();
        }
    }
    app.event_proxy = Some(event_loop.create_proxy());
    let _ = event_loop.run_app(&mut app);
}
