// src/file_dialog.rs
//! Cross-platform file dialog module supporting both native and WASM targets.
//!
//! Native: Uses rfd's AsyncFileDialog
//! WASM: Uses a custom web-sys implementation for seamless file selection

// ============================================================================
// Native Implementation (using rfd)
// ============================================================================
#[cfg(not(target_arch = "wasm32"))]
mod native {
    use rfd::AsyncFileDialog;

    pub async fn pick_nifti_file() -> Option<(String, Vec<u8>)> {
        let file = AsyncFileDialog::new()
            .add_filter("NIfTI Files", &["nii", "nii.gz", "gz"])
            .add_filter("All Files", &["*"])
            .set_title("Open NIfTI Volume")
            .pick_file()
            .await?;

        let filename = file.file_name();
        let data = file.read().await;

        Some((filename, data))
    }

    pub fn spawn_file_picker<F>(callback: F)
    where
        F: FnOnce(Option<(String, Vec<u8>)>) + Send + 'static,
    {
        std::thread::spawn(move || {
            let result = pollster::block_on(pick_nifti_file());
            callback(result);
        });
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::spawn_file_picker;

// ============================================================================
// WASM Implementation (using web-sys directly)
// ============================================================================
#[cfg(target_arch = "wasm32")]
mod wasm {
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsCast;
    use web_sys::{Document, FileReader, HtmlInputElement};

    /// Spawn a file picker that triggers immediately from a user gesture.
    /// The callback receives the filename and file contents.
    pub fn spawn_file_picker<F>(callback: F)
    where
        F: FnOnce(Option<(String, Vec<u8>)>) + 'static,
    {
        let window = match web_sys::window() {
            Some(w) => w,
            None => {
                log::error!("No window object");
                callback(None);
                return;
            }
        };

        let document: Document = match window.document() {
            Some(d) => d,
            None => {
                log::error!("No document object");
                callback(None);
                return;
            }
        };

        // Create a hidden file input element
        let input: HtmlInputElement = match document.create_element("input") {
            Ok(el) => match el.dyn_into::<HtmlInputElement>() {
                Ok(input) => input,
                Err(_) => {
                    log::error!("Failed to cast to HtmlInputElement");
                    callback(None);
                    return;
                }
            },
            Err(_) => {
                log::error!("Failed to create input element");
                callback(None);
                return;
            }
        };

        input.set_type("file");
        input.set_accept(".nii,.nii.gz,.gz");
        input.style().set_property("display", "none").ok();

        // Append to document body temporarily
        if let Some(body) = document.body() {
            let _ = body.append_child(&input);
        }

        // Wrap callback in Rc<RefCell> so it can be moved into the closure
        let callback = Rc::new(RefCell::new(Some(callback)));
        let input_clone = input.clone();

        // Set up the change handler
        let callback_clone = callback.clone();
        let onchange = Closure::wrap(Box::new(move |_event: web_sys::Event| {
            let input = &input_clone;

            let files = match input.files() {
                Some(files) => files,
                None => {
                    if let Some(cb) = callback_clone.borrow_mut().take() {
                        cb(None);
                    }
                    cleanup_input(input);
                    return;
                }
            };

            let file = match files.get(0) {
                Some(f) => f,
                None => {
                    if let Some(cb) = callback_clone.borrow_mut().take() {
                        cb(None);
                    }
                    cleanup_input(input);
                    return;
                }
            };

            let filename = file.name();
            let reader = match FileReader::new() {
                Ok(r) => r,
                Err(_) => {
                    log::error!("Failed to create FileReader");
                    if let Some(cb) = callback_clone.borrow_mut().take() {
                        cb(None);
                    }
                    cleanup_input(input);
                    return;
                }
            };

            // Set up the load handler for the FileReader
            let callback_for_load = callback_clone.clone();
            let input_for_cleanup = input.clone();
            let filename_clone = filename.clone();
            let onload = Closure::wrap(Box::new(move |_event: web_sys::Event| {
                let reader: FileReader = match _event.target() {
                    Some(t) => match t.dyn_into::<FileReader>() {
                        Ok(r) => r,
                        Err(_) => {
                            if let Some(cb) = callback_for_load.borrow_mut().take() {
                                cb(None);
                            }
                            cleanup_input(&input_for_cleanup);
                            return;
                        }
                    },
                    None => {
                        if let Some(cb) = callback_for_load.borrow_mut().take() {
                            cb(None);
                        }
                        cleanup_input(&input_for_cleanup);
                        return;
                    }
                };

                let result = reader.result();
                match result {
                    Ok(value) => {
                        // Convert ArrayBuffer to Vec<u8>
                        let array_buffer = match value.dyn_into::<js_sys::ArrayBuffer>() {
                            Ok(ab) => ab,
                            Err(_) => {
                                if let Some(cb) = callback_for_load.borrow_mut().take() {
                                    cb(None);
                                }
                                cleanup_input(&input_for_cleanup);
                                return;
                            }
                        };
                        let uint8_array = js_sys::Uint8Array::new(&array_buffer);
                        let data = uint8_array.to_vec();

                        if let Some(cb) = callback_for_load.borrow_mut().take() {
                            cb(Some((filename_clone.clone(), data)));
                        }
                    }
                    Err(_) => {
                        log::error!("Failed to read file result");
                        if let Some(cb) = callback_for_load.borrow_mut().take() {
                            cb(None);
                        }
                    }
                }
                cleanup_input(&input_for_cleanup);
            }) as Box<dyn FnMut(web_sys::Event)>);

            reader.set_onload(Some(onload.as_ref().unchecked_ref()));
            onload.forget(); // Prevent closure from being dropped

            // Read the file as ArrayBuffer
            if let Err(e) = reader.read_as_array_buffer(&file) {
                log::error!("Failed to start reading file: {:?}", e);
                if let Some(cb) = callback_clone.borrow_mut().take() {
                    cb(None);
                }
                cleanup_input(input);
            }
        }) as Box<dyn FnMut(web_sys::Event)>);

        input.set_onchange(Some(onchange.as_ref().unchecked_ref()));
        onchange.forget(); // Prevent closure from being dropped

        // Trigger the file picker immediately (must be in user gesture context)
        input.click();
    }

    fn cleanup_input(input: &HtmlInputElement) {
        if let Some(parent) = input.parent_node() {
            let _ = parent.remove_child(input);
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm::spawn_file_picker;
