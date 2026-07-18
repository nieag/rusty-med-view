use crate::app::components::{
    CacheViewState, ContourData, LayerSettings, MeshData, Roi, RoiAuthoritativeData, RoiCacheKind,
    MAX_VOXEL_OVERLAY_SLOTS,
};
use hecs::{Entity, World};

pub const DEFAULT_MAX_VOXEL_OVERLAYS: usize = MAX_VOXEL_OVERLAY_SLOTS;
const PLANE_ORIGIN_TOLERANCE_MM: f32 = 0.5;
const PLANE_NORMAL_ALIGNMENT_COS: f32 = 0.999;

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

fn normalized(v: [f32; 3]) -> Option<glam::Vec3> {
    let vec = glam::Vec3::from_array(v);
    let len_sq = vec.length_squared();
    if !len_sq.is_finite() || len_sq <= 1e-12 {
        None
    } else {
        Some(vec / len_sq.sqrt())
    }
}

pub(crate) fn planes_are_slice_compatible(
    displayed: crate::convert::PlaneDefinition,
    stored: crate::convert::PlaneDefinition,
) -> bool {
    if displayed.family != stored.family {
        return false;
    }
    let Some(displayed_normal) = normalized(displayed.normal_mm) else {
        return false;
    };
    let Some(stored_normal) = normalized(stored.normal_mm) else {
        return false;
    };
    if displayed_normal.dot(stored_normal).abs() < PLANE_NORMAL_ALIGNMENT_COS {
        return false;
    }
    let displayed_origin = glam::Vec3::from_array(displayed.origin_mm);
    let stored_origin = glam::Vec3::from_array(stored.origin_mm);
    let signed_distance = (stored_origin - displayed_origin)
        .dot(displayed_normal)
        .abs();
    signed_distance <= PLANE_ORIGIN_TOLERANCE_MM
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
            .map(|(_, volume)| crate::app::components::VoxelGeometry {
                dimensions: volume.dimensions,
                spacing: volume.spacing,
                origin: volume.origin,
                orientation: volume.orientation,
            })
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
        planes_are_slice_compatible(displayed_plane, slice.plane) && !slice.loops.is_empty()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::components::{
        ContourLoop, ContourPoint, ContourSlice, GpuVolumeResources, LayerSettings, MeshFace,
        MeshVertex, RoiId, VoxelGeometry,
    };
    use crate::convert::PlaneFamily;

    fn test_geometry() -> VoxelGeometry {
        VoxelGeometry {
            dimensions: [2, 2, 2],
            spacing: [1.0, 1.0, 1.0],
            origin: [0.0, 0.0, 0.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        }
    }

    fn spawn_voxel(world: &mut World, id: u64) -> Entity {
        world.spawn((
            Roi::new_voxel_with_cache(
                RoiId(id),
                format!("roi-{id}"),
                test_geometry(),
                vec![1; 8],
                None,
            ),
            LayerSettings { opacity: 0.5 },
        ))
    }

    fn mark_voxel_current_without_gpu(world: &mut World, entity: Entity) {
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        roi.dirty_state.voxel_cache_dirty = false;
        roi.dirty_state.generations.voxel = roi.dirty_state.generations.authoritative;
    }

    fn make_dummy_gpu_resources() -> Option<GpuVolumeResources> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: true,
        }))
        .ok()?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
        let (texture, view, sampler) = crate::io::volume::create_dummy_r8_texture(&device, &queue);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("roi-views-test-empty-layout"),
            entries: &[],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("roi-views-test-empty-bind-group"),
            layout: &layout,
            entries: &[],
        });
        Some(GpuVolumeResources {
            texture,
            view,
            sampler,
            bind_group,
        })
    }

    fn mark_voxel_renderable_with_gpu(world: &mut World, entity: Entity) -> bool {
        let Some(resources) = make_dummy_gpu_resources() else {
            return false;
        };
        let mut roi = world.get::<&mut Roi>(entity).unwrap();
        roi.dirty_state.voxel_cache_dirty = false;
        roi.dirty_state.generations.voxel = roi.dirty_state.generations.authoritative;
        if let Some(cache) = roi.voxel_cache_mut() {
            cache.gpu_resources = Some(resources);
            true
        } else {
            false
        }
    }

    fn spawn_contour_with_loops(world: &mut World, id: u64) -> Entity {
        world.spawn((
            Roi::new_contour(
                RoiId(id),
                format!("contour-{id}"),
                ContourData {
                    active_plane_family: PlaneFamily::Axial,
                    slices: vec![ContourSlice {
                        plane: crate::convert::orthogonal_plane_from_volume_uv(
                            PlaneFamily::Axial,
                            [0.5, 0.5, 0.5],
                            test_geometry(),
                        )
                        .unwrap(),
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
                },
            ),
            LayerSettings { opacity: 0.5 },
        ))
    }

    fn spawn_mesh_with_faces(world: &mut World, id: u64) -> Entity {
        world.spawn((
            Roi::new_mesh(
                RoiId(id),
                format!("mesh-{id}"),
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
                },
            ),
            LayerSettings { opacity: 0.5 },
        ))
    }

    #[test]
    fn test_active_contour_candidate_preferred_when_active() {
        let mut world = World::new();
        let active = spawn_contour_with_loops(&mut world, 1);
        let _other = spawn_contour_with_loops(&mut world, 2);
        let views = RoiRenderViews::for_world(
            &world,
            RenderRepresentationRequest {
                active_roi: Some(active),
                ..RenderRepresentationRequest::default()
            },
        );
        assert_eq!(views.contour_overlays.len(), 1);
        assert_eq!(views.contour_overlays[0].entity, active);
        assert!(views
            .contour_skips
            .iter()
            .any(|skip| skip.reason == "contour_inactive"));
    }

    #[test]
    fn test_multi_contour_request_keeps_active_first_and_includes_inactive() {
        let mut world = World::new();
        let active = spawn_contour_with_loops(&mut world, 2);
        let other = spawn_contour_with_loops(&mut world, 1);
        let views = RoiRenderViews::for_world(
            &world,
            RenderRepresentationRequest {
                active_roi: Some(active),
                contour_active_only: false,
                ..RenderRepresentationRequest::default()
            },
        );

        assert_eq!(views.contour_overlays.len(), 2);
        assert_eq!(views.contour_overlays[0].entity, active);
        assert_eq!(views.contour_overlays[1].entity, other);
    }

    #[test]
    fn test_default_cap_accepts_eight_voxel_overlays_and_truncates_ninth() {
        let mut world = World::new();
        let entities: Vec<_> = (1..=9).map(|id| spawn_voxel(&mut world, id)).collect();
        for entity in entities {
            if !mark_voxel_renderable_with_gpu(&mut world, entity) {
                return;
            }
        }

        let views = RoiRenderViews::for_world(&world, RenderRepresentationRequest::default());
        assert_eq!(views.voxel_overlays.len(), MAX_VOXEL_OVERLAY_SLOTS);
        assert_eq!(views.overlay_cap.truncated_count, 1);
        assert!(views
            .voxel_skips
            .iter()
            .any(|skip| skip.reason == "overlay_cap_exceeded"));
    }

    #[test]
    fn test_overlay_cap_skip_uses_same_active_first_ordering() {
        let mut world = World::new();
        let a = spawn_voxel(&mut world, 1);
        let b = spawn_voxel(&mut world, 2);
        let c = spawn_voxel(&mut world, 3);
        if !mark_voxel_renderable_with_gpu(&mut world, a)
            || !mark_voxel_renderable_with_gpu(&mut world, b)
            || !mark_voxel_renderable_with_gpu(&mut world, c)
        {
            return;
        }
        let views = RoiRenderViews::for_world(
            &world,
            RenderRepresentationRequest {
                active_roi: Some(c),
                max_voxel_overlays: 2,
                ..RenderRepresentationRequest::default()
            },
        );
        assert_eq!(views.voxel_overlays.len(), 2);
        assert_eq!(views.voxel_overlays[0].entity, c);
        assert_eq!(views.voxel_overlays[1].entity, a);
        let cap_skip = views
            .voxel_skips
            .iter()
            .find(|skip| skip.reason == "overlay_cap_exceeded")
            .copied()
            .expect("expected cap skip");
        assert_eq!(cap_skip.entity, b);
    }

    #[test]
    fn test_contour_roi_with_loops_produces_contour_candidate() {
        let mut world = World::new();
        let active = spawn_contour_with_loops(&mut world, 1);
        let views = RoiRenderViews::for_world(
            &world,
            RenderRepresentationRequest {
                active_roi: Some(active),
                ..RenderRepresentationRequest::default()
            },
        );
        assert_eq!(views.contour_overlays.len(), 1);
        assert_eq!(views.contour_overlays[0].entity, active);
    }

    #[test]
    fn test_mesh_roi_with_faces_produces_mesh_candidate() {
        let mut world = World::new();
        let mesh = spawn_mesh_with_faces(&mut world, 1);
        let views = RoiRenderViews::for_world(&world, RenderRepresentationRequest::default());
        assert_eq!(views.mesh_overlays.len(), 1);
        assert_eq!(views.mesh_overlays[0].entity, mesh);
    }

    #[test]
    fn test_missing_voxel_gpu_cache_has_reason() {
        let mut world = World::new();
        let voxel = spawn_voxel(&mut world, 1);
        mark_voxel_current_without_gpu(&mut world, voxel);
        let views = RoiRenderViews::for_world(
            &world,
            RenderRepresentationRequest {
                active_roi: Some(voxel),
                ..RenderRepresentationRequest::default()
            },
        );
        assert!(
            views
                .voxel_skips
                .iter()
                .any(|skip| skip.entity == voxel && skip.reason == "voxel_cache_dirty")
                || views
                    .voxel_skips
                    .iter()
                    .any(|skip| skip.entity == voxel && skip.reason == "voxel_gpu_missing")
        );
    }
}
