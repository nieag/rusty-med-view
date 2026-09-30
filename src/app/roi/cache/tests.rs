use super::*;
use crate::app::components::{RoiId, VoxelGeometry};
use crate::convert::PlaneFamily;
use crate::model::ContourData;
use crate::model::OrthogonalFamily;

fn test_roi() -> Roi {
    Roi::new_contour(
        RoiId(1),
        "test".to_string(),
        ContourData {
            active_plane_family: OrthogonalFamily::Axial,
            slices: Vec::new(),
        },
    )
    .0
}

fn test_voxel_cache(value: u8) -> VoxelCache {
    VoxelCache {
        data: crate::app::roi::VoxelData {
            geometry: VoxelGeometry::new([1, 1, 1], [1.0; 3], [0.0; 3], [0.0, 0.0, 0.0, 1.0])
                .unwrap(),
            raw_data: vec![value],
        },
        gpu_resources: None,
    }
}

#[test]
fn test_voxel_cache_result_with_mismatched_reference_geometry_is_rejected() {
    let reference_geometry = VoxelGeometry::new(
        [2, 2, 2],
        [1.0, 1.0, 1.0],
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap();
    let (mut roi, _) = Roi::new_contour_with_geometry(
        RoiId(1),
        "test".to_string(),
        reference_geometry,
        ContourData {
            active_plane_family: OrthogonalFamily::Axial,
            slices: Vec::new(),
        },
    );
    let generation = roi.dirty_state.authoritative.shape;

    let result = roi.install_voxel_cache_result(test_voxel_cache(1), generation);

    assert_eq!(result, Err(CacheInstallError::GeometryMismatch));
    assert!(roi.voxel_cache().is_none());
}

#[test]
fn test_stale_voxel_result_is_rejected_without_overwriting_cache() {
    let mut roi = test_roi();
    let generation = roi.dirty_state.authoritative.shape;
    roi.install_voxel_cache_result(test_voxel_cache(1), generation)
        .unwrap();
    roi.mark_contour_authoritative_changed();

    let error = roi
        .install_voxel_cache_result(test_voxel_cache(2), generation)
        .unwrap_err();

    assert_eq!(
        error,
        CacheInstallError::StaleGeneration {
            expected: generation + 1,
            actual: generation,
        }
    );
    assert_eq!(roi.voxel_cache().unwrap().data.raw_data, vec![1]);
    assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
}

#[test]
fn test_superseded_preview_result_is_rejected_without_overwriting_cache() {
    let mut roi = test_roi();
    let generation = roi.dirty_state.authoritative.shape;
    let stale_revision = roi.begin_preview();
    let current_revision = roi.begin_preview();

    let error = roi
        .install_preview_voxel_result(PreviewVoxelCache {
            data: test_voxel_cache(1).data,
            source_generation: generation,
            preview_revision: stale_revision,
        })
        .unwrap_err();

    assert_eq!(
        error,
        CacheInstallError::StalePreviewRevision {
            expected: current_revision,
            actual: stale_revision,
        }
    );
    assert!(roi.session_caches.preview_voxel.is_none());
}

#[test]
fn test_current_voxel_result_installs_payload_and_freshness_together() {
    let mut roi = test_roi();
    let generation = roi.dirty_state.authoritative.shape;

    roi.install_voxel_cache_result(test_voxel_cache(7), generation)
        .unwrap();

    assert_eq!(roi.voxel_cache().unwrap().data.raw_data, vec![7]);
    assert!(roi.is_cache_current(RoiCacheKind::Voxel));
    assert_eq!(roi.cache_generation(RoiCacheKind::Voxel), generation);
}

#[test]
fn test_mesh_result_is_stamped_with_roi_geometry_identity() {
    let mut roi = test_roi();
    let generation = roi.dirty_state.authoritative.shape;

    roi.install_mesh_cache_result(
        MeshCache {
            data: crate::app::roi::MeshData {
                vertices: Vec::new(),
                faces: Vec::new(),
            },
            chunks: None,
        },
        generation,
    )
    .unwrap();

    assert_eq!(
        roi.session_caches.mesh_geometry_identity,
        Some(roi.reference_geometry().identity())
    );
}

#[test]
fn test_contour_view_result_is_stamped_with_roi_geometry_identity() {
    let mut roi = test_roi();
    let generation = roi.dirty_state.authoritative;
    let key = ContourViewKey::from_plane(
        crate::convert::orthogonal_plane_from_volume_uv(
            PlaneFamily::Axial,
            [0.5, 0.5, 0.5],
            VoxelGeometry::new([1, 1, 1], [1.0; 3], [0.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap(),
        )
        .unwrap(),
    );

    roi.install_current_contour_view_result(
        key.clone(),
        ContourData {
            active_plane_family: OrthogonalFamily::Axial,
            slices: Vec::new(),
        }
        .slices,
        generation,
    )
    .unwrap();

    assert_eq!(
        roi.contour_view_cache(&key).unwrap().geometry_identity,
        roi.reference_geometry().identity()
    );
}

#[test]
fn test_mismatched_mesh_geometry_stamp_is_not_current() {
    let mut roi = test_roi();
    let generation = roi.dirty_state.authoritative.shape;
    roi.install_mesh_cache_result(
        MeshCache {
            data: crate::app::roi::MeshData {
                vertices: Vec::new(),
                faces: Vec::new(),
            },
            chunks: None,
        },
        generation,
    )
    .unwrap();
    roi.session_caches.mesh_geometry_identity = Some(
        VoxelGeometry::new([2, 1, 1], [1.0; 3], [0.0; 3], [0.0, 0.0, 0.0, 1.0])
            .unwrap()
            .identity(),
    );

    assert!(!roi.is_cache_current(RoiCacheKind::Mesh));
}
