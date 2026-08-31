pub mod components;
pub mod context;
pub mod events;
pub mod qa;
pub mod roi;
pub mod roi_runtime;

use crate::app::components::*;
use crate::app::context::RenderingContext;
use crate::app::events::AppEvent;
use crate::io::handlers;
#[cfg(target_arch = "wasm32")]
use crate::io::nifti::{load_label_from_bytes, load_nifti_from_bytes};
use crate::render::pipeline;
use crate::render::protocols;
use crate::render::roi_views::{RenderRepresentationRequest, RoiRenderViews};
use crate::systems;
use std::collections::BTreeMap;
use std::sync::Arc;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::JsFuture;
use winit::{
    application::ApplicationHandler,
    event::*,
    event_loop::{ActiveEventLoop, EventLoopProxy},
    window::WindowAttributes,
};

/// Pixel-to-line scroll normalization factor for trackpad deltas.
const PIXEL_SCROLL_FACTOR: f64 = 0.05;
const QA_SAMPLE_LIVER_0: &str = "liver_0";
const QA_PRESET_IMAGE_LABEL_MPR_BASIC: &str = "image_label_mpr_basic";

fn non_empty_voxel_bounds(raw: &[u8], dims: [u32; 3]) -> Option<[[u32; 3]; 2]> {
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
fn spawn_qa_fetch_volume(proxy: EventLoopProxy<AppEvent>) {
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
fn spawn_qa_fetch_label(proxy: EventLoopProxy<AppEvent>) {
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
async fn fetch_bytes(path: &str) -> Result<Vec<u8>, String> {
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

fn apply_image_label_mpr_basic_preset(
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
                let Some(main_geometry) = roi_runtime::main_volume_voxel_geometry(world) else {
                    return false;
                };
                let center = [
                    (bounds[0][0] + bounds[1][0]) as f32 * 0.5,
                    (bounds[0][1] + bounds[1][1]) as f32 * 0.5,
                    (bounds[0][2] + bounds[1][2]) as f32 * 0.5,
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
                            EditorTool::MeshDeform => "mesh_deform".to_string(),
                        }),
                    )
                })
                .unwrap_or((None, None));

            let mut active_roi_id = None;
            let mut active_roi_name = None;
            let roi_views = RoiRenderViews::for_world(
                &ctx.scene.world,
                RenderRepresentationRequest {
                    active_roi: active_roi_entity,
                    max_voxel_overlays: roi_runtime::MAX_SIMULTANEOUS_ROI_OVERLAYS,
                    contour_active_only: false,
                },
            );
            let mut overlay_slots: std::collections::HashMap<hecs::Entity, u32> =
                std::collections::HashMap::new();
            for (idx, overlay) in roi_views.voxel_overlays.iter().enumerate() {
                overlay_slots.insert(overlay.entity, idx as u32);
            }

            let mut rois = Vec::new();
            let mut active_roi_non_empty_bounds = None;
            let mut active_roi_dims = None;
            let mut active_roi_geometry = None;
            let mut active_roi_overlay_slot = None;
            let mut active_roi_visible = false;
            let mut active_roi_has_renderable_cache = false;
            for (entity, roi) in ctx.scene.world.query::<&Roi>().iter() {
                if Some(entity) == active_roi_entity {
                    active_roi_id = Some(roi.metadata.roi_id.0);
                    active_roi_name = Some(roi.metadata.name.clone());
                    active_roi_visible = roi.metadata.is_visible;
                    active_roi_has_renderable_cache = overlay_slots.contains_key(&entity);
                }
                let voxel_dimensions = roi
                    .voxel_cache()
                    .map(|cache| cache.data.geometry.dimensions);
                let non_empty_bounds = roi.voxel_cache().and_then(|cache| {
                    non_empty_voxel_bounds(&cache.data.raw_data, cache.data.geometry.dimensions)
                });
                if Some(entity) == active_roi_entity {
                    active_roi_non_empty_bounds = non_empty_bounds;
                    active_roi_dims = voxel_dimensions;
                    active_roi_geometry = roi.voxel_cache().map(|cache| cache.data.geometry);
                    active_roi_overlay_slot = overlay_slots.get(&entity).copied();
                }
                rois.push(qa::QaSnapshotRoi {
                    id: roi.metadata.roi_id.0,
                    name: roi.metadata.name.clone(),
                    visible: roi.metadata.is_visible,
                    active: Some(entity) == active_roi_entity,
                    overlay_slot: overlay_slots.get(&entity).copied(),
                    reference_geometry_dimensions: roi
                        .reference_geometry()
                        .map(|geometry| geometry.dimensions()),
                    reference_geometry_identity: roi
                        .reference_geometry()
                        .map(|geometry| format!("{:?}", geometry.identity())),
                    voxel_dimensions,
                    non_empty_voxel_bounds: non_empty_bounds,
                    preview_active: roi.preview_state.active,
                    preview_revision: roi.preview_state.revision,
                    running_job: roi.running_job_kind().map(|kind| kind.as_str().to_string()),
                    pending_jobs: roi
                        .job_state
                        .pending
                        .iter()
                        .map(|request| request.kind.as_str().to_string())
                        .collect(),
                    completed_job_count: roi.job_metrics.completed_count,
                    discarded_job_count: roi.job_metrics.discarded_count,
                    failed_job_count: roi.job_metrics.failed_count,
                    last_job_duration_ms: roi.job_metrics.last_duration_ms,
                    max_job_queue_depth: roi.job_metrics.max_queue_depth,
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
            let mut has_axial = false;
            let mut has_coronal = false;
            let mut has_sagittal = false;
            let mut has_three_d = false;
            let mut all_required_rects_non_zero = true;
            let cursor_pos = ctx
                .scene
                .world
                .get::<&Transform>(ctx.scene.entities.cursor)
                .map(|cursor| cursor.position)
                .unwrap_or([0.5, 0.5, 0.5]);
            for (_, (vp, vp_state)) in ctx
                .scene
                .world
                .query::<(&Viewport, &ViewportState)>()
                .iter()
            {
                let mode = match vp.mode {
                    ViewMode::ThreeD => "three_d",
                    ViewMode::Axial => "axial",
                    ViewMode::Coronal => "coronal",
                    ViewMode::Sagittal => "sagittal",
                    ViewMode::Oblique => "oblique",
                };
                let valid_rect = vp.rect[2] > 0.0 && vp.rect[3] > 0.0;
                let mut blockers = Vec::new();
                let mut overlay_blockers = Vec::new();
                let mut contour_blockers = Vec::new();
                let mut mesh_blockers = Vec::new();
                if !valid_rect {
                    blockers.push("rect_non_positive".to_string());
                }
                let is_required = matches!(
                    vp.mode,
                    ViewMode::Axial | ViewMode::Coronal | ViewMode::Sagittal | ViewMode::ThreeD
                );
                if is_required && !valid_rect {
                    all_required_rects_non_zero = false;
                }
                let mut overlay_renderable = false;
                let image_renderable = valid_rect && volume.loaded;
                if !volume.loaded {
                    blockers.push("main_volume_not_loaded".to_string());
                }
                let mut contour_renderable = false;
                let mut mesh_renderable = false;
                let mut representation_requests = Vec::new();
                let mut voxel_cache_state = "unsupported".to_string();
                let mut contour_view_cache_state = "unsupported".to_string();
                let mut mesh_cache_state = "unsupported".to_string();
                let mut contour_editable = false;
                let mut contour_promotable = false;
                let mut volume_slice_in_bounds = None;
                let mut cursor_intersects_active_roi = None;
                if let Some(active) = active_roi_entity {
                    let voxel_req = roi::request_viewport_voxel_overlay_state(
                        &ctx.scene.world,
                        vp.mode,
                        active,
                    );
                    voxel_cache_state = voxel_req.state.as_str().to_string();
                    if let Some(reason) = voxel_req.reason {
                        overlay_blockers.push(reason);
                    }
                    representation_requests.push("voxel_overlay".to_string());

                    let mesh_req =
                        roi::request_viewport_mesh_state(&ctx.scene.world, vp.mode, active);
                    mesh_cache_state = mesh_req.state.as_str().to_string();
                    if let Some(reason) = mesh_req.reason {
                        mesh_blockers.push(reason);
                    }
                    representation_requests.push("mesh_3d".to_string());

                    if let Some(geometry) = active_roi_geometry
                        .or_else(|| roi_runtime::main_volume_voxel_geometry(&ctx.scene.world))
                    {
                        let displayed_plane =
                            crate::render::roi_views::displayed_plane_for_viewport(
                                vp.mode,
                                cursor_pos,
                                vp_state.user_rotation,
                                geometry,
                            );
                        if let Some(plane) = displayed_plane {
                            let contour_key = ContourViewKey::from_plane(plane);
                            let contour_req = roi::request_contour_view_state(
                                &ctx.scene.world,
                                active,
                                &contour_key,
                            );
                            contour_view_cache_state =
                                contour_req.request.state.as_str().to_string();
                            contour_editable = contour_req.editable;
                            contour_promotable = contour_req.promotable;
                            if let Some(reason) = contour_req.request.reason {
                                contour_blockers.push(reason);
                            }
                            representation_requests.push("contour_view".to_string());
                        }
                    }
                }
                match vp.mode {
                    ViewMode::Axial | ViewMode::Coronal | ViewMode::Sagittal => {
                        mesh_blockers.push("mesh_not_applicable_in_2d_view".to_string());
                        if !active_roi_visible {
                            overlay_blockers.push("active_roi_not_visible".to_string());
                            contour_blockers.push("active_roi_not_visible".to_string());
                        }
                        if active_roi_overlay_slot.is_none() {
                            overlay_blockers.push("active_roi_overlay_slot_missing".to_string());
                            if let Some(active) = active_roi_entity {
                                if let Some(skip) = roi_views
                                    .voxel_skips
                                    .iter()
                                    .find(|skip| skip.entity == active)
                                {
                                    overlay_blockers.push(skip.reason.to_string());
                                }
                            }
                        }
                        if active_roi_non_empty_bounds.is_none() {
                            overlay_blockers
                                .push("active_roi_non_empty_voxel_bounds_missing".to_string());
                        }
                        if let (Some(bounds), Some(dims), Some(roi_geometry), Some(main_geometry)) = (
                            active_roi_non_empty_bounds,
                            active_roi_dims,
                            active_roi_geometry,
                            roi_runtime::main_volume_voxel_geometry(&ctx.scene.world),
                        ) {
                            let axis = match vp.mode {
                                ViewMode::Axial => 2,
                                ViewMode::Coronal => 1,
                                ViewMode::Sagittal => 0,
                                ViewMode::ThreeD | ViewMode::Oblique => 2,
                            };
                            let roi_min = crate::convert::voxel_index_to_volume_uv(
                                [
                                    bounds[0][0] as f32,
                                    bounds[0][1] as f32,
                                    bounds[0][2] as f32,
                                ],
                                dims,
                            );
                            let roi_max = crate::convert::voxel_index_to_volume_uv(
                                [
                                    bounds[1][0] as f32,
                                    bounds[1][1] as f32,
                                    bounds[1][2] as f32,
                                ],
                                dims,
                            );
                            let cursor_world =
                                crate::convert::volume_uv_to_world_mm(cursor_pos, main_geometry);
                            let cursor_roi_uv =
                                crate::convert::world_mm_to_volume_uv(cursor_world, roi_geometry);
                            let min_uv = roi_min[axis].min(roi_max[axis]);
                            let max_uv = roi_min[axis].max(roi_max[axis]);
                            let uv = cursor_roi_uv[axis];
                            let intersects = uv >= min_uv && uv <= max_uv;
                            cursor_intersects_active_roi = Some(intersects);
                            if !intersects {
                                overlay_blockers
                                    .push("slice_does_not_intersect_active_roi_bounds".to_string());
                            }
                            volume_slice_in_bounds = Some((0.0..=1.0).contains(&cursor_pos[axis]));
                            if volume_slice_in_bounds == Some(false) {
                                overlay_blockers
                                    .push("cursor_slice_out_of_volume_bounds".to_string());
                            }
                        }
                        overlay_renderable =
                            overlay_blockers.is_empty() && active_roi_has_renderable_cache;
                        if !active_roi_has_renderable_cache {
                            overlay_blockers
                                .push("active_roi_renderable_voxel_cache_missing".to_string());
                        }
                        contour_renderable = active_roi_entity
                            .map(|active| {
                                crate::render::roi_views::contour_renderable_in_viewport(
                                    &ctx.scene.world,
                                    vp,
                                    vp_state,
                                    cursor_pos,
                                    active,
                                )
                            })
                            .unwrap_or(false);
                        if !contour_renderable {
                            contour_blockers
                                .push("active_contour_data_missing_or_empty".to_string());
                            if let Some(active) = active_roi_entity {
                                if let Some(skip) = roi_views
                                    .contour_skips
                                    .iter()
                                    .find(|skip| skip.entity == active)
                                {
                                    contour_blockers.push(skip.reason.to_string());
                                }
                            }
                        }
                        contour_renderable = contour_renderable && image_renderable;
                    }
                    ViewMode::ThreeD => {
                        overlay_blockers.push("overlay_not_applicable_in_three_d_view".to_string());
                        contour_blockers.push("contour_not_applicable_in_three_d_view".to_string());
                        mesh_renderable = !roi_views.mesh_overlays.is_empty() && image_renderable;
                        if !mesh_renderable {
                            mesh_blockers.push("visible_mesh_roi_missing".to_string());
                            if let Some(skip) = roi_views.mesh_skips.first() {
                                mesh_blockers.push(skip.reason.to_string());
                            }
                        }
                    }
                    ViewMode::Oblique => {
                        mesh_blockers.push("mesh_not_applicable_in_2d_view".to_string());
                        if !active_roi_visible {
                            overlay_blockers.push("active_roi_not_visible".to_string());
                            contour_blockers.push("active_roi_not_visible".to_string());
                        }
                        if active_roi_overlay_slot.is_none() {
                            overlay_blockers.push("active_roi_overlay_slot_missing".to_string());
                            if let Some(active) = active_roi_entity {
                                if let Some(skip) = roi_views
                                    .voxel_skips
                                    .iter()
                                    .find(|skip| skip.entity == active)
                                {
                                    overlay_blockers.push(skip.reason.to_string());
                                }
                            }
                        }
                        if !active_roi_has_renderable_cache {
                            overlay_blockers
                                .push("active_roi_renderable_voxel_cache_missing".to_string());
                        }
                        overlay_renderable = overlay_blockers.is_empty()
                            && active_roi_has_renderable_cache
                            && image_renderable;
                        contour_renderable = active_roi_entity
                            .map(|active| {
                                crate::render::roi_views::contour_renderable_in_viewport(
                                    &ctx.scene.world,
                                    vp,
                                    vp_state,
                                    cursor_pos,
                                    active,
                                )
                            })
                            .unwrap_or(false);
                        if !contour_renderable && contour_blockers.is_empty() {
                            contour_blockers
                                .push("active_contour_data_missing_or_empty".to_string());
                        }
                        contour_renderable = contour_renderable && image_renderable;
                    }
                }
                blockers.extend(overlay_blockers);
                blockers.extend(contour_blockers);
                blockers.extend(mesh_blockers);
                match vp.mode {
                    ViewMode::Axial => has_axial = true,
                    ViewMode::Coronal => has_coronal = true,
                    ViewMode::Sagittal => has_sagittal = true,
                    ViewMode::ThreeD => has_three_d = true,
                    ViewMode::Oblique => {}
                }
                viewports.push(qa::QaSnapshotViewport {
                    mode: mode.to_string(),
                    rect: vp.rect,
                    ready: valid_rect,
                    representation_requests,
                    voxel_cache_state: voxel_cache_state.clone(),
                    contour_view_cache_state: contour_view_cache_state.clone(),
                    mesh_cache_state: mesh_cache_state.clone(),
                    contour_editable,
                    contour_promotable,
                    stale: [
                        voxel_cache_state.as_str(),
                        contour_view_cache_state.as_str(),
                        mesh_cache_state.as_str(),
                    ]
                    .iter()
                    .any(|state| matches!(*state, "preview" | "stale" | "queued" | "rebuilding")),
                    image_renderable,
                    overlay_renderable,
                    contour_renderable,
                    mesh_renderable,
                    volume_slice_in_bounds,
                    cursor_intersects_active_roi,
                    render_blockers: blockers.clone(),
                    readiness_blockers: blockers,
                });
            }

            let is_qa2_request = self.qa.requested_sample.as_deref() == Some(QA_SAMPLE_LIVER_0)
                && self.qa.requested_preset.as_deref() == Some(QA_PRESET_IMAGE_LABEL_MPR_BASIC);
            let mut readiness_blockers = Vec::new();
            if !self.qa.enabled {
                readiness_blockers.push("qa_disabled".to_string());
            }
            if is_qa2_request {
                if self.qa.sample_phase != qa::QaSamplePhase::Loaded {
                    readiness_blockers.push("sample_not_loaded".to_string());
                }
                if self.qa.preset_phase != qa::QaPresetPhase::Applied {
                    readiness_blockers.push("preset_not_applied".to_string());
                }
                if !volume.loaded {
                    readiness_blockers.push("main_volume_not_loaded".to_string());
                }
                if active_roi_id.is_none() {
                    readiness_blockers.push("active_roi_missing".to_string());
                }
                if !active_roi_visible {
                    readiness_blockers.push("active_roi_not_visible".to_string());
                }
                if active_roi_non_empty_bounds.is_none() {
                    readiness_blockers
                        .push("active_roi_non_empty_voxel_bounds_missing".to_string());
                }
                if !active_roi_has_renderable_cache {
                    readiness_blockers
                        .push("active_roi_renderable_voxel_cache_missing".to_string());
                }
                if !has_axial {
                    readiness_blockers.push("viewport_axial_missing".to_string());
                }
                if !has_coronal {
                    readiness_blockers.push("viewport_coronal_missing".to_string());
                }
                if !has_sagittal {
                    readiness_blockers.push("viewport_sagittal_missing".to_string());
                }
                if !has_three_d {
                    readiness_blockers.push("viewport_three_d_missing".to_string());
                }
                if !all_required_rects_non_zero {
                    readiness_blockers.push("required_viewport_rect_non_positive".to_string());
                }
                for vp in &viewports {
                    if matches!(vp.mode.as_str(), "axial" | "coronal" | "sagittal")
                        && !vp.overlay_renderable
                    {
                        readiness_blockers.push(format!("{}_overlay_not_renderable", vp.mode));
                    }
                    if vp.mode == "three_d" && !vp.ready {
                        readiness_blockers.push("three_d_not_ready".to_string());
                    }
                }
                if let Some(preset_frame) = self.qa.preset_applied_frame {
                    match self.qa.last_presented_frame {
                        Some(last) if last > preset_frame => {}
                        _ => readiness_blockers.push("no_presented_frame_after_preset".to_string()),
                    }
                } else {
                    readiness_blockers.push("preset_applied_frame_missing".to_string());
                }
            }
            if self.qa.last_error.is_some() {
                readiness_blockers.push("last_error_present".to_string());
            }

            let qa_info = qa::QaSnapshotQa {
                enabled: self.qa.enabled,
                version: qa::QA_API_VERSION,
                requested_sample: self.qa.requested_sample.clone(),
                requested_preset: self.qa.requested_preset.clone(),
                sample_phase: self.qa.sample_phase,
                preset_phase: self.qa.preset_phase,
                readiness_blockers: readiness_blockers.clone(),
                preset_applied_frame: self.qa.preset_applied_frame,
                last_presented_frame: self.qa.last_presented_frame,
                active_qa_roi: self.qa.active_qa_roi.clone(),
                ready: readiness_blockers.is_empty(),
                last_error: self.qa.last_error.clone(),
            };

            let app = qa::QaSnapshotApp {
                status,
                active_tool,
                active_roi_id,
                active_roi_name,
                frame_counter: self.qa.frame_counter,
            };

            let render = qa::QaSnapshotRender {
                frame_counter: self.qa.frame_counter,
                last_presented_frame: self.qa.last_presented_frame,
                viewport_uniform_count: self.qa.viewport_uniform_count,
                overlay_slots_used: roi_views.overlay_cap.selected_count as u32,
                overlay_slots_max: roi_views.overlay_cap.max_count as u32,
                contour_batch_count: self.qa.contour_batch_count,
                mesh_batch_count: self.qa.mesh_batch_count,
                mesh_chunks_uploaded: self.qa.mesh_chunks_uploaded,
                mesh_chunks_reused: self.qa.mesh_chunks_reused,
                last_warning: self.qa.last_render_warning.clone(),
                last_error: self.qa.last_render_error.clone(),
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
            let mut readiness_blockers = Vec::new();
            let is_qa2_request = self.qa.requested_sample.as_deref() == Some(QA_SAMPLE_LIVER_0)
                && self.qa.requested_preset.as_deref() == Some(QA_PRESET_IMAGE_LABEL_MPR_BASIC);
            if self.qa.enabled && is_qa2_request {
                readiness_blockers.push("app_context_not_ready".to_string());
            }
            if self.qa.last_error.is_some() {
                readiness_blockers.push("last_error_present".to_string());
            }
            let qa_info = qa::QaSnapshotQa {
                enabled: self.qa.enabled,
                version: qa::QA_API_VERSION,
                requested_sample: self.qa.requested_sample.clone(),
                requested_preset: self.qa.requested_preset.clone(),
                sample_phase: self.qa.sample_phase,
                preset_phase: self.qa.preset_phase,
                readiness_blockers: readiness_blockers.clone(),
                preset_applied_frame: self.qa.preset_applied_frame,
                last_presented_frame: self.qa.last_presented_frame,
                active_qa_roi: self.qa.active_qa_roi.clone(),
                ready: readiness_blockers.is_empty(),
                last_error: self.qa.last_error.clone(),
            };
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
                    frame_counter: self.qa.frame_counter,
                    last_presented_frame: self.qa.last_presented_frame,
                    viewport_uniform_count: self.qa.viewport_uniform_count,
                    overlay_slots_used: 0,
                    overlay_slots_max: roi_runtime::MAX_SIMULTANEOUS_ROI_OVERLAYS as u32,
                    contour_batch_count: self.qa.contour_batch_count,
                    mesh_batch_count: self.qa.mesh_batch_count,
                    mesh_chunks_uploaded: self.qa.mesh_chunks_uploaded,
                    mesh_chunks_reused: self.qa.mesh_chunks_reused,
                    last_warning: self.qa.last_render_warning.clone(),
                    last_error: self.qa.last_render_error.clone(),
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
        let overlay_slots_used = self.context.as_ref().map_or(0, |ctx| {
            RoiRenderViews::for_world(&ctx.scene.world, RenderRepresentationRequest::default())
                .overlay_cap
                .selected_count as u32
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
            overlay_slots_used,
            overlay_slots_max: roi_runtime::MAX_SIMULTANEOUS_ROI_OVERLAYS as u32,
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
                &ctx.scene.entities,
                position.x,
                position.y,
            );
            systems::sys_handle_mouse_drag(&mut ctx.scene.world, &ctx.scene.entities);
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
                if let Some(category) = frame_stats.last_warning {
                    qa_runtime.last_render_warning = Some(category.to_string());
                }
                if let Some(category) = frame_stats.last_error {
                    qa_runtime.last_render_error = Some(category.to_string());
                }

                if repaint_after.is_zero() || frame_stats.roi_work_pending {
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
                                let active_roi = ctx
                                    .scene
                                    .world
                                    .get::<&EditorState>(ctx.scene.entities.editor)
                                    .ok()
                                    .and_then(|editor| editor.active_roi);
                                if let Some(active_roi) = active_roi {
                                    let ok = apply_image_label_mpr_basic_preset(
                                        &mut ctx.scene.world,
                                        &ctx.scene.entities,
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
