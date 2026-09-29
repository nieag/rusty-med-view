use super::*;
use crate::app::roi::authority::*;
use crate::app::roi::history::*;
use crate::app::roi::preview::*;
use crate::app::roi::requests::*;
use crate::convert::{orthogonal_plane_from_volume_uv, world_mm_to_voxel_index};

fn spawn_test_roi(world: &mut World) -> hecs::Entity {
    world.spawn((Roi::new_voxel_with_cache(
        RoiId(1),
        "Test".to_string(),
        VoxelGeometry::new(
            [4, 4, 4],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap(),
        vec![1; 64],
        None,
    ),))
}

fn spawn_sparse_voxel_roi(world: &mut World) -> hecs::Entity {
    let mut raw = vec![0_u8; 64];
    raw[(2 * 4 + 1) * 4 + 1] = 1;
    world.spawn((Roi::new_voxel_with_cache(
        RoiId(2),
        "Sparse".to_string(),
        VoxelGeometry::new(
            [4, 4, 4],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap(),
        raw,
        None,
    ),))
}

fn spawn_main_volume(world: &mut World, spacing: [f32; 3], origin: [f32; 3]) {
    world.spawn((
        VolumeData {
            dimensions: [4, 4, 4],
            geometry: Some(
                VoxelGeometry::new([4, 4, 4], spacing, origin, [0.0, 0.0, 0.0, 1.0]).unwrap(),
            ),
            intensities: vec![],
            intensity_range: [0.0, 1.0],
        },
        MainVolumeTag,
    ));
}

fn test_plane_definition(family: PlaneFamily) -> PlaneDefinition {
    PlaneDefinition {
        family,
        origin_mm: [0.0, 0.0, 0.0],
        u_axis_mm: [1.0, 0.0, 0.0],
        v_axis_mm: [0.0, 1.0, 0.0],
        normal_mm: [0.0, 0.0, 1.0],
    }
}

fn spawn_test_contour_roi(
    world: &mut World,
    family: PlaneFamily,
    with_loops: bool,
) -> hecs::Entity {
    let geometry = VoxelGeometry::new(
        [4, 4, 4],
        [1.0, 1.0, 1.0],
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap();
    let slices = if with_loops {
        vec![ContourSlice {
            plane: test_plane_definition(family),
            loops: vec![ContourLoop {
                points: vec![
                    ContourPoint {
                        local_mm: [0.0, 0.0],
                    },
                    ContourPoint {
                        local_mm: [1.0, 0.0],
                    },
                    ContourPoint {
                        local_mm: [0.0, 1.0],
                    },
                ],
                is_closed: true,
            }],
        }]
    } else {
        Vec::new()
    };

    let entity = world.spawn((Roi::new_contour_with_geometry(
        RoiId(100),
        "Contour".to_string(),
        geometry,
        ContourData {
            active_plane_family: family,
            slices,
        },
    ),));
    let mut roi = world.get::<&mut Roi>(entity).unwrap();
    roi.session_caches.voxel = Some(VoxelCache {
        data: VoxelData {
            geometry,
            raw_data: vec![0; 64],
        },
        gpu_resources: None,
    });
    entity
}

fn seed_current_voxel_cache_for_contour_roi(world: &mut World, entity: hecs::Entity) {
    let mut roi = world.get::<&mut Roi>(entity).unwrap();
    roi.session_caches.voxel = Some(VoxelCache {
        data: VoxelData {
            geometry: VoxelGeometry::new(
                [4, 4, 4],
                [1.0, 1.0, 1.0],
                [0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            )
            .unwrap(),
            raw_data: vec![0; 64],
        },
        gpu_resources: None,
    });
    roi.dirty_state.voxel.dirty = false;
    roi.dirty_state.voxel.built_from = roi.dirty_state.authoritative;
}

fn square_contour_data_for_main_volume(world: &World, half_extent: f32) -> ContourData {
    let geometry = main_volume_geometry(world).expect("main volume geometry must exist");
    let plane = orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 0.5], geometry)
        .expect("axial plane should resolve");
    ContourData {
        active_plane_family: PlaneFamily::Axial,
        slices: vec![ContourSlice {
            plane,
            loops: vec![ContourLoop {
                points: vec![
                    ContourPoint {
                        local_mm: [-half_extent, -half_extent],
                    },
                    ContourPoint {
                        local_mm: [half_extent, -half_extent],
                    },
                    ContourPoint {
                        local_mm: [half_extent, half_extent],
                    },
                    ContourPoint {
                        local_mm: [-half_extent, half_extent],
                    },
                ],
                is_closed: true,
            }],
        }],
    }
}

fn simple_mesh_data() -> MeshData {
    MeshData {
        vertices: vec![
            MeshVertex {
                world_mm: [0.0, 0.0, 0.0],
            },
            MeshVertex {
                world_mm: [1.0, 0.0, 0.0],
            },
            MeshVertex {
                world_mm: [0.0, 1.0, 0.0],
            },
        ],
        faces: vec![MeshFace {
            vertex_indices: [0, 1, 2],
        }],
    }
}

fn closed_tetra_mesh_data() -> MeshData {
    MeshData {
        vertices: vec![
            MeshVertex {
                world_mm: [0.0, 0.0, 0.0],
            },
            MeshVertex {
                world_mm: [2.0, 0.0, 0.0],
            },
            MeshVertex {
                world_mm: [0.0, 2.0, 0.0],
            },
            MeshVertex {
                world_mm: [0.0, 0.0, 2.0],
            },
        ],
        faces: vec![
            MeshFace {
                vertex_indices: [0, 2, 1],
            },
            MeshFace {
                vertex_indices: [0, 1, 3],
            },
            MeshFace {
                vertex_indices: [0, 3, 2],
            },
            MeshFace {
                vertex_indices: [1, 2, 3],
            },
        ],
    }
}

#[test]
fn test_request_cache_rebuild_marks_cache_dirty_and_queues_job() {
    let mut world = World::new();
    let entity = spawn_test_roi(&mut world);

    let job = request_cache_rebuild(&mut world, entity, RoiCacheKind::Contour);
    let status = cache_status(&world, entity, RoiCacheKind::Contour).unwrap();
    let roi = world.get::<&Roi>(entity).unwrap();

    assert_eq!(job, Some(RoiJobKind::RebuildContourCache));
    assert!(status.is_dirty);
    assert!(!status.is_current);
    assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildContourCache));
}

#[test]
fn test_begin_and_complete_job_update_runtime_status() {
    let mut world = World::new();
    let entity = spawn_test_roi(&mut world);

    request_cache_rebuild(&mut world, entity, RoiCacheKind::Voxel);
    assert_eq!(
        begin_next_job(&mut world, entity),
        Some(RoiJobKind::RebuildVoxelCache)
    );

    let status_before = cache_status(&world, entity, RoiCacheKind::Voxel).unwrap();
    assert!(status_before.is_dirty);

    assert!(complete_cache_rebuild(
        &mut world,
        entity,
        RoiCacheKind::Voxel
    ));

    let status_after = cache_status(&world, entity, RoiCacheKind::Voxel).unwrap();
    let roi = world.get::<&Roi>(entity).unwrap();
    assert!(!status_after.is_dirty);
    assert!(status_after.is_current);
    assert_eq!(roi.running_job_kind(), None);
}

#[test]
fn test_voxel_roi_stats_use_nonzero_voxels_and_volume_spacing() {
    let mut world = World::new();
    spawn_main_volume(&mut world, [0.5, 0.5, 2.0], [0.0, 0.0, 0.0]);
    let entity = world.spawn((Roi::new_voxel_with_cache(
        RoiId(2),
        "Mask".to_string(),
        VoxelGeometry::new(
            [2, 2, 2],
            [0.5, 0.5, 2.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap(),
        vec![0, 1, 2, 0, 0, 3, 4, 0],
        None,
    ),));

    let stats = roi_voxel_stats(&world, entity).unwrap();

    assert_eq!(stats.occupied_voxels, 4);
    assert!((stats.volume_mm3 - 2.0).abs() < f32::EPSILON);
}

#[test]
fn test_roi_voxel_stats_contour_primary_returns_none_before_rebuild() {
    let mut world = World::new();
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, true);
    let stats = roi_voxel_stats(&world, entity);
    assert!(stats.is_none());
}

#[test]
fn test_roi_voxel_stats_contour_primary_returns_derived_stats_after_rebuild() {
    let mut world = World::new();
    spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
    let replacement = square_contour_data_for_main_volume(&world, 1.4);
    replace_contour_data(&mut world, entity, replacement).unwrap();
    process_contour_voxel_rebuild_jobs(&mut world);

    let expected_occupied = {
        let roi = world.get::<&Roi>(entity).unwrap();
        roi.voxel_cache()
            .expect("expected derived voxel cache")
            .data
            .raw_data
            .iter()
            .filter(|v| **v != 0)
            .count() as u64
    };
    let stats = roi_voxel_stats(&world, entity).expect("expected derived stats");
    assert!(expected_occupied > 0);
    assert_eq!(stats.occupied_voxels, expected_occupied);
    assert!((stats.volume_mm3 - expected_occupied as f32).abs() < f32::EPSILON);
}

#[test]
fn test_mesh_viewport_sync_keeps_current_mesh_cache_current() {
    let mut world = World::new();
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, true);
    seed_current_voxel_cache_for_contour_roi(&mut world, entity);
    world.spawn((Viewport {
        mode: ViewMode::ThreeD,
        rect: [0.0, 0.0, 800.0, 600.0],
        uniform_index: 0,
    },));
    world.spawn((EditorState {
        active_roi: Some(entity),
        ..EditorState::default()
    },));
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        let generation = roi.dirty_state.authoritative.shape;
        roi.install_mesh_cache_result(
            MeshCache {
                data: MeshData {
                    vertices: Vec::new(),
                    faces: Vec::new(),
                },
                chunks: None,
            },
            generation,
        )
        .unwrap();
    }

    sync_active_roi_mesh_cache_for_viewports(&mut world);

    let roi = world.get::<&Roi>(entity).unwrap();
    assert!(roi.is_cache_current(RoiCacheKind::Mesh));
    assert!(!roi.has_queued_job(RoiJobKind::RebuildMeshCache));
}

#[test]
fn test_advance_roi_work_reports_pending_queued_work() {
    let mut world = World::new();
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
    world
        .get::<&mut Roi>(entity)
        .unwrap()
        .enqueue_rebuild(RoiJobKind::RebuildMeshCache);

    let status = advance_roi_work(&mut world, None);

    assert!(status.pending);
    let roi = world.get::<&Roi>(entity).unwrap();
    assert!(roi.has_queued_job(RoiJobKind::RebuildMeshCache));
}

#[test]
fn test_advance_roi_work_rebuilds_contour_voxels_from_roi_reference_grid() {
    let mut world = World::new();
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, true);
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
        roi.job_metrics.last_work_convergence_ms = 42.0;
    }

    for _ in 0..32 {
        if !advance_roi_work(&mut world, None).pending {
            break;
        }
    }

    let roi = world.get::<&Roi>(entity).unwrap();
    assert!(roi.is_cache_current(RoiCacheKind::Voxel));
    assert_eq!(roi.reference_geometry().dimensions(), [4, 4, 4]);
    assert!(roi.job_state.work_cycle_started_at.is_none());
    assert_ne!(roi.job_metrics.last_work_convergence_ms, 42.0);
}

#[test]
fn test_main_volume_geometry_reads_main_volume_fields() {
    let mut world = World::new();
    spawn_main_volume(&mut world, [0.25, 0.5, 2.0], [3.0, -1.5, 2.25]);

    let geometry = main_volume_geometry(&world).unwrap();

    assert_eq!(geometry.dimensions, [4, 4, 4]);
    assert_eq!(geometry.spacing(), [0.25, 0.5, 2.0]);
    assert_eq!(geometry.origin(), [3.0, -1.5, 2.25]);
    assert_eq!(geometry.orientation(), [0.0, 0.0, 0.0, 1.0]);
}

#[test]
fn test_main_volume_geometry_is_none_for_the_placeholder_volume() {
    let mut world = World::new();
    spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
    let (_, volume) = world
        .query_mut::<&mut VolumeData>()
        .with::<&MainVolumeTag>()
        .into_iter()
        .next()
        .unwrap();
    volume.geometry = None;

    assert!(main_volume_geometry(&world).is_none());
}

#[test]
fn test_display_volume_change_does_not_retarget_contour_roi_geometry() {
    let mut world = World::new();
    spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, true);
    let (identity_before, voxel_geometry_before) = {
        let roi = world.get::<&Roi>(entity).unwrap();
        (
            roi.reference_geometry().identity(),
            roi.voxel_cache().unwrap().data.geometry,
        )
    };

    let (_, volume) = world
        .query_mut::<&mut VolumeData>()
        .with::<&MainVolumeTag>()
        .into_iter()
        .next()
        .unwrap();
    volume.dimensions = [9, 8, 7];
    volume.geometry = Some(
        VoxelGeometry::new(
            [9, 8, 7],
            [0.25, 2.0, 3.0],
            [10.0, -4.0, 2.0],
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap(),
    );

    let roi = world.get::<&Roi>(entity).unwrap();
    assert_eq!(roi.reference_geometry().identity(), identity_before);
    assert_eq!(
        roi.voxel_cache().unwrap().data.geometry,
        voxel_geometry_before
    );
}

#[test]
fn test_main_volume_geometry_returns_none_when_missing() {
    let world = World::new();
    assert!(main_volume_geometry(&world).is_none());
}

#[test]
fn test_visible_voxel_overlay_count_ignores_non_renderable_rois() {
    let mut world = World::new();
    let first = spawn_test_roi(&mut world);
    let second = spawn_test_roi(&mut world);
    let contour = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);

    {
        let mut roi = world.get::<&mut Roi>(first).unwrap();
        roi.metadata.is_visible = true;
    }
    {
        let mut roi = world.get::<&mut Roi>(second).unwrap();
        roi.metadata.is_visible = true;
    }
    {
        let mut roi = world.get::<&mut Roi>(contour).unwrap();
        roi.metadata.is_visible = true;
    }

    assert_eq!(visible_voxel_overlay_count(&world), 0);
    assert!(renderable_voxel_overlay_rois(&world, Some(contour)).is_empty());
    assert!(can_enable_roi_visibility(&world, contour));
}

#[test]
fn test_voxel_overlay_request_blocks_when_cpu_cache_is_current_but_gpu_missing() {
    let mut world = World::new();
    let entity = spawn_sparse_voxel_roi(&mut world);

    let status = request_voxel_overlay_state(&world, entity);

    assert_eq!(status.state, RepresentationRequestState::Blocked);
    assert_eq!(status.reason.as_deref(), Some("voxel_gpu_missing"));
}

#[test]
fn test_prepare_voxel_roi_import_uses_label_geometry_without_main_volume() {
    let world = World::new();
    let loaded_label = LoadedLabel {
        dimensions: [2, 2, 2],
        geometry: VoxelGeometry::new(
            [2, 2, 2],
            [1.25, 1.5, 2.0],
            [5.0, 6.0, 7.0],
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap(),
        data: vec![0; 8],
        filename: "Label".to_string(),
    };

    let import_spec = prepare_voxel_roi_import(&world, &loaded_label).unwrap();
    assert_eq!(import_spec.geometry.dimensions, [2, 2, 2]);
    assert_eq!(import_spec.geometry.spacing(), [1.25, 1.5, 2.0]);
    assert_eq!(import_spec.geometry.origin(), [5.0, 6.0, 7.0]);
    assert_eq!(import_spec.geometry.orientation(), [0.0, 0.0, 0.0, 1.0]);
    assert!(import_spec.geometry_matches_main);
    assert!(import_spec.start_visible);
}

#[test]
fn test_prepare_voxel_roi_import_preserves_label_geometry_even_when_main_volume_differs() {
    let mut world = World::new();
    spawn_main_volume(&mut world, [0.5, 0.5, 2.0], [10.0, 10.0, 10.0]);
    let loaded_label = LoadedLabel {
        dimensions: [3, 4, 5],
        geometry: VoxelGeometry::new(
            [3, 4, 5],
            [0.75, 0.8, 1.25],
            [-2.0, 4.5, 6.0],
            [0.0, 0.0, 1.0, 0.0],
        )
        .unwrap(),
        data: vec![0; 60],
        filename: "Label".to_string(),
    };

    let import_spec = prepare_voxel_roi_import(&world, &loaded_label).unwrap();

    assert_eq!(import_spec.geometry.dimensions, [3, 4, 5]);
    assert_eq!(import_spec.geometry.spacing(), [0.75, 0.8, 1.25]);
    assert_eq!(import_spec.geometry.origin(), [-2.0, 4.5, 6.0]);
    assert_eq!(import_spec.geometry.orientation(), [0.0, 0.0, 1.0, 0.0]);
    assert!(!import_spec.geometry_matches_main);
    assert!(import_spec.start_visible);
}

#[test]
fn test_create_empty_contour_roi_creates_contour_primary_with_requested_plane_family() {
    let mut world = World::new();
    spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
    let editor = world.spawn((EditorState::default(),));

    let entity = create_empty_contour_roi(&mut world, editor, PlaneFamily::Coronal).unwrap();

    let roi = world.get::<&Roi>(entity).unwrap();
    assert_eq!(roi.primary_representation(), PrimaryRepresentation::Contour);
    let contour_data = roi.contour_data().expect("expected contour roi");
    assert_eq!(contour_data.active_plane_family, PlaneFamily::Coronal);
    assert!(contour_data.slices.is_empty());
    assert!(roi.is_cache_current(RoiCacheKind::Voxel));
    assert!(roi.voxel_cache().is_some_and(|cache| cache
        .data
        .raw_data
        .iter()
        .all(|value| *value == 0)));
}

#[test]
fn test_create_empty_contour_roi_sets_active_roi_to_new_entity() {
    let mut world = World::new();
    spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
    let editor = world.spawn((EditorState::default(),));

    let entity = create_empty_contour_roi(&mut world, editor, PlaneFamily::Axial).unwrap();

    let editor_state = world.get::<&EditorState>(editor).unwrap();
    assert_eq!(editor_state.active_roi, Some(entity));
}

#[test]
fn test_promote_voxel_to_contour_authority_preserves_roi_and_rebases_voxel_cache() {
    let mut world = World::new();
    let entity = spawn_sparse_voxel_roi(&mut world);
    let (roi_id, name, source_voxel) = {
        let roi = world.get::<&Roi>(entity).unwrap();
        (
            roi.metadata.roi_id,
            roi.metadata.name.clone(),
            roi.voxel_cache().unwrap().data.clone(),
        )
    };
    let roi_count_before = world.query::<&Roi>().iter().count();

    promote_voxel_roi_to_contour_authority(&mut world, entity, PlaneFamily::Axial).unwrap();

    assert_eq!(world.query::<&Roi>().iter().count(), roi_count_before);
    let roi = world.get::<&Roi>(entity).unwrap();
    assert_eq!(roi.metadata.roi_id, roi_id);
    assert_eq!(roi.metadata.name, name);
    assert_eq!(roi.primary_representation(), PrimaryRepresentation::Contour);
    assert_eq!(
        roi.contour_data().unwrap().active_plane_family,
        PlaneFamily::Axial
    );
    assert!(roi.contour_data().unwrap().has_loops());
    assert_eq!(roi.dirty_state.authoritative.shape, 2);
    assert_eq!(roi.voxel_cache().unwrap().data, source_voxel);
    assert!(roi.is_cache_current(RoiCacheKind::Voxel));
    assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildMeshCache));
}

#[test]
fn test_promote_voxel_to_contour_authority_rejects_locked_roi() {
    let mut world = World::new();
    let entity = spawn_sparse_voxel_roi(&mut world);
    world.get::<&mut Roi>(entity).unwrap().metadata.is_locked = true;

    let result = promote_voxel_roi_to_contour_authority(&mut world, entity, PlaneFamily::Axial);

    assert_eq!(result, Err(VoxelContourPromotionError::Locked));
    assert_eq!(
        world.get::<&Roi>(entity).unwrap().primary_representation(),
        PrimaryRepresentation::Voxel
    );
}

#[test]
fn test_promote_current_mesh_cache_to_authority_preserves_roi_and_voxel_cache() {
    let mut world = World::new();
    let entity = spawn_sparse_voxel_roi(&mut world);
    let mesh = closed_tetra_mesh_data();
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        roi.session_caches.mesh = Some(MeshCache {
            data: mesh.clone(),
            chunks: None,
        });
        roi.dirty_state.mesh.dirty = false;
        roi.dirty_state.mesh.built_from = roi.dirty_state.authoritative;
    }
    let (roi_id, name) = {
        let roi = world.get::<&Roi>(entity).unwrap();
        (roi.metadata.roi_id, roi.metadata.name.clone())
    };
    let roi_count_before = world.query::<&Roi>().iter().count();

    promote_current_mesh_cache_to_authority(&mut world, entity).unwrap();

    assert_eq!(world.query::<&Roi>().iter().count(), roi_count_before);
    let roi = world.get::<&Roi>(entity).unwrap();
    assert_eq!(roi.metadata.roi_id, roi_id);
    assert_eq!(roi.metadata.name, name);
    assert_eq!(roi.primary_representation(), PrimaryRepresentation::Mesh);
    assert_eq!(roi.mesh_data(), Some(&mesh));
    assert_eq!(roi.dirty_state.authoritative.shape, 2);
    assert!(roi.is_cache_current(RoiCacheKind::Voxel));
    assert!(!roi.is_cache_dirty(RoiCacheKind::Mesh));
    assert!(roi.session_caches.mesh.is_none());
}

#[test]
fn test_promote_current_mesh_cache_to_authority_rejects_stale_cache() {
    let mut world = World::new();
    let entity = spawn_sparse_voxel_roi(&mut world);
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        roi.session_caches.mesh = Some(MeshCache {
            data: closed_tetra_mesh_data(),
            chunks: None,
        });
        roi.dirty_state.mesh.dirty = true;
    }

    let result = promote_current_mesh_cache_to_authority(&mut world, entity);

    assert_eq!(
        result,
        Err(MeshAuthorityPromotionError::MeshCacheNotCurrent)
    );
    assert_eq!(
        world.get::<&Roi>(entity).unwrap().primary_representation(),
        PrimaryRepresentation::Voxel
    );
}

#[test]
fn test_authority_roundtrip_voxel_contour_voxel_preserves_entity() {
    let mut world = World::new();
    let entity = spawn_sparse_voxel_roi(&mut world);
    let roi_id = world.get::<&Roi>(entity).unwrap().metadata.roi_id;

    promote_roi_to_contour_authority(&mut world, entity, PlaneFamily::Axial).unwrap();
    promote_current_voxel_cache_to_authority(&mut world, entity).unwrap();

    let roi = world.get::<&Roi>(entity).unwrap();
    assert_eq!(roi.metadata.roi_id, roi_id);
    assert_eq!(roi.primary_representation(), PrimaryRepresentation::Voxel);
    assert!(matches!(
        roi.authoritative_data,
        RoiAuthoritativeData::Voxel(_)
    ));
    assert!(roi.is_cache_current(RoiCacheKind::Voxel));
    assert_eq!(roi.dirty_state.authoritative.shape, 3);
}

#[test]
fn test_authority_roundtrip_mesh_voxel_mesh_reuses_current_mesh() {
    let mut world = World::new();
    let entity = spawn_sparse_voxel_roi(&mut world);
    let mesh = closed_tetra_mesh_data();
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        roi.session_caches.mesh = Some(MeshCache {
            data: mesh.clone(),
            chunks: None,
        });
        roi.dirty_state.mesh.dirty = false;
        roi.dirty_state.mesh.built_from = roi.dirty_state.authoritative;
    }

    promote_current_mesh_cache_to_authority(&mut world, entity).unwrap();
    promote_current_voxel_cache_to_authority(&mut world, entity).unwrap();
    assert!(world
        .get::<&Roi>(entity)
        .unwrap()
        .is_cache_current(RoiCacheKind::Mesh));
    promote_current_mesh_cache_to_authority(&mut world, entity).unwrap();

    let roi = world.get::<&Roi>(entity).unwrap();
    assert_eq!(roi.primary_representation(), PrimaryRepresentation::Mesh);
    assert_eq!(roi.mesh_data(), Some(&mesh));
    assert_eq!(roi.dirty_state.authoritative.shape, 4);
}

#[test]
fn test_replace_mesh_data_leaves_voxel_cache_stale_until_explicitly_requested() {
    let mut world = World::new();
    let entity = world.spawn((Roi::new_mesh(
        RoiId(300),
        "Mesh".to_string(),
        MeshData {
            vertices: Vec::new(),
            faces: Vec::new(),
        },
    ),));
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        roi.dirty_state.authoritative_dirty = false;
        roi.dirty_state.voxel.dirty = false;
        roi.dirty_state.contour.dirty = false;
        roi.dirty_state.mesh.dirty = false;
        roi.dirty_state.voxel.built_from = roi.dirty_state.authoritative;
        roi.dirty_state.contour.built_from = roi.dirty_state.authoritative;
        roi.dirty_state.mesh.built_from = roi.dirty_state.authoritative;
    }

    let replacement = closed_tetra_mesh_data();
    let result = replace_mesh_data(&mut world, entity, replacement.clone());
    assert_eq!(result, Ok(()));

    let roi = world.get::<&Roi>(entity).unwrap();
    assert_eq!(roi.mesh_data(), Some(&replacement));
    assert!(roi.dirty_state.authoritative_dirty);
    assert_eq!(roi.dirty_state.authoritative.shape, 2);
    assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
    assert!(roi.is_cache_dirty(RoiCacheKind::Contour));
    assert!(!roi.is_cache_dirty(RoiCacheKind::Mesh));
    assert_eq!(roi.queued_job_kind(), None);
    drop(roi);

    assert_eq!(
        crate::app::roi::request_mesh_voxel_cache_rebuild(&mut world, entity),
        Ok(())
    );
    assert_eq!(
        world.get::<&Roi>(entity).unwrap().queued_job_kind(),
        Some(RoiJobKind::RebuildVoxelCache)
    );
}

#[test]
fn test_mesh_rebuild_revalidates_after_authority_generation_changes() {
    let mut world = World::new();
    let entity = world.spawn((Roi::new_mesh(
        RoiId(301),
        "Mesh".to_string(),
        closed_tetra_mesh_data(),
    ),));
    crate::app::roi::request_mesh_voxel_cache_rebuild(&mut world, entity).unwrap();
    assert_eq!(
        world.get::<&Roi>(entity).unwrap().validated_mesh_generation,
        Some(1)
    );

    replace_mesh_data(&mut world, entity, simple_mesh_data()).unwrap();
    let roi = world.get::<&Roi>(entity).unwrap();
    assert_ne!(
        roi.validated_mesh_generation,
        Some(roi.dirty_state.authoritative.shape)
    );
    drop(roi);
    assert!(matches!(
        crate::app::roi::request_mesh_voxel_cache_rebuild(&mut world, entity),
        Err(MeshMutationError::InvalidMesh(_))
    ));
    let roi = world.get::<&Roi>(entity).unwrap();
    assert_eq!(roi.validated_mesh_generation, None);
    assert_eq!(roi.queued_job_kind(), None);
}

#[test]
fn test_replace_mesh_data_rejects_non_mesh_roi() {
    let mut world = World::new();
    let entity = spawn_test_roi(&mut world);
    let result = replace_mesh_data(&mut world, entity, simple_mesh_data());
    assert_eq!(result, Err(MeshMutationError::NotMeshRoi));
}

#[test]
fn test_translate_mesh_data_changes_authority_without_eager_voxel_rebuild() {
    let mut world = World::new();
    let original = closed_tetra_mesh_data();
    let entity = world.spawn((Roi::new_mesh(
        RoiId(302),
        "Translated".to_string(),
        original.clone(),
    ),));

    translate_mesh_data(&mut world, entity, [1.0, -2.0, 0.5]).unwrap();

    let roi = world.get::<&Roi>(entity).unwrap();
    let translated = roi.mesh_data().unwrap();
    assert_eq!(translated.vertices[0].world_mm, [1.0, -2.0, 0.5]);
    assert_eq!(translated.faces, original.faces);
    assert_eq!(roi.queued_job_kind(), None);
    assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
    assert!(roi.is_cache_dirty(RoiCacheKind::Contour));
}

#[test]
fn test_mesh_edit_preview_updates_direct_contour_view_before_commit() {
    let mut world = World::new();
    spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
    let editor = world.spawn((EditorState::default(),));
    let original = closed_tetra_mesh_data();
    let entity = world.spawn((Roi::new_mesh(
        RoiId(303),
        "Preview".to_string(),
        original.clone(),
    ),));
    world.get::<&mut EditorState>(editor).unwrap().active_roi = Some(entity);
    let geometry = main_volume_geometry(&world).unwrap();
    let plane =
        orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 0.25], geometry).unwrap();
    let key = ContourViewKey::from_plane(plane);

    let revision =
        begin_mesh_translation_preview(&mut world, editor, entity, [0.25, 0.0, 0.0]).unwrap();
    let status = ensure_contour_view_cache(&mut world, entity, &key);

    assert_eq!(revision, 1);
    assert_eq!(status.state, RepresentationRequestState::Preview);
    {
        let history = world.get::<&Roi>(entity).unwrap().history.clone();
        assert!(history.undo.is_empty());
        assert!(history.redo.is_empty());
    }
    {
        let roi = world.get::<&Roi>(entity).unwrap();
        assert_eq!(roi.mesh_data(), Some(&original));
        assert!(roi.preview_state.active);
        assert!(matches!(
            roi.contour_view_cache(&key).map(|cache| &cache.state),
            Some(CacheViewState::Preview { revision: 1 })
        ));
    }

    commit_mesh_edit_preview(&mut world, editor).unwrap();
    let roi = world.get::<&Roi>(entity).unwrap();
    assert_eq!(roi.mesh_data().unwrap().vertices[0].world_mm[0], 0.25);
    assert!(!roi.preview_state.active);
    assert_eq!(roi.queued_job_kind(), None);
    assert!(roi.contour_view_data_for_render(&key).is_none());
    drop(roi);
    assert_eq!(
        ensure_contour_view_cache(&mut world, entity, &key).state,
        RepresentationRequestState::Current
    );

    {
        let history = world.get::<&Roi>(entity).unwrap().history.clone();
        assert_eq!(history.undo.len(), 1);
        assert!(history.redo.is_empty());
    }

    assert_eq!(undo_roi_edit(&mut world, editor), Ok(entity));
    {
        let roi = world.get::<&Roi>(entity).unwrap();
        assert_eq!(roi.mesh_data(), Some(&original));
        assert_eq!(roi.queued_job_kind(), None);
        assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert!(roi.is_cache_dirty(RoiCacheKind::Contour));
    }
    {
        let history = world.get::<&Roi>(entity).unwrap().history.clone();
        assert!(history.undo.is_empty());
        assert_eq!(history.redo.len(), 1);
    }

    assert_eq!(redo_roi_edit(&mut world, editor), Ok(entity));
    let roi = world.get::<&Roi>(entity).unwrap();
    assert_eq!(roi.mesh_data().unwrap().vertices[0].world_mm[0], 0.25);
    drop(roi);
    let history = world.get::<&Roi>(entity).unwrap().history.clone();
    assert_eq!(history.undo.len(), 1);
    assert!(history.redo.is_empty());
}

#[test]
fn test_cancel_mesh_edit_preview_preserves_authority() {
    let mut world = World::new();
    let editor = world.spawn((EditorState::default(),));
    let original = closed_tetra_mesh_data();
    let entity = world.spawn((Roi::new_mesh(
        RoiId(304),
        "Cancel".to_string(),
        original.clone(),
    ),));
    let geometry = VoxelGeometry::new([4; 3], [1.0; 3], [0.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap();
    let plane =
        orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 0.25], geometry).unwrap();
    let key = ContourViewKey::from_plane(plane);

    begin_mesh_translation_preview(&mut world, editor, entity, [1.0, 0.0, 0.0]).unwrap();
    assert_eq!(
        ensure_contour_view_cache(&mut world, entity, &key).state,
        RepresentationRequestState::Preview
    );
    cancel_mesh_edit_preview(&mut world, editor).unwrap();

    let roi = world.get::<&Roi>(entity).unwrap();
    assert_eq!(roi.mesh_data(), Some(&original));
    assert!(!roi.preview_state.active);
    assert_eq!(roi.queued_job_kind(), None);
    assert!(roi.contour_view_data_for_render(&key).is_none());
    drop(roi);
    assert_eq!(
        ensure_contour_view_cache(&mut world, entity, &key).state,
        RepresentationRequestState::Current
    );
    assert_eq!(
        world
            .get::<&Roi>(entity)
            .unwrap()
            .contour_view_data_for_render(&key),
        Some(&intersect_mesh_with_plane(&original, plane).unwrap())
    );
}

#[test]
fn test_mesh_rebuild_contract_builds_voxel_then_enables_contour_refresh() {
    let mut world = World::new();
    let entity = spawn_sparse_voxel_roi(&mut world);
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        roi.session_caches.mesh = Some(MeshCache {
            data: closed_tetra_mesh_data(),
            chunks: None,
        });
        roi.dirty_state.mesh.dirty = false;
        roi.dirty_state.mesh.built_from = roi.dirty_state.authoritative;
    }
    promote_current_mesh_cache_to_authority(&mut world, entity).unwrap();

    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        roi.mark_cache_dirty(RoiCacheKind::Voxel);
        roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
    }
    {
        let roi = world.get::<&Roi>(entity).unwrap();
        assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildVoxelCache));
    }
    process_mesh_voxel_rebuild_jobs(&mut world);
    let voxel_status = cache_status(&world, entity, RoiCacheKind::Voxel).unwrap();
    assert!(voxel_status.is_current);
    assert!(world
        .get::<&Roi>(entity)
        .unwrap()
        .voxel_cache()
        .is_some_and(|cache| cache.data.raw_data.iter().any(|value| *value != 0)));
    assert!(roi_voxel_stats(&world, entity).is_some_and(|stats| stats.occupied_voxels > 0));

    world
        .get::<&mut Roi>(entity)
        .unwrap()
        .mark_cache_dirty(RoiCacheKind::Contour);
    assert!(
        cache_status(&world, entity, RoiCacheKind::Contour)
            .unwrap()
            .is_dirty
    );
}

#[test]
fn test_mesh_voxel_rebuild_keeps_cache_stale_until_incremental_work_completes() {
    let mut world = World::new();
    let entity = spawn_sparse_voxel_roi(&mut world);
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        roi.session_caches.mesh = Some(MeshCache {
            data: closed_tetra_mesh_data(),
            chunks: None,
        });
        roi.dirty_state.mesh.dirty = false;
        roi.dirty_state.mesh.built_from = roi.dirty_state.authoritative;
    }
    promote_current_mesh_cache_to_authority(&mut world, entity).unwrap();
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        roi.mark_cache_dirty(RoiCacheKind::Voxel);
        roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
    }

    assert!(!process_mesh_voxel_rebuild_for_entity(
        &mut world,
        entity,
        None,
        Instant::now(),
        Duration::ZERO,
    ));
    assert!(world.get::<&MeshVoxelRebuildWork>(entity).is_ok());
    let roi = world.get::<&Roi>(entity).unwrap();
    assert_eq!(roi.running_job_kind(), Some(RoiJobKind::RebuildVoxelCache));
    assert!(!roi.is_cache_current(RoiCacheKind::Voxel));
    drop(roi);

    for _ in 0..10 {
        process_mesh_voxel_rebuild_jobs(&mut world);
        if world
            .get::<&Roi>(entity)
            .unwrap()
            .is_cache_current(RoiCacheKind::Voxel)
        {
            break;
        }
    }
    assert!(world
        .get::<&Roi>(entity)
        .unwrap()
        .is_cache_current(RoiCacheKind::Voxel));
    assert!(world.get::<&MeshVoxelRebuildWork>(entity).is_err());

    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        roi.mark_cache_dirty(RoiCacheKind::Voxel);
        roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
    }
    assert!(!process_mesh_voxel_rebuild_for_entity(
        &mut world,
        entity,
        None,
        Instant::now(),
        Duration::ZERO,
    ));
    replace_mesh_data(&mut world, entity, closed_tetra_mesh_data()).unwrap();
    process_mesh_voxel_rebuild_jobs(&mut world);
    let roi = world.get::<&Roi>(entity).unwrap();
    assert!(!roi.is_cache_current(RoiCacheKind::Voxel));
    assert_eq!(roi.running_job_kind(), None);
    assert_eq!(roi.queued_job_kind(), None);
    assert!(roi.job_metrics.discarded_count > 0);
    assert!(world.get::<&MeshVoxelRebuildWork>(entity).is_err());
}

#[test]
fn test_mesh_authority_keeps_direct_plane_contour_after_voxel_rebuild() {
    let mut world = World::new();
    let entity = spawn_sparse_voxel_roi(&mut world);
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        roi.session_caches.mesh = Some(MeshCache {
            data: closed_tetra_mesh_data(),
            chunks: None,
        });
        roi.dirty_state.mesh.dirty = false;
        roi.dirty_state.mesh.built_from = roi.dirty_state.authoritative;
    }
    promote_current_mesh_cache_to_authority(&mut world, entity).unwrap();
    let editor = world.spawn((EditorState::default(),));
    let deformed = crate::systems::mesh_editing::deform_mesh_surface_brush(
        world.get::<&Roi>(entity).unwrap().mesh_data().unwrap(),
        [0, 1, 3],
        [0.5, 0.0, 0.5],
        [0.25, 0.0, 0.0],
        1.5,
        1.0,
    );
    crate::app::roi::begin_mesh_edit_preview(&mut world, editor, entity, deformed.clone()).unwrap();
    commit_mesh_edit_preview(&mut world, editor).unwrap();
    assert_eq!(
        world.get::<&Roi>(entity).unwrap().mesh_data(),
        Some(&deformed)
    );

    let geometry = world
        .get::<&Roi>(entity)
        .unwrap()
        .voxel_cache()
        .unwrap()
        .data
        .geometry;
    let inverse_sqrt_two = std::f32::consts::FRAC_1_SQRT_2;
    let planes = [
        orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5, 0.5, 1.0 / 3.0], geometry)
            .unwrap(),
        PlaneDefinition {
            family: PlaneFamily::Oblique,
            origin_mm: [0.5, 0.5, 0.5],
            u_axis_mm: [inverse_sqrt_two, 0.0, -inverse_sqrt_two],
            v_axis_mm: [0.0, 1.0, 0.0],
            normal_mm: [inverse_sqrt_two, 0.0, inverse_sqrt_two],
        },
    ];
    let expected = planes.map(|plane| {
        let key = ContourViewKey::from_plane(plane);
        let expected = {
            let roi = world.get::<&Roi>(entity).unwrap();
            intersect_mesh_with_plane(roi.mesh_data().unwrap(), plane).unwrap()
        };
        assert!(!expected.slices.is_empty());
        let before_rebuild = ensure_contour_view_cache(&mut world, entity, &key);
        assert_eq!(before_rebuild.state, RepresentationRequestState::Current);
        assert_eq!(
            request_contour_view_state(&world, entity, &key)
                .request
                .state,
            RepresentationRequestState::Current,
            "a direct mesh contour must remain current while voxelization is queued"
        );
        (key, expected)
    });

    process_mesh_voxel_rebuild_jobs(&mut world);
    for (key, expected) in expected {
        assert_eq!(
            world
                .get::<&Roi>(entity)
                .unwrap()
                .contour_view_cache(&key)
                .unwrap()
                .state,
            CacheViewState::Current,
            "voxel resampling must not invalidate direct mesh contours"
        );
        let status = ensure_contour_view_cache(&mut world, entity, &key);
        assert_eq!(status.state, RepresentationRequestState::Current);
        let roi = world.get::<&Roi>(entity).unwrap();
        assert_eq!(
            roi.contour_view_cache(&key).unwrap().data,
            expected,
            "mesh authority must retain direct mesh-plane contours after voxelization"
        );
    }
}

#[test]
fn test_visible_inactive_mesh_roi_keeps_direct_contours_in_multiple_viewports() {
    let mut world = World::new();
    spawn_main_volume(&mut world, [1.0; 3], [0.0; 3]);
    let active_roi = spawn_sparse_voxel_roi(&mut world);
    world.spawn((EditorState {
        active_roi: Some(active_roi),
        ..EditorState::default()
    },));
    let cursor_uv = [0.5, 0.5, 0.25];
    world.spawn((Transform {
        position: cursor_uv,
    },));
    let mesh = closed_tetra_mesh_data();
    let mesh_roi = world.spawn((Roi::new_mesh(RoiId(305), "Mesh".to_string(), mesh.clone()),));
    let viewports = [
        (ViewMode::Axial, ViewportState::default()),
        (
            ViewMode::Oblique,
            ViewportState {
                user_rotation: glam::Quat::from_rotation_y(0.3).to_array(),
                ..ViewportState::default()
            },
        ),
    ];
    for (index, (mode, state)) in viewports.iter().enumerate() {
        world.spawn((
            Viewport {
                mode: *mode,
                rect: [0.0, 0.0, 800.0, 600.0],
                uniform_index: index as u32,
            },
            *state,
        ));
    }

    sync_roi_contour_view_caches_for_viewports(&mut world);
    let geometry = main_volume_geometry(&world).unwrap();
    for (mode, state) in viewports {
        let plane = crate::render::roi_views::displayed_plane_for_viewport(
            mode,
            cursor_uv,
            state.user_rotation,
            geometry,
        )
        .unwrap();
        let key = ContourViewKey::from_plane(plane);
        let expected = intersect_mesh_with_plane(&mesh, plane).unwrap();
        assert!(!expected.slices.is_empty());
        let roi = world.get::<&Roi>(mesh_roi).unwrap();
        let cache = roi.contour_view_cache(&key).unwrap();
        assert_eq!(cache.state, CacheViewState::Current);
        assert_eq!(cache.data, expected);
        drop(roi);
        let viewport = Viewport {
            mode,
            rect: [0.0, 0.0, 800.0, 600.0],
            uniform_index: 0,
        };
        assert!(crate::render::roi_views::contour_renderable_in_viewport(
            &world, &viewport, &state, cursor_uv, mesh_roi,
        ));
    }
}

#[test]
fn test_rotated_anisotropic_roi_keeps_direct_contours_through_mesh_resample() {
    let mut world = World::new();
    let geometry = VoxelGeometry::new(
        [8; 3],
        [0.7, 1.3, 2.1],
        [10.0, 20.0, 30.0],
        glam::Quat::from_rotation_y(0.4).to_array(),
    )
    .unwrap();
    let mut raw_data = vec![0; 8 * 8 * 8];
    for z in 2..=5 {
        for y in 2..=5 {
            for x in 2..=5 {
                raw_data[(z * 8 + y) * 8 + x] = 1;
            }
        }
    }
    let entity = world.spawn((Roi::new_voxel_with_cache(
        RoiId(306),
        "Rotated".to_string(),
        geometry,
        raw_data,
        None,
    ),));
    let editor = world.spawn((EditorState {
        active_roi: Some(entity),
        ..EditorState::default()
    },));
    world.spawn((
        Viewport {
            mode: ViewMode::ThreeD,
            rect: [0.0, 0.0, 800.0, 600.0],
            uniform_index: 0,
        },
        ViewportState::default(),
    ));
    for _ in 0..32 {
        advance_roi_work(&mut world, None);
        if world
            .get::<&Roi>(entity)
            .unwrap()
            .is_cache_current(RoiCacheKind::Mesh)
        {
            break;
        }
    }
    assert!(world
        .get::<&Roi>(entity)
        .unwrap()
        .is_cache_current(RoiCacheKind::Mesh));
    promote_current_mesh_cache_to_authority(&mut world, entity).unwrap();
    let original = world
        .get::<&Roi>(entity)
        .unwrap()
        .mesh_data()
        .unwrap()
        .clone();
    let seeds = original.faces[original.faces.len() / 2].vertex_indices;
    let anchor = original.vertices[seeds[0] as usize].world_mm;
    let deformed = crate::systems::mesh_editing::deform_mesh_surface_brush(
        &original,
        seeds,
        anchor,
        [0.1, 0.0, 0.0],
        2.0,
        1.0,
    );
    assert_ne!(deformed, original);
    crate::app::roi::begin_mesh_edit_preview(&mut world, editor, entity, deformed.clone()).unwrap();
    commit_mesh_edit_preview(&mut world, editor).unwrap();

    let planes = [
        orthogonal_plane_from_volume_uv(PlaneFamily::Axial, [0.5; 3], geometry).unwrap(),
        crate::convert::oblique_plane_from_view_rotation(
            [0.5; 3],
            glam::Quat::from_rotation_y(0.25).to_array(),
            geometry,
        )
        .unwrap(),
    ];
    let expected = planes.map(|plane| {
        let key = ContourViewKey::from_plane(plane);
        let direct = intersect_mesh_with_plane(&deformed, plane).unwrap();
        assert!(direct.has_loops());
        assert_eq!(
            ensure_contour_view_cache(&mut world, entity, &key).state,
            RepresentationRequestState::Current
        );
        (key, direct)
    });
    crate::app::roi::request_mesh_voxel_cache_rebuild(&mut world, entity).unwrap();
    for _ in 0..128 {
        advance_roi_work(&mut world, None);
        if world
            .get::<&Roi>(entity)
            .unwrap()
            .is_cache_current(RoiCacheKind::Voxel)
        {
            break;
        }
    }
    let roi = world.get::<&Roi>(entity).unwrap();
    assert!(roi.is_cache_current(RoiCacheKind::Voxel));
    assert!(matches!(
        roi.authoritative_data,
        RoiAuthoritativeData::Mesh(_)
    ));
    assert!(roi
        .voxel_cache()
        .unwrap()
        .data
        .raw_data
        .iter()
        .any(|value| *value != 0));
    for (key, direct) in expected {
        let cache = roi.contour_view_cache(&key).unwrap();
        assert_eq!(cache.state, CacheViewState::Current);
        assert_eq!(cache.data, direct);
    }
}

#[test]
fn test_create_contour_roi_from_voxel_roi_keeps_source_unchanged() {
    let mut world = World::new();
    let source = spawn_test_roi(&mut world);
    let source_before = {
        let roi = world.get::<&Roi>(source).unwrap();
        let RoiAuthoritativeData::Voxel(voxel) = &roi.authoritative_data else {
            panic!("expected voxel roi");
        };
        voxel.clone()
    };

    let _new = create_contour_roi_from_voxel_roi(&mut world, source, PlaneFamily::Axial)
        .expect("extraction should create contour roi");

    let roi = world.get::<&Roi>(source).unwrap();
    let RoiAuthoritativeData::Voxel(voxel_after) = &roi.authoritative_data else {
        panic!("source should remain voxel authoritative");
    };
    assert_eq!(*voxel_after, source_before);
    assert_eq!(roi.primary_representation(), PrimaryRepresentation::Voxel);
}

#[test]
fn test_create_contour_roi_from_voxel_roi_returns_contour_primary_roi() {
    let mut world = World::new();
    let source = spawn_test_roi(&mut world);

    let created = create_contour_roi_from_voxel_roi(&mut world, source, PlaneFamily::Axial)
        .expect("extraction should create contour roi");

    let roi = world.get::<&Roi>(created).unwrap();
    assert_eq!(roi.primary_representation(), PrimaryRepresentation::Contour);
    assert!(matches!(
        roi.authoritative_data,
        RoiAuthoritativeData::Contour(_)
    ));
    assert_eq!(roi.metadata.name, "Test (Axial Contour)");
}

#[test]
fn test_create_contour_roi_from_voxel_roi_populates_extractable_contours() {
    let mut world = World::new();
    let source = spawn_sparse_voxel_roi(&mut world);

    let created = create_contour_roi_from_voxel_roi(&mut world, source, PlaneFamily::Axial)
        .expect("extraction should create contour roi");

    let roi = world.get::<&Roi>(created).unwrap();
    let contour_data = roi.contour_data().expect("expected contour data");
    assert_eq!(contour_data.active_plane_family, PlaneFamily::Axial);
    assert!(contour_data.has_loops());
    let seeded_geometry = roi
        .voxel_cache()
        .expect("expected source geometry cache on extracted contour roi")
        .data
        .geometry;
    let source_geometry = world
        .get::<&Roi>(source)
        .ok()
        .and_then(|source_roi| source_roi.voxel_cache().map(|cache| cache.data.geometry))
        .expect("expected source geometry");
    assert_eq!(seeded_geometry, source_geometry);
}

#[test]
fn test_extracted_contour_roi_seeds_current_cpu_voxel_cache() {
    let mut world = World::new();
    let source = spawn_sparse_voxel_roi(&mut world);

    let created = create_contour_roi_from_voxel_roi(&mut world, source, PlaneFamily::Axial)
        .expect("extraction should create contour roi");

    let roi = world.get::<&Roi>(created).unwrap();
    assert!(roi.is_cache_current(RoiCacheKind::Voxel));
    assert!(roi.voxel_gpu_cache().is_none());
    assert_eq!(roi.queued_job_kind(), None);
}

#[test]
fn test_mesh_authoritative_roi_reports_current_mesh_representation() {
    let mut world = World::new();
    let entity = world.spawn((Roi::new_mesh(
        RoiId(1),
        "Mesh".to_string(),
        simple_mesh_data(),
    ),));

    let status = request_mesh_cache_state(&world, entity);

    assert_eq!(status.state, RepresentationRequestState::Current);
    assert_eq!(status.reason, None);
}

#[test]
fn test_create_contour_roi_from_voxel_roi_rejects_non_voxel_source() {
    let mut world = World::new();
    let source = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
    let result = create_contour_roi_from_voxel_roi(&mut world, source, PlaneFamily::Axial);
    assert_eq!(result, Err(VoxelContourCreationError::NotVoxelRoi));
}

#[test]
fn test_create_contour_roi_from_voxel_roi_rejects_missing_source() {
    let mut world = World::new();
    let result =
        create_contour_roi_from_voxel_roi(&mut world, hecs::Entity::DANGLING, PlaneFamily::Axial);
    assert_eq!(result, Err(VoxelContourCreationError::MissingRoi));
}

#[test]
fn test_create_contour_roi_from_voxel_roi_surfaces_extraction_errors() {
    let mut world = World::new();
    let source = spawn_test_roi(&mut world);
    let result = create_contour_roi_from_voxel_roi(&mut world, source, PlaneFamily::Oblique);
    assert_eq!(
        result,
        Err(VoxelContourCreationError::ExtractionFailed(
            VoxelContourExtractionError::UnsupportedPlaneFamily {
                family: PlaneFamily::Oblique
            }
        ))
    );
}

#[test]
fn test_create_mesh_roi_from_voxel_roi_keeps_source_unchanged() {
    let mut world = World::new();
    spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
    let source = spawn_sparse_voxel_roi(&mut world);
    let before = {
        let roi = world.get::<&Roi>(source).unwrap();
        match &roi.authoritative_data {
            RoiAuthoritativeData::Voxel(voxel) => voxel.clone(),
            RoiAuthoritativeData::Contour(_) | RoiAuthoritativeData::Mesh(_) => {
                panic!("source must remain voxel-primary")
            }
        }
    };

    let _mesh = create_mesh_roi_from_voxel_roi(&mut world, source)
        .expect("mesh ROI creation should succeed for voxel source");

    let after = {
        let roi = world.get::<&Roi>(source).unwrap();
        match &roi.authoritative_data {
            RoiAuthoritativeData::Voxel(voxel) => voxel.clone(),
            RoiAuthoritativeData::Contour(_) | RoiAuthoritativeData::Mesh(_) => {
                panic!("source must remain voxel-primary")
            }
        }
    };
    assert_eq!(before, after);
}

#[test]
fn test_create_mesh_roi_from_voxel_roi_returns_mesh_primary_roi() {
    let mut world = World::new();
    spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
    let source = spawn_sparse_voxel_roi(&mut world);

    let created = create_mesh_roi_from_voxel_roi(&mut world, source)
        .expect("mesh ROI creation should succeed for voxel source");
    let roi = world
        .get::<&Roi>(created)
        .expect("created ROI should exist");

    assert_eq!(roi.primary_representation(), PrimaryRepresentation::Mesh);
    assert!(roi.mesh_data().is_some());
    assert!(roi.voxel_cache().is_some());
}

#[test]
fn test_create_mesh_roi_from_voxel_roi_rejects_non_voxel_source() {
    let mut world = World::new();
    spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
    let source = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);

    let result = create_mesh_roi_from_voxel_roi(&mut world, source);
    assert_eq!(result, Err(VoxelMeshCreationError::NotVoxelRoi));
}

#[test]
fn test_create_mesh_roi_from_voxel_roi_uses_source_grid_without_main_volume() {
    let mut world = World::new();
    let source = spawn_sparse_voxel_roi(&mut world);
    let result = create_mesh_roi_from_voxel_roi(&mut world, source).unwrap();
    let roi = world.get::<&Roi>(result).unwrap();
    assert_eq!(roi.reference_geometry().dimensions(), [4, 4, 4]);
}

#[test]
fn test_voxel_data_for_display_surface_extraction_returns_authoritative_when_geometry_matches() {
    let mut world = World::new();
    spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
    let source = spawn_sparse_voxel_roi(&mut world);
    let source_voxel = world
        .get::<&Roi>(source)
        .ok()
        .and_then(|roi| roi.voxel_cache().map(|cache| cache.data.clone()))
        .expect("source voxel");

    let display_voxel =
        voxel_data_for_display_surface_extraction(&world, source).expect("display source");
    assert_eq!(display_voxel, source_voxel);
}

#[test]
fn test_voxel_data_for_display_surface_extraction_accepts_mismatched_geometry() {
    let mut world = World::new();
    spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
    let source = world.spawn((Roi::new_voxel_with_cache(
        RoiId(42),
        "Mismatched".to_string(),
        VoxelGeometry::new(
            [4, 4, 4],
            [2.0, 2.0, 2.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap(),
        {
            let mut raw = vec![0_u8; 64];
            raw[21] = 1;
            raw
        },
        None,
    ),));

    let result = voxel_data_for_display_surface_extraction(&world, source);
    assert!(result.is_ok());
}

#[test]
fn test_create_mesh_roi_from_voxel_roi_accepts_mismatched_geometry() {
    let mut world = World::new();
    spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
    let source = world.spawn((Roi::new_voxel_with_cache(
        RoiId(43),
        "Mismatched".to_string(),
        VoxelGeometry::new(
            [4, 4, 4],
            [2.0, 2.0, 2.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap(),
        {
            let mut raw = vec![0_u8; 64];
            raw[21] = 1;
            raw
        },
        None,
    ),));

    let result = create_mesh_roi_from_voxel_roi(&mut world, source);
    assert!(result.is_ok());
}

#[test]
fn test_create_mesh_roi_from_voxel_roi_spawns_mesh_when_geometry_is_mismatched() {
    let mut world = World::new();
    spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
    let source = world.spawn((Roi::new_voxel_with_cache(
        RoiId(44),
        "Mismatched".to_string(),
        VoxelGeometry::new(
            [4, 4, 4],
            [2.0, 2.0, 2.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap(),
        {
            let mut raw = vec![0_u8; 64];
            raw[21] = 1;
            raw
        },
        None,
    ),));

    let roi_count_before = world.query::<&Roi>().iter().count();
    let result = create_mesh_roi_from_voxel_roi(&mut world, source);
    let roi_count_after = world.query::<&Roi>().iter().count();

    assert!(result.is_ok());
    assert!(roi_count_after > roi_count_before);
}

#[test]
fn test_create_mesh_roi_from_contour_roi_succeeds_with_current_voxel_cache() {
    let mut world = World::new();
    let source = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
    let source_voxel = VoxelData {
        geometry: VoxelGeometry::new(
            [4, 4, 4],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap(),
        raw_data: {
            let mut raw = vec![0_u8; 64];
            raw[(2 * 4 + 1) * 4 + 1] = 1;
            raw
        },
    };
    {
        let mut roi = world.get::<&mut Roi>(source).unwrap();
        roi.session_caches.voxel = Some(VoxelCache {
            data: source_voxel,
            gpu_resources: None,
        });
        roi.dirty_state.voxel.dirty = false;
        roi.dirty_state.voxel.built_from = roi.dirty_state.authoritative;
    }

    let created = create_mesh_roi_from_contour_roi(&mut world, source)
        .expect("contour source should succeed when current voxel cache exists");
    let roi = world.get::<&Roi>(created).unwrap();
    assert_eq!(roi.primary_representation(), PrimaryRepresentation::Mesh);
    assert!(roi.mesh_data().is_some());
    assert!(roi.voxel_cache().is_some());
}

#[test]
fn test_create_mesh_roi_from_contour_roi_requires_current_voxel_cache() {
    let mut world = World::new();
    let source = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);

    let result = create_mesh_roi_from_contour_roi(&mut world, source);
    assert_eq!(
        result,
        Err(ContourMeshCreationError::MissingCurrentVoxelCache)
    );
}

#[test]
fn test_extracted_contour_roi_supports_replace_contour_data_edit_path() {
    let mut world = World::new();
    let source = spawn_sparse_voxel_roi(&mut world);
    let extracted = create_contour_roi_from_voxel_roi(&mut world, source, PlaneFamily::Axial)
        .expect("expected extracted contour roi");

    let replacement = ContourData {
        active_plane_family: PlaneFamily::Axial,
        slices: vec![ContourSlice {
            plane: test_plane_definition(PlaneFamily::Axial),
            loops: vec![ContourLoop {
                points: vec![
                    ContourPoint {
                        local_mm: [0.0, 0.0],
                    },
                    ContourPoint {
                        local_mm: [2.0, 0.0],
                    },
                    ContourPoint {
                        local_mm: [1.0, 2.0],
                    },
                ],
                is_closed: true,
            }],
        }],
    };

    let result = replace_contour_data(&mut world, extracted, replacement.clone());
    assert_eq!(result, Ok(()));
    let roi = world.get::<&Roi>(extracted).unwrap();
    assert_eq!(roi.contour_data(), Some(&replacement));
}

#[test]
fn test_extracted_contour_roi_edit_queues_rebuild_voxel_cache_job() {
    let mut world = World::new();
    let source = spawn_sparse_voxel_roi(&mut world);
    let extracted = create_contour_roi_from_voxel_roi(&mut world, source, PlaneFamily::Axial)
        .expect("expected extracted contour roi");

    let replacement = ContourData {
        active_plane_family: PlaneFamily::Axial,
        slices: vec![ContourSlice {
            plane: test_plane_definition(PlaneFamily::Axial),
            loops: vec![ContourLoop {
                points: vec![
                    ContourPoint {
                        local_mm: [0.0, 0.0],
                    },
                    ContourPoint {
                        local_mm: [1.5, 0.0],
                    },
                    ContourPoint {
                        local_mm: [0.0, 1.5],
                    },
                ],
                is_closed: true,
            }],
        }],
    };

    replace_contour_data(&mut world, extracted, replacement).expect("replace should succeed");
    let roi = world.get::<&Roi>(extracted).unwrap();
    assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildVoxelCache));
    assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
}

#[test]
fn test_contour_commit_converges_voxel_and_mesh_through_work_coordinator() {
    let mut world = World::new();
    spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
    let source = spawn_sparse_voxel_roi(&mut world);
    let extracted = create_contour_roi_from_voxel_roi(&mut world, source, PlaneFamily::Axial)
        .expect("expected extracted contour roi");

    let replacement = square_contour_data_for_main_volume(&world, 1.2);
    replace_contour_data(&mut world, extracted, replacement.clone())
        .expect("replace should succeed");
    for _ in 0..32 {
        if !advance_roi_work(&mut world, None).pending {
            break;
        }
    }

    let roi = world.get::<&Roi>(extracted).unwrap();
    assert_eq!(roi.primary_representation(), PrimaryRepresentation::Contour);
    assert_eq!(roi.contour_data(), Some(&replacement));
    assert!(roi.is_cache_current(RoiCacheKind::Voxel));
    assert!(roi.voxel_cache().is_some());
    assert!(roi
        .voxel_cache()
        .expect("expected derived voxel cache")
        .data
        .raw_data
        .iter()
        .any(|v| *v != 0));
    assert_eq!(roi.running_job_kind(), None);
    assert_eq!(roi.queued_job_kind(), None);
    assert!(roi.is_cache_current(RoiCacheKind::Mesh));
    assert!(roi.mesh_cache().is_some_and(|cache| cache.chunks.is_some()));
}

#[test]
fn test_set_active_contour_plane_family_updates_empty_contour_and_marks_derived_dirty() {
    let mut world = World::new();
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        roi.dirty_state.authoritative_dirty = false;
        roi.dirty_state.voxel.dirty = false;
        roi.dirty_state.contour.dirty = false;
        roi.dirty_state.mesh.dirty = false;
        roi.dirty_state.voxel.built_from = roi.dirty_state.authoritative;
        roi.dirty_state.contour.built_from = roi.dirty_state.authoritative;
        roi.dirty_state.mesh.built_from = roi.dirty_state.authoritative;
    }

    let result = set_active_contour_plane_family(&mut world, entity, PlaneFamily::Coronal);
    assert_eq!(result, Ok(()));

    let roi = world.get::<&Roi>(entity).unwrap();
    let contour = roi.contour_data().unwrap();
    assert_eq!(contour.active_plane_family, PlaneFamily::Coronal);
    assert!(roi.dirty_state.authoritative_dirty);
    assert_eq!(roi.dirty_state.authoritative.shape, 2);
    assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
    assert!(!roi.is_cache_dirty(RoiCacheKind::Contour));
    assert!(roi.is_cache_dirty(RoiCacheKind::Mesh));
}

#[test]
fn test_set_active_contour_plane_family_is_noop_when_unchanged() {
    let mut world = World::new();
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Sagittal, false);
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        roi.dirty_state.authoritative_dirty = false;
        roi.dirty_state.voxel.dirty = false;
        roi.dirty_state.contour.dirty = false;
        roi.dirty_state.mesh.dirty = false;
        roi.dirty_state.voxel.built_from = roi.dirty_state.authoritative;
        roi.dirty_state.contour.built_from = roi.dirty_state.authoritative;
        roi.dirty_state.mesh.built_from = roi.dirty_state.authoritative;
    }

    let result = set_active_contour_plane_family(&mut world, entity, PlaneFamily::Sagittal);
    assert_eq!(result, Ok(()));

    let roi = world.get::<&Roi>(entity).unwrap();
    let contour = roi.contour_data().unwrap();
    assert_eq!(contour.active_plane_family, PlaneFamily::Sagittal);
    assert!(!roi.dirty_state.authoritative_dirty);
    assert_eq!(roi.dirty_state.authoritative.shape, 1);
    assert!(!roi.is_cache_dirty(RoiCacheKind::Voxel));
    assert!(!roi.is_cache_dirty(RoiCacheKind::Contour));
    assert!(!roi.is_cache_dirty(RoiCacheKind::Mesh));
}

#[test]
fn test_set_active_contour_plane_family_rejects_non_empty_contour() {
    let mut world = World::new();
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, true);

    let result = set_active_contour_plane_family(&mut world, entity, PlaneFamily::Coronal);
    assert_eq!(
        result,
        Err(ContourPlaneFamilySwitchError::RequiresConversion)
    );

    let roi = world.get::<&Roi>(entity).unwrap();
    let contour = roi.contour_data().unwrap();
    assert_eq!(contour.active_plane_family, PlaneFamily::Axial);
}

#[test]
fn test_set_active_contour_plane_family_rejects_voxel_roi() {
    let mut world = World::new();
    let entity = spawn_test_roi(&mut world);

    let result = set_active_contour_plane_family(&mut world, entity, PlaneFamily::Oblique);
    assert_eq!(result, Err(ContourPlaneFamilySwitchError::NotContourRoi));
}

#[test]
fn test_replace_contour_data_updates_authoritative_state_and_queues_voxel_rebuild() {
    let mut world = World::new();
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        roi.dirty_state.authoritative_dirty = false;
        roi.dirty_state.voxel.dirty = false;
        roi.dirty_state.contour.dirty = false;
        roi.dirty_state.mesh.dirty = false;
        roi.dirty_state.voxel.built_from = roi.dirty_state.authoritative;
        roi.dirty_state.contour.built_from = roi.dirty_state.authoritative;
        roi.dirty_state.mesh.built_from = roi.dirty_state.authoritative;
    }

    let replacement = ContourData {
        active_plane_family: PlaneFamily::Coronal,
        slices: vec![ContourSlice {
            plane: test_plane_definition(PlaneFamily::Coronal),
            loops: vec![ContourLoop {
                points: vec![
                    ContourPoint {
                        local_mm: [0.0, 0.0],
                    },
                    ContourPoint {
                        local_mm: [2.0, 0.0],
                    },
                    ContourPoint {
                        local_mm: [0.0, 2.0],
                    },
                ],
                is_closed: true,
            }],
        }],
    };

    let result = replace_contour_data(&mut world, entity, replacement.clone());
    assert_eq!(result, Ok(()));

    {
        let roi = world.get::<&Roi>(entity).unwrap();
        assert_eq!(roi.contour_data(), Some(&replacement));
        assert!(roi.dirty_state.authoritative_dirty);
        assert_eq!(roi.dirty_state.authoritative.shape, 2);
        assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert!(!roi.is_cache_dirty(RoiCacheKind::Contour));
        assert!(roi.is_cache_dirty(RoiCacheKind::Mesh));
        assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildVoxelCache));
    }

    assert_eq!(
        begin_next_job(&mut world, entity),
        Some(RoiJobKind::RebuildVoxelCache)
    );

    let status_after_begin = cache_status(&world, entity, RoiCacheKind::Voxel).unwrap();
    assert!(status_after_begin.is_dirty);
    assert!(!status_after_begin.is_current);
}

#[test]
fn test_contour_edit_history_undo_redo_restores_authority_and_requeues_rebuild() {
    let mut world = World::new();
    let editor = world.spawn((EditorState::default(),));
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
    world.get::<&mut EditorState>(editor).unwrap().active_roi = Some(entity);
    let original = world
        .get::<&Roi>(entity)
        .unwrap()
        .contour_data()
        .unwrap()
        .clone();
    let replacement = ContourData {
        active_plane_family: PlaneFamily::Coronal,
        slices: vec![ContourSlice {
            plane: test_plane_definition(PlaneFamily::Coronal),
            loops: vec![ContourLoop {
                points: vec![
                    ContourPoint {
                        local_mm: [0.0, 0.0],
                    },
                    ContourPoint {
                        local_mm: [2.0, 0.0],
                    },
                    ContourPoint {
                        local_mm: [0.0, 2.0],
                    },
                ],
                is_closed: true,
            }],
        }],
    };

    replace_contour_data_with_history(&mut world, entity, replacement.clone()).unwrap();
    let generation_after_commit = world
        .get::<&Roi>(entity)
        .unwrap()
        .dirty_state
        .authoritative
        .shape;
    {
        let history = world.get::<&Roi>(entity).unwrap().history.clone();
        assert_eq!(history.undo.len(), 1);
        assert!(history.redo.is_empty());
    }

    assert_eq!(undo_roi_edit(&mut world, editor), Ok(entity));
    {
        let roi = world.get::<&Roi>(entity).unwrap();
        assert_eq!(roi.contour_data(), Some(&original));
        assert_eq!(
            roi.dirty_state.authoritative.shape,
            generation_after_commit + 1
        );
        assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildVoxelCache));
        assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert!(roi.is_cache_dirty(RoiCacheKind::Mesh));
    }
    {
        let history = world.get::<&Roi>(entity).unwrap().history.clone();
        assert!(history.undo.is_empty());
        assert_eq!(history.redo.len(), 1);
    }

    assert_eq!(redo_roi_edit(&mut world, editor), Ok(entity));
    {
        let roi = world.get::<&Roi>(entity).unwrap();
        assert_eq!(roi.contour_data(), Some(&replacement));
        assert_eq!(
            roi.dirty_state.authoritative.shape,
            generation_after_commit + 2
        );
        assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildVoxelCache));
    }
    let history = world.get::<&Roi>(entity).unwrap().history.clone();
    assert_eq!(history.undo.len(), 1);
    assert!(history.redo.is_empty());
}

#[test]
fn test_slice_local_contour_history_preserves_dirty_plane_for_undo_and_redo() {
    let mut world = World::new();
    let editor = world.spawn((EditorState::default(),));
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, true);
    world.get::<&mut EditorState>(editor).unwrap().active_roi = Some(entity);
    let original = world
        .get::<&Roi>(entity)
        .unwrap()
        .contour_data()
        .unwrap()
        .clone();
    let plane = original.slices[0].plane;
    let expected_dirty_region = RoiDirtyRegion::ContourSlice(ContourSliceKey::from_plane(plane));
    let mut replacement = original.clone();
    replacement.slices[0].loops[0].points[1].local_mm[0] += 0.25;

    replace_contour_data_for_slice_with_history(&mut world, entity, replacement, plane).unwrap();
    assert_eq!(undo_roi_edit(&mut world, editor), Ok(entity));
    assert_eq!(
        world.get::<&Roi>(entity).unwrap().job_state.pending[0].dirty_region,
        expected_dirty_region
    );

    assert_eq!(redo_roi_edit(&mut world, editor), Ok(entity));
    assert_eq!(
        world.get::<&Roi>(entity).unwrap().job_state.pending[0].dirty_region,
        expected_dirty_region
    );
}

#[test]
fn test_new_contour_slice_history_uses_full_rebuild_for_safe_undo() {
    let mut world = World::new();
    let editor = world.spawn((EditorState::default(),));
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
    world.get::<&mut EditorState>(editor).unwrap().active_roi = Some(entity);
    let plane = test_plane_definition(PlaneFamily::Axial);
    let replacement = ContourData {
        active_plane_family: PlaneFamily::Axial,
        slices: vec![ContourSlice {
            plane,
            loops: Vec::new(),
        }],
    };

    replace_contour_data_for_slice_with_history(&mut world, entity, replacement, plane).unwrap();
    assert_eq!(undo_roi_edit(&mut world, editor), Ok(entity));
    assert_eq!(
        world.get::<&Roi>(entity).unwrap().job_state.pending[0].dirty_region,
        RoiDirtyRegion::Full
    );
}

#[test]
fn test_noop_contour_history_commit_does_not_advance_generation_or_record_history() {
    let mut world = World::new();
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, true);
    let contour = world
        .get::<&Roi>(entity)
        .unwrap()
        .contour_data()
        .unwrap()
        .clone();
    let generation_before = world
        .get::<&Roi>(entity)
        .unwrap()
        .dirty_state
        .authoritative
        .shape;

    replace_contour_data_with_history(&mut world, entity, contour).unwrap();

    let roi = world.get::<&Roi>(entity).unwrap();
    assert_eq!(roi.dirty_state.authoritative.shape, generation_before);
    drop(roi);
    let history = world.get::<&Roi>(entity).unwrap().history.clone();
    assert!(history.undo.is_empty());
    assert!(history.redo.is_empty());
}

#[test]
fn test_new_contour_commit_after_undo_clears_redo_history() {
    let mut world = World::new();
    let editor = world.spawn((EditorState::default(),));
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
    world.get::<&mut EditorState>(editor).unwrap().active_roi = Some(entity);
    let first = ContourData {
        active_plane_family: PlaneFamily::Axial,
        slices: vec![ContourSlice {
            plane: test_plane_definition(PlaneFamily::Axial),
            loops: Vec::new(),
        }],
    };
    let second = ContourData {
        active_plane_family: PlaneFamily::Coronal,
        slices: vec![ContourSlice {
            plane: test_plane_definition(PlaneFamily::Coronal),
            loops: Vec::new(),
        }],
    };

    replace_contour_data_with_history(&mut world, entity, first).unwrap();
    undo_roi_edit(&mut world, editor).unwrap();
    replace_contour_data_with_history(&mut world, entity, second.clone()).unwrap();

    let roi = world.get::<&Roi>(entity).unwrap();
    assert_eq!(roi.contour_data(), Some(&second));
    drop(roi);
    let history = world.get::<&Roi>(entity).unwrap().history.clone();
    assert_eq!(history.undo.len(), 1);
    assert!(history.redo.is_empty());
    assert_eq!(
        redo_roi_edit(&mut world, editor),
        Err(RoiEditHistoryError::NoRedo)
    );
}

#[test]
fn test_replace_contour_data_rejects_voxel_roi() {
    let mut world = World::new();
    let entity = spawn_test_roi(&mut world);

    let result = replace_contour_data(
        &mut world,
        entity,
        ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: Vec::new(),
        },
    );
    assert_eq!(result, Err(ContourMutationError::NotContourRoi));
}

#[test]
fn test_replace_contour_data_rejects_missing_roi() {
    let mut world = World::new();
    let missing = hecs::Entity::DANGLING;

    let result = replace_contour_data(
        &mut world,
        missing,
        ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: Vec::new(),
        },
    );
    assert_eq!(result, Err(ContourMutationError::MissingRoi));
}

#[test]
fn test_process_contour_voxel_rebuild_jobs_builds_current_voxel_cache_for_contour_roi() {
    let mut world = World::new();
    spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
    let replacement = square_contour_data_for_main_volume(&world, 1.4);
    replace_contour_data(&mut world, entity, replacement.clone()).unwrap();
    world
        .get::<&mut Roi>(entity)
        .unwrap()
        .job_metrics
        .last_cpu_cache_install_ms = 42.0;

    process_contour_voxel_rebuild_jobs(&mut world);

    let roi = world.get::<&Roi>(entity).unwrap();
    assert_eq!(roi.contour_data(), Some(&replacement));
    assert!(roi.is_cache_current(RoiCacheKind::Voxel));
    assert_eq!(roi.running_job_kind(), None);
    assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildMeshCache));
    let voxel_cache = roi.voxel_cache().expect("voxel cache should exist");
    assert_eq!(voxel_cache.data.geometry.dimensions, [4, 4, 4]);
    assert!(voxel_cache.data.raw_data.iter().any(|v| *v != 0));
    assert!(voxel_cache.gpu_resources.is_none());
    assert_ne!(roi.job_metrics.last_cpu_cache_install_ms, 42.0);
}

#[test]
fn test_slice_commit_updates_retained_voxel_cache_for_coronal_and_sagittal_contours() {
    for family in [PlaneFamily::Coronal, PlaneFamily::Sagittal] {
        let mut world = World::new();
        spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
        let entity = spawn_test_contour_roi(&mut world, family, false);
        seed_current_voxel_cache_for_contour_roi(&mut world, entity);
        {
            let mut roi = world.get::<&mut Roi>(entity).unwrap();
            roi.voxel_cache_mut().unwrap().data.raw_data[0] = 1;
        }

        let geometry = main_volume_geometry(&world).unwrap();
        let plane = orthogonal_plane_from_volume_uv(family, [0.5, 0.5, 0.5], geometry)
            .expect("orthogonal edit plane");
        let replacement = ContourData {
            active_plane_family: family,
            slices: vec![ContourSlice {
                plane,
                loops: vec![ContourLoop {
                    points: vec![
                        ContourPoint {
                            local_mm: [-1.0, -1.0],
                        },
                        ContourPoint {
                            local_mm: [1.0, -1.0],
                        },
                        ContourPoint {
                            local_mm: [1.0, 1.0],
                        },
                        ContourPoint {
                            local_mm: [-1.0, 1.0],
                        },
                    ],
                    is_closed: true,
                }],
            }],
        };

        replace_contour_data_for_slice(&mut world, entity, replacement, plane).unwrap();
        process_contour_voxel_rebuild_jobs(&mut world);

        let roi = world.get::<&Roi>(entity).unwrap();
        assert!(roi.is_cache_current(RoiCacheKind::Voxel), "{family:?}");
        let voxel = &roi.voxel_cache().unwrap().data;
        assert_eq!(
            voxel.raw_data[0], 1,
            "unrelated slab changed for {family:?}"
        );
        let depth_axis = match family {
            PlaneFamily::Coronal => 1,
            PlaneFamily::Sagittal => 0,
            _ => unreachable!(),
        };
        let depth = world_mm_to_voxel_index(plane.origin_mm, geometry)[depth_axis].round() as u32;
        let mut occupied_on_edited_slab = 0;
        for z in 0..4 {
            for y in 0..4 {
                for x in 0..4 {
                    let index = [x, y, z];
                    let linear = ((z * 4 + y) * 4 + x) as usize;
                    if index[depth_axis] == depth && voxel.raw_data[linear] != 0 {
                        occupied_on_edited_slab += 1;
                    }
                }
            }
        }
        assert!(occupied_on_edited_slab > 0, "{family:?}");

        let mesh_request = roi
            .job_state
            .pending
            .iter()
            .find(|request| request.kind == RoiJobKind::RebuildMeshCache)
            .expect("mesh rebuild queued");
        let RoiDirtyRegion::VoxelAabb { min, max } = mesh_request.dirty_region else {
            panic!("expected slab-local mesh rebuild for {family:?}");
        };
        assert_eq!(min[depth_axis], depth, "{family:?}");
        assert_eq!(max[depth_axis], depth + 1, "{family:?}");
    }
}

#[test]
fn test_process_contour_voxel_rebuild_jobs_uses_roi_reference_grid_without_main_volume() {
    let mut world = World::new();
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, true);
    let contour_before = world
        .get::<&Roi>(entity)
        .unwrap()
        .contour_data()
        .unwrap()
        .clone();
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
    }

    process_contour_voxel_rebuild_jobs(&mut world);

    let roi = world.get::<&Roi>(entity).unwrap();
    assert_eq!(roi.contour_data(), Some(&contour_before));
    assert!(roi.is_cache_current(RoiCacheKind::Voxel));
    assert_eq!(roi.running_job_kind(), None);
    assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildMeshCache));
}

#[test]
fn test_process_contour_voxel_rebuild_jobs_discards_stale_generation_results() {
    let mut world = World::new();
    spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
    let replacement = square_contour_data_for_main_volume(&world, 1.2);
    replace_contour_data(&mut world, entity, replacement).unwrap();

    process_contour_voxel_rebuild_jobs_with_hook(
        &mut world,
        |world, hook_entity| {
            if hook_entity != entity {
                return;
            }
            let mut roi = world.get::<&mut Roi>(hook_entity).unwrap();
            if let RoiAuthoritativeData::Contour(contour) = &mut roi.authoritative_data {
                contour.slices.clear();
            }
            roi.mark_contour_authoritative_changed();
        },
        None,
        None,
    );

    let roi = world.get::<&Roi>(entity).unwrap();
    assert!(!roi.is_cache_current(RoiCacheKind::Voxel));
    assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
    assert_eq!(roi.running_job_kind(), None);
    assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildVoxelCache));
}

#[test]
fn test_process_contour_voxel_rebuild_jobs_clears_running_state_on_success() {
    let mut world = World::new();
    spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
    let replacement = square_contour_data_for_main_volume(&world, 1.0);
    replace_contour_data(&mut world, entity, replacement).unwrap();

    process_contour_voxel_rebuild_jobs(&mut world);

    let roi = world.get::<&Roi>(entity).unwrap();
    assert_eq!(roi.running_job_kind(), None);
    assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildMeshCache));
    assert!(roi.is_cache_current(RoiCacheKind::Voxel));
    assert_eq!(
        roi.cache_generation(RoiCacheKind::Voxel),
        roi.dirty_state.authoritative.shape
    );
}

#[test]
fn test_oblique_dirty_slice_rebuild_preserves_other_authoritative_planes() {
    let mut world = World::new();
    spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
    let make_slice = |z: f32| ContourSlice {
        plane: PlaneDefinition {
            family: PlaneFamily::Oblique,
            origin_mm: [0.0, 0.0, z],
            u_axis_mm: [1.0, 0.0, 0.0],
            v_axis_mm: [0.0, 1.0, 0.0],
            normal_mm: [0.0, 0.0, 1.0],
        },
        loops: vec![ContourLoop {
            points: vec![
                ContourPoint {
                    local_mm: [0.5, 0.5],
                },
                ContourPoint {
                    local_mm: [2.5, 0.5],
                },
                ContourPoint {
                    local_mm: [2.5, 2.5],
                },
                ContourPoint {
                    local_mm: [0.5, 2.5],
                },
            ],
            is_closed: true,
        }],
    };
    let first_slice = make_slice(1.0);
    let first_key = ContourSliceKey::from_plane(first_slice.plane);
    let geometry = VoxelGeometry::new(
        [4, 4, 4],
        [1.0, 1.0, 1.0],
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap();
    let entity = world.spawn((Roi::new_contour_with_geometry(
        RoiId(101),
        "Oblique".to_string(),
        geometry,
        ContourData {
            active_plane_family: PlaneFamily::Oblique,
            slices: vec![first_slice, make_slice(2.0)],
        },
    ),));
    seed_current_voxel_cache_for_contour_roi(&mut world, entity);
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        let source_generation = roi.dirty_state.authoritative.shape;
        roi.enqueue_job(RoiJobRequest {
            kind: RoiJobKind::RebuildVoxelCache,
            source_generation,
            preview_revision: None,
            priority: RoiJobPriority::VisibleCommitted,
            dirty_region: RoiDirtyRegion::ContourSlice(first_key),
        });
    }

    process_contour_voxel_rebuild_jobs(&mut world);

    let roi = world.get::<&Roi>(entity).unwrap();
    let raw = &roi.voxel_cache().unwrap().data.raw_data;
    let center_index = |z: usize| z * 16 + 5;
    assert_ne!(raw[center_index(1)], 0);
    assert_ne!(raw[center_index(2)], 0);
}

#[test]
fn test_process_contour_voxel_rebuild_jobs_hard_raster_failure_does_not_requeue() {
    let mut world = World::new();
    spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);

    let invalid_contour = ContourData {
        active_plane_family: PlaneFamily::Axial,
        slices: vec![ContourSlice {
            plane: PlaneDefinition {
                family: PlaneFamily::Axial,
                origin_mm: [0.0, 0.0, 0.0],
                u_axis_mm: [1.0, 0.0, 0.0],
                v_axis_mm: [0.0, 1.0, 0.0],
                normal_mm: [0.0, 0.0, 0.0],
            },
            loops: vec![ContourLoop {
                points: vec![
                    ContourPoint {
                        local_mm: [0.0, 0.0],
                    },
                    ContourPoint {
                        local_mm: [1.0, 0.0],
                    },
                    ContourPoint {
                        local_mm: [0.0, 1.0],
                    },
                ],
                is_closed: true,
            }],
        }],
    };
    replace_contour_data(&mut world, entity, invalid_contour).unwrap();

    process_contour_voxel_rebuild_jobs(&mut world);

    let roi = world.get::<&Roi>(entity).unwrap();
    assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
    assert_eq!(roi.running_job_kind(), None);
    assert_eq!(roi.queued_job_kind(), None);
}

#[test]
fn test_request_contour_view_state_reports_active_family_editable() {
    let mut world = World::new();
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
    let key = ContourViewKey::from_plane(test_plane_definition(PlaneFamily::Axial));

    let status = request_contour_view_state(&world, entity, &key);
    assert_eq!(status.request.state, RepresentationRequestState::Current);
    assert!(status.editable);
    assert!(!status.promotable);
}

#[test]
fn test_request_contour_view_state_reports_derived_promotable_when_current() {
    let mut world = World::new();
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
    seed_current_voxel_cache_for_contour_roi(&mut world, entity);
    let key = ContourViewKey::from_plane(test_plane_definition(PlaneFamily::Coronal));
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        let gen = roi.dirty_state.authoritative;
        let mut promoted_data = roi.contour_data().unwrap().clone();
        promoted_data.active_plane_family = PlaneFamily::Coronal;
        roi.upsert_contour_view_cache(key.clone(), promoted_data, gen, CacheViewState::Current);
    }

    let status = request_contour_view_state(&world, entity, &key);
    assert_eq!(status.request.state, RepresentationRequestState::Current);
    assert!(!status.editable);
    assert!(status.promotable);
}

#[test]
fn test_voxel_primary_roi_can_build_and_request_orthogonal_contour_view_cache() {
    let mut world = World::new();
    let entity = spawn_sparse_voxel_roi(&mut world);
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        roi.dirty_state.voxel.dirty = false;
        roi.dirty_state.voxel.built_from = roi.dirty_state.authoritative;
    }
    let geometry = {
        let roi = world.get::<&Roi>(entity).unwrap();
        roi.voxel_cache().unwrap().data.geometry
    };
    let plane =
        orthogonal_plane_from_volume_uv(PlaneFamily::Coronal, [0.5, 2.0 / 3.0, 0.5], geometry)
            .unwrap();
    let key = ContourViewKey::from_plane(plane);

    let build_status = ensure_contour_view_cache(&mut world, entity, &key);
    assert_eq!(build_status.state, RepresentationRequestState::Current);

    let status = request_contour_view_state(&world, entity, &key);
    assert_eq!(status.request.state, RepresentationRequestState::Current);
    assert!(!status.editable);
    assert!(!status.promotable);

    let roi = world.get::<&Roi>(entity).unwrap();
    let cache = roi
        .contour_view_cache(&key)
        .expect("expected derived contour view cache for voxel roi");
    assert_eq!(cache.state, CacheViewState::Current);
    assert_eq!(cache.data.active_plane_family, PlaneFamily::Coronal);
}

#[test]
fn test_ensure_contour_view_cache_builds_orthogonal_view_from_current_voxel_cache() {
    let mut world = World::new();
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        let mut raw_data = vec![0_u8; 64];
        raw_data[(2 * 4 + 1) * 4 + 1] = 1;
        roi.session_caches.voxel = Some(VoxelCache {
            data: VoxelData {
                geometry: VoxelGeometry::new(
                    [4, 4, 4],
                    [1.0, 1.0, 1.0],
                    [0.0, 0.0, 0.0],
                    [0.0, 0.0, 0.0, 1.0],
                )
                .unwrap(),
                raw_data,
            },
            gpu_resources: None,
        });
        roi.dirty_state.voxel.dirty = false;
        roi.dirty_state.voxel.built_from = roi.dirty_state.authoritative;
    }
    let plane = orthogonal_plane_from_volume_uv(
        PlaneFamily::Coronal,
        [0.5, 2.0 / 3.0, 0.5],
        VoxelGeometry::new(
            [4, 4, 4],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap(),
    )
    .unwrap();
    let key = ContourViewKey::from_plane(plane);

    let status = ensure_contour_view_cache(&mut world, entity, &key);

    assert_eq!(status.state, RepresentationRequestState::Current);
    let roi = world.get::<&Roi>(entity).unwrap();
    let cache = roi
        .contour_view_cache(&key)
        .expect("expected derived contour view cache");
    assert_eq!(cache.state, CacheViewState::Current);
    assert_eq!(cache.built_from, roi.dirty_state.authoritative);
    assert_eq!(cache.data.active_plane_family, PlaneFamily::Coronal);
}

#[test]
fn test_ensure_contour_view_cache_reuses_current_matching_generation_cache() {
    let mut world = World::new();
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
    seed_current_voxel_cache_for_contour_roi(&mut world, entity);
    let key = ContourViewKey::from_plane(test_plane_definition(PlaneFamily::Coronal));
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        let generation = roi.dirty_state.authoritative;
        roi.upsert_contour_view_cache(
            key.clone(),
            ContourData {
                active_plane_family: PlaneFamily::Coronal,
                slices: vec![ContourSlice {
                    plane: test_plane_definition(PlaneFamily::Coronal),
                    loops: vec![ContourLoop {
                        points: vec![
                            ContourPoint {
                                local_mm: [10.0, 10.0],
                            },
                            ContourPoint {
                                local_mm: [12.0, 10.0],
                            },
                            ContourPoint {
                                local_mm: [10.0, 12.0],
                            },
                        ],
                        is_closed: true,
                    }],
                }],
            },
            generation,
            CacheViewState::Current,
        );
    }

    let status = ensure_contour_view_cache(&mut world, entity, &key);

    assert_eq!(status.state, RepresentationRequestState::Current);
    let roi = world.get::<&Roi>(entity).unwrap();
    let cache = roi.contour_view_cache(&key).unwrap();
    assert_eq!(cache.state, CacheViewState::Current);
    assert_eq!(cache.data.slices.len(), 1);
    assert_eq!(cache.data.slices[0].loops.len(), 1);
    assert_eq!(
        cache.data.slices[0].loops[0].points[0].local_mm,
        [10.0, 10.0]
    );
}

#[test]
fn test_ensure_contour_view_cache_matches_extracted_slice_with_tolerance() {
    let mut world = World::new();
    let entity = spawn_sparse_voxel_roi(&mut world);
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        roi.dirty_state.voxel.dirty = false;
        roi.dirty_state.voxel.built_from = roi.dirty_state.authoritative;
    }
    let geometry = {
        let roi = world.get::<&Roi>(entity).unwrap();
        roi.voxel_cache().unwrap().data.geometry
    };
    let mut near_plane =
        orthogonal_plane_from_volume_uv(PlaneFamily::Coronal, [0.5, 1.0 / 3.0, 0.5], geometry)
            .unwrap();
    near_plane.origin_mm[1] += 0.4;
    let key = ContourViewKey::from_plane(near_plane);

    let status = ensure_contour_view_cache(&mut world, entity, &key);

    assert_eq!(status.state, RepresentationRequestState::Current);
    let roi = world.get::<&Roi>(entity).unwrap();
    let cache = roi.contour_view_cache(&key).unwrap();
    assert_eq!(cache.state, CacheViewState::Current);
    assert_eq!(cache.data.slices.len(), 1);
    assert_eq!(cache.data.slices[0].loops.len(), 1);
    assert!(!cache.data.slices[0].loops[0].points.is_empty());
}

#[test]
fn test_request_contour_view_state_reflects_voxel_stale_rebuilding_and_missing_states() {
    let mut world = World::new();
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
    let key = ContourViewKey::from_plane(test_plane_definition(PlaneFamily::Coronal));

    world
        .get::<&mut Roi>(entity)
        .unwrap()
        .discard_cache(RoiCacheKind::Voxel);

    let blocked = request_contour_view_state(&world, entity, &key);
    assert_eq!(blocked.request.state, RepresentationRequestState::Blocked);
    assert_eq!(
        blocked.request.reason.as_deref(),
        Some("voxel_cache_missing_for_contour_view")
    );

    seed_current_voxel_cache_for_contour_roi(&mut world, entity);
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        let gen = roi.dirty_state.authoritative;
        let mut derived_data = roi.contour_data().unwrap().clone();
        derived_data.active_plane_family = PlaneFamily::Coronal;
        roi.upsert_contour_view_cache(key.clone(), derived_data, gen, CacheViewState::Current);
        roi.dirty_state.voxel.dirty = true;
    }

    let stale = request_contour_view_state(&world, entity, &key);
    assert_eq!(stale.request.state, RepresentationRequestState::Stale);
    assert_eq!(
        stale.request.reason.as_deref(),
        Some("voxel_cache_stale_for_contour_view")
    );

    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        roi.job_state.running_request = Some(RoiJobRequest {
            kind: RoiJobKind::RebuildVoxelCache,
            source_generation: roi.dirty_state.authoritative.shape,
            preview_revision: None,
            priority: RoiJobPriority::VisibleCommitted,
            dirty_region: RoiDirtyRegion::Full,
        });
    }

    let rebuilding = request_contour_view_state(&world, entity, &key);
    assert_eq!(
        rebuilding.request.state,
        RepresentationRequestState::Rebuilding
    );
    assert_eq!(
        rebuilding.request.reason.as_deref(),
        Some("voxel_cache_rebuilding_for_contour_view")
    );
}

#[test]
fn test_promote_contour_view_to_authoritative_marks_other_caches_stale_and_rebuilds() {
    let mut world = World::new();
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
    seed_current_voxel_cache_for_contour_roi(&mut world, entity);
    let coronal_key = ContourViewKey::from_plane(test_plane_definition(PlaneFamily::Coronal));
    let sagittal_key = ContourViewKey::from_plane(test_plane_definition(PlaneFamily::Sagittal));
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        let gen = roi.dirty_state.authoritative;
        let mut coronal_data = roi.contour_data().unwrap().clone();
        coronal_data.active_plane_family = PlaneFamily::Coronal;
        roi.upsert_contour_view_cache(
            coronal_key.clone(),
            coronal_data,
            gen,
            CacheViewState::Current,
        );
        let mut sagittal_data = roi.contour_data().unwrap().clone();
        sagittal_data.active_plane_family = PlaneFamily::Sagittal;
        roi.upsert_contour_view_cache(
            sagittal_key.clone(),
            sagittal_data,
            gen,
            CacheViewState::Current,
        );
    }

    let result = promote_contour_view_to_authoritative(&mut world, entity, &coronal_key);
    assert_eq!(result, Ok(()));
    let roi = world.get::<&Roi>(entity).unwrap();
    let contour = roi.contour_data().unwrap();
    assert_eq!(contour.active_plane_family, PlaneFamily::Coronal);
    assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
    assert!(roi.is_cache_dirty(RoiCacheKind::Mesh));
    assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildVoxelCache));
    assert_eq!(
        roi.contour_view_cache(&sagittal_key).unwrap().state,
        CacheViewState::Stale
    );
}

#[test]
fn test_promote_contour_view_rejects_stale_or_missing_cache() {
    let mut world = World::new();
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
    let key = ContourViewKey::from_plane(test_plane_definition(PlaneFamily::Coronal));
    let missing_result = promote_contour_view_to_authoritative(&mut world, entity, &key);
    assert_eq!(missing_result, Err(ContourPromotionError::ViewCacheMissing));
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        let gen = roi.dirty_state.authoritative;
        let mut promoted_data = roi.contour_data().unwrap().clone();
        promoted_data.active_plane_family = PlaneFamily::Coronal;
        roi.upsert_contour_view_cache(key.clone(), promoted_data, gen, CacheViewState::Stale);
    }
    let stale_result = promote_contour_view_to_authoritative(&mut world, entity, &key);
    assert_eq!(
        stale_result,
        Err(ContourPromotionError::ViewCacheNotCurrent)
    );

    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        let mut promoted_data = roi.contour_data().unwrap().clone();
        promoted_data.active_plane_family = PlaneFamily::Coronal;
        roi.upsert_contour_view_cache(
            key.clone(),
            promoted_data,
            Revision::from_shape(1),
            CacheViewState::Current,
        );
        roi.dirty_state.authoritative = Revision::from_shape(2);
    }
    let generation_stale = promote_contour_view_to_authoritative(&mut world, entity, &key);
    assert_eq!(
        generation_stale,
        Err(ContourPromotionError::ViewCacheNotCurrent)
    );
}

#[test]
fn test_request_contour_view_state_marks_generation_mismatch_stale() {
    let mut world = World::new();
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
    let key = ContourViewKey::from_plane(test_plane_definition(PlaneFamily::Coronal));
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        let mut promoted_data = roi.contour_data().unwrap().clone();
        promoted_data.active_plane_family = PlaneFamily::Coronal;
        roi.upsert_contour_view_cache(
            key.clone(),
            promoted_data,
            Revision::from_shape(1),
            CacheViewState::Current,
        );
        roi.dirty_state.authoritative = Revision::from_shape(2);
    }
    let status = request_contour_view_state(&world, entity, &key);
    assert_eq!(status.request.state, RepresentationRequestState::Stale);
    assert_eq!(
        status.request.reason.as_deref(),
        Some("contour_view_cache_generation_stale")
    );
    assert!(!status.promotable);
}

#[test]
fn test_oblique_contour_view_builds_current_and_can_be_promoted() {
    let mut world = World::new();
    spawn_main_volume(&mut world, [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
    seed_current_voxel_cache_for_contour_roi(&mut world, entity);
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        let voxel = &mut roi.session_caches.voxel.as_mut().unwrap().data;
        for z in 1..=2 {
            for y in 1..=2 {
                for x in 1..=2 {
                    voxel.raw_data[(z * 16 + y * 4 + x) as usize] = 1;
                }
            }
        }
    }
    let inverse_sqrt_two = std::f32::consts::FRAC_1_SQRT_2;
    let key = ContourViewKey::from_plane(PlaneDefinition {
        family: PlaneFamily::Oblique,
        origin_mm: [1.5, 1.5, 1.5],
        u_axis_mm: [inverse_sqrt_two, inverse_sqrt_two, 0.0],
        v_axis_mm: [0.0, 0.0, 1.0],
        normal_mm: [inverse_sqrt_two, -inverse_sqrt_two, 0.0],
    });

    let build_status = ensure_contour_view_cache(&mut world, entity, &key);
    let status = request_contour_view_state(&world, entity, &key);

    assert_eq!(build_status.state, RepresentationRequestState::Current);
    assert_eq!(status.request.state, RepresentationRequestState::Current);
    assert!(!status.editable);
    assert!(status.promotable);

    let (cached_data, generation) = {
        let roi = world.get::<&Roi>(entity).unwrap();
        (
            roi.contour_view_cache(&key).unwrap().data.clone(),
            roi.dirty_state.authoritative.shape,
        )
    };
    assert!(cached_data.has_loops());
    promote_contour_view_to_authoritative(&mut world, entity, &key).unwrap();

    let roi = world.get::<&Roi>(entity).unwrap();
    assert_eq!(roi.contour_data(), Some(&cached_data));
    assert_eq!(
        roi.contour_data().unwrap().active_plane_family,
        PlaneFamily::Oblique
    );
    assert_eq!(roi.dirty_state.authoritative.shape, generation + 1);
    assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
    assert!(roi.has_queued_job(RoiJobKind::RebuildVoxelCache));
    drop(roi);

    process_contour_voxel_rebuild_jobs(&mut world);
    let roi = world.get::<&Roi>(entity).unwrap();
    assert!(roi.is_cache_current(RoiCacheKind::Voxel));
    assert!(roi
        .voxel_cache()
        .unwrap()
        .data
        .raw_data
        .iter()
        .any(|value| *value != 0));
}

#[test]
fn test_oblique_voxel_overlay_uses_same_cache_state_contract_as_orthogonal_views() {
    let mut world = World::new();
    let entity = spawn_sparse_voxel_roi(&mut world);
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        roi.dirty_state.voxel.dirty = false;
        roi.dirty_state.voxel.built_from = roi.dirty_state.authoritative;
    }

    let axial = request_viewport_voxel_overlay_state(&world, ViewMode::Axial, entity);
    let oblique = request_viewport_voxel_overlay_state(&world, ViewMode::Oblique, entity);

    assert_eq!(oblique, axial);
    assert_ne!(
        oblique.reason.as_deref(),
        Some("overlay_not_supported_in_oblique")
    );
}

#[test]
fn test_replace_contour_data_marks_existing_derived_contour_views_stale() {
    let mut world = World::new();
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
    let key = ContourViewKey::from_plane(test_plane_definition(PlaneFamily::Coronal));
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        let mut derived_data = roi.contour_data().unwrap().clone();
        derived_data.active_plane_family = PlaneFamily::Coronal;
        let gen = roi.dirty_state.authoritative;
        roi.upsert_contour_view_cache(key.clone(), derived_data, gen, CacheViewState::Current);
    }
    replace_contour_data(
        &mut world,
        entity,
        ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: Vec::new(),
        },
    )
    .unwrap();
    let roi = world.get::<&Roi>(entity).unwrap();
    assert_eq!(
        roi.contour_view_cache(&key).unwrap().state,
        CacheViewState::Stale
    );
}

#[test]
fn test_promotion_preserves_previous_active_family_as_stale_derived_cache() {
    let mut world = World::new();
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, true);
    seed_current_voxel_cache_for_contour_roi(&mut world, entity);
    let coronal_key = ContourViewKey::from_plane(test_plane_definition(PlaneFamily::Coronal));
    let previous_active_key = {
        let roi = world.get::<&Roi>(entity).unwrap();
        let contour = roi.contour_data().unwrap();
        ContourViewKey::from_plane(contour.slices[0].plane)
    };
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        let gen = roi.dirty_state.authoritative;
        let mut promoted_data = roi.contour_data().unwrap().clone();
        promoted_data.active_plane_family = PlaneFamily::Coronal;
        roi.upsert_contour_view_cache(
            coronal_key.clone(),
            promoted_data,
            gen,
            CacheViewState::Current,
        );
    }

    promote_contour_view_to_authoritative(&mut world, entity, &coronal_key).unwrap();
    let roi = world.get::<&Roi>(entity).unwrap();
    let preserved = roi
        .contour_view_cache(&previous_active_key)
        .expect("expected previous active family stale derived cache");
    assert_eq!(preserved.state, CacheViewState::Stale);
}

#[test]
#[ignore = "full liver mesh-to-voxel timing; run explicitly for milestone QA"]
fn test_liver_explicit_voxel_rebuild_frame_timing() {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/qa_samples/liver_0_label.nii"
    ))
    .unwrap();
    let label =
        crate::nifti_loader::load_label_from_bytes(&bytes, "liver_0_label.nii".into()).unwrap();
    let geometry = label.geometry;
    let voxel_data = VoxelData {
        geometry,
        raw_data: label.data,
    };
    let mesh = crate::convert::extract_chunked_mesh_from_voxel_data(
        &voxel_data,
        crate::convert::DEFAULT_MESH_CHUNK_SIZE,
    )
    .unwrap()
    .merged_mesh();
    let mut world = World::new();
    let entity = world.spawn((Roi::new_voxel_with_cache(
        RoiId(900),
        "Liver timing".to_string(),
        geometry,
        voxel_data.raw_data,
        None,
    ),));
    {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        roi.session_caches.mesh = Some(MeshCache {
            data: mesh.clone(),
            chunks: None,
        });
        roi.dirty_state.mesh.dirty = false;
        roi.dirty_state.mesh.built_from = roi.dirty_state.authoritative;
    }
    promote_current_mesh_cache_to_authority(&mut world, entity).unwrap();
    let seeds = mesh.faces[mesh.faces.len() / 2].vertex_indices;
    let anchor = mesh.vertices[seeds[0] as usize].world_mm;
    let deformed = crate::systems::mesh_editing::deform_mesh_surface_brush(
        &mesh,
        seeds,
        anchor,
        [2.0, 1.0, -1.0],
        20.0,
        1.0,
    );
    let editor = world.spawn((EditorState {
        active_roi: Some(entity),
        ..EditorState::default()
    },));
    crate::app::roi::begin_mesh_edit_preview(&mut world, editor, entity, deformed).unwrap();
    commit_mesh_edit_preview(&mut world, editor).unwrap();
    let roi = world.get::<&Roi>(entity).unwrap();
    assert_eq!(
        roi.validated_mesh_generation,
        Some(roi.dirty_state.authoritative.shape)
    );
    drop(roi);

    let started = Instant::now();
    crate::app::roi::request_mesh_voxel_cache_rebuild(&mut world, entity).unwrap();
    let request = started.elapsed();
    let mut first_frame = Duration::ZERO;
    let mut max_frame = Duration::ZERO;
    let mut frames = 0;
    while !world
        .get::<&Roi>(entity)
        .unwrap()
        .is_cache_current(RoiCacheKind::Voxel)
    {
        let started = Instant::now();
        advance_roi_work(&mut world, None);
        let frame = started.elapsed();
        frames += 1;
        if frames == 1 {
            first_frame = frame;
        }
        max_frame = max_frame.max(frame);
        assert!(frames < 2000, "voxel rebuild failed to converge");
    }
    eprintln!(
        "liver explicit rebuild: request {request:?}, first frame {first_frame:?}, max frame {max_frame:?}, frames {frames}"
    );
}

#[test]
fn test_two_slice_commits_before_one_rebuild_keep_every_slice_in_voxel_cache() {
    let mut world = World::new();
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, false);
    seed_current_voxel_cache_for_contour_roi(&mut world, entity);
    let plane_at = |z: f32| PlaneDefinition {
        family: PlaneFamily::Axial,
        origin_mm: [0.0, 0.0, z],
        u_axis_mm: [1.0, 0.0, 0.0],
        v_axis_mm: [0.0, 1.0, 0.0],
        normal_mm: [0.0, 0.0, 1.0],
    };
    let square_slice = |z: f32| ContourSlice {
        plane: plane_at(z),
        loops: vec![ContourLoop {
            points: [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]]
                .into_iter()
                .map(|local_mm| ContourPoint { local_mm })
                .collect(),
            is_closed: true,
        }],
    };
    let occupied_per_layer = |world: &World| -> Vec<usize> {
        let roi = world.get::<&Roi>(entity).unwrap();
        let data = &roi.voxel_cache().unwrap().data;
        (0..4)
            .map(|z| {
                data.raw_data[z * 16..(z + 1) * 16]
                    .iter()
                    .filter(|value| **value != 0)
                    .count()
            })
            .collect()
    };

    let mut contour = ContourData {
        active_plane_family: PlaneFamily::Axial,
        slices: vec![square_slice(0.0)],
    };
    replace_contour_data_for_slice_with_history(&mut world, entity, contour.clone(), plane_at(0.0))
        .unwrap();
    process_contour_voxel_rebuild_jobs(&mut world);
    assert_eq!(occupied_per_layer(&world), vec![9, 0, 0, 0]);

    // Two commits land before the coordinator runs again, so the retained cache is two
    // generations behind and the slice-local rasterizer has no valid base to patch.
    contour.slices.push(square_slice(1.0));
    replace_contour_data_for_slice_with_history(&mut world, entity, contour.clone(), plane_at(1.0))
        .unwrap();
    contour.slices.push(square_slice(2.0));
    replace_contour_data_for_slice_with_history(&mut world, entity, contour.clone(), plane_at(2.0))
        .unwrap();
    process_contour_voxel_rebuild_jobs(&mut world);

    assert_eq!(
        occupied_per_layer(&world),
        vec![9, 9, 9, 0],
        "voxel cache must contain every authoritative slice, not only the last dirty one"
    );
}

fn label_import_geometry() -> VoxelGeometry {
    VoxelGeometry::new([4, 4, 4], [1.0; 3], [0.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap()
}

fn spawn_labels(world: &mut World, filename: &str, data: &[u8]) -> Vec<hecs::Entity> {
    let masks = label_masks_for_import(data).expect("within the import budget");
    spawn_label_rois(world, label_import_geometry(), filename, masks, |_| {
        Ok(None)
    })
    .unwrap()
}

#[test]
fn test_multi_label_import_creates_one_roi_per_label_with_its_own_mask() {
    let mut world = World::new();
    let mut data = vec![0_u8; 64];
    data[5] = 1;
    data[6] = 1;
    data[40] = 2;

    let entities = spawn_labels(&mut world, "liver.nii", &data);

    assert_eq!(entities.len(), 2);
    let names: Vec<String> = entities
        .iter()
        .map(|entity| world.get::<&Roi>(*entity).unwrap().metadata.name.clone())
        .collect();
    assert_eq!(names, vec!["liver.nii [label 1]", "liver.nii [label 2]"]);
    let ids: Vec<u64> = entities
        .iter()
        .map(|entity| world.get::<&Roi>(*entity).unwrap().metadata.roi_id.0)
        .collect();
    assert_eq!(ids, vec![1, 2]);

    for (entity, (label, expected)) in entities.iter().zip([(1_u8, vec![5, 6]), (2, vec![40])]) {
        let roi = world.get::<&Roi>(*entity).unwrap();
        assert_eq!(roi.primary_representation(), PrimaryRepresentation::Voxel);
        assert!(roi.is_cache_current(RoiCacheKind::Voxel));
        assert_eq!(roi.metadata.color, label_color(label));
        let RoiAuthoritativeData::Voxel(voxel) = &roi.authoritative_data else {
            panic!("voxel authority");
        };
        let occupied: Vec<usize> = voxel
            .raw_data
            .iter()
            .enumerate()
            .filter(|(_, value)| **value != 0)
            .map(|(index, _)| index)
            .collect();
        assert_eq!(
            occupied, expected,
            "label {label} must hold only its own voxels"
        );
        assert!(voxel
            .raw_data
            .iter()
            .all(|value| *value == 0 || *value == label));
    }
}

#[test]
fn test_single_label_import_keeps_the_file_name_and_id() {
    let mut world = World::new();
    let mut data = vec![0_u8; 64];
    data[3] = 5;

    let entities = spawn_labels(&mut world, "one.nii", &data);

    assert_eq!(entities.len(), 1);
    let roi = world.get::<&Roi>(entities[0]).unwrap();
    assert_eq!(roi.metadata.name, "one.nii");
    assert_eq!(roi.metadata.color, label_color(5));
}

#[test]
fn test_labelmap_without_labels_imports_as_one_empty_roi() {
    let mut world = World::new();

    let entities = spawn_labels(&mut world, "empty.nii", &[0_u8; 64]);

    assert_eq!(entities.len(), 1);
    let roi = world.get::<&Roi>(entities[0]).unwrap();
    assert_eq!(roi.metadata.name, "empty.nii");
    assert_eq!(
        roi.metadata.color,
        [1.0, 0.2, 0.2, 1.0],
        "keeps the default colour"
    );
    let RoiAuthoritativeData::Voxel(voxel) = &roi.authoritative_data else {
        panic!("voxel authority");
    };
    assert!(voxel.raw_data.iter().all(|value| *value == 0));
}

#[test]
fn test_label_import_hides_rois_beyond_the_overlay_cap() {
    let mut world = World::new();
    // Ten labels, one voxel each.
    let data: Vec<u8> = (1..=10_u8).chain(std::iter::repeat_n(0, 54)).collect();

    let entities = spawn_labels(&mut world, "many.nii", &data);

    let visible: Vec<bool> = entities
        .iter()
        .map(|entity| world.get::<&Roi>(*entity).unwrap().metadata.is_visible)
        .collect();
    let expected: Vec<bool> = (0..10)
        .map(|index| index < MAX_SIMULTANEOUS_ROI_OVERLAYS)
        .collect();
    assert_eq!(visible, expected);
}

#[test]
fn test_label_import_budget_is_enforced_and_normal_maps_pass() {
    // Check the budget with a large volume directly; allocating such a map would defeat the test.
    assert!(check_label_import_budget(512 * 512 * 300, 20).is_err());
    assert!(label_masks_for_import(&[1_u8; 64]).is_ok());
}

fn contour_shifted_by(world: &World, roi: hecs::Entity, shift: f32) -> ContourData {
    let mut contour = world
        .get::<&Roi>(roi)
        .unwrap()
        .contour_data()
        .unwrap()
        .clone();
    contour.slices[0].loops[0].points[1].local_mm[0] += shift;
    contour
}

fn first_loop_x(world: &World, roi: hecs::Entity) -> f32 {
    world
        .get::<&Roi>(roi)
        .unwrap()
        .contour_data()
        .unwrap()
        .slices[0]
        .loops[0]
        .points[1]
        .local_mm[0]
}

#[test]
fn test_each_roi_keeps_its_own_undo_history_and_undo_never_changes_the_active_roi() {
    let mut world = World::new();
    let editor = world.spawn((EditorState::default(),));
    let first = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, true);
    let second = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, true);
    let (first_start, second_start) = (first_loop_x(&world, first), first_loop_x(&world, second));

    let edit = contour_shifted_by(&world, first, 0.25);
    replace_contour_data_with_history(&mut world, first, edit).unwrap();
    let edit = contour_shifted_by(&world, second, 0.5);
    replace_contour_data_with_history(&mut world, second, edit).unwrap();

    // Undo acts on the active ROI only, and leaves it active.
    world.get::<&mut EditorState>(editor).unwrap().active_roi = Some(second);
    assert!(can_undo_roi_edit(&world, editor));
    assert_eq!(undo_roi_edit(&mut world, editor), Ok(second));
    assert_eq!(first_loop_x(&world, second), second_start);
    assert_eq!(first_loop_x(&world, first), first_start + 0.25);
    assert_eq!(
        world.get::<&EditorState>(editor).unwrap().active_roi,
        Some(second)
    );
    assert!(
        !can_undo_roi_edit(&world, editor),
        "second has nothing left"
    );
    assert_eq!(
        undo_roi_edit(&mut world, editor),
        Err(RoiEditHistoryError::NoUndo)
    );

    // The first ROI's step is untouched and available once it is active.
    world.get::<&mut EditorState>(editor).unwrap().active_roi = Some(first);
    assert!(can_undo_roi_edit(&world, editor));
    assert_eq!(undo_roi_edit(&mut world, editor), Ok(first));
    assert_eq!(first_loop_x(&world, first), first_start);
    assert_eq!(
        world.get::<&EditorState>(editor).unwrap().active_roi,
        Some(first)
    );

    // Redo is per ROI as well.
    assert!(can_redo_roi_edit(&world, editor));
    assert_eq!(redo_roi_edit(&mut world, editor), Ok(first));
    assert_eq!(first_loop_x(&world, first), first_start + 0.25);
    assert_eq!(first_loop_x(&world, second), second_start);
}

#[test]
fn test_undo_without_an_active_roi_is_reported_not_guessed() {
    let mut world = World::new();
    let editor = world.spawn((EditorState::default(),));
    let entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, true);
    let edit = contour_shifted_by(&world, entity, 0.25);
    replace_contour_data_with_history(&mut world, entity, edit).unwrap();

    assert!(!can_undo_roi_edit(&world, editor));
    assert_eq!(
        undo_roi_edit(&mut world, editor),
        Err(RoiEditHistoryError::NoActiveRoi)
    );
    assert_eq!(world.get::<&Roi>(entity).unwrap().history.undo.len(), 1);
}

#[test]
fn test_history_is_capped_per_roi_and_clearing_one_roi_leaves_the_other() {
    let mut world = World::new();
    let busy = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, true);
    let quiet = spawn_test_contour_roi(&mut world, PlaneFamily::Axial, true);
    let edit = contour_shifted_by(&world, quiet, 1.0);
    replace_contour_data_with_history(&mut world, quiet, edit).unwrap();

    for _ in 0..(MAX_ROI_EDIT_HISTORY + 8) {
        let edit = contour_shifted_by(&world, busy, 0.01);
        replace_contour_data_with_history(&mut world, busy, edit).unwrap();
    }

    assert_eq!(
        world.get::<&Roi>(busy).unwrap().history.undo.len(),
        MAX_ROI_EDIT_HISTORY
    );
    assert_eq!(world.get::<&Roi>(quiet).unwrap().history.undo.len(), 1);

    clear_roi_edit_history_for_roi(&mut world, busy);
    assert!(world.get::<&Roi>(busy).unwrap().history.undo.is_empty());
    assert_eq!(world.get::<&Roi>(quiet).unwrap().history.undo.len(), 1);
}
