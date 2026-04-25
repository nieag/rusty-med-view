use crate::components::{
    AppEntities, ContourData, InputState, MainVolumeTag, Transform, ViewMode, Viewport,
    VoxelGeometry,
};
use crate::convert::{
    oblique_plane_from_view_rotation, orthogonal_plane_from_volume_uv,
    plane_local_mm_to_viewport_uv, viewport_uv_to_plane_local_mm, PlaneDefinition, PlaneFamily,
    ViewportMapping,
};
use hecs::World;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContourEditMappingError {
    MissingActiveViewport,
    MissingViewport,
    MissingViewportState,
    MissingCursor,
    MissingMainVolume,
    UnsupportedViewportMode,
    PlaneUnavailable,
    PlaneFamilyMismatch {
        contour_family: PlaneFamily,
        viewport_family: PlaneFamily,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContourEditViewport {
    pub viewport_entity: hecs::Entity,
    pub plane: PlaneDefinition,
    pub mapping: ViewportMapping,
    pub geometry: VoxelGeometry,
}

pub fn resolve_active_contour_edit_viewport(
    world: &World,
    entities: &AppEntities,
    contour_data: &ContourData,
) -> Result<ContourEditViewport, ContourEditMappingError> {
    let input = world
        .get::<&InputState>(entities.input)
        .map_err(|_| ContourEditMappingError::MissingActiveViewport)?;
    let viewport_entity = input
        .active_viewport
        .ok_or(ContourEditMappingError::MissingActiveViewport)?;
    drop(input);

    let viewport = world
        .get::<&Viewport>(viewport_entity)
        .map_err(|_| ContourEditMappingError::MissingViewport)?;
    let viewport_state = world
        .get::<&crate::components::ViewportState>(viewport_entity)
        .map_err(|_| ContourEditMappingError::MissingViewportState)?;

    let geometry = {
        let mut main_volume_query = world
            .query::<&crate::components::VolumeData>()
            .with::<&MainVolumeTag>();
        let (_, volume) = main_volume_query
            .iter()
            .next()
            .ok_or(ContourEditMappingError::MissingMainVolume)?;
        VoxelGeometry {
            dimensions: volume.dimensions,
            spacing: volume.spacing,
            origin: volume.origin,
            orientation: volume.orientation,
        }
    };

    let cursor = world
        .get::<&Transform>(entities.cursor)
        .map_err(|_| ContourEditMappingError::MissingCursor)?;

    let plane = match viewport.mode {
        ViewMode::Axial => {
            orthogonal_plane_from_volume_uv(PlaneFamily::Axial, cursor.position, geometry)
        }
        ViewMode::Coronal => {
            orthogonal_plane_from_volume_uv(PlaneFamily::Coronal, cursor.position, geometry)
        }
        ViewMode::Sagittal => {
            orthogonal_plane_from_volume_uv(PlaneFamily::Sagittal, cursor.position, geometry)
        }
        ViewMode::Oblique => oblique_plane_from_view_rotation(
            cursor.position,
            viewport_state.user_rotation,
            geometry,
        ),
        ViewMode::ThreeD => return Err(ContourEditMappingError::UnsupportedViewportMode),
    }
    .ok_or(ContourEditMappingError::PlaneUnavailable)?;

    if plane.family != contour_data.active_plane_family {
        return Err(ContourEditMappingError::PlaneFamilyMismatch {
            contour_family: contour_data.active_plane_family,
            viewport_family: plane.family,
        });
    }

    let screen_aspect = if viewport.rect[3] > 0.0 {
        viewport.rect[2] / viewport.rect[3]
    } else {
        1.0
    };
    Ok(ContourEditViewport {
        viewport_entity,
        plane,
        mapping: ViewportMapping {
            zoom: viewport_state.zoom,
            pan: viewport_state.pan,
            pivot: viewport_state.pivot,
            screen_aspect,
        },
        geometry,
    })
}

pub fn viewport_uv_to_contour_plane_local_mm(
    viewport_uv: [f32; 2],
    viewport: ContourEditViewport,
) -> Option<[f32; 2]> {
    viewport_uv_to_plane_local_mm(
        viewport_uv,
        viewport.plane,
        viewport.geometry,
        viewport.mapping,
    )
}

pub fn contour_plane_local_mm_to_viewport_uv(
    local_mm: [f32; 2],
    viewport: ContourEditViewport,
) -> Option<[f32; 2]> {
    plane_local_mm_to_viewport_uv(
        local_mm,
        viewport.plane,
        viewport.geometry,
        viewport.mapping,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{ContourData, InputState, ViewportState, WindowSettings};

    fn test_geometry() -> VoxelGeometry {
        VoxelGeometry {
            dimensions: [64, 48, 32],
            spacing: [1.0, 1.0, 1.0],
            origin: [0.0, 0.0, 0.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        }
    }

    fn spawn_test_entities(
        world: &mut World,
        mode: ViewMode,
        user_rotation: [f32; 4],
        active_viewport: Option<hecs::Entity>,
    ) -> AppEntities {
        let cursor = world.spawn((Transform {
            position: [0.4, 0.55, 0.2],
        },));
        let viewport = world.spawn((
            Viewport {
                mode,
                rect: [0.0, 0.0, 800.0, 600.0],
                uniform_index: 0,
            },
            ViewportState {
                zoom: 1.1,
                pan: [0.02, -0.01],
                pivot: [0.5, 0.5],
                user_rotation,
            },
        ));
        let input = world.spawn((InputState {
            active_viewport: active_viewport.or(Some(viewport)),
            ..InputState::default()
        },));
        world.spawn((
            crate::components::VolumeData {
                dimensions: test_geometry().dimensions,
                spacing: test_geometry().spacing,
                origin: test_geometry().origin,
                intensities: vec![],
                intensity_range: [0.0, 1.0],
                orientation: test_geometry().orientation,
            },
            MainVolumeTag,
        ));

        let editor = world.spawn((crate::components::EditorState::default(),));
        let gui_state = world.spawn((crate::components::GuiState {
            status_message: None,
        },));
        let volume_windowing = world.spawn((crate::components::VolumeWindowing::default(),));
        let annotations = world.spawn((crate::components::AnnotationState::default(),));
        let overlay = world.spawn((crate::overlay::OverlayManager::default(),));
        let protocol = world.spawn((crate::components::ProtocolState::default(),));
        let window_settings = world.spawn((WindowSettings {
            width: 800,
            height: 600,
            viewport_rect: [0.0, 0.0, 800.0, 600.0],
        },));

        AppEntities {
            input,
            editor,
            gui_state,
            volume_windowing,
            annotations,
            overlay,
            protocol,
            cursor,
            window_settings,
        }
    }

    #[test]
    fn test_contour_edit_rejects_viewport_family_mismatch() {
        let mut world = World::new();
        let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
        let contour = ContourData {
            active_plane_family: PlaneFamily::Coronal,
            slices: Vec::new(),
        };

        let result = resolve_active_contour_edit_viewport(&world, &entities, &contour);
        assert_eq!(
            result,
            Err(ContourEditMappingError::PlaneFamilyMismatch {
                contour_family: PlaneFamily::Coronal,
                viewport_family: PlaneFamily::Axial,
            })
        );
    }

    #[test]
    fn test_contour_edit_rejects_three_d_viewport() {
        let mut world = World::new();
        let entities =
            spawn_test_entities(&mut world, ViewMode::ThreeD, [0.0, 0.0, 0.0, 1.0], None);
        let contour = ContourData {
            active_plane_family: PlaneFamily::Axial,
            slices: Vec::new(),
        };

        let result = resolve_active_contour_edit_viewport(&world, &entities, &contour);
        assert_eq!(
            result,
            Err(ContourEditMappingError::UnsupportedViewportMode)
        );
    }

    #[test]
    fn test_contour_edit_oblique_path_uses_plane_definition_roundtrip() {
        let mut world = World::new();
        let oblique_rotation =
            glam::Quat::from_euler(glam::EulerRot::XYZ, 0.3, -0.2, 0.15).to_array();
        let entities = spawn_test_entities(&mut world, ViewMode::Oblique, oblique_rotation, None);
        let contour = ContourData {
            active_plane_family: PlaneFamily::Oblique,
            slices: Vec::new(),
        };

        let viewport =
            resolve_active_contour_edit_viewport(&world, &entities, &contour).expect("viewport");
        assert_eq!(viewport.plane.family, PlaneFamily::Oblique);

        let click_a = [0.32, 0.67];
        let click_b = [0.61, 0.28];
        let local_a = viewport_uv_to_contour_plane_local_mm(click_a, viewport).expect("local mm a");
        let local_b = viewport_uv_to_contour_plane_local_mm(click_b, viewport).expect("local mm b");
        let viewport_a =
            contour_plane_local_mm_to_viewport_uv(local_a, viewport).expect("viewport uv a");
        let viewport_b =
            contour_plane_local_mm_to_viewport_uv(local_b, viewport).expect("viewport uv b");

        assert!(local_a[0].is_finite() && local_a[1].is_finite());
        assert!(local_b[0].is_finite() && local_b[1].is_finite());
        assert_ne!(local_a, local_b);
        assert!(viewport_a[0].is_finite() && viewport_a[1].is_finite());
        assert!(viewport_b[0].is_finite() && viewport_b[1].is_finite());
    }
}
