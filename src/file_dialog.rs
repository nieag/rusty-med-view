// src/file_dialog.rs
//! Cross-platform file dialog module supporting both native and WASM targets.
//!
//! Uses rfd's AsyncFileDialog which works on both platforms.

use rfd::AsyncFileDialog;

/// Pick a NIfTI file and return its contents as bytes.
/// Returns None if the user cancels the dialog.
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

/// Spawn an async file loading task.
/// This works on both native (using pollster) and WASM (using wasm_bindgen_futures).
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn_file_picker<F>(callback: F)
where
    F: FnOnce(Option<(String, Vec<u8>)>) + Send + 'static,
{
    std::thread::spawn(move || {
        let result = pollster::block_on(pick_nifti_file());
        callback(result);
    });
}

#[cfg(target_arch = "wasm32")]
pub fn spawn_file_picker<F>(callback: F)
where
    F: FnOnce(Option<(String, Vec<u8>)>) + 'static,
{
    wasm_bindgen_futures::spawn_local(async move {
        let result = pick_nifti_file().await;
        callback(result);
    });
}
