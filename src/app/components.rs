use crate::convert::{PlaneDefinition, PlaneFamily};
use glam::Vec3;

use winit::keyboard::ModifiersState;

// --- Basic Tags ---
pub struct CursorTag;
pub struct MainVolumeTag;
pub struct RoiTag;

// --- Window & View ---
pub struct WindowSettings {
    pub width: u32,
    pub height: u32,
    pub viewport_rect: [f32; 4],
}

pub struct Transform {
    pub position: [f32; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    ThreeD = 0,
    Axial = 1,
    Coronal = 2,
    Sagittal = 3,
    Oblique = 4,
}

pub struct Viewport {
    pub mode: ViewMode,
    pub rect: [f32; 4], // [x, y, w, h] in pixels (physical)
    pub uniform_index: u32,
}

#[derive(Clone, Copy)]
pub struct ViewportState {
    pub zoom: f32,
    pub pan: [f32; 2],
    pub pivot: [f32; 2],
    pub user_rotation: [f32; 4],
}

/// Viewport Layout (Normalized 0..1 coordinates)
pub struct ViewportLayout {
    pub relative_rect: [f32; 4], // [x, y, w, h]
}

/// Global Protocol State
pub struct ProtocolState {
    pub active_protocol: String,
    pub last_protocol: Option<String>,
}

impl Default for ProtocolState {
    fn default() -> Self {
        Self {
            active_protocol: "Standard 2x2".to_string(),
            last_protocol: None,
        }
    }
}

impl Default for ViewportState {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: [0.0, 0.0],
            pivot: [0.5, 0.5],
            user_rotation: [0.0, 0.0, 0.0, 1.0],
        }
    }
}

#[derive(Default)]
pub struct InputState {
    pub last_mouse_pos: [f64; 2],
    pub mouse_uv: [f32; 2],
    pub active_viewport: Option<hecs::Entity>,
    pub modifiers: ModifiersState,
    pub is_dragging: bool,
    pub is_panning: bool,
    pub drag_start_pos: [f32; 2],
    pub drag_start_pan: [f32; 2],
    pub is_rotating: bool,
    pub rotation_start_pos: [f32; 2],
    pub rotation_start_val: [f32; 4],
    pub egui_wants_input: bool,
    pub scroll_accumulator: [f32; 4], // Accumulate sub-slice deltas per viewport
    pub contour_move_pending_commit: bool,
}

// --- Volume Data ---
pub struct VolumeData {
    pub dimensions: [u32; 3],
    pub spacing: [f32; 3],
    pub origin: [f32; 3],
    pub intensities: Vec<f32>,
    pub intensity_range: [f32; 2],
    pub orientation: [f32; 4], // Quaternion
}

impl VolumeData {
    pub fn aspect_ratios(&self) -> [f32; 3] {
        let d = self.dimensions;
        let s = self.spacing;
        // avoid div by zero if empty
        if d[0] == 0 || d[1] == 0 || d[2] == 0 {
            return [1.0, 1.0, 1.0];
        }
        let max_dim = (d[0] as f32 * s[0])
            .max(d[1] as f32 * s[1])
            .max(d[2] as f32 * s[2]);
        [
            (d[0] as f32 * s[0]) / max_dim,
            (d[1] as f32 * s[1]) / max_dim,
            (d[2] as f32 * s[2]) / max_dim,
        ]
    }
}

// --- GPU Resources ---
#[derive(Clone)]
pub struct GpuVolumeResources {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub bind_group: wgpu::BindGroup,
}

// --- Uniforms ---
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniforms {
    pub cursor_pos: [f32; 4],
    pub volume_dims: [u32; 4],
    pub volume_spacing: [f32; 4],
    pub overlay1_dims: [u32; 4],
    pub overlay2_dims: [u32; 4],
    pub overlay_opacities: [f32; 4],
    pub overlay1_main_to_roi_row0: [f32; 4],
    pub overlay1_main_to_roi_row1: [f32; 4],
    pub overlay1_main_to_roi_row2: [f32; 4],
    pub overlay2_main_to_roi_row0: [f32; 4],
    pub overlay2_main_to_roi_row1: [f32; 4],
    pub overlay2_main_to_roi_row2: [f32; 4],
    pub window_params: [f32; 4],
    pub resolution: [f32; 2],
    pub mouse_uv: [f32; 2],
    pub pan: [f32; 2],
    pub zoom_pivot: [f32; 2],
    pub rotation: [f32; 4], // Quaternion
    // --- Overlay primitive fields ---
    pub overlay_mouse_uv: [f32; 2], // Mouse position for dragged primitive
    pub overlay_primitive_count: u32, // Number of active primitives
    pub overlay_dragging_idx: u32,  // Index being dragged (u32::MAX = none)
    pub zoom: f32,
    pub view_mode: u32,
    pub overlay_flags: u32,
    pub _padding: u32,
}

// --- GUI / Editor ---
#[derive(Clone)]
pub struct GuiState {
    pub status_message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditorTool {
    #[default]
    Navigation,
    ContourSelect,
    ContourDraw,
}

#[derive(Default)]
pub struct EditorState {
    pub active_roi: Option<hecs::Entity>,
    pub active_tool: EditorTool,
    pub contour_draft: Option<ContourDraft>,
    pub contour_selection: Option<ContourSelection>,
    pub contour_move_preview: Option<ContourMovePreview>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RoiId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimaryRepresentation {
    Voxel,
    Contour,
    Mesh,
}

#[derive(Debug, Clone)]
pub struct RoiMetadata {
    pub roi_id: RoiId,
    pub name: String,
    pub is_visible: bool,
    pub is_locked: bool,
    pub color: [f32; 4],
}

pub struct LayerSettings {
    pub opacity: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VoxelData {
    pub geometry: VoxelGeometry,
    pub raw_data: Vec<u8>,
}

#[derive(Clone)]
pub struct VoxelCache {
    pub data: VoxelData,
    pub gpu_resources: Option<GpuVolumeResources>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoxelGeometry {
    pub dimensions: [u32; 3],
    pub spacing: [f32; 3],
    pub origin: [f32; 3],
    pub orientation: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContourPoint {
    pub local_mm: [f32; 2],
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContourLoop {
    pub points: Vec<ContourPoint>,
    pub is_closed: bool,
}

impl ContourLoop {
    pub fn is_valid_closed_loop(&self) -> bool {
        self.is_closed && self.points.len() >= 3
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContourSlice {
    pub plane: PlaneDefinition,
    pub loops: Vec<ContourLoop>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContourData {
    pub active_plane_family: PlaneFamily,
    pub slices: Vec<ContourSlice>,
}

impl ContourData {
    pub fn is_empty(&self) -> bool {
        self.slices.is_empty()
    }

    pub fn has_loops(&self) -> bool {
        self.slices.iter().any(|slice| !slice.loops.is_empty())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshVertex {
    pub world_mm: [f32; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeshFace {
    pub vertex_indices: [u32; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct MeshData {
    pub vertices: Vec<MeshVertex>,
    pub faces: Vec<MeshFace>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContourDraft {
    pub roi_entity: hecs::Entity,
    pub plane: PlaneDefinition,
    pub points: Vec<ContourPoint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContourSelection {
    pub roi_entity: hecs::Entity,
    pub slice_index: usize,
    pub loop_index: usize,
    pub point_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContourMovePreview {
    pub roi_entity: hecs::Entity,
    pub contour_data: ContourData,
}

pub enum RoiAuthoritativeData {
    Voxel(VoxelData),
    Contour(ContourData),
    Mesh(MeshData),
}

#[derive(Default)]
pub struct RoiSessionCaches {
    pub voxel: Option<VoxelCache>,
    pub contour: Option<ContourCache>,
    pub mesh: Option<MeshCache>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContourCache {
    pub views: Vec<ContourViewCache>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContourViewCache {
    pub key: ContourViewKey,
    pub data: ContourData,
    pub source_generation: u64,
    pub state: CacheViewState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContourViewKey {
    pub family: PlaneFamily,
    pub plane: PlaneDefinition,
    pub slice_key: ContourSliceKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContourSliceKey {
    pub origin_quantized_mm: [i32; 3],
    pub normal_quantized: [i32; 3],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheViewState {
    Current,
    Stale,
    Rebuilding,
    Blocked { reason: String },
}

impl ContourSliceKey {
    pub fn from_plane(plane: PlaneDefinition) -> Self {
        Self {
            origin_quantized_mm: plane.origin_mm.map(|v| (v * 1000.0).round() as i32),
            normal_quantized: plane.normal_mm.map(|v| (v * 1_000_000.0).round() as i32),
        }
    }
}

impl ContourViewKey {
    pub fn from_plane(plane: PlaneDefinition) -> Self {
        Self {
            family: plane.family,
            plane,
            slice_key: ContourSliceKey::from_plane(plane),
        }
    }

    pub fn logical_eq(&self, other: &Self) -> bool {
        self.family == other.family && self.slice_key == other.slice_key
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MeshCache {
    pub data: MeshData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoiCacheKind {
    Voxel,
    Contour,
    Mesh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheGeneration {
    pub authoritative: u64,
    pub voxel: u64,
    pub contour: u64,
    pub mesh: u64,
}

impl Default for CacheGeneration {
    fn default() -> Self {
        Self {
            authoritative: 1,
            voxel: 0,
            contour: 0,
            mesh: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RoiDirtyState {
    pub authoritative_dirty: bool,
    pub voxel_cache_dirty: bool,
    pub contour_cache_dirty: bool,
    pub mesh_cache_dirty: bool,
    pub generations: CacheGeneration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoiJobKind {
    RebuildVoxelCache,
    RebuildContourCache,
    RebuildMeshCache,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RoiJobState {
    pub running: Option<RoiJobKind>,
    pub queued: Option<RoiJobKind>,
}

pub struct Roi {
    pub metadata: RoiMetadata,
    pub primary_representation: PrimaryRepresentation,
    pub authoritative_data: RoiAuthoritativeData,
    pub session_caches: RoiSessionCaches,
    pub dirty_state: RoiDirtyState,
    pub job_state: RoiJobState,
}

impl Roi {
    pub fn new_voxel(
        roi_id: RoiId,
        name: String,
        geometry: VoxelGeometry,
        raw_data: Vec<u8>,
        gpu_resources: GpuVolumeResources,
    ) -> Self {
        Self::new_voxel_with_cache(roi_id, name, geometry, raw_data, Some(gpu_resources))
    }

    pub fn new_voxel_with_cache(
        roi_id: RoiId,
        name: String,
        geometry: VoxelGeometry,
        raw_data: Vec<u8>,
        gpu_resources: Option<GpuVolumeResources>,
    ) -> Self {
        let has_voxel_gpu_cache = gpu_resources.is_some();
        let voxel_data = VoxelData { geometry, raw_data };
        Self {
            metadata: RoiMetadata {
                roi_id,
                name,
                is_visible: true,
                is_locked: false,
                color: [1.0, 0.2, 0.2, 1.0],
            },
            primary_representation: PrimaryRepresentation::Voxel,
            authoritative_data: RoiAuthoritativeData::Voxel(voxel_data.clone()),
            session_caches: RoiSessionCaches {
                voxel: Some(VoxelCache {
                    data: voxel_data,
                    gpu_resources,
                }),
                contour: None,
                mesh: None,
            },
            dirty_state: RoiDirtyState {
                voxel_cache_dirty: !has_voxel_gpu_cache,
                generations: CacheGeneration {
                    voxel: if has_voxel_gpu_cache { 1 } else { 0 },
                    ..CacheGeneration::default()
                },
                ..RoiDirtyState::default()
            },
            job_state: RoiJobState::default(),
        }
    }

    pub fn new_contour(roi_id: RoiId, name: String, contour_data: ContourData) -> Self {
        Self {
            metadata: RoiMetadata {
                roi_id,
                name,
                is_visible: true,
                is_locked: false,
                color: [1.0, 0.2, 0.2, 1.0],
            },
            primary_representation: PrimaryRepresentation::Contour,
            authoritative_data: RoiAuthoritativeData::Contour(contour_data),
            session_caches: RoiSessionCaches {
                voxel: None,
                contour: None,
                mesh: None,
            },
            dirty_state: RoiDirtyState {
                voxel_cache_dirty: true,
                ..RoiDirtyState::default()
            },
            job_state: RoiJobState::default(),
        }
    }

    pub fn new_mesh(roi_id: RoiId, name: String, mesh_data: MeshData) -> Self {
        Self {
            metadata: RoiMetadata {
                roi_id,
                name,
                is_visible: true,
                is_locked: false,
                color: [1.0, 0.2, 0.2, 1.0],
            },
            primary_representation: PrimaryRepresentation::Mesh,
            authoritative_data: RoiAuthoritativeData::Mesh(mesh_data),
            session_caches: RoiSessionCaches {
                voxel: None,
                contour: None,
                mesh: None,
            },
            dirty_state: RoiDirtyState {
                voxel_cache_dirty: true,
                contour_cache_dirty: true,
                ..RoiDirtyState::default()
            },
            job_state: RoiJobState::default(),
        }
    }

    pub fn contour_data(&self) -> Option<&ContourData> {
        match &self.authoritative_data {
            RoiAuthoritativeData::Contour(contour) => Some(contour),
            RoiAuthoritativeData::Voxel(_) | RoiAuthoritativeData::Mesh(_) => None,
        }
    }

    pub fn mesh_data(&self) -> Option<&MeshData> {
        match &self.authoritative_data {
            RoiAuthoritativeData::Mesh(mesh) => Some(mesh),
            RoiAuthoritativeData::Voxel(_) | RoiAuthoritativeData::Contour(_) => None,
        }
    }

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
        let cache = self.ensure_contour_cache();
        if let Some(existing) = cache
            .views
            .iter_mut()
            .find(|view| view.key.logical_eq(&key))
        {
            existing.data = data;
            existing.source_generation = source_generation;
            existing.state = state;
            existing.key = key;
        } else {
            cache.views.push(ContourViewCache {
                key,
                data,
                source_generation,
                state,
            });
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

    pub fn mark_all_contour_view_caches_stale(&mut self) {
        if let Some(cache) = self.contour_cache_mut() {
            for view in &mut cache.views {
                if !matches!(view.state, CacheViewState::Blocked { .. }) {
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

    pub fn enqueue_rebuild(&mut self, kind: RoiJobKind) {
        if self.job_state.running == Some(kind) {
            return;
        }
        self.job_state.queued = Some(kind);
    }

    pub fn start_queued_job(&mut self) -> Option<RoiJobKind> {
        if self.job_state.running.is_some() {
            return None;
        }
        let next = self.job_state.queued.take()?;
        self.job_state.running = Some(next);
        Some(next)
    }

    pub fn finish_job(&mut self, kind: RoiJobKind) {
        if self.job_state.running == Some(kind) {
            self.job_state.running = None;
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
}

#[derive(Clone, Copy)]
pub struct VolumeWindowing {
    pub center: f32,
    pub width: f32,
}

impl Default for VolumeWindowing {
    fn default() -> Self {
        Self {
            center: 40.0,
            width: 400.0,
        }
    }
}

// --- Load Result ---
#[derive(Debug)]
pub enum LoadResult {
    Volume(crate::io::nifti::LoadedVolume),
    Label(LoadedLabel), // Changed to local LoadedLabel
}

#[derive(Debug)]
pub struct LoadedLabel {
    pub dimensions: [u32; 3],
    pub spacing: [f32; 3],
    pub origin: [f32; 3],
    pub orientation: [f32; 4],
    pub data: Vec<u8>,
    pub filename: String,
}

// --- ANNOTATIONS (Threaded discussions and notes) ---
#[derive(Clone, Debug)]
pub struct Comment {
    pub author: String,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct Annotation {
    pub id: uuid::Uuid,
    pub world_pos: Vec3,
    pub label: String,
    pub note: String,
    pub comments: Vec<Comment>,
}

#[derive(Clone, Debug, Default)]
pub struct AnnotationState {
    pub annotations: Vec<Annotation>,
    pub focused_id: Option<uuid::Uuid>,
    pub show_right_sidebar: bool,
}

// --- Singleton Entity Registry ---
#[derive(Clone, Copy)]
pub struct AppEntities {
    pub input: hecs::Entity,
    pub editor: hecs::Entity,
    pub gui_state: hecs::Entity,
    pub volume_windowing: hecs::Entity,
    pub annotations: hecs::Entity,
    pub overlay: hecs::Entity,
    pub protocol: hecs::Entity,
    pub cursor: hecs::Entity,
    pub window_settings: hecs::Entity,
}

#[cfg(test)]
mod tests {
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
            spacing: [1.0, 1.0, 1.0],
            origin: [0.0, 0.0, 0.0],
            intensities: vec![],
            intensity_range: [0.0, 1.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
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
            spacing: [1.0, 1.0, 2.0], // Physical size is 256, 256, 256
            origin: [0.0, 0.0, 0.0],
            intensities: vec![],
            intensity_range: [0.0, 1.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
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
            spacing: [1.0, 1.0, 1.0],
            origin: [0.0, 0.0, 0.0],
            intensities: vec![],
            intensity_range: [0.0, 1.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
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
            VoxelGeometry {
                dimensions: [16, 16, 8],
                spacing: [1.0, 1.0, 1.0],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            vec![1; 16 * 16 * 8],
            None,
        );

        assert_eq!(roi.metadata.roi_id, RoiId(7));
        assert_eq!(roi.metadata.name, "Liver");
        assert_eq!(roi.primary_representation, PrimaryRepresentation::Voxel);
        assert!(matches!(
            roi.authoritative_data,
            RoiAuthoritativeData::Voxel(VoxelData {
                geometry: VoxelGeometry {
                    dimensions: [16, 16, 8],
                    spacing: [1.0, 1.0, 1.0],
                    origin: [0.0, 0.0, 0.0],
                    orientation: [0.0, 0.0, 0.0, 1.0],
                },
                ..
            })
        ));
        let voxel_cache = roi.voxel_cache().expect("voxel cache should exist");
        assert_eq!(voxel_cache.data.raw_data.len(), 16 * 16 * 8);
        assert_eq!(voxel_cache.data.geometry.dimensions, [16, 16, 8]);
        assert!(voxel_cache.gpu_resources.is_none());
        assert!(roi.voxel_gpu_cache().is_none());
        assert!(roi.session_caches.contour.is_none());
        assert!(roi.session_caches.mesh.is_none());
        assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert!(!roi.is_cache_current(RoiCacheKind::Voxel));
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
    fn test_new_voxel_roi_without_cache_starts_with_dirty_voxel_cache() {
        let roi = Roi::new_voxel_with_cache(
            RoiId(8),
            "Kidney".to_string(),
            VoxelGeometry {
                dimensions: [8, 8, 8],
                spacing: [0.5, 0.5, 0.5],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            vec![1; 8 * 8 * 8],
            None,
        );

        assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert!(!roi.is_cache_current(RoiCacheKind::Voxel));
        assert!(roi.voxel_cache().is_some());
        assert!(roi.voxel_gpu_cache().is_none());
    }

    #[test]
    fn test_new_voxel_roi_copies_authoritative_data_into_session_voxel_cache() {
        let geometry = VoxelGeometry {
            dimensions: [6, 5, 4],
            spacing: [0.9, 1.1, 1.3],
            origin: [1.0, 2.0, 3.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
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
            VoxelGeometry {
                dimensions: [4, 4, 4],
                spacing: [1.0, 1.0, 1.0],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            vec![1; 64],
            None,
        );

        roi.metadata.is_visible = true;
        roi.dirty_state.voxel_cache_dirty = false;
        roi.dirty_state.generations.voxel = roi.dirty_state.generations.authoritative;

        assert!(roi.is_cache_current(RoiCacheKind::Voxel));
        assert!(roi.voxel_gpu_cache().is_none());
        assert!(roi.renderable_voxel_cache().is_none());
    }

    #[test]
    fn test_cache_current_requires_matching_generation_and_clean_state() {
        let mut roi = Roi::new_voxel_with_cache(
            RoiId(12),
            "Aorta".to_string(),
            VoxelGeometry {
                dimensions: [8, 8, 8],
                spacing: [0.75, 0.75, 0.75],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
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
            VoxelGeometry {
                dimensions: [4, 4, 4],
                spacing: [1.0, 1.0, 1.0],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
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
    fn test_enqueue_rebuild_supersedes_previous_queued_job() {
        let mut roi = Roi::new_voxel_with_cache(
            RoiId(10),
            "Pancreas".to_string(),
            VoxelGeometry {
                dimensions: [4, 4, 4],
                spacing: [1.0, 1.0, 1.0],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            vec![0; 64],
            None,
        );

        roi.enqueue_rebuild(RoiJobKind::RebuildContourCache);
        roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);

        assert_eq!(roi.job_state.queued, Some(RoiJobKind::RebuildVoxelCache));
        assert_eq!(roi.start_queued_job(), Some(RoiJobKind::RebuildVoxelCache));
        assert_eq!(roi.job_state.running, Some(RoiJobKind::RebuildVoxelCache));
    }

    #[test]
    fn test_finish_cache_rebuild_marks_cache_current_and_clears_job() {
        let mut roi = Roi::new_voxel_with_cache(
            RoiId(11),
            "Heart".to_string(),
            VoxelGeometry {
                dimensions: [4, 4, 4],
                spacing: [1.0, 1.0, 1.0],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
            vec![0; 64],
            None,
        );

        roi.enqueue_rebuild(RoiJobKind::RebuildVoxelCache);
        assert_eq!(roi.start_queued_job(), Some(RoiJobKind::RebuildVoxelCache));

        roi.finish_cache_rebuild(RoiCacheKind::Voxel);

        assert!(!roi.dirty_state.authoritative_dirty);
        assert!(!roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert!(roi.is_cache_current(RoiCacheKind::Voxel));
        assert_eq!(roi.job_state.running, None);
    }

    #[test]
    fn test_voxel_geometry_is_preserved_on_constructor() {
        let geometry = VoxelGeometry {
            dimensions: [12, 10, 8],
            spacing: [0.8, 0.8, 1.5],
            origin: [0.0, 0.0, 0.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };

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
        let roi = Roi::new_contour(RoiId(14), "GTV".to_string(), contour_data.clone());

        assert_eq!(roi.metadata.roi_id, RoiId(14));
        assert_eq!(roi.metadata.name, "GTV");
        assert_eq!(roi.primary_representation, PrimaryRepresentation::Contour);
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
            VoxelGeometry {
                dimensions: [8, 8, 8],
                spacing: [1.0, 1.0, 1.0],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
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
        assert_eq!(roi.primary_representation, PrimaryRepresentation::Mesh);
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
            VoxelGeometry {
                dimensions: [4, 4, 4],
                spacing: [1.0, 1.0, 1.0],
                origin: [0.0, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            },
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
        roi.session_caches.mesh = Some(MeshCache { data: mesh_data });
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
        roi.session_caches.mesh = Some(MeshCache { data: mesh_data });
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
        assert_eq!(roi.job_state.running, None);
    }
}
