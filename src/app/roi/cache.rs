use crate::app::components::{
    CacheFreshness, CacheViewState, ContourCache, ContourSlice, ContourViewCache, ContourViewKey,
    GpuVolumeResources, MeshCache, PreviewMeshCache, PreviewVoxelCache, Revision, Roi,
    RoiCacheKind, RoiDirtyState, RoiJobKind, VoxelCache, VoxelData, MAX_CONTOUR_VIEW_CACHE_ENTRIES,
};

/// How a shape change treats one derived cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Invalidate {
    /// Dirty even when the cache is absent, so it is rebuilt when next demanded.
    Always,
    /// Dirty only when a cache exists.
    WhenPresent,
}

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
        data: Vec<ContourSlice>,
        built_from: Revision,
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
            existing.built_from = built_from;
            existing.geometry_identity = geometry_identity;
            existing.state = state;
            existing.key = key;
        } else {
            cache.views.push(ContourViewCache {
                key,
                data,
                built_from,
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

    /// The slices to draw for one view: the authoritative contours when the view is in the
    /// authoritative family, else a derived view that matches the current revision.
    pub fn contour_view_data_for_render(&self, key: &ContourViewKey) -> Option<&[ContourSlice]> {
        if let Some(contour) = self.contour_data() {
            if contour.active_plane_family == key.family {
                return Some(&contour.slices);
            }
        }

        // Nothing stale is ever drawn: a derived view is used only when it was built from the
        // ROI's current revision, or when it is the preview of the edit in progress.
        let view = self.contour_view_cache(key)?;
        let usable = match view.state {
            CacheViewState::Current => view.built_from == self.dirty_state.authoritative,
            CacheViewState::Preview { .. } => self.preview_state.active,
            _ => false,
        };
        usable.then_some(view.data.as_slice())
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
        self.dirty_state.freshness(kind).built_from.shape
    }

    pub fn is_cache_dirty(&self, kind: RoiCacheKind) -> bool {
        self.dirty_state.freshness(kind).dirty
    }

    /// The single freshness rule: not invalidated, built from the current authoritative
    /// revision, and on the ROI's reference grid.
    pub fn is_cache_current(&self, kind: RoiCacheKind) -> bool {
        self.dirty_state
            .freshness(kind)
            .is_current(self.dirty_state.authoritative, kind)
            && self.cache_geometry_matches(kind)
    }

    /// Every derived cache is invalidated, including absent ones.
    #[cfg(test)]
    pub fn mark_authoritative_changed(&mut self) {
        self.invalidate_for_shape_change([Invalidate::Always; 3]);
    }

    /// A contour edit invalidates the voxel and mesh caches always, and the contour views only
    /// when some exist.
    pub fn mark_contour_authoritative_changed(&mut self) {
        self.invalidate_for_shape_change([
            Invalidate::Always,
            Invalidate::WhenPresent,
            Invalidate::Always,
        ]);
    }

    /// A voxel body was installed: the voxel cache must be rebuilt from it, and the derived
    /// contour and mesh forms are stale.
    pub fn mark_voxel_authoritative_changed(&mut self) {
        self.invalidate_for_shape_change([
            Invalidate::Always,
            Invalidate::WhenPresent,
            Invalidate::Always,
        ]);
        self.session_caches.contour = None;
        self.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
    }

    /// A mesh edit invalidates the voxel and contour caches always, and the mesh cache only when
    /// one exists.
    pub fn mark_mesh_authoritative_changed(&mut self) {
        self.validated_mesh_generation = None;
        self.invalidate_for_shape_change([
            Invalidate::Always,
            Invalidate::Always,
            Invalidate::WhenPresent,
        ]);
    }

    /// Bumps the shape revision and invalidates caches by rule, in voxel, contour, mesh order.
    fn invalidate_for_shape_change(&mut self, rules: [Invalidate; 3]) {
        self.dirty_state.authoritative_dirty = true;
        self.dirty_state.authoritative = self.dirty_state.authoritative.next_shape();
        for (kind, rule) in [
            RoiCacheKind::Voxel,
            RoiCacheKind::Contour,
            RoiCacheKind::Mesh,
        ]
        .into_iter()
        .zip(rules)
        {
            if rule == Invalidate::Always || self.has_cache(kind) {
                self.mark_cache_dirty(kind);
            }
        }
    }

    fn has_cache(&self, kind: RoiCacheKind) -> bool {
        match kind {
            RoiCacheKind::Voxel => self.session_caches.voxel.is_some(),
            RoiCacheKind::Contour => self.session_caches.contour.is_some(),
            RoiCacheKind::Mesh => self.session_caches.mesh.is_some(),
        }
    }

    pub fn mark_cache_dirty(&mut self, kind: RoiCacheKind) {
        self.dirty_state.freshness_mut(kind).dirty = true;
    }

    pub fn finish_cache_rebuild(&mut self, kind: RoiCacheKind) {
        *self.dirty_state.freshness_mut(kind) =
            CacheFreshness::built_from(self.dirty_state.authoritative);
        self.finish_job(kind.rebuild_job());
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
        self.dirty_state.voxel = CacheFreshness::invalidated();
    }

    #[cfg(test)]
    pub fn discard_cache(&mut self, kind: RoiCacheKind) {
        match kind {
            RoiCacheKind::Voxel => self.session_caches.voxel = None,
            RoiCacheKind::Contour => self.session_caches.contour = None,
            RoiCacheKind::Mesh => {
                self.session_caches.mesh = None;
                self.session_caches.mesh_geometry_identity = None;
            }
        }
        *self.dirty_state.freshness_mut(kind) = CacheFreshness::invalidated();
    }

    pub(crate) fn rebase_after_switch_to_contour(
        &mut self,
        source_voxel: VoxelData,
        mesh_cache_is_current: bool,
    ) {
        self.session_caches.contour = None;
        if let Some(voxel_cache) = self.session_caches.voxel.as_mut() {
            voxel_cache.data = source_voxel;
        } else {
            self.session_caches.voxel = Some(VoxelCache {
                data: source_voxel,
                gpu_resources: None,
            });
        }

        let revision = self.dirty_state.authoritative.next_shape();
        self.dirty_state = RoiDirtyState {
            authoritative_dirty: true,
            authoritative: revision,
            voxel: CacheFreshness::built_from(revision),
            contour: CacheFreshness::built_from(revision),
            mesh: if mesh_cache_is_current {
                CacheFreshness::built_from(revision)
            } else {
                CacheFreshness::invalidated()
            },
        };
    }

    /// A mesh ROI became contours by cutting its mesh, which keeps the mesh as the current mesh
    /// cache. The voxels are no longer those of the shape (they were made from the mesh, not from
    /// the new contours), so they are stale until rebuilt from the contours.
    pub(crate) fn rebase_after_cut_to_contour(&mut self, previous_mesh: MeshCache) {
        self.validated_mesh_generation = None;
        self.session_caches.mesh = Some(previous_mesh);
        self.session_caches.mesh_geometry_identity = Some(self.reference_geometry().identity());
        self.session_caches.contour = None;
        let revision = self.dirty_state.authoritative.next_shape();
        self.dirty_state = RoiDirtyState {
            authoritative_dirty: true,
            authoritative: revision,
            voxel: CacheFreshness::invalidated(),
            contour: CacheFreshness::built_from(revision),
            mesh: CacheFreshness::built_from(revision),
        };
    }

    pub(crate) fn rebase_after_switch_to_mesh(&mut self, voxel_cache_is_current: bool) {
        self.validated_mesh_generation = None;
        self.session_caches.mesh = None;
        self.session_caches.mesh_geometry_identity = None;
        self.session_caches.contour = None;

        let revision = self.dirty_state.authoritative.next_shape();
        self.dirty_state = RoiDirtyState {
            authoritative_dirty: true,
            authoritative: revision,
            voxel: if voxel_cache_is_current {
                CacheFreshness::built_from(revision)
            } else {
                CacheFreshness::invalidated()
            },
            contour: CacheFreshness::invalidated(),
            mesh: CacheFreshness::built_from(revision),
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
        data: Vec<ContourSlice>,
        built_from: Revision,
    ) -> Result<(), CacheInstallError> {
        self.install_contour_view_result(key, data, built_from, CacheViewState::Current)
    }

    pub fn install_contour_view_result(
        &mut self,
        key: ContourViewKey,
        data: Vec<ContourSlice>,
        built_from: Revision,
        state: CacheViewState,
    ) -> Result<(), CacheInstallError> {
        self.validate_source_revision(built_from)?;
        let is_current = state == CacheViewState::Current;
        self.upsert_contour_view_cache(key, data, built_from, state);
        if is_current {
            self.dirty_state.contour = CacheFreshness::built_from(built_from);
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

    pub fn renderable_voxel_cache(&self, is_visible: bool) -> Option<&GpuVolumeResources> {
        if is_visible && self.is_cache_current(RoiCacheKind::Voxel) {
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

    /// Contour views follow the form as well as the shape, so their results must match the full
    /// authoritative revision.
    fn validate_source_revision(&self, built_from: Revision) -> Result<(), CacheInstallError> {
        self.validate_source_generation(built_from.shape)?;
        if built_from.form != self.dirty_state.authoritative.form {
            return Err(CacheInstallError::StaleGeneration {
                expected: self.dirty_state.authoritative.form,
                actual: built_from.form,
            });
        }
        Ok(())
    }

    fn validate_source_generation(&self, source_generation: u64) -> Result<(), CacheInstallError> {
        let expected = self.dirty_state.authoritative.shape;
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

    /// Voxel data is valid for this ROI when it is the whole reference grid or a box of it.
    fn validate_voxel_data_geometry(&self, data: &VoxelData) -> Result<(), CacheInstallError> {
        (data.geometry.identity() == self.reference_geometry().identity()
            || data.geometry.offset_in(self.reference_geometry()).is_some())
        .then_some(())
        .ok_or(CacheInstallError::GeometryMismatch)
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
mod tests;

/// An estimate of the bytes a ROI holds, for the scale budget (QA state and the scale guard). It
/// counts the big arrays: voxel body and cache (each mirrored once on the GPU), meshes, contour
/// points, and the undo and redo snapshots. It does not count allocator overhead.
impl Roi {
    pub fn approx_bytes(&self) -> usize {
        use crate::app::components::{MeshData, RoiBody, RoiEditSnapshot};
        fn voxel(data: &VoxelData) -> usize {
            data.raw_data.len()
        }
        fn mesh(data: &MeshData) -> usize {
            data.vertices.len() * 12 + data.faces.len() * 12
        }
        fn contours(slices: &[ContourSlice]) -> usize {
            slices
                .iter()
                .flat_map(|slice| &slice.loops)
                .map(|contour| contour.points.len() * 8)
                .sum()
        }
        fn snapshot(snapshot: &RoiEditSnapshot) -> usize {
            match snapshot {
                RoiEditSnapshot::Voxel(data) => voxel(data),
                RoiEditSnapshot::Contour(data) => contours(&data.slices),
                RoiEditSnapshot::Mesh(data) => mesh(data),
            }
        }
        let body = match &self.body {
            RoiBody::Voxel(body) => voxel(&body.data),
            RoiBody::Contour(body) => contours(&body.data.slices),
            RoiBody::Mesh(body) => mesh(&body.data),
        };
        let caches = &self.session_caches;
        let voxel_cache = caches.voxel.as_ref().map_or(0, |cache| {
            voxel(&cache.data) * if cache.gpu_resources.is_some() { 2 } else { 1 }
        });
        let mesh_cache = caches.mesh.as_ref().map_or(0, |cache| {
            mesh(&cache.data)
                + cache.chunks.as_ref().map_or(0, |chunked| {
                    chunked.chunks.iter().map(|chunk| mesh(&chunk.data)).sum()
                })
        });
        let contour_cache = caches.contour.as_ref().map_or(0, |cache| {
            cache.views.iter().map(|view| contours(&view.data)).sum()
        });
        let history: usize = self
            .history
            .undo
            .iter()
            .chain(&self.history.redo)
            .map(|entry| snapshot(&entry.snapshot))
            .sum();
        body + voxel_cache + mesh_cache + contour_cache + history
    }
}
