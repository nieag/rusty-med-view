use crate::app::components::{
    CacheViewState, ContourData, LayerSettings, MeshData, Roi, RoiAuthoritativeData, RoiCacheKind,
    MAX_VOXEL_OVERLAY_SLOTS,
};
use hecs::{Entity, World};

pub const DEFAULT_MAX_VOXEL_OVERLAYS: usize = MAX_VOXEL_OVERLAY_SLOTS;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoxelOverlayView {
    pub entity: Entity,
    pub opacity: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContourOverlayView {
    pub entity: Entity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeshOverlayView {
    pub entity: Entity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayCapReport {
    pub selected_count: usize,
    pub max_count: usize,
    pub truncated_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderRepresentationRequest {
    pub active_roi: Option<Entity>,
    pub max_voxel_overlays: usize,
    pub contour_active_only: bool,
}

impl Default for RenderRepresentationRequest {
    fn default() -> Self {
        Self {
            active_roi: None,
            max_voxel_overlays: DEFAULT_MAX_VOXEL_OVERLAYS,
            contour_active_only: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoiRenderSkip {
    pub entity: Entity,
    pub reason: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RoiRenderViews {
    pub voxel_overlays: Vec<VoxelOverlayView>,
    pub contour_overlays: Vec<ContourOverlayView>,
    pub mesh_overlays: Vec<MeshOverlayView>,
    pub voxel_skips: Vec<RoiRenderSkip>,
    pub contour_skips: Vec<RoiRenderSkip>,
    pub mesh_skips: Vec<RoiRenderSkip>,
    pub overlay_cap: OverlayCapReport,
}

impl RoiRenderViews {
    pub fn for_world(world: &World, request: RenderRepresentationRequest) -> Self {
        let mut views = Self {
            voxel_overlays: Vec::new(),
            contour_overlays: Vec::new(),
            mesh_overlays: Vec::new(),
            voxel_skips: Vec::new(),
            contour_skips: Vec::new(),
            mesh_skips: Vec::new(),
            overlay_cap: OverlayCapReport {
                selected_count: 0,
                max_count: request.max_voxel_overlays,
                truncated_count: 0,
            },
        };

        let mut voxel_candidates: Vec<VoxelOverlayView> = Vec::new();
        if let Some(active) = request.active_roi {
            if let Ok(active_roi) = world.get::<&Roi>(active) {
                if let Some(overlay) = voxel_overlay_candidate(world, active) {
                    voxel_candidates.push(overlay);
                } else if active_roi.metadata.is_visible {
                    views.voxel_skips.push(RoiRenderSkip {
                        entity: active,
                        reason: voxel_non_renderable_reason(&active_roi),
                    });
                }
            }
        }

        let mut contour_mesh_acc = ContourMeshAcc {
            contour_views: &mut views.contour_overlays,
            mesh_views: &mut views.mesh_overlays,
            contour_skips: &mut views.contour_skips,
            mesh_skips: &mut views.mesh_skips,
        };
        for (entity, roi) in world.query::<&Roi>().iter() {
            if Some(entity) == request.active_roi {
                collect_contour_and_mesh_views(
                    roi,
                    entity,
                    request.active_roi,
                    request.contour_active_only,
                    &mut contour_mesh_acc,
                );
                continue;
            }
            if let Ok(settings) = world.get::<&LayerSettings>(entity) {
                if roi.renderable_voxel_cache().is_some() {
                    voxel_candidates.push(VoxelOverlayView {
                        entity,
                        opacity: settings.opacity,
                    });
                } else if roi.metadata.is_visible {
                    views.voxel_skips.push(RoiRenderSkip {
                        entity,
                        reason: voxel_non_renderable_reason(roi),
                    });
                }
            } else if roi.metadata.is_visible {
                views.voxel_skips.push(RoiRenderSkip {
                    entity,
                    reason: voxel_non_renderable_reason(roi),
                });
            }

            collect_contour_and_mesh_views(
                roi,
                entity,
                request.active_roi,
                request.contour_active_only,
                &mut contour_mesh_acc,
            );
        }

        let active_prefix = usize::from(
            request.active_roi.is_some()
                && voxel_candidates
                    .first()
                    .is_some_and(|candidate| Some(candidate.entity) == request.active_roi),
        );
        voxel_candidates[active_prefix..].sort_by_key(|candidate| {
            world
                .get::<&Roi>(candidate.entity)
                .map(|roi| roi.metadata.roi_id.0)
                .unwrap_or(u64::MAX)
        });
        views.contour_overlays.sort_by_key(|view| {
            (
                Some(view.entity) != request.active_roi,
                world
                    .get::<&Roi>(view.entity)
                    .map(|roi| roi.metadata.roi_id.0)
                    .unwrap_or(u64::MAX),
            )
        });
        views.mesh_overlays.sort_by_key(|view| {
            (
                Some(view.entity) != request.active_roi,
                world
                    .get::<&Roi>(view.entity)
                    .map(|roi| roi.metadata.roi_id.0)
                    .unwrap_or(u64::MAX),
            )
        });

        let selected = voxel_candidates.len().min(request.max_voxel_overlays);
        let truncated = voxel_candidates.len().saturating_sub(selected);
        views.voxel_overlays = voxel_candidates[..selected].to_vec();
        if truncated > 0 {
            let skipped_due_to_cap: Vec<_> = voxel_candidates[selected..]
                .iter()
                .map(|candidate| RoiRenderSkip {
                    entity: candidate.entity,
                    reason: "overlay_cap_exceeded",
                })
                .collect();
            views.voxel_skips.extend(skipped_due_to_cap);
        }
        views.overlay_cap = OverlayCapReport {
            selected_count: selected,
            max_count: request.max_voxel_overlays,
            truncated_count: truncated,
        };
        views
    }
}

fn voxel_overlay_candidate(world: &World, entity: Entity) -> Option<VoxelOverlayView> {
    let roi = world.get::<&Roi>(entity).ok()?;
    let settings = world.get::<&LayerSettings>(entity).ok()?;
    roi.renderable_voxel_cache()?;
    Some(VoxelOverlayView {
        entity,
        opacity: settings.opacity,
    })
}

fn voxel_non_renderable_reason(roi: &Roi) -> &'static str {
    if roi.voxel_cache().is_none() {
        return "voxel_cache_missing";
    }
    if roi.is_cache_dirty(RoiCacheKind::Voxel) {
        return "voxel_cache_dirty";
    }
    if !roi.is_cache_current(RoiCacheKind::Voxel) {
        return "voxel_cache_stale";
    }
    if roi.voxel_gpu_cache().is_none() {
        return "voxel_gpu_missing";
    }
    "voxel_not_renderable"
}

fn has_contour_loops(contour: &ContourData) -> bool {
    contour.has_loops()
}

fn has_mesh_geometry(mesh: &MeshData) -> bool {
    !mesh.vertices.is_empty() && !mesh.faces.is_empty()
}

pub fn displayed_plane_for_viewport(
    mode: crate::app::components::ViewMode,
    cursor_uv: [f32; 3],
    user_rotation: [f32; 4],
    geometry: crate::app::components::VoxelGeometry,
) -> Option<crate::convert::PlaneDefinition> {
    match mode {
        crate::app::components::ViewMode::Axial => crate::convert::orthogonal_plane_from_volume_uv(
            crate::convert::PlaneFamily::Axial,
            cursor_uv,
            geometry,
        ),
        crate::app::components::ViewMode::Coronal => {
            crate::convert::orthogonal_plane_from_volume_uv(
                crate::convert::PlaneFamily::Coronal,
                cursor_uv,
                geometry,
            )
        }
        crate::app::components::ViewMode::Sagittal => {
            crate::convert::orthogonal_plane_from_volume_uv(
                crate::convert::PlaneFamily::Sagittal,
                cursor_uv,
                geometry,
            )
        }
        crate::app::components::ViewMode::Oblique => {
            crate::convert::oblique_plane_from_view_rotation(cursor_uv, user_rotation, geometry)
        }
        crate::app::components::ViewMode::ThreeD => None,
    }
}

fn collect_contour_and_mesh_views(
    roi: &Roi,
    entity: Entity,
    active_roi: Option<Entity>,
    contour_active_only: bool,
    acc: &mut ContourMeshAcc<'_>,
) {
    if !roi.metadata.is_visible {
        return;
    }
    if let Some(contour) = contour_data_for_adapter(roi) {
        if !has_contour_loops(contour) {
            acc.contour_skips.push(RoiRenderSkip {
                entity,
                reason: "contour_data_missing_or_empty",
            });
        } else if contour_active_only && Some(entity) != active_roi {
            acc.contour_skips.push(RoiRenderSkip {
                entity,
                reason: "contour_inactive",
            });
        } else {
            acc.contour_views.push(ContourOverlayView { entity });
        }
    } else if matches!(roi.authoritative_data, RoiAuthoritativeData::Contour(_))
        || roi.contour_cache().is_some()
    {
        acc.contour_skips.push(RoiRenderSkip {
            entity,
            reason: "contour_data_missing_or_empty",
        });
    }

    if let Some(mesh) = mesh_data_for_adapter(roi) {
        if has_mesh_geometry(mesh) {
            acc.mesh_views.push(MeshOverlayView { entity });
        } else {
            acc.mesh_skips.push(RoiRenderSkip {
                entity,
                reason: "mesh_data_missing_or_empty",
            });
        }
    } else if matches!(roi.authoritative_data, RoiAuthoritativeData::Mesh(_))
        || roi.mesh_cache().is_some()
    {
        acc.mesh_skips.push(RoiRenderSkip {
            entity,
            reason: "mesh_data_missing_or_empty",
        });
    }
}

fn contour_data_for_adapter(roi: &Roi) -> Option<&ContourData> {
    match &roi.authoritative_data {
        RoiAuthoritativeData::Contour(contour) => Some(contour),
        RoiAuthoritativeData::Voxel(_) | RoiAuthoritativeData::Mesh(_) => roi
            .contour_cache()?
            .views
            .iter()
            .find(|view| {
                !matches!(
                    view.state,
                    CacheViewState::Blocked { .. } | CacheViewState::Unsupported { .. }
                ) && has_contour_loops(&view.data)
            })
            .map(|view| &view.data),
    }
}

pub(crate) fn mesh_data_for_adapter(roi: &Roi) -> Option<&MeshData> {
    if roi.preview_state.active {
        if let Some(preview) = roi.session_caches.preview_mesh.as_ref().filter(|cache| {
            cache.source_generation == roi.dirty_state.generations.authoritative
                && cache.preview_revision == roi.preview_state.revision
        }) {
            return Some(&preview.data);
        }
    }
    match &roi.authoritative_data {
        RoiAuthoritativeData::Mesh(mesh) => Some(mesh),
        RoiAuthoritativeData::Voxel(_) | RoiAuthoritativeData::Contour(_) => {
            if roi.is_cache_current(RoiCacheKind::Mesh) {
                roi.mesh_cache().map(|cache| &cache.data)
            } else {
                None
            }
        }
    }
}

struct ContourMeshAcc<'a> {
    contour_views: &'a mut Vec<ContourOverlayView>,
    mesh_views: &'a mut Vec<MeshOverlayView>,
    contour_skips: &'a mut Vec<RoiRenderSkip>,
    mesh_skips: &'a mut Vec<RoiRenderSkip>,
}

pub fn contour_renderable_in_viewport(
    world: &World,
    viewport: &crate::app::components::Viewport,
    viewport_state: &crate::app::components::ViewportState,
    cursor_uv: [f32; 3],
    active_roi: Entity,
) -> bool {
    let roi = match world.get::<&Roi>(active_roi) {
        Ok(roi) => roi,
        Err(_) => return false,
    };
    if !roi.metadata.is_visible {
        return false;
    }
    let geometry = {
        let mut volume_query = world
            .query::<&crate::app::components::VolumeData>()
            .with::<&crate::app::components::MainVolumeTag>();
        volume_query
            .iter()
            .next()
            .and_then(|(_, volume)| volume.geometry)
    }
    .or_else(|| roi.voxel_cache().map(|cache| cache.data.geometry));
    let Some(geometry) = geometry else {
        return false;
    };

    let displayed_plane = displayed_plane_for_viewport(
        viewport.mode,
        cursor_uv,
        viewport_state.user_rotation,
        geometry,
    );
    let Some(displayed_plane) = displayed_plane else {
        return false;
    };
    let key = crate::app::components::ContourViewKey::from_plane(displayed_plane);
    let Some(contour_data) = roi.contour_view_data_for_render(&key) else {
        return false;
    };

    contour_data.slices.iter().any(|slice| {
        crate::convert::planes_are_same_slice(displayed_plane, slice.plane, geometry)
            && !slice.loops.is_empty()
    })
}

#[cfg(test)]
mod tests;
