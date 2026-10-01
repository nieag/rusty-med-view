use crate::app::components::{
    CacheViewState, ContourBody, ContourSlice, LayerSettings, MeshBody, MeshData, Roi, RoiBody,
    RoiCacheKind, RoiMetadata, MAX_VOXEL_OVERLAY_SLOTS,
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
                } else if crate::app::roi::is_roi_visible(world, active) {
                    views.voxel_skips.push(RoiRenderSkip {
                        entity: active,
                        reason: voxel_skip_reason(world, active, &active_roi),
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
        for (entity, (roi, metadata)) in world.query::<(&Roi, &RoiMetadata)>().iter() {
            if Some(entity) == request.active_roi {
                collect_contour_and_mesh_views(
                    roi,
                    metadata.is_visible,
                    entity,
                    request.active_roi,
                    request.contour_active_only,
                    &mut contour_mesh_acc,
                );
                continue;
            }
            if let Ok(settings) = world.get::<&LayerSettings>(entity) {
                if settings.show_voxel_fill
                    && roi.renderable_voxel_cache(metadata.is_visible).is_some()
                {
                    voxel_candidates.push(VoxelOverlayView {
                        entity,
                        opacity: settings.opacity,
                    });
                } else if metadata.is_visible {
                    views.voxel_skips.push(RoiRenderSkip {
                        entity,
                        reason: voxel_skip_reason(world, entity, roi),
                    });
                }
            } else if metadata.is_visible {
                views.voxel_skips.push(RoiRenderSkip {
                    entity,
                    reason: voxel_skip_reason(world, entity, roi),
                });
            }

            collect_contour_and_mesh_views(
                roi,
                metadata.is_visible,
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
                .get::<&RoiMetadata>(candidate.entity)
                .map(|metadata| metadata.roi_id.0)
                .unwrap_or(u64::MAX)
        });
        views.contour_overlays.sort_by_key(|view| {
            (
                Some(view.entity) != request.active_roi,
                world
                    .get::<&RoiMetadata>(view.entity)
                    .map(|metadata| metadata.roi_id.0)
                    .unwrap_or(u64::MAX),
            )
        });
        views.mesh_overlays.sort_by_key(|view| {
            (
                Some(view.entity) != request.active_roi,
                world
                    .get::<&RoiMetadata>(view.entity)
                    .map(|metadata| metadata.roi_id.0)
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
    if !settings.show_voxel_fill {
        return None;
    }
    roi.renderable_voxel_cache(crate::app::roi::is_roi_visible(world, entity))?;
    Some(VoxelOverlayView {
        entity,
        opacity: settings.opacity,
    })
}

fn voxel_skip_reason(world: &World, entity: Entity, roi: &Roi) -> &'static str {
    let fill_hidden = world
        .get::<&LayerSettings>(entity)
        .is_ok_and(|settings| !settings.show_voxel_fill);
    if fill_hidden {
        "voxel_fill_hidden"
    } else {
        voxel_non_renderable_reason(roi)
    }
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

fn has_contour_loops(slices: &[ContourSlice]) -> bool {
    slices.iter().any(|slice| !slice.loops.is_empty())
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
    is_visible: bool,
    entity: Entity,
    active_roi: Option<Entity>,
    contour_active_only: bool,
    acc: &mut ContourMeshAcc<'_>,
) {
    if !is_visible {
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
    } else if matches!(roi.body, RoiBody::Contour(_)) || roi.contour_cache().is_some() {
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
    } else if matches!(roi.body, RoiBody::Mesh(_)) || roi.mesh_cache().is_some() {
        acc.mesh_skips.push(RoiRenderSkip {
            entity,
            reason: "mesh_data_missing_or_empty",
        });
    }
}

fn contour_data_for_adapter(roi: &Roi) -> Option<&[ContourSlice]> {
    match &roi.body {
        RoiBody::Contour(ContourBody { data: contour, .. }) => Some(&contour.slices),
        RoiBody::Voxel(_) | RoiBody::Mesh(_) => roi
            .contour_cache()?
            .views
            .iter()
            .find(|view| {
                !matches!(
                    view.state,
                    CacheViewState::Blocked { .. } | CacheViewState::Unsupported { .. }
                ) && has_contour_loops(&view.data)
            })
            .map(|view| view.data.as_slice()),
    }
}

pub(crate) fn mesh_data_for_adapter(roi: &Roi) -> Option<&MeshData> {
    if roi.preview_state.active {
        if let Some(preview) = roi.session_caches.preview_mesh.as_ref().filter(|cache| {
            cache.source_generation == roi.dirty_state.authoritative.shape
                && cache.preview_revision == roi.preview_state.revision
        }) {
            return Some(&preview.data);
        }
    }
    match &roi.body {
        RoiBody::Mesh(MeshBody { data: mesh, .. }) => Some(mesh),
        RoiBody::Voxel(_) | RoiBody::Contour(_) => {
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
    if !crate::app::roi::is_roi_visible(world, active_roi) {
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

    contour_data.iter().any(|slice| {
        crate::convert::planes_are_same_slice(displayed_plane, slice.plane, geometry)
            && !slice.loops.is_empty()
    })
}

#[cfg(test)]
mod tests;
