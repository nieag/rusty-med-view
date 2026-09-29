use crate::app::components::*;
#[cfg(target_arch = "wasm32")]
use crate::app::events::AppEvent;
use crate::app::roi_runtime;
#[cfg(target_arch = "wasm32")]
use crate::io::nifti::{load_label_from_bytes, load_nifti_from_bytes};
use crate::render::protocols;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::JsFuture;
#[cfg(target_arch = "wasm32")]
use winit::event_loop::EventLoopProxy;

pub(crate) const QA_SAMPLE_LIVER_0: &str = "liver_0";
pub(crate) const QA_PRESET_IMAGE_LABEL_MPR_BASIC: &str = "image_label_mpr_basic";

pub(crate) fn non_empty_voxel_bounds(raw: &[u8], dims: [u32; 3]) -> Option<[[u32; 3]; 2]> {
    if dims.contains(&0) {
        return None;
    }
    let [dx, dy, dz] = dims;
    let mut min = [u32::MAX; 3];
    let mut max = [0_u32; 3];
    let mut found = false;
    let stride_y = dx as usize;
    let stride_z = (dx as usize).saturating_mul(dy as usize);
    for z in 0..dz {
        for y in 0..dy {
            for x in 0..dx {
                let idx = (z as usize)
                    .saturating_mul(stride_z)
                    .saturating_add((y as usize).saturating_mul(stride_y))
                    .saturating_add(x as usize);
                if raw.get(idx).copied().unwrap_or(0) == 0 {
                    continue;
                }
                found = true;
                min[0] = min[0].min(x);
                min[1] = min[1].min(y);
                min[2] = min[2].min(z);
                max[0] = max[0].max(x);
                max[1] = max[1].max(y);
                max[2] = max[2].max(z);
            }
        }
    }
    found.then_some([min, max])
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn spawn_qa_fetch_volume(proxy: EventLoopProxy<AppEvent>) {
    wasm_bindgen_futures::spawn_local(async move {
        let result = fetch_bytes("/qa_samples/liver_0.nii")
            .await
            .and_then(|bytes| {
                load_nifti_from_bytes(&bytes)
                    .map(LoadResult::Volume)
                    .map_err(|e| e.to_string())
            });
        let event = result.map_err(crate::io::nifti::LoadError::DimensionError);
        let _ = proxy.send_event(AppEvent::VolumeLoaded(event));
    });
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn spawn_qa_fetch_label(proxy: EventLoopProxy<AppEvent>) {
    wasm_bindgen_futures::spawn_local(async move {
        let result = fetch_bytes("/qa_samples/liver_0_label.nii")
            .await
            .and_then(|bytes| {
                load_label_from_bytes(&bytes, "liver_0_label.nii".to_string())
                    .map(LoadResult::Label)
                    .map_err(|e| e.to_string())
            });
        let event = result.map_err(crate::io::nifti::LoadError::DimensionError);
        let _ = proxy.send_event(AppEvent::VolumeLoaded(event));
    });
}

#[cfg(target_arch = "wasm32")]
pub(crate) async fn fetch_bytes(path: &str) -> Result<Vec<u8>, String> {
    let window = web_sys::window().ok_or_else(|| "window missing".to_string())?;
    let response_js = JsFuture::from(window.fetch_with_str(path))
        .await
        .map_err(|err| format!("{err:?}"))?;
    let ok = js_sys::Reflect::get(&response_js, &"ok".into())
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !ok {
        let status = js_sys::Reflect::get(&response_js, &"status".into())
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(-1.0);
        return Err(format!("http {status} for {path}"));
    }
    let array_buffer_fn = js_sys::Reflect::get(&response_js, &"arrayBuffer".into())
        .map_err(|_| format!("arrayBuffer missing for {path}"))?;
    let array_buffer_promise = js_sys::Function::from(array_buffer_fn)
        .call0(&response_js)
        .map_err(|_| format!("arrayBuffer call failed for {path}"))?;
    let array_buffer = JsFuture::from(js_sys::Promise::from(array_buffer_promise))
        .await
        .map_err(|err| format!("{err:?}"))?;
    let bytes = js_sys::Uint8Array::new(&array_buffer).to_vec();
    Ok(bytes)
}

pub(crate) fn apply_image_label_mpr_basic_preset(
    world: &mut hecs::World,
    entities: &AppEntities,
    active_roi: hecs::Entity,
) -> bool {
    protocols::apply_protocol(world, entities, "ROI MPR + Oblique");
    if let Ok(mut editor) = world.get::<&mut EditorState>(entities.editor) {
        editor.active_roi = Some(active_roi);
    }
    if let Ok(mut roi) = world.get::<&mut Roi>(active_roi) {
        roi.metadata.is_visible = true;
        if let Some(cache) = roi.voxel_cache() {
            if let Some(bounds) =
                non_empty_voxel_bounds(&cache.data.raw_data, cache.data.geometry.dimensions)
            {
                let Some(main_geometry) = roi_runtime::main_volume_geometry(world) else {
                    return false;
                };
                // Round to a voxel centre so the preset's slice planes do not sit on a layer boundary.
                let center = [
                    ((bounds[0][0] + bounds[1][0]) as f32 * 0.5).round(),
                    ((bounds[0][1] + bounds[1][1]) as f32 * 0.5).round(),
                    ((bounds[0][2] + bounds[1][2]) as f32 * 0.5).round(),
                ];
                let roi_uv = crate::convert::voxel_index_to_volume_uv(
                    center,
                    cache.data.geometry.dimensions,
                );
                let world_mm = crate::convert::volume_uv_to_world_mm(roi_uv, cache.data.geometry);
                let uv = crate::convert::world_mm_to_volume_uv(world_mm, main_geometry);
                if let Ok(mut cursor) = world.get::<&mut Transform>(entities.cursor) {
                    cursor.position = uv;
                }
                return true;
            }
        }
    }
    false
}
