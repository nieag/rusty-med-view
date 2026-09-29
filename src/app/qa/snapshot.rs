use crate::app::components::*;
use crate::app::qa::sample::{
    non_empty_voxel_bounds, QA_PRESET_IMAGE_LABEL_MPR_BASIC, QA_SAMPLE_LIVER_0,
};
use crate::app::{qa, roi, roi_runtime, AppState};
use crate::render::roi_views::{RenderRepresentationRequest, RoiRenderViews};

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
                    authority: format!("{:?}", roi.primary_representation()),
                    visible: roi.metadata.is_visible,
                    active: Some(entity) == active_roi_entity,
                    overlay_slot: overlay_slots.get(&entity).copied(),
                    reference_geometry_dimensions: roi.reference_geometry().dimensions(),
                    reference_geometry_identity: format!(
                        "{:?}",
                        roi.reference_geometry().identity()
                    ),
                    voxel_cache_generation: roi.cache_generation(RoiCacheKind::Voxel),
                    contour_cache_generation: roi.cache_generation(RoiCacheKind::Contour),
                    mesh_cache_generation: roi.cache_generation(RoiCacheKind::Mesh),
                    voxel_cache_current: roi.is_cache_current(RoiCacheKind::Voxel),
                    contour_cache_current: roi.is_cache_current(RoiCacheKind::Contour),
                    mesh_cache_current: roi.is_cache_current(RoiCacheKind::Mesh),
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
                    last_completed_job: roi
                        .job_metrics
                        .last_completed_kind
                        .map(|kind| kind.as_str().to_string()),
                    discarded_job_count: roi.job_metrics.discarded_count,
                    failed_job_count: roi.job_metrics.failed_count,
                    last_job_duration_ms: roi.job_metrics.last_duration_ms,
                    last_queue_delay_ms: roi.job_metrics.last_queue_delay_ms,
                    last_contour_raster_ms: roi.job_metrics.last_contour_raster_ms,
                    last_mesh_voxelization_ms: roi.job_metrics.last_mesh_voxelization_ms,
                    last_cpu_cache_install_ms: roi.job_metrics.last_cpu_cache_install_ms,
                    last_gpu_upload_ms: roi.job_metrics.last_gpu_upload_ms,
                    last_work_convergence_ms: roi.job_metrics.last_work_convergence_ms,
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
                    // Axis-aligned bounds of the voxel centres, from the full affine so rotated,
                    // reflected, or sheared grids are bounded correctly.
                    let world_bounds = v.geometry.map(|geometry| {
                        let last = v.dimensions.map(|d| f64::from(d.saturating_sub(1)));
                        let mut min = [f32::INFINITY; 3];
                        let mut max = [f32::NEG_INFINITY; 3];
                        for corner in 0..8_u32 {
                            let ijk = std::array::from_fn(|axis| {
                                if corner >> axis & 1 == 1 {
                                    last[axis]
                                } else {
                                    0.0
                                }
                            });
                            let world = geometry.ijk_to_world_mm(ijk);
                            for axis in 0..3 {
                                min[axis] = min[axis].min(world[axis] as f32);
                                max[axis] = max[axis].max(world[axis] as f32);
                            }
                        }
                        [min, max]
                    });
                    qa::QaSnapshotVolume {
                        loaded: v.dimensions != [0, 0, 0],
                        dimensions: v.dimensions,
                        spacing: v.spacing(),
                        origin: v.geometry.map_or([0.0; 3], VoxelGeometry::origin),
                        orientation: v.orientation(),
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
                        .or_else(|| roi_runtime::main_volume_geometry(&ctx.scene.world))
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
                            roi_runtime::main_volume_geometry(&ctx.scene.world),
                        ) {
                            let axis = match vp.mode {
                                ViewMode::Axial => 2,
                                ViewMode::Coronal => 1,
                                ViewMode::Sagittal => 0,
                                ViewMode::ThreeD | ViewMode::Oblique => 2,
                            };
                            // Occupied bounds are inclusive voxel indices; use their outer cell
                            // faces so a cursor on the first or last occupied slice is inside.
                            let roi_min = crate::convert::voxel_index_to_volume_uv(
                                [
                                    bounds[0][0] as f32 - 0.5,
                                    bounds[0][1] as f32 - 0.5,
                                    bounds[0][2] as f32 - 0.5,
                                ],
                                dims,
                            );
                            let roi_max = crate::convert::voxel_index_to_volume_uv(
                                [
                                    bounds[1][0] as f32 + 0.5,
                                    bounds[1][1] as f32 + 0.5,
                                    bounds[1][2] as f32 + 0.5,
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
