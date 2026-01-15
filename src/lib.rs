// src/lib.rs

pub mod app;
pub mod gui;
pub mod io;
pub mod render;
pub mod systems;
pub mod util;

pub mod file_dialog;
pub mod overlay;

pub use crate::app::App;
pub use crate::io::handlers as load_handlers;
pub use crate::io::nifti as nifti_loader;
pub use crate::io::volume;
pub use crate::util::orientation;
pub use app::components;
pub use app::components::*;
pub use app::events::AppEvent;

use winit::event_loop::EventLoop;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

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
    let mut app = app::App::new();
    app.event_proxy = Some(event_loop.create_proxy());
    let _ = event_loop.run_app(&mut app);
}
