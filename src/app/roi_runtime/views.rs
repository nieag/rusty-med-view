use super::*;

pub fn renderable_voxel_overlay_rois(
    world: &World,
    active_roi: Option<hecs::Entity>,
) -> Vec<RenderableVoxelOverlay> {
    let views = RoiRenderViews::for_world(
        world,
        RenderRepresentationRequest {
            active_roi,
            max_voxel_overlays: MAX_SIMULTANEOUS_ROI_OVERLAYS,
            contour_active_only: true,
        },
    );
    views
        .voxel_overlays
        .into_iter()
        .map(|overlay| RenderableVoxelOverlay {
            entity: overlay.entity,
            opacity: overlay.opacity,
        })
        .collect()
}

pub fn visible_voxel_overlay_count(world: &World) -> usize {
    renderable_voxel_overlay_rois(world, None).len()
}

pub(super) fn build_contour_view_data_for_plane(
    voxel_data: &VoxelData,
    view_key: &ContourViewKey,
) -> Result<Vec<ContourSlice>, VoxelContourExtractionError> {
    extract_contour_slice_from_voxel_data(voxel_data, view_key.plane)
}

pub(crate) fn ensure_contour_view_cache(
    world: &mut World,
    roi_entity: hecs::Entity,
    view_key: &ContourViewKey,
) -> RepresentationRequestStatus {
    let mesh_preview = crate::app::roi::preview::mesh_edit_preview_for_roi(world, roi_entity);
    let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) else {
        return RepresentationRequestStatus::blocked("roi_missing");
    };
    if let Some(contour) = roi.contour_data() {
        if contour.active_plane_family == view_key.family {
            return RepresentationRequestStatus::current();
        }
    }
    let mesh_preview_revision = roi.preview_state.revision;
    if matches!(&roi.body, RoiBody::Mesh(_)) {
        let mesh = mesh_preview
            .as_ref()
            .or_else(|| roi.mesh_data())
            .expect("mesh-authoritative ROI must expose mesh data");
        let state = if mesh_preview.is_some() {
            CacheViewState::Preview {
                revision: mesh_preview_revision,
            }
        } else {
            CacheViewState::Current
        };
        let built_from = roi.dirty_state.authoritative;
        if roi
            .contour_view_cache(view_key)
            .is_some_and(|cache| cache.built_from == built_from && cache.state == state)
        {
            return if mesh_preview.is_some() {
                RepresentationRequestStatus::preview("mesh_plane_intersection_preview")
            } else {
                RepresentationRequestStatus::current()
            };
        }
        return match intersect_mesh_with_plane(mesh, view_key.plane) {
            Ok(data) => {
                let install =
                    roi.install_contour_view_result(view_key.clone(), data, built_from, state);
                if install.is_err() {
                    RepresentationRequestStatus::stale("mesh_plane_intersection_superseded")
                } else if mesh_preview.is_some() {
                    RepresentationRequestStatus::preview("mesh_plane_intersection_preview")
                } else {
                    RepresentationRequestStatus::current()
                }
            }
            Err(_) => RepresentationRequestStatus::blocked("mesh_plane_intersection_failed"),
        };
    }
    let contour_preview_voxel = roi.session_caches.preview_voxel.as_ref().and_then(|cache| {
        (roi.preview_state.active
            && cache.source_generation == roi.dirty_state.authoritative.shape
            && cache.preview_revision == roi.preview_state.revision)
            .then(|| cache.data.clone())
    });
    if let Some(preview_voxel) = contour_preview_voxel {
        return match build_contour_view_data_for_plane(&preview_voxel, view_key) {
            Ok(data) => {
                let built_from = roi.dirty_state.authoritative;
                let revision = roi.preview_state.revision;
                let _ = roi.install_contour_view_result(
                    view_key.clone(),
                    data,
                    built_from,
                    CacheViewState::Preview { revision },
                );
                RepresentationRequestStatus::preview("contour_edit_cross_plane_preview")
            }
            Err(_) => {
                RepresentationRequestStatus::unsupported("contour_edit_preview_plane_unsupported")
            }
        };
    }
    if matches!(roi.body, RoiBody::Contour(_)) {
        return cut_contour_roi_view(&mut roi, view_key);
    }
    if roi.running_job_kind() == Some(RoiJobKind::RebuildVoxelCache) {
        if let Some(cache) = roi.contour_view_cache_mut(view_key) {
            if !matches!(
                cache.state,
                CacheViewState::Blocked { .. } | CacheViewState::Unsupported { .. }
            ) {
                cache.state = CacheViewState::Rebuilding;
            }
        }
        return RepresentationRequestStatus::rebuilding("voxel_cache_rebuilding_for_contour_view");
    }

    if roi.has_queued_job(RoiJobKind::RebuildVoxelCache) {
        if let Some(cache) = roi.contour_view_cache_mut(view_key) {
            if !matches!(
                cache.state,
                CacheViewState::Blocked { .. } | CacheViewState::Unsupported { .. }
            ) {
                cache.state = CacheViewState::Queued;
            }
        }
        return RepresentationRequestStatus::queued("voxel_cache_queued_for_contour_view");
    }

    if roi.voxel_cache().is_none() {
        return RepresentationRequestStatus::blocked("voxel_cache_missing_for_contour_view");
    }
    if roi.is_cache_dirty(RoiCacheKind::Voxel) || !roi.is_cache_current(RoiCacheKind::Voxel) {
        if let Some(cache) = roi.contour_view_cache_mut(view_key) {
            if !matches!(
                cache.state,
                CacheViewState::Blocked { .. } | CacheViewState::Unsupported { .. }
            ) {
                cache.state = CacheViewState::Stale;
            }
        }
        return RepresentationRequestStatus::stale("voxel_cache_stale_for_contour_view");
    }

    let built_from = roi.dirty_state.authoritative;
    if let Some(cache) = roi.contour_view_cache(view_key) {
        if cache.built_from == built_from && cache.state == CacheViewState::Current {
            return RepresentationRequestStatus::current();
        }
    }
    let voxel_data = roi
        .voxel_cache()
        .expect("voxel cache presence already checked")
        .data
        .clone();

    match build_contour_view_data_for_plane(&voxel_data, view_key) {
        Ok(data) => {
            match roi.install_current_contour_view_result(view_key.clone(), data, built_from) {
                Ok(()) => RepresentationRequestStatus::current(),
                Err(_) => RepresentationRequestStatus::stale("contour_view_result_superseded"),
            }
        }
        Err(VoxelContourExtractionError::UnsupportedPlaneGeometry) => {
            let built_from = roi.dirty_state.authoritative;
            let _ = roi.install_contour_view_result(
                view_key.clone(),
                Vec::new(),
                built_from,
                CacheViewState::Unsupported {
                    reason: "derived_contour_view_geometry_unsupported".to_string(),
                },
            );
            RepresentationRequestStatus::unsupported("derived_contour_view_geometry_unsupported")
        }
    }
}

/// The contours of a contour ROI in a plane of another family: its surface (the mesh of the
/// field of its loops) cut with that plane, so they are the true section and not a voxel
/// staircase. The surface is built for contour ROIs for exactly this; until it is current for the
/// ROI's revision, no view is drawn (nothing stale is ever drawn).
fn cut_contour_roi_view(roi: &mut Roi, view_key: &ContourViewKey) -> RepresentationRequestStatus {
    let built_from = roi.dirty_state.authoritative;
    if roi.contour_view_cache(view_key).is_some_and(|cache| {
        cache.built_from == built_from && cache.state == CacheViewState::Current
    }) {
        return RepresentationRequestStatus::current();
    }
    if !roi.is_cache_current(RoiCacheKind::Mesh) {
        if let Some(cache) = roi.contour_view_cache_mut(view_key) {
            if !matches!(
                cache.state,
                CacheViewState::Blocked { .. } | CacheViewState::Unsupported { .. }
            ) {
                cache.state = CacheViewState::Stale;
            }
        }
        return RepresentationRequestStatus::stale("mesh_cache_pending_for_contour_view");
    }
    let Some(mesh) = roi.mesh_cache().map(|cache| &cache.data) else {
        return RepresentationRequestStatus::blocked("mesh_cache_missing_for_contour_view");
    };
    match intersect_mesh_with_plane(mesh, view_key.plane) {
        Ok(data) => {
            match roi.install_current_contour_view_result(view_key.clone(), data, built_from) {
                Ok(()) => RepresentationRequestStatus::current(),
                Err(_) => RepresentationRequestStatus::stale("contour_view_result_superseded"),
            }
        }
        Err(_) => RepresentationRequestStatus::blocked("mesh_plane_intersection_failed"),
    }
}

pub(crate) fn sync_roi_contour_view_caches_for_viewports(world: &mut World, focus: &ViewFocus) {
    // Every visible ROI is shown in every view, so every visible ROI that does not carry its own
    // contours gets derived ones; the active ROI first. Hidden ROIs keep what they have.
    let roi_entities = demanded_roi_order(world, focus.active_roi, |_| true);
    let main_geometry = main_volume_geometry(world);

    let cursor_uv = focus.cursor_uv;

    for roi_entity in roi_entities {
        let geometry = main_geometry.or_else(|| {
            world
                .get::<&Roi>(roi_entity)
                .ok()
                .and_then(|roi| roi.voxel_cache().map(|cache| cache.data.geometry))
        });
        let Some(geometry) = geometry else {
            continue;
        };
        let viewport_requests: Vec<_> = world
            .query::<(&Viewport, &ViewportState)>()
            .iter()
            .filter_map(|(_, (viewport, viewport_state))| {
                crate::render::roi_views::displayed_plane_for_viewport(
                    viewport.mode,
                    cursor_uv,
                    viewport_state.user_rotation,
                    geometry,
                )
                .map(ContourViewKey::from_plane)
            })
            .collect();
        for view_key in viewport_requests {
            let _ = ensure_contour_view_cache(world, roi_entity, &view_key);
        }
    }
}

/// The visible ROIs that satisfy `wanted`, the active one first and the rest by ROI id.
fn demanded_roi_order(
    world: &World,
    active_roi: Option<hecs::Entity>,
    wanted: impl Fn(&Roi) -> bool,
) -> Vec<hecs::Entity> {
    let mut others: Vec<(u64, hecs::Entity)> = world
        .query::<(&Roi, &RoiMetadata)>()
        .iter()
        .filter(|(entity, (roi, metadata))| {
            Some(*entity) != active_roi && metadata.is_visible && wanted(roi)
        })
        .map(|(entity, (_, metadata))| (metadata.roi_id.0, entity))
        .collect();
    others.sort_unstable();
    active_roi
        .filter(|entity| {
            world
                .get::<&Roi>(*entity)
                .is_ok_and(|roi| is_roi_visible(world, *entity) && wanted(&roi))
        })
        .into_iter()
        .chain(others.into_iter().map(|(_, entity)| entity))
        .collect()
}

/// Demands a derived mesh for visible ROIs while a 3D view exists: the active ROI first, then
/// the others one at a time, so ten labels never queue ten builds at once.
pub(crate) fn sync_mesh_caches_for_viewports(world: &mut World, active_roi: Option<hecs::Entity>) {
    // Contour ROIs always need their surface (the contours of the other plane families are cut
    // from it); the other kinds only when a 3D view shows it.
    let has_three_d = world
        .query::<&Viewport>()
        .iter()
        .any(|(_, viewport)| viewport.mode == ViewMode::ThreeD);
    // A queued build that cannot start yet (its voxel cache is not current) must not block the
    // others, or one stuck ROI would starve every label.
    let busy = world.query::<&Roi>().iter().any(|(_, roi)| {
        roi.running_job_kind() == Some(RoiJobKind::RebuildMeshCache)
            || (roi.has_queued_job(RoiJobKind::RebuildMeshCache)
                && roi.is_cache_current(RoiCacheKind::Voxel))
    });
    if busy {
        return;
    }
    let candidates = demanded_roi_order(world, active_roi, |roi| {
        !matches!(roi.body, RoiBody::Mesh(_))
            && (has_three_d || matches!(roi.body, RoiBody::Contour(_)))
    });
    for entity in candidates {
        let Ok(mut roi) = world.get::<&mut Roi>(entity) else {
            continue;
        };
        if !roi.is_cache_current(RoiCacheKind::Mesh)
            && roi.voxel_cache().is_some()
            && roi.is_cache_current(RoiCacheKind::Voxel)
        {
            roi.mark_cache_dirty(RoiCacheKind::Mesh);
            roi.enqueue_rebuild(RoiJobKind::RebuildMeshCache);
            return;
        }
    }
}

/// After an edit of the active mesh ROI, rebuilds its voxels while the user is idle, so that a
/// switch to a contour tool finds them ready instead of starting a rebuild then. It only acts on
/// a mesh that edit validation already passed (checking an unvalidated mesh would itself stall
/// the frame) and that has a voxel grid to rebuild onto, when no edit preview is running and no
/// rebuild is queued or running. It tries each revision once: a failure is reported by the job and
/// not retried every frame.
pub(crate) fn sync_speculative_voxel_cache(world: &mut World, active_roi: Option<hecs::Entity>) {
    let Some(entity) = active_roi else {
        return;
    };
    let wanted = world.get::<&Roi>(entity).is_ok_and(|roi| {
        matches!(roi.body, RoiBody::Mesh(_))
            && roi.job_state.speculative_voxel_shape != Some(roi.dirty_state.authoritative.shape)
            && !roi.preview_state.active
            && roi.validated_mesh_generation == Some(roi.dirty_state.authoritative.shape)
            && !roi.is_cache_current(RoiCacheKind::Voxel)
            && !roi.has_queued_job(RoiJobKind::RebuildVoxelCache)
            && roi.running_job_kind().is_none()
            && roi.job_state.pending_switch.is_none()
    });
    if wanted {
        if let Ok(mut roi) = world.get::<&mut Roi>(entity) {
            roi.job_state.speculative_voxel_shape = Some(roi.dirty_state.authoritative.shape);
        }
        let _ = crate::app::roi::request_mesh_voxel_cache_rebuild(world, entity);
    }
}
