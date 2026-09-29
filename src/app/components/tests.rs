use super::*;

fn test_plane_definition(family: PlaneFamily) -> PlaneDefinition {
    PlaneDefinition {
        family,
        origin_mm: [10.0, 20.0, 30.0],
        u_axis_mm: [1.0, 0.0, 0.0],
        v_axis_mm: [0.0, 1.0, 0.0],
        normal_mm: [0.0, 0.0, 1.0],
    }
}

#[test]
fn test_aspect_ratio_cubic() {
    let vol = VolumeData {
        dimensions: [100, 100, 100],
        geometry: Some(
            VoxelGeometry::new(
                [100, 100, 100],
                [1.0, 1.0, 1.0],
                [0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            )
            .unwrap(),
        ),
        intensities: vec![],
        intensity_range: [0.0, 1.0],
    };
    let ar = vol.aspect_ratios();
    assert!((ar[0] - 1.0).abs() < 1e-6);
    assert!((ar[1] - 1.0).abs() < 1e-6);
    assert!((ar[2] - 1.0).abs() < 1e-6);
}

#[test]
fn test_aspect_ratio_anisotropic() {
    let vol = VolumeData {
        dimensions: [256, 256, 128],
        // Physical size is 256, 256, 256
        geometry: Some(
            VoxelGeometry::new(
                [256, 256, 128],
                [1.0, 1.0, 2.0],
                [0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            )
            .unwrap(),
        ),
        intensities: vec![],
        intensity_range: [0.0, 1.0],
    };
    let ar = vol.aspect_ratios();
    assert!((ar[0] - 1.0).abs() < 1e-6);
    assert!((ar[1] - 1.0).abs() < 1e-6);
    assert!((ar[2] - 1.0).abs() < 1e-6);
}

#[test]
fn test_aspect_ratio_zero_dims() {
    let vol = VolumeData {
        dimensions: [0, 0, 0],
        geometry: None,
        intensities: vec![],
        intensity_range: [0.0, 1.0],
    };
    let ar = vol.aspect_ratios();
    assert_eq!(ar, [1.0, 1.0, 1.0]);
}

#[test]
fn test_contour_view_key_from_plane_excludes_viewport_identity() {
    let plane = test_plane_definition(PlaneFamily::Axial);
    let key_a = ContourViewKey::from_plane(plane);
    let key_b = ContourViewKey::from_plane(plane);
    assert!(key_a.logical_eq(&key_b));
}

#[test]
fn test_contour_view_key_lookup_uses_slice_key_not_exact_plane_float() {
    let contour = ContourData {
        active_plane_family: PlaneFamily::Axial,
        slices: Vec::new(),
    };
    let mut roi = Roi::new_contour(RoiId(12), "C".to_string(), contour.clone());
    let plane = test_plane_definition(PlaneFamily::Coronal);
    let key_a = ContourViewKey::from_plane(plane);
    roi.upsert_contour_view_cache(
        key_a.clone(),
        contour,
        roi.dirty_state.generations.authoritative,
        CacheViewState::Current,
    );

    let mut drifted = plane;
    drifted.normal_mm = [0.0, 0.0, 1.0 + 1e-8];
    let key_b = ContourViewKey::from_plane(drifted);
    assert!(roi.contour_view_cache(&key_b).is_some());
}

#[test]
fn test_oblique_slice_key_distinguishes_same_origin_different_normal() {
    let contour = ContourData {
        active_plane_family: PlaneFamily::Oblique,
        slices: Vec::new(),
    };
    let mut roi = Roi::new_contour(RoiId(13), "Oblique".to_string(), contour.clone());
    let plane_a = PlaneDefinition {
        family: PlaneFamily::Oblique,
        origin_mm: [12.0, -3.0, 7.0],
        u_axis_mm: [1.0, 0.0, 0.0],
        v_axis_mm: [0.0, 1.0, 0.0],
        normal_mm: [0.0, 0.0, 1.0],
    };
    let plane_b = PlaneDefinition {
        family: PlaneFamily::Oblique,
        origin_mm: [12.0, -3.0, 7.0],
        u_axis_mm: [1.0, 0.0, 0.0],
        v_axis_mm: [0.0, 0.70710677, 0.70710677],
        normal_mm: [0.0, -0.70710677, 0.70710677],
    };
    let key_a = ContourViewKey::from_plane(plane_a);
    let key_b = ContourViewKey::from_plane(plane_b);
    assert!(!key_a.logical_eq(&key_b));

    let gen = roi.dirty_state.generations.authoritative;
    roi.upsert_contour_view_cache(key_a, contour.clone(), gen, CacheViewState::Current);
    roi.upsert_contour_view_cache(key_b.clone(), contour, gen, CacheViewState::Current);
    let cache = roi.contour_cache().unwrap();
    assert_eq!(cache.views.len(), 2);
    assert!(roi.contour_view_cache(&key_b).is_some());
}

#[test]
fn test_mark_contour_authoritative_changed_marks_existing_derived_views_dirty() {
    let contour = ContourData {
        active_plane_family: PlaneFamily::Axial,
        slices: Vec::new(),
    };
    let mut roi = Roi::new_contour(RoiId(11), "C".to_string(), contour.clone());
    let plane = test_plane_definition(PlaneFamily::Coronal);
    roi.upsert_contour_view_cache(
        ContourViewKey::from_plane(plane),
        contour,
        roi.dirty_state.generations.authoritative,
        CacheViewState::Current,
    );
    roi.dirty_state.contour_cache_dirty = false;
    roi.dirty_state.generations.contour = roi.dirty_state.generations.authoritative;

    roi.mark_contour_authoritative_changed();
    roi.mark_all_contour_view_caches_stale();

    let cache = roi.contour_cache().unwrap();
    assert_eq!(cache.views.len(), 1);
    assert_eq!(cache.views[0].state, CacheViewState::Stale);
}

#[test]
fn test_new_voxel_roi_initializes_voxel_primary_state() {
    let roi = Roi::new_voxel_with_cache(
        RoiId(7),
        "Liver".to_string(),
        VoxelGeometry::new(
            [16, 16, 8],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap(),
        vec![1; 16 * 16 * 8],
        None,
    );

    assert_eq!(roi.metadata.roi_id, RoiId(7));
    assert_eq!(roi.metadata.name, "Liver");
    assert_eq!(roi.primary_representation(), PrimaryRepresentation::Voxel);
    assert_eq!(roi.reference_geometry().dimensions(), [16, 16, 8]);
    let RoiAuthoritativeData::Voxel(authoritative_voxel) = &roi.authoritative_data else {
        panic!("a new voxel ROI has voxel authority");
    };
    assert_eq!(authoritative_voxel.geometry.dimensions(), [16, 16, 8]);
    assert_eq!(authoritative_voxel.geometry.spacing(), [1.0, 1.0, 1.0]);
    let voxel_cache = roi.voxel_cache().expect("voxel cache should exist");
    assert_eq!(voxel_cache.data.raw_data.len(), 16 * 16 * 8);
    assert_eq!(voxel_cache.data.geometry.dimensions, [16, 16, 8]);
    assert!(voxel_cache.gpu_resources.is_none());
    assert!(roi.voxel_gpu_cache().is_none());
    assert!(roi.session_caches.contour.is_none());
    assert!(roi.session_caches.mesh.is_none());
    assert!(!roi.is_cache_dirty(RoiCacheKind::Voxel));
    assert!(roi.is_cache_current(RoiCacheKind::Voxel));
    assert!(roi.renderable_voxel_cache().is_none());
}

#[test]
fn test_editor_tool_default_remains_navigation() {
    let editor = EditorState::default();
    assert_eq!(editor.active_tool, EditorTool::Navigation);
}

#[test]
fn test_roi_dirty_state_defaults_match_clean_voxel_baseline() {
    let state = RoiDirtyState::default();

    assert!(!state.authoritative_dirty);
    assert!(!state.voxel_cache_dirty);
    assert!(!state.contour_cache_dirty);
    assert!(!state.mesh_cache_dirty);
    assert_eq!(state.generations.authoritative, 1);
    assert_eq!(state.generations.voxel, 0);
}

#[test]
fn test_new_voxel_roi_without_gpu_still_has_current_cpu_voxel_cache() {
    let roi = Roi::new_voxel_with_cache(
        RoiId(8),
        "Kidney".to_string(),
        VoxelGeometry::new(
            [8, 8, 8],
            [0.5, 0.5, 0.5],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap(),
        vec![1; 8 * 8 * 8],
        None,
    );

    assert!(!roi.is_cache_dirty(RoiCacheKind::Voxel));
    assert!(roi.is_cache_current(RoiCacheKind::Voxel));
    assert!(roi.voxel_cache().is_some());
    assert!(roi.voxel_gpu_cache().is_none());
}

#[test]
fn test_new_voxel_roi_copies_authoritative_data_into_session_voxel_cache() {
    let geometry = VoxelGeometry::new(
        [6, 5, 4],
        [0.9, 1.1, 1.3],
        [1.0, 2.0, 3.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap();
    let raw_data = vec![0, 1, 0, 1, 1, 0, 1, 0];
    let roi = Roi::new_voxel_with_cache(
        RoiId(77),
        "Cache Copy".to_string(),
        geometry,
        raw_data.clone(),
        None,
    );

    let authoritative = match &roi.authoritative_data {
        RoiAuthoritativeData::Voxel(voxel) => voxel,
        RoiAuthoritativeData::Contour(_) | RoiAuthoritativeData::Mesh(_) => {
            panic!("expected voxel-authoritative ROI");
        }
    };
    let cached = roi.voxel_cache().expect("voxel cache should exist");

    assert_eq!(cached.data.geometry, authoritative.geometry);
    assert_eq!(cached.data.raw_data, authoritative.raw_data);
    assert_eq!(cached.data.raw_data, raw_data);
}

#[test]
fn test_renderable_voxel_cache_requires_gpu_resources_even_when_cache_current() {
    let mut roi = Roi::new_voxel_with_cache(
        RoiId(88),
        "No GPU".to_string(),
        VoxelGeometry::new(
            [4, 4, 4],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap(),
        vec![1; 64],
        None,
    );

    roi.metadata.is_visible = true;
    assert!(roi.is_cache_current(RoiCacheKind::Voxel));
    assert!(roi.voxel_gpu_cache().is_none());
    assert!(roi.renderable_voxel_cache().is_none());
}

#[test]
fn test_cache_current_requires_matching_generation_and_clean_state() {
    let mut roi = Roi::new_voxel_with_cache(
        RoiId(12),
        "Aorta".to_string(),
        VoxelGeometry::new(
            [8, 8, 8],
            [0.75, 0.75, 0.75],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap(),
        vec![1; 8 * 8 * 8],
        None,
    );

    roi.dirty_state.voxel_cache_dirty = false;
    roi.dirty_state.generations.voxel = roi.dirty_state.generations.authoritative;
    assert!(roi.is_cache_current(RoiCacheKind::Voxel));

    roi.dirty_state.generations.voxel -= 1;
    assert!(!roi.is_cache_current(RoiCacheKind::Voxel));
}

#[test]
fn test_mark_authoritative_changed_invalidates_all_derived_caches() {
    let mut roi = Roi::new_voxel_with_cache(
        RoiId(9),
        "Spleen".to_string(),
        VoxelGeometry::new(
            [4, 4, 4],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap(),
        vec![1; 64],
        None,
    );

    roi.dirty_state.voxel_cache_dirty = false;
    roi.dirty_state.contour_cache_dirty = false;
    roi.dirty_state.mesh_cache_dirty = false;
    roi.dirty_state.generations.voxel = roi.dirty_state.generations.authoritative;
    roi.dirty_state.generations.contour = roi.dirty_state.generations.authoritative;
    roi.dirty_state.generations.mesh = roi.dirty_state.generations.authoritative;

    roi.mark_authoritative_changed();

    assert!(roi.dirty_state.authoritative_dirty);
    assert_eq!(roi.dirty_state.generations.authoritative, 2);
    assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
    assert!(roi.is_cache_dirty(RoiCacheKind::Contour));
    assert!(roi.is_cache_dirty(RoiCacheKind::Mesh));
    assert!(!roi.is_cache_current(RoiCacheKind::Voxel));
}

#[test]
fn test_mark_contour_authoritative_changed_invalidates_only_derived_by_default() {
    let mut roi = Roi::new_contour(
        RoiId(16),
        "CTV".to_string(),
        ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: Vec::new(),
        },
    );

    roi.dirty_state.voxel_cache_dirty = false;
    roi.dirty_state.contour_cache_dirty = false;
    roi.dirty_state.mesh_cache_dirty = false;
    roi.dirty_state.generations.voxel = roi.dirty_state.generations.authoritative;
    roi.dirty_state.generations.contour = roi.dirty_state.generations.authoritative;
    roi.dirty_state.generations.mesh = roi.dirty_state.generations.authoritative;

    roi.mark_contour_authoritative_changed();

    assert!(roi.dirty_state.authoritative_dirty);
    assert_eq!(roi.dirty_state.generations.authoritative, 2);
    assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
    assert!(!roi.is_cache_dirty(RoiCacheKind::Contour));
    assert!(roi.is_cache_dirty(RoiCacheKind::Mesh));
}

#[test]
fn test_enqueue_rebuild_preserves_multiple_representation_jobs() {
    let mut roi = Roi::new_voxel_with_cache(
        RoiId(10),
        "Pancreas".to_string(),
        VoxelGeometry::new(
            [4, 4, 4],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap(),
        vec![0; 64],
        None,
    );

    roi.enqueue_rebuild(RoiJobKind::RebuildContourCache);
    roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);

    assert_eq!(roi.job_state.pending.len(), 2);
    assert_eq!(roi.queued_job_kind(), Some(RoiJobKind::RebuildVoxelCache));
    assert_eq!(roi.start_queued_job(), Some(RoiJobKind::RebuildVoxelCache));
    roi.finish_job(RoiJobKind::RebuildVoxelCache);
    assert_eq!(
        roi.start_queued_job(),
        Some(RoiJobKind::RebuildContourCache)
    );
    assert_eq!(
        roi.running_job_kind(),
        Some(RoiJobKind::RebuildContourCache)
    );
}

#[test]
fn test_interactive_job_priority_and_preview_supersession() {
    let mut roi = Roi::new_voxel_with_cache(
        RoiId(20),
        "Priority".to_string(),
        VoxelGeometry::new([8, 8, 8], [1.0; 3], [0.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap(),
        vec![0; 512],
        None,
    );
    roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
    roi.enqueue_job(RoiJobRequest {
        kind: RoiJobKind::RebuildMeshCache,
        source_generation: 1,
        preview_revision: Some(1),
        priority: RoiJobPriority::InteractivePreview,
        dirty_region: RoiDirtyRegion::VoxelAabb {
            min: [2, 2, 2],
            max: [3, 3, 3],
        },
    });
    roi.enqueue_job(RoiJobRequest {
        kind: RoiJobKind::RebuildMeshCache,
        source_generation: 1,
        preview_revision: Some(2),
        priority: RoiJobPriority::InteractivePreview,
        dirty_region: RoiDirtyRegion::VoxelAabb {
            min: [4, 4, 4],
            max: [5, 5, 5],
        },
    });

    assert_eq!(roi.job_state.pending.len(), 2);
    let request = roi.job_state.pending[0];
    assert_eq!(request.kind, RoiJobKind::RebuildMeshCache);
    assert_eq!(request.preview_revision, Some(2));
    assert_eq!(
        request.dirty_region,
        RoiDirtyRegion::VoxelAabb {
            min: [2, 2, 2],
            max: [5, 5, 5]
        }
    );
    assert_eq!(roi.job_metrics.max_queue_depth, 2);
}

#[test]
fn test_preview_revision_is_monotonic_and_explicitly_ends() {
    let mut roi = Roi::new_contour(
        RoiId(21),
        "Preview".to_string(),
        ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: Vec::new(),
        },
    );

    assert_eq!(roi.begin_preview(), 1);
    assert_eq!(roi.begin_preview(), 2);
    assert!(roi.preview_state.active);
    roi.end_preview();
    assert!(!roi.preview_state.active);
    assert_eq!(roi.preview_state.revision, 2);
}

#[test]
fn test_contour_view_cache_is_bounded() {
    let contour = ContourData {
        active_plane_family: PlaneFamily::Axial,
        slices: Vec::new(),
    };
    let mut roi = Roi::new_contour(RoiId(22), "Bounded".to_string(), contour.clone());
    for index in 0..=MAX_CONTOUR_VIEW_CACHE_ENTRIES {
        let mut plane = test_plane_definition(PlaneFamily::Coronal);
        plane.origin_mm[1] = index as f32;
        roi.upsert_contour_view_cache(
            ContourViewKey::from_plane(plane),
            contour.clone(),
            1,
            CacheViewState::Current,
        );
    }

    let cache = roi.contour_cache().unwrap();
    assert_eq!(cache.views.len(), MAX_CONTOUR_VIEW_CACHE_ENTRIES);
    assert_eq!(cache.views[0].key.plane.origin_mm[1], 1.0);
}

#[test]
fn test_finish_cache_rebuild_marks_cache_current_and_clears_job() {
    let mut roi = Roi::new_voxel_with_cache(
        RoiId(11),
        "Heart".to_string(),
        VoxelGeometry::new(
            [4, 4, 4],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap(),
        vec![0; 64],
        None,
    );

    roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
    assert_eq!(roi.start_queued_job(), Some(RoiJobKind::RebuildVoxelCache));

    roi.finish_cache_rebuild(RoiCacheKind::Voxel);

    assert!(!roi.dirty_state.authoritative_dirty);
    assert!(!roi.is_cache_dirty(RoiCacheKind::Voxel));
    assert!(roi.is_cache_current(RoiCacheKind::Voxel));
    assert_eq!(roi.running_job_kind(), None);
}

#[test]
fn test_voxel_geometry_is_preserved_on_constructor() {
    let geometry = VoxelGeometry::new(
        [12, 10, 8],
        [0.8, 0.8, 1.5],
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap();

    let roi = Roi::new_voxel_with_cache(
        RoiId(13),
        "Gallbladder".to_string(),
        geometry,
        vec![0; 12 * 10 * 8],
        None,
    );

    match roi.authoritative_data {
        RoiAuthoritativeData::Voxel(voxel) => assert_eq!(voxel.geometry, geometry),
        _ => panic!("expected voxel roi"),
    }
}

#[test]
fn test_contour_data_preserves_active_plane_family() {
    let contour = ContourData {
        active_plane_family: PlaneFamily::Oblique,
        slices: Vec::new(),
    };
    assert_eq!(contour.active_plane_family, PlaneFamily::Oblique);
    assert!(contour.is_empty());
    assert!(!contour.has_loops());
}

#[test]
fn test_contour_slice_preserves_plane_definition() {
    let plane = test_plane_definition(PlaneFamily::Coronal);
    let slice = ContourSlice {
        plane,
        loops: vec![ContourLoop {
            points: vec![
                ContourPoint {
                    local_mm: [0.0, 0.0],
                },
                ContourPoint {
                    local_mm: [2.0, 0.0],
                },
                ContourPoint {
                    local_mm: [2.0, 2.0],
                },
            ],
            is_closed: true,
        }],
    };

    assert_eq!(slice.plane, plane);
    assert!(slice.loops[0].is_valid_closed_loop());
}

#[test]
fn test_contour_loop_requires_three_points_when_closed() {
    let loop_with_two_points = ContourLoop {
        points: vec![
            ContourPoint {
                local_mm: [0.0, 0.0],
            },
            ContourPoint {
                local_mm: [1.0, 0.0],
            },
        ],
        is_closed: true,
    };
    let loop_with_three_points = ContourLoop {
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
    };

    assert!(!loop_with_two_points.is_valid_closed_loop());
    assert!(loop_with_three_points.is_valid_closed_loop());
}

#[test]
fn test_new_contour_roi_initializes_contour_primary_state() {
    let contour_data = ContourData {
        active_plane_family: PlaneFamily::Axial,
        slices: vec![ContourSlice {
            plane: test_plane_definition(PlaneFamily::Axial),
            loops: vec![ContourLoop {
                points: vec![
                    ContourPoint {
                        local_mm: [0.0, 0.0],
                    },
                    ContourPoint {
                        local_mm: [1.0, 0.0],
                    },
                    ContourPoint {
                        local_mm: [1.0, 1.0],
                    },
                ],
                is_closed: true,
            }],
        }],
    };
    let reference_geometry = VoxelGeometry::new(
        [16, 16, 8],
        [1.0, 1.0, 1.0],
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap();
    let expected_identity = reference_geometry.identity();
    let roi = Roi::new_contour_with_geometry(
        RoiId(14),
        "GTV".to_string(),
        reference_geometry,
        contour_data.clone(),
    );

    assert_eq!(roi.metadata.roi_id, RoiId(14));
    assert_eq!(roi.metadata.name, "GTV");
    assert_eq!(roi.primary_representation(), PrimaryRepresentation::Contour);
    assert_eq!(roi.reference_geometry().identity(), expected_identity);
    assert!(matches!(
        roi.authoritative_data,
        RoiAuthoritativeData::Contour(_)
    ));
    assert_eq!(roi.contour_data(), Some(&contour_data));
    assert!(roi.voxel_cache().is_none());
    assert!(roi.session_caches.contour.is_none());
    assert!(roi.session_caches.mesh.is_none());
    assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
    assert!(!roi.is_cache_current(RoiCacheKind::Voxel));
}

#[test]
fn test_contour_accessor_rejects_voxel_roi() {
    let voxel_roi = Roi::new_voxel_with_cache(
        RoiId(15),
        "Body".to_string(),
        VoxelGeometry::new(
            [8, 8, 8],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap(),
        vec![0; 8 * 8 * 8],
        None,
    );

    assert!(voxel_roi.contour_data().is_none());
}

#[test]
fn test_new_mesh_roi_initializes_mesh_primary_state() {
    let mesh_data = MeshData {
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
    };
    let roi = Roi::new_mesh(RoiId(21), "Surface".to_string(), mesh_data.clone());

    assert_eq!(roi.metadata.roi_id, RoiId(21));
    assert_eq!(roi.metadata.name, "Surface");
    assert_eq!(roi.primary_representation(), PrimaryRepresentation::Mesh);
    assert!(matches!(
        roi.authoritative_data,
        RoiAuthoritativeData::Mesh(_)
    ));
    assert_eq!(roi.mesh_data(), Some(&mesh_data));
    assert!(roi.voxel_cache().is_none());
    assert!(roi.session_caches.contour.is_none());
    assert!(roi.session_caches.mesh.is_none());
    assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
    assert!(roi.is_cache_dirty(RoiCacheKind::Contour));
    assert!(!roi.is_cache_dirty(RoiCacheKind::Mesh));
}

#[test]
fn test_mesh_accessor_rejects_non_mesh_rois() {
    let voxel_roi = Roi::new_voxel_with_cache(
        RoiId(22),
        "Voxel".to_string(),
        VoxelGeometry::new(
            [4, 4, 4],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap(),
        vec![0; 4 * 4 * 4],
        None,
    );
    let contour_roi = Roi::new_contour(
        RoiId(23),
        "Contour".to_string(),
        ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: Vec::new(),
        },
    );

    assert!(voxel_roi.mesh_data().is_none());
    assert!(contour_roi.mesh_data().is_none());
}

#[test]
fn test_mark_mesh_authoritative_changed_invalidates_voxel_and_contour_without_mesh_cache() {
    let mesh_data = MeshData {
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
    };
    let mut roi = Roi::new_mesh(RoiId(24), "Mesh".to_string(), mesh_data);

    roi.dirty_state.voxel_cache_dirty = false;
    roi.dirty_state.contour_cache_dirty = false;
    roi.dirty_state.mesh_cache_dirty = false;
    roi.dirty_state.generations.voxel = roi.dirty_state.generations.authoritative;
    roi.dirty_state.generations.contour = roi.dirty_state.generations.authoritative;
    roi.dirty_state.generations.mesh = roi.dirty_state.generations.authoritative;

    roi.mark_mesh_authoritative_changed();

    assert!(roi.dirty_state.authoritative_dirty);
    assert_eq!(roi.dirty_state.generations.authoritative, 2);
    assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
    assert!(roi.is_cache_dirty(RoiCacheKind::Contour));
    assert!(!roi.is_cache_dirty(RoiCacheKind::Mesh));
    assert!(!roi.is_cache_current(RoiCacheKind::Voxel));
    assert!(!roi.is_cache_current(RoiCacheKind::Contour));
}

#[test]
fn test_mark_mesh_authoritative_changed_invalidates_mesh_cache_when_present() {
    let mesh_data = MeshData {
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
    };
    let mut roi = Roi::new_mesh(RoiId(25), "Mesh Cached".to_string(), mesh_data.clone());
    roi.session_caches.mesh = Some(MeshCache {
        data: mesh_data,
        chunks: None,
    });
    roi.dirty_state.voxel_cache_dirty = false;
    roi.dirty_state.contour_cache_dirty = false;
    roi.dirty_state.mesh_cache_dirty = false;
    roi.dirty_state.generations.voxel = roi.dirty_state.generations.authoritative;
    roi.dirty_state.generations.contour = roi.dirty_state.generations.authoritative;
    roi.dirty_state.generations.mesh = roi.dirty_state.generations.authoritative;

    roi.mark_mesh_authoritative_changed();

    assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
    assert!(roi.is_cache_dirty(RoiCacheKind::Contour));
    assert!(roi.is_cache_dirty(RoiCacheKind::Mesh));
    assert!(!roi.is_cache_current(RoiCacheKind::Mesh));
}

#[test]
fn test_finish_mesh_cache_rebuild_marks_mesh_cache_current_to_authoritative_generation() {
    let mesh_data = MeshData {
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
    };
    let mut roi = Roi::new_mesh(RoiId(26), "Mesh Rebuild".to_string(), mesh_data.clone());
    roi.session_caches.mesh = Some(MeshCache {
        data: mesh_data,
        chunks: None,
    });
    roi.dirty_state.mesh_cache_dirty = true;
    roi.enqueue_rebuild(RoiJobKind::RebuildMeshCache);
    assert_eq!(roi.start_queued_job(), Some(RoiJobKind::RebuildMeshCache));

    roi.finish_cache_rebuild(RoiCacheKind::Mesh);

    assert!(!roi.is_cache_dirty(RoiCacheKind::Mesh));
    assert!(roi.is_cache_current(RoiCacheKind::Mesh));
    assert_eq!(
        roi.cache_generation(RoiCacheKind::Mesh),
        roi.dirty_state.generations.authoritative
    );
    assert_eq!(roi.running_job_kind(), None);
}
