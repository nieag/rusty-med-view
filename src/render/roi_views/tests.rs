use super::*;
use crate::app::components::{
    ContourLoop, ContourPoint, ContourSlice, GpuVolumeResources, LayerSettings, MeshFace,
    MeshVertex, RoiId, VoxelGeometry,
};
use crate::convert::PlaneFamily;

fn test_geometry() -> VoxelGeometry {
    VoxelGeometry::new(
        [2, 2, 2],
        [1.0, 1.0, 1.0],
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    )
    .unwrap()
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
    roi.dirty_state.voxel.dirty = false;
    roi.dirty_state.voxel.built_from = roi.dirty_state.authoritative;
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
    roi.dirty_state.voxel.dirty = false;
    roi.dirty_state.voxel.built_from = roi.dirty_state.authoritative;
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
