use crate::app::components::{
    CacheGeneration, CacheViewState, ContourCache, ContourData, ContourViewCache, ContourViewKey,
    GpuVolumeResources, MeshCache, PreviewMeshCache, PreviewVoxelCache, Roi, RoiCacheKind,
    RoiDirtyState, RoiJobKind, VoxelCache, VoxelData, MAX_CONTOUR_VIEW_CACHE_ENTRIES,
};
use crate::convert::RoiGeometry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheInstallError {
    StaleGeneration { expected: u64, actual: u64 },
    PreviewInactive,
    StalePreviewRevision { expected: u64, actual: u64 },
    GeometryMismatch,
}

impl Roi {
    pub fn mesh_cache(&self) -> Option<&MeshCache> {
        self.session_caches.mesh.as_ref()
    }

    pub fn mesh_cache_mut(&mut self) -> Option<&mut MeshCache> {
        self.session_caches.mesh.as_mut()
    }

    pub fn voxel_cache(&self) -> Option<&VoxelCache> {
        self.session_caches.voxel.as_ref()
    }

    pub fn voxel_cache_mut(&mut self) -> Option<&mut VoxelCache> {
        self.session_caches.voxel.as_mut()
    }

    pub fn voxel_gpu_cache(&self) -> Option<&GpuVolumeResources> {
        self.voxel_cache()?.gpu_resources.as_ref()
    }

    pub fn voxel_gpu_cache_mut(&mut self) -> Option<&mut GpuVolumeResources> {
        self.voxel_cache_mut()?.gpu_resources.as_mut()
    }

    pub fn contour_cache(&self) -> Option<&ContourCache> {
        self.session_caches.contour.as_ref()
    }

    pub fn contour_cache_mut(&mut self) -> Option<&mut ContourCache> {
        self.session_caches.contour.as_mut()
    }

    pub fn ensure_contour_cache(&mut self) -> &mut ContourCache {
        self.session_caches
            .contour
            .get_or_insert_with(|| ContourCache { views: Vec::new() })
    }

    pub fn upsert_contour_view_cache(
        &mut self,
        key: ContourViewKey,
        data: ContourData,
        source_generation: u64,
        state: CacheViewState,
    ) {
        let geometry_identity = self.reference_geometry().identity();
        let cache = self.ensure_contour_cache();
        if let Some(existing) = cache
            .views
            .iter_mut()
            .find(|view| view.key.logical_eq(&key))
        {
            existing.data = data;
            existing.source_generation = source_generation;
            existing.geometry_identity = geometry_identity;
            existing.state = state;
            existing.key = key;
        } else {
            cache.views.push(ContourViewCache {
                key,
                data,
                source_generation,
                geometry_identity,
                state,
            });
            if cache.views.len() > MAX_CONTOUR_VIEW_CACHE_ENTRIES {
                cache.views.remove(0);
            }
        }
    }

    pub fn contour_view_cache(&self, key: &ContourViewKey) -> Option<&ContourViewCache> {
        self.contour_cache()?
            .views
            .iter()
            .find(|view| view.key.logical_eq(key))
    }

    pub fn contour_view_cache_mut(
        &mut self,
        key: &ContourViewKey,
    ) -> Option<&mut ContourViewCache> {
        self.contour_cache_mut()?
            .views
            .iter_mut()
            .find(|view| view.key.logical_eq(key))
    }

    pub fn contour_view_data_for_render(&self, key: &ContourViewKey) -> Option<&ContourData> {
        if let Some(contour) = self.contour_data() {
            if contour.active_plane_family == key.family {
                return Some(contour);
            }
        }

        let view = self.contour_view_cache(key)?;
        let mesh_view_usable = match view.state {
            CacheViewState::Current => true,
            CacheViewState::Preview { .. } => self.preview_state.active,
            _ => false,
        };
        if self.mesh_data().is_some()
            && (view.source_generation != self.dirty_state.generations.authoritative
                || !mesh_view_usable)
        {
            return None;
        }
        if matches!(
            view.state,
            CacheViewState::Blocked { .. } | CacheViewState::Unsupported { .. }
        ) {
            return None;
        }
        Some(&view.data)
    }

    pub fn mark_all_contour_view_caches_stale(&mut self) {
        if let Some(cache) = self.contour_cache_mut() {
            for view in &mut cache.views {
                if !matches!(
                    view.state,
                    CacheViewState::Blocked { .. } | CacheViewState::Unsupported { .. }
                ) {
                    view.state = CacheViewState::Stale;
                }
            }
        }
    }

    pub fn cache_generation(&self, kind: RoiCacheKind) -> u64 {
        match kind {
            RoiCacheKind::Voxel => self.dirty_state.generations.voxel,
            RoiCacheKind::Contour => self.dirty_state.generations.contour,
            RoiCacheKind::Mesh => self.dirty_state.generations.mesh,
        }
    }

    pub fn is_cache_dirty(&self, kind: RoiCacheKind) -> bool {
        match kind {
            RoiCacheKind::Voxel => self.dirty_state.voxel_cache_dirty,
            RoiCacheKind::Contour => self.dirty_state.contour_cache_dirty,
            RoiCacheKind::Mesh => self.dirty_state.mesh_cache_dirty,
        }
    }

    pub fn is_cache_current(&self, kind: RoiCacheKind) -> bool {
        !self.is_cache_dirty(kind)
            && self.cache_generation(kind) == self.dirty_state.generations.authoritative
            && self.cache_geometry_matches(kind)
    }

    pub fn mark_authoritative_changed(&mut self) {
        self.dirty_state.authoritative_dirty = true;
        self.dirty_state.generations.authoritative += 1;
        self.mark_cache_dirty(RoiCacheKind::Voxel);
        self.mark_cache_dirty(RoiCacheKind::Contour);
        self.mark_cache_dirty(RoiCacheKind::Mesh);
    }

    pub fn mark_contour_authoritative_changed(&mut self) {
        self.dirty_state.authoritative_dirty = true;
        self.dirty_state.generations.authoritative += 1;
        self.mark_cache_dirty(RoiCacheKind::Voxel);
        self.mark_cache_dirty(RoiCacheKind::Mesh);
        if self.session_caches.contour.is_some() {
            self.mark_cache_dirty(RoiCacheKind::Contour);
        }
    }

    pub fn mark_mesh_authoritative_changed(&mut self) {
        self.dirty_state.authoritative_dirty = true;
        self.dirty_state.generations.authoritative += 1;
        self.validated_mesh_generation = None;
        self.mark_cache_dirty(RoiCacheKind::Voxel);
        self.mark_cache_dirty(RoiCacheKind::Contour);
        if self.session_caches.mesh.is_some() {
            self.mark_cache_dirty(RoiCacheKind::Mesh);
        }
    }

    pub fn mark_cache_dirty(&mut self, kind: RoiCacheKind) {
        match kind {
            RoiCacheKind::Voxel => self.dirty_state.voxel_cache_dirty = true,
            RoiCacheKind::Contour => self.dirty_state.contour_cache_dirty = true,
            RoiCacheKind::Mesh => self.dirty_state.mesh_cache_dirty = true,
        }
    }

    pub fn finish_cache_rebuild(&mut self, kind: RoiCacheKind) {
        let authoritative_generation = self.dirty_state.generations.authoritative;
        match kind {
            RoiCacheKind::Voxel => {
                self.dirty_state.voxel_cache_dirty = false;
                self.dirty_state.generations.voxel = authoritative_generation;
                self.finish_job(RoiJobKind::RebuildVoxelCache);
            }
            RoiCacheKind::Contour => {
                self.dirty_state.contour_cache_dirty = false;
                self.dirty_state.generations.contour = authoritative_generation;
                self.finish_job(RoiJobKind::RebuildContourCache);
            }
            RoiCacheKind::Mesh => {
                self.dirty_state.mesh_cache_dirty = false;
                self.dirty_state.generations.mesh = authoritative_generation;
                self.finish_job(RoiJobKind::RebuildMeshCache);
            }
        }
        self.dirty_state.authoritative_dirty = false;
    }

    pub fn install_voxel_cache_result(
        &mut self,
        cache: VoxelCache,
        source_generation: u64,
    ) -> Result<(), CacheInstallError> {
        self.validate_source_generation(source_generation)?;
        self.validate_voxel_cache_geometry(&cache)?;
        self.session_caches.voxel = Some(cache);
        self.finish_cache_rebuild(RoiCacheKind::Voxel);
        Ok(())
    }

    pub fn store_stale_voxel_cache(&mut self, cache: VoxelCache) {
        self.session_caches.voxel = Some(cache);
        self.dirty_state.voxel_cache_dirty = true;
        self.dirty_state.generations.voxel = 0;
    }

    pub fn discard_cache(&mut self, kind: RoiCacheKind) {
        match kind {
            RoiCacheKind::Voxel => {
                self.session_caches.voxel = None;
                self.dirty_state.generations.voxel = 0;
            }
            RoiCacheKind::Contour => {
                self.session_caches.contour = None;
                self.dirty_state.generations.contour = 0;
            }
            RoiCacheKind::Mesh => {
                self.session_caches.mesh = None;
                self.session_caches.mesh_geometry_identity = None;
                self.dirty_state.generations.mesh = 0;
            }
        }
        self.mark_cache_dirty(kind);
    }

    pub(crate) fn rebase_after_contour_promotion(
        &mut self,
        source_voxel: VoxelData,
        replacement_mesh: Option<MeshCache>,
        mesh_cache_is_current: bool,
    ) {
        if let Some(mesh) = replacement_mesh {
            self.session_caches.mesh = Some(mesh);
            self.session_caches.mesh_geometry_identity = Some(self.reference_geometry().identity());
        }
        self.session_caches.contour = None;
        if let Some(voxel_cache) = self.session_caches.voxel.as_mut() {
            voxel_cache.data = source_voxel;
        } else {
            self.session_caches.voxel = Some(VoxelCache {
                data: source_voxel,
                gpu_resources: None,
            });
        }

        let generation = self.next_authoritative_generation();
        self.dirty_state = RoiDirtyState {
            authoritative_dirty: true,
            voxel_cache_dirty: false,
            contour_cache_dirty: false,
            mesh_cache_dirty: !mesh_cache_is_current,
            generations: CacheGeneration {
                authoritative: generation,
                voxel: generation,
                contour: generation,
                mesh: if mesh_cache_is_current { generation } else { 0 },
            },
        };
    }

    pub(crate) fn rebase_after_voxel_promotion(
        &mut self,
        source_voxel: VoxelData,
        retained_mesh: Option<MeshCache>,
    ) {
        let mesh_cache_is_current = retained_mesh.is_some();
        if let Some(voxel_cache) = self.session_caches.voxel.as_mut() {
            voxel_cache.data = source_voxel;
        } else {
            unreachable!("current voxel cache was checked before promotion");
        }
        self.session_caches.contour = None;
        self.session_caches.mesh = retained_mesh;
        self.session_caches.mesh_geometry_identity = self
            .session_caches
            .mesh
            .as_ref()
            .map(|_| self.reference_geometry().identity());

        let generation = self.next_authoritative_generation();
        self.dirty_state = RoiDirtyState {
            authoritative_dirty: true,
            voxel_cache_dirty: false,
            contour_cache_dirty: true,
            mesh_cache_dirty: !mesh_cache_is_current,
            generations: CacheGeneration {
                authoritative: generation,
                voxel: generation,
                contour: 0,
                mesh: if mesh_cache_is_current { generation } else { 0 },
            },
        };
    }

    pub(crate) fn rebase_after_mesh_promotion(&mut self, voxel_cache_is_current: bool) {
        self.validated_mesh_generation = None;
        self.session_caches.mesh = None;
        self.session_caches.mesh_geometry_identity = None;
        self.session_caches.contour = None;

        let generation = self.next_authoritative_generation();
        self.dirty_state = RoiDirtyState {
            authoritative_dirty: true,
            voxel_cache_dirty: !voxel_cache_is_current,
            contour_cache_dirty: true,
            mesh_cache_dirty: false,
            generations: CacheGeneration {
                authoritative: generation,
                voxel: if voxel_cache_is_current {
                    generation
                } else {
                    0
                },
                contour: 0,
                mesh: generation,
            },
        };
    }

    pub fn install_mesh_cache_result(
        &mut self,
        cache: MeshCache,
        source_generation: u64,
    ) -> Result<(), CacheInstallError> {
        self.validate_source_generation(source_generation)?;
        self.session_caches.mesh = Some(cache);
        self.session_caches.mesh_geometry_identity = Some(self.reference_geometry().identity());
        self.finish_cache_rebuild(RoiCacheKind::Mesh);
        Ok(())
    }

    pub fn install_current_contour_view_result(
        &mut self,
        key: ContourViewKey,
        data: ContourData,
        source_generation: u64,
    ) -> Result<(), CacheInstallError> {
        self.install_contour_view_result(key, data, source_generation, CacheViewState::Current)
    }

    pub fn install_contour_view_result(
        &mut self,
        key: ContourViewKey,
        data: ContourData,
        source_generation: u64,
        state: CacheViewState,
    ) -> Result<(), CacheInstallError> {
        self.validate_source_generation(source_generation)?;
        let is_current = state == CacheViewState::Current;
        self.upsert_contour_view_cache(key, data, source_generation, state);
        if is_current {
            self.dirty_state.contour_cache_dirty = false;
            self.dirty_state.generations.contour = source_generation;
        }
        Ok(())
    }

    pub fn install_preview_voxel_result(
        &mut self,
        cache: PreviewVoxelCache,
    ) -> Result<(), CacheInstallError> {
        self.validate_preview_source(cache.source_generation, cache.preview_revision)?;
        self.validate_voxel_data_geometry(&cache.data)?;
        self.session_caches.preview_voxel = Some(cache);
        Ok(())
    }

    pub fn install_preview_mesh_result(
        &mut self,
        cache: PreviewMeshCache,
    ) -> Result<(), CacheInstallError> {
        self.validate_preview_source(cache.source_generation, cache.preview_revision)?;
        self.session_caches.preview_mesh = Some(cache);
        self.session_caches.preview_mesh_geometry_identity =
            Some(self.reference_geometry().identity());
        Ok(())
    }

    pub fn renderable_voxel_cache(&self) -> Option<&GpuVolumeResources> {
        if self.metadata.is_visible && self.is_cache_current(RoiCacheKind::Voxel) {
            self.voxel_gpu_cache()
        } else {
            None
        }
    }

    pub fn update_voxel_bind_group(&mut self, bind_group: wgpu::BindGroup) {
        if let Some(resources) = self.voxel_gpu_cache_mut() {
            resources.bind_group = bind_group;
        }
    }

    fn validate_source_generation(&self, source_generation: u64) -> Result<(), CacheInstallError> {
        let expected = self.dirty_state.generations.authoritative;
        if source_generation != expected {
            return Err(CacheInstallError::StaleGeneration {
                expected,
                actual: source_generation,
            });
        }
        Ok(())
    }

    fn validate_voxel_cache_geometry(&self, cache: &VoxelCache) -> Result<(), CacheInstallError> {
        self.validate_voxel_data_geometry(&cache.data)
    }

    fn cache_geometry_matches(&self, kind: RoiCacheKind) -> bool {
        match kind {
            RoiCacheKind::Voxel => self
                .voxel_cache()
                .is_none_or(|cache| self.validate_voxel_cache_geometry(cache).is_ok()),
            RoiCacheKind::Contour => self.contour_cache().is_none_or(|cache| {
                cache
                    .views
                    .iter()
                    .all(|view| view.geometry_identity == self.reference_geometry().identity())
            }),
            // Test fixtures may install a mesh directly. Production installation stamps it.
            RoiCacheKind::Mesh => self
                .session_caches
                .mesh_geometry_identity
                .is_none_or(|identity| identity == self.reference_geometry().identity()),
        }
    }

    fn validate_voxel_data_geometry(&self, data: &VoxelData) -> Result<(), CacheInstallError> {
        let reference = self.reference_geometry();
        let actual = RoiGeometry::from_legacy_parts(
            data.geometry.dimensions,
            data.geometry.spacing,
            data.geometry.origin,
            data.geometry.orientation,
        )
        .map_err(|_| CacheInstallError::GeometryMismatch)?;
        (actual.identity() == reference.identity())
            .then_some(())
            .ok_or(CacheInstallError::GeometryMismatch)
    }

    fn next_authoritative_generation(&self) -> u64 {
        self.dirty_state.generations.authoritative.saturating_add(1)
    }

    fn validate_preview_source(
        &self,
        source_generation: u64,
        preview_revision: u64,
    ) -> Result<(), CacheInstallError> {
        self.validate_source_generation(source_generation)?;
        if !self.preview_state.active {
            return Err(CacheInstallError::PreviewInactive);
        }
        let expected = self.preview_state.revision;
        if preview_revision != expected {
            return Err(CacheInstallError::StalePreviewRevision {
                expected,
                actual: preview_revision,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::components::{RoiId, VoxelGeometry};
    use crate::convert::PlaneFamily;

    fn test_roi() -> Roi {
        Roi::new_contour(
            RoiId(1),
            "test".to_string(),
            ContourData {
                active_plane_family: PlaneFamily::Axial,
                slices: Vec::new(),
            },
        )
    }

    fn test_voxel_cache(value: u8) -> VoxelCache {
        VoxelCache {
            data: crate::app::roi::VoxelData {
                geometry: VoxelGeometry {
                    dimensions: [1, 1, 1],
                    spacing: [1.0; 3],
                    origin: [0.0; 3],
                    orientation: [0.0, 0.0, 0.0, 1.0],
                },
                raw_data: vec![value],
            },
            gpu_resources: None,
        }
    }

    #[test]
    fn test_voxel_cache_result_with_mismatched_reference_geometry_is_rejected() {
        let reference_geometry = RoiGeometry::from_legacy_parts(
            [2, 2, 2],
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap();
        let mut roi = Roi::new_contour_with_geometry(
            RoiId(1),
            "test".to_string(),
            reference_geometry,
            ContourData {
                active_plane_family: PlaneFamily::Axial,
                slices: Vec::new(),
            },
        );
        let generation = roi.dirty_state.generations.authoritative;

        let result = roi.install_voxel_cache_result(test_voxel_cache(1), generation);

        assert_eq!(result, Err(CacheInstallError::GeometryMismatch));
        assert!(roi.voxel_cache().is_none());
    }

    #[test]
    fn test_stale_voxel_result_is_rejected_without_overwriting_cache() {
        let mut roi = test_roi();
        let generation = roi.dirty_state.generations.authoritative;
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
        let generation = roi.dirty_state.generations.authoritative;
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
        let generation = roi.dirty_state.generations.authoritative;

        roi.install_voxel_cache_result(test_voxel_cache(7), generation)
            .unwrap();

        assert_eq!(roi.voxel_cache().unwrap().data.raw_data, vec![7]);
        assert!(roi.is_cache_current(RoiCacheKind::Voxel));
        assert_eq!(roi.cache_generation(RoiCacheKind::Voxel), generation);
    }

    #[test]
    fn test_mesh_result_is_stamped_with_roi_geometry_identity() {
        let mut roi = test_roi();
        let generation = roi.dirty_state.generations.authoritative;

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
        let generation = roi.dirty_state.generations.authoritative;
        let key = ContourViewKey::from_plane(
            crate::convert::orthogonal_plane_from_volume_uv(
                PlaneFamily::Axial,
                [0.5, 0.5, 0.5],
                VoxelGeometry {
                    dimensions: [1, 1, 1],
                    spacing: [1.0; 3],
                    origin: [0.0; 3],
                    orientation: [0.0, 0.0, 0.0, 1.0],
                },
            )
            .unwrap(),
        );

        roi.install_current_contour_view_result(
            key.clone(),
            ContourData {
                active_plane_family: PlaneFamily::Axial,
                slices: Vec::new(),
            },
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
        let generation = roi.dirty_state.generations.authoritative;
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
            RoiGeometry::from_legacy_parts([2, 1, 1], [1.0; 3], [0.0; 3], [0.0, 0.0, 0.0, 1.0])
                .unwrap()
                .identity(),
        );

        assert!(!roi.is_cache_current(RoiCacheKind::Mesh));
    }
}
