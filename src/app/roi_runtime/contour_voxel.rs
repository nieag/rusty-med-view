use super::*;

pub(super) struct ContourPreviewMeshWork {
    source_generation: u64,
    preview_revision: u64,
    preview_aabb: Option<([u32; 3], [u32; 3])>,
    voxel_data: VoxelData,
    rebuild: IncrementalChunkedMeshRebuild,
    started_at: Instant,
}

pub(crate) fn process_contour_voxel_rebuild_jobs(world: &mut World) {
    let _ = process_contour_voxel_rebuild_jobs_with_hook(world, |_world, _entity| {}, None, None);
}

pub(crate) fn process_contour_voxel_rebuild_jobs_with_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    world: &mut World,
    resources: &BindGroupResources<'_>,
    active_roi: Option<hecs::Entity>,
) {
    let rebuilt_any = process_contour_voxel_rebuild_jobs_with_hook(
        world,
        |_world, _entity| {},
        Some((device, queue)),
        active_roi,
    );
    if rebuilt_any {
        recreate_scene_bind_groups(device, world, resources, active_roi);
    }
}

pub(super) fn process_contour_voxel_rebuild_jobs_with_hook(
    world: &mut World,
    mut before_commit: impl FnMut(&mut World, hecs::Entity),
    upload_context: Option<(&wgpu::Device, &wgpu::Queue)>,
    preferred_roi: Option<hecs::Entity>,
) -> bool {
    const MAX_JOBS_PER_FRAME: usize = 1;
    const FRAME_JOB_BUDGET: Duration = Duration::from_millis(4);
    let frame_started_at = Instant::now();
    let mut rebuilt_any = false;

    let mut running_preview_entities = world
        .query::<&ContourPreviewMeshWork>()
        .iter()
        .map(|(entity, _)| entity)
        .collect::<Vec<_>>();
    prioritize_entity(&mut running_preview_entities, preferred_roi);
    if let Some(entity) = running_preview_entities.first().copied() {
        rebuilt_any |=
            resume_contour_preview_mesh_work(world, entity, frame_started_at, FRAME_JOB_BUDGET);
    }

    if frame_started_at.elapsed() >= FRAME_JOB_BUDGET {
        return rebuilt_any;
    }

    let mut rebuild_entities = Vec::new();
    for (entity, roi) in world.query::<&Roi>().iter() {
        if !matches!(roi.body, RoiBody::Contour(_)) {
            continue;
        }
        if roi.running_job_kind().is_none() && roi.has_queued_job(RoiJobKind::RebuildVoxelCache) {
            rebuild_entities.push(entity);
        }
    }

    prioritize_entity(&mut rebuild_entities, preferred_roi);

    for entity in rebuild_entities.into_iter().take(MAX_JOBS_PER_FRAME) {
        if frame_started_at.elapsed() >= FRAME_JOB_BUDGET {
            break;
        }
        rebuilt_any |= process_contour_voxel_rebuild_for_entity(
            world,
            entity,
            &mut before_commit,
            upload_context,
        );
    }
    rebuilt_any
}

pub(super) fn prioritize_entity(entities: &mut [hecs::Entity], preferred: Option<hecs::Entity>) {
    let Some(preferred) = preferred else {
        return;
    };
    if let Some(index) = entities.iter().position(|entity| *entity == preferred) {
        entities.swap(0, index);
    }
}

pub(super) fn resume_contour_preview_mesh_work(
    world: &mut World,
    roi_entity: hecs::Entity,
    frame_started_at: Instant,
    frame_budget: Duration,
) -> bool {
    let Ok(mut work) = world.remove_one::<ContourPreviewMeshWork>(roi_entity) else {
        return false;
    };
    let is_current = world.get::<&Roi>(roi_entity).is_ok_and(|roi| {
        roi.preview_state.active
            && roi.preview_state.revision == work.preview_revision
            && roi.dirty_state.authoritative.shape == work.source_generation
            && roi.job_state.running_request.is_some_and(|request| {
                request.preview_revision == Some(work.preview_revision)
                    && request.source_generation == work.source_generation
            })
    });
    if !is_current {
        record_job_discarded(world, roi_entity, work.started_at.elapsed());
        if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
            roi.finish_job(RoiJobKind::RebuildVoxelCache);
        }
        return false;
    }

    let mut completed = work.rebuild.is_complete();
    while !completed {
        match work.rebuild.step(&work.voxel_data) {
            Ok(done) => completed = done,
            Err(error) => {
                log::warn!("Contour preview mesh extraction failed: {error:?}");
                if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
                    roi.finish_job(RoiJobKind::RebuildVoxelCache);
                    roi.job_metrics.failed_count = roi.job_metrics.failed_count.saturating_add(1);
                }
                return false;
            }
        }
        if !completed && frame_started_at.elapsed() >= frame_budget {
            break;
        }
    }

    if !completed {
        if world.insert_one(roi_entity, work).is_err() {
            if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
                roi.finish_job(RoiJobKind::RebuildVoxelCache);
                roi.job_metrics.failed_count = roi.job_metrics.failed_count.saturating_add(1);
            }
        }
        return false;
    }

    let duration = work.started_at.elapsed();
    let source_generation = work.source_generation;
    let preview_revision = work.preview_revision;
    let preview_aabb = work.preview_aabb;
    let Some(chunked_mesh) = work.rebuild.into_result() else {
        return false;
    };
    let mesh_data = chunked_mesh.merged_mesh();
    let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) else {
        return false;
    };
    if !roi.preview_state.active
        || roi.preview_state.revision != preview_revision
        || roi.dirty_state.authoritative.shape != source_generation
    {
        roi.job_metrics.discarded_count = roi.job_metrics.discarded_count.saturating_add(1);
        roi.finish_job(RoiJobKind::RebuildVoxelCache);
        return false;
    }
    if roi
        .install_preview_mesh_result(PreviewMeshCache {
            data: mesh_data,
            chunks: Some(chunked_mesh),
            dirty_voxel_aabb: preview_aabb,
            source_generation,
            preview_revision,
        })
        .is_err()
    {
        roi.job_metrics.discarded_count = roi.job_metrics.discarded_count.saturating_add(1);
        roi.finish_job(RoiJobKind::RebuildVoxelCache);
        return false;
    }
    roi.finish_job(RoiJobKind::RebuildVoxelCache);
    roi.job_metrics.completed_count = roi.job_metrics.completed_count.saturating_add(1);
    roi.job_metrics.last_completed_kind = Some(RoiJobKind::RebuildVoxelCache);
    roi.job_metrics.last_duration_ms = duration.as_secs_f32() * 1000.0;
    true
}

pub(super) fn process_contour_voxel_rebuild_for_entity(
    world: &mut World,
    roi_entity: hecs::Entity,
    before_commit: &mut impl FnMut(&mut World, hecs::Entity),
    upload_context: Option<(&wgpu::Device, &wgpu::Queue)>,
) -> bool {
    let authoritative_generation = {
        let Ok(roi) = world.get::<&Roi>(roi_entity) else {
            return false;
        };
        let RoiBody::Contour(_) = &roi.body else {
            return false;
        };
        roi.dirty_state.authoritative.shape
    };

    if begin_next_job(world, roi_entity) != Some(RoiJobKind::RebuildVoxelCache) {
        return false;
    }
    let started_at = Instant::now();
    let running_request = world
        .get::<&Roi>(roi_entity)
        .ok()
        .and_then(|roi| roi.job_state.running_request);
    let preview_revision = running_request.and_then(|request| request.preview_revision);
    let dirty_slice_key = running_request
        .and_then(|request| match request.dirty_region {
            RoiDirtyRegion::ContourSlice(key) => Some(key),
            _ => None,
        })
        .filter(|_| {
            world.get::<&Roi>(roi_entity).is_ok_and(|roi| {
                roi.contour_data()
                    .is_some_and(|contour| contour.active_plane_family != PlaneFamily::Oblique)
            })
        });
    let previous_dirty_contour = dirty_slice_key.and_then(|key| {
        world.get::<&Roi>(roi_entity).ok().and_then(|roi| {
            let mut contour = roi.contour_data()?.clone();
            contour
                .slices
                .retain(|slice| ContourSliceKey::from_plane(slice.plane) == key);
            Some(contour)
        })
    });
    let mut contour_data = if let Some(revision) = preview_revision {
        let preview = world.get::<&Roi>(roi_entity).ok().and_then(|roi| {
            roi.contour_move_preview()
                .map(|preview| preview.contour_data.clone())
        });
        let is_current_preview = world.get::<&Roi>(roi_entity).is_ok_and(|roi| {
            roi.preview_state.active
                && roi.preview_state.revision == revision
                && roi.dirty_state.authoritative.shape == authoritative_generation
        });
        if !is_current_preview || preview.is_none() {
            record_job_discarded(world, roi_entity, started_at.elapsed());
            if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
                roi.finish_job(RoiJobKind::RebuildVoxelCache);
            }
            return false;
        }
        preview.expect("preview presence checked")
    } else {
        let Ok(roi) = world.get::<&Roi>(roi_entity) else {
            return false;
        };
        let RoiBody::Contour(ContourBody {
            data: contour_data, ..
        }) = &roi.body
        else {
            return false;
        };
        contour_data.clone()
    };
    // The slice-local rasterizer patches a retained cache, so it is only valid while that cache is
    // at most one generation behind. Otherwise every authoritative slice must be rasterized.
    let base_slice_voxel = dirty_slice_key.and_then(|_| {
        world.get::<&Roi>(roi_entity).ok().and_then(|roi| {
            let cache_generation = roi.dirty_state.voxel.built_from.shape;
            let authoritative_generation = roi.dirty_state.authoritative.shape;
            if cache_generation == authoritative_generation
                || cache_generation.saturating_add(1) == authoritative_generation
            {
                roi.voxel_cache().map(|cache| cache.data.clone())
            } else {
                None
            }
        })
    });
    if let (Some(slice_key), Some(_)) = (dirty_slice_key, base_slice_voxel.as_ref()) {
        contour_data
            .slices
            .retain(|slice| ContourSliceKey::from_plane(slice.plane) == slice_key);
    }

    let target_geometry = world
        .get::<&Roi>(roi_entity)
        .ok()
        .and_then(|roi| roi.voxel_cache().map(|cache| cache.data.geometry));
    let Some(target_geometry) = target_geometry else {
        log::warn!(
            "Skipping contour voxel rebuild for ROI {:?}: missing ROI reference grid",
            roi_entity
        );
        report_roi_status(
            world,
            roi_entity,
            "Contour voxel rebuild failed: ROI reference geometry is unavailable.".to_string(),
        );
        fail_contour_voxel_rebuild(world, roi_entity);
        return false;
    };

    let raster_started_at = Instant::now();
    let raster_result = if let Some(base) = base_slice_voxel.as_ref() {
        rasterize_contour_preview_slices_to_voxel_data(&contour_data, base)
    } else {
        rasterize_contours_to_voxel_data(&contour_data, target_geometry)
    };
    let voxel_data = match raster_result {
        Ok(voxel_data) => voxel_data,
        Err(err) => {
            log::warn!(
                "Skipping contour voxel rebuild for ROI {:?}: rasterization failed: {:?}",
                roi_entity,
                err
            );
            report_roi_status(
                world,
                roi_entity,
                format!("Contour voxel rebuild failed: rasterization error ({err:?})."),
            );
            fail_contour_voxel_rebuild(world, roi_entity);
            return false;
        }
    };
    let slice_local_rebuild_lost_base = dirty_slice_key.is_some() && base_slice_voxel.is_none();
    let committed_mesh_dirty_region =
        if preview_revision.is_none() && !slice_local_rebuild_lost_base {
            contour_slices_voxel_aabb(&contour_data, voxel_data.geometry)
                .map(|(min, max)| RoiDirtyRegion::VoxelAabb { min, max })
                .unwrap_or(RoiDirtyRegion::Full)
        } else {
            RoiDirtyRegion::Full
        };

    if let Some(revision) = preview_revision {
        let (base_chunks, prior_preview_aabb) = world
            .get::<&Roi>(roi_entity)
            .ok()
            .map(|roi| {
                let previous = roi
                    .session_caches
                    .preview_mesh
                    .as_ref()
                    .filter(|cache| cache.source_generation == authoritative_generation)
                    .map(|cache| (cache.chunks.clone(), cache.dirty_voxel_aabb));
                match previous {
                    Some((Some(chunks), aabb)) => (Some(chunks), aabb),
                    _ => (
                        roi.mesh_cache().and_then(|cache| cache.chunks.clone()),
                        None,
                    ),
                }
            })
            .unwrap_or((None, None));
        let raster_geometry = voxel_data.geometry;
        let preview_aabb = contour_geometry_voxel_aabb(&contour_data, raster_geometry);
        let previous_aabb = previous_dirty_contour
            .as_ref()
            .and_then(|contour| contour_geometry_voxel_aabb(contour, raster_geometry));
        let dirty_aabb = merge_voxel_aabbs(
            merge_voxel_aabbs(preview_aabb, previous_aabb),
            prior_preview_aabb,
        );
        let rebuild_result = if let (Some(chunks), Some((min, max))) = (base_chunks, dirty_aabb) {
            IncrementalChunkedMeshRebuild::begin_for_voxel_aabb(chunks, &voxel_data, min, max)
        } else {
            IncrementalChunkedMeshRebuild::begin_full(&voxel_data, DEFAULT_MESH_CHUNK_SIZE)
        };
        let rebuild = match rebuild_result {
            Ok(rebuild) => rebuild,
            Err(error) => {
                log::warn!("Contour preview mesh extraction failed: {error:?}");
                if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
                    roi.finish_job(RoiJobKind::RebuildVoxelCache);
                    roi.job_metrics.failed_count = roi.job_metrics.failed_count.saturating_add(1);
                }
                return false;
            }
        };
        let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) else {
            return false;
        };
        if !roi.preview_state.active
            || roi.preview_state.revision != revision
            || roi.dirty_state.authoritative.shape != authoritative_generation
        {
            roi.job_metrics.discarded_count = roi.job_metrics.discarded_count.saturating_add(1);
            roi.finish_job(RoiJobKind::RebuildVoxelCache);
            return false;
        }
        if roi
            .install_preview_voxel_result(PreviewVoxelCache {
                data: voxel_data.clone(),
                source_generation: authoritative_generation,
                preview_revision: revision,
            })
            .is_err()
        {
            roi.job_metrics.discarded_count = roi.job_metrics.discarded_count.saturating_add(1);
            roi.finish_job(RoiJobKind::RebuildVoxelCache);
            return false;
        }
        roi.mark_all_contour_view_caches_stale();
        drop(roi);
        let work = ContourPreviewMeshWork {
            source_generation: authoritative_generation,
            preview_revision: revision,
            preview_aabb,
            voxel_data,
            rebuild,
            started_at,
        };
        if world.insert_one(roi_entity, work).is_err() {
            if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
                roi.finish_job(RoiJobKind::RebuildVoxelCache);
                roi.job_metrics.failed_count = roi.job_metrics.failed_count.saturating_add(1);
            }
            return false;
        }
        return true;
    }

    before_commit(world, roi_entity);

    let is_stale = match world.get::<&Roi>(roi_entity) {
        Ok(roi) => roi.dirty_state.authoritative.shape != authoritative_generation,
        Err(_) => return false,
    };
    if is_stale {
        log::info!(
            "Discarding stale contour voxel rebuild for ROI {:?}: generation changed from {}",
            roi_entity,
            authoritative_generation,
        );
        record_job_discarded(world, roi_entity, started_at.elapsed());
        requeue_contour_voxel_rebuild(world, roi_entity);
        return false;
    }

    let (gpu_resources, gpu_upload_ms) = if let Some((device, queue)) = upload_context {
        let Some(placeholder_bg) = main_volume_bind_group(world) else {
            log::warn!(
                "Skipping contour voxel rebuild GPU upload for ROI {:?}: missing main volume bind group",
                roi_entity
            );
            requeue_contour_voxel_rebuild(world, roi_entity);
            report_roi_status(
                world,
                roi_entity,
                "Contour voxel rebuild failed: main volume bind-group is unavailable.".to_string(),
            );
            fail_contour_voxel_rebuild(world, roi_entity);
            return false;
        };

        let upload_started_at = Instant::now();
        let (texture, view, sampler) =
            match crate::io::volume::create_texture_from_voxel_data(device, queue, &voxel_data) {
                Ok(gpu_tuple) => gpu_tuple,
                Err(err) => {
                    log::warn!(
                        "Skipping contour voxel rebuild for ROI {:?}: GPU upload failed: {}",
                        roi_entity,
                        err
                    );
                    report_roi_status(
                        world,
                        roi_entity,
                        format!("Contour voxel rebuild failed: GPU upload error ({err})."),
                    );
                    fail_contour_voxel_rebuild(world, roi_entity);
                    return false;
                }
            };

        (
            Some(GpuVolumeResources {
                texture,
                view,
                sampler,
                bind_group: placeholder_bg,
            }),
            Some(upload_started_at.elapsed().as_secs_f32() * 1000.0),
        )
    } else {
        (None, None)
    };

    let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) else {
        return false;
    };
    roi.job_metrics.last_contour_raster_ms = raster_started_at.elapsed().as_secs_f32() * 1000.0;
    if let Some(upload_ms) = gpu_upload_ms {
        roi.job_metrics.last_gpu_upload_ms = upload_ms;
    }

    let cache_install_started_at = Instant::now();
    let installed = roi.install_voxel_cache_result(
        VoxelCache {
            data: voxel_data,
            gpu_resources,
        },
        authoritative_generation,
    );
    roi.job_metrics.last_cpu_cache_install_ms =
        cache_install_started_at.elapsed().as_secs_f32() * 1000.0;
    if installed.is_err() {
        roi.job_metrics.discarded_count = roi.job_metrics.discarded_count.saturating_add(1);
        roi.finish_job(RoiJobKind::RebuildVoxelCache);
        return false;
    }
    roi.mark_cache_dirty(RoiCacheKind::Mesh);
    roi.enqueue_job(RoiJobRequest {
        kind: RoiJobKind::RebuildMeshCache,
        source_generation: authoritative_generation,
        preview_revision: None,
        priority: RoiJobPriority::VisibleCommitted,
        dirty_region: committed_mesh_dirty_region,
    });
    roi.job_metrics.completed_count = roi.job_metrics.completed_count.saturating_add(1);
    roi.job_metrics.last_completed_kind = Some(RoiJobKind::RebuildVoxelCache);
    roi.job_metrics.last_duration_ms = started_at.elapsed().as_secs_f32() * 1000.0;
    drop(roi);
    report_roi_status(
        world,
        roi_entity,
        "Contour voxel cache rebuilt.".to_string(),
    );
    true
}

pub(super) fn requeue_contour_voxel_rebuild(world: &mut World, roi_entity: hecs::Entity) {
    if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
        roi.finish_job(RoiJobKind::RebuildVoxelCache);
        roi.mark_cache_dirty(RoiCacheKind::Voxel);
        roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
    }
}

pub(super) fn fail_contour_voxel_rebuild(world: &mut World, roi_entity: hecs::Entity) {
    if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
        roi.finish_job(RoiJobKind::RebuildVoxelCache);
        roi.mark_cache_dirty(RoiCacheKind::Voxel);
        roi.job_metrics.failed_count = roi.job_metrics.failed_count.saturating_add(1);
    }
}

pub(super) fn merge_voxel_aabbs(
    left: Option<([u32; 3], [u32; 3])>,
    right: Option<([u32; 3], [u32; 3])>,
) -> Option<([u32; 3], [u32; 3])> {
    match (left, right) {
        (Some((left_min, left_max)), Some((right_min, right_max))) => Some((
            std::array::from_fn(|axis| left_min[axis].min(right_min[axis])),
            std::array::from_fn(|axis| left_max[axis].max(right_max[axis])),
        )),
        (left, right) => left.or(right),
    }
}
