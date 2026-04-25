use crate::app::roi_runtime;
use crate::components::{
    AppEntities, ContourData, ContourDraft, ContourLoop, ContourPoint, ContourSlice, EditorState,
    EditorTool, InputState, MainVolumeTag, Roi, Transform, ViewMode, Viewport, VoxelGeometry,
};
use crate::convert::{
    oblique_plane_from_view_rotation, orthogonal_plane_from_volume_uv,
    plane_local_mm_to_viewport_uv, viewport_uv_to_plane_local_mm, PlaneDefinition, PlaneFamily,
    ViewportMapping,
};
use hecs::World;

const LOOP_CLOSE_RADIUS_PX: f32 = 10.0;
const SLICE_MATCH_DISTANCE_MM: f32 = 0.5;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContourDrawClickOutcome {
    PointAdded,
    LoopCommitted,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ContourDrawClickError {
    ToolNotActive,
    MissingActiveRoi,
    ActiveRoiNotContour,
    Mapping(ContourEditMappingError),
    ProjectionFailed,
    LoopNeedsThreePoints,
    CommitFailed,
}

impl From<ContourEditMappingError> for ContourDrawClickError {
    fn from(value: ContourEditMappingError) -> Self {
        ContourDrawClickError::Mapping(value)
    }
}

fn planes_match_for_slice(lhs: PlaneDefinition, rhs: PlaneDefinition) -> bool {
    if lhs.family != rhs.family {
        return false;
    }
    let lhs_n = glam::Vec3::from_array(lhs.normal_mm).normalize_or_zero();
    let rhs_n = glam::Vec3::from_array(rhs.normal_mm).normalize_or_zero();
    if lhs_n.length_squared() <= 1e-12 || rhs_n.length_squared() <= 1e-12 {
        return false;
    }
    if lhs_n.dot(rhs_n).abs() < 0.999 {
        return false;
    }

    let lhs_o = glam::Vec3::from_array(lhs.origin_mm);
    let rhs_o = glam::Vec3::from_array(rhs.origin_mm);
    (lhs_o - rhs_o).dot(lhs_n).abs() <= SLICE_MATCH_DISTANCE_MM
}

fn contour_data_for_active_roi(world: &World, roi_entity: hecs::Entity) -> Option<ContourData> {
    world
        .get::<&Roi>(roi_entity)
        .ok()
        .and_then(|roi| roi.contour_data().cloned())
}

pub fn clear_contour_draft_if_inactive(world: &mut World, editor_entity: hecs::Entity) {
    if let Ok(mut editor) = world.get::<&mut EditorState>(editor_entity) {
        if editor.active_tool != EditorTool::ContourDraw {
            editor.contour_draft = None;
        }
    }
}

pub fn clear_contour_draft_for_roi_change(
    world: &mut World,
    editor_entity: hecs::Entity,
    new_active_roi: Option<hecs::Entity>,
) {
    if let Ok(mut editor) = world.get::<&mut EditorState>(editor_entity) {
        if editor.active_roi != new_active_roi {
            editor.contour_draft = None;
        }
    }
}

pub fn handle_contour_draw_click(
    world: &mut World,
    entities: &AppEntities,
    viewport_uv: [f32; 2],
) -> Result<ContourDrawClickOutcome, ContourDrawClickError> {
    let (active_tool, active_roi) = world
        .get::<&EditorState>(entities.editor)
        .map(|editor| (editor.active_tool, editor.active_roi))
        .map_err(|_| ContourDrawClickError::ToolNotActive)?;
    if active_tool != EditorTool::ContourDraw {
        return Err(ContourDrawClickError::ToolNotActive);
    }
    let roi_entity = active_roi.ok_or(ContourDrawClickError::MissingActiveRoi)?;

    let contour_data = contour_data_for_active_roi(world, roi_entity)
        .ok_or(ContourDrawClickError::ActiveRoiNotContour)?;
    let viewport = resolve_active_contour_edit_viewport(world, entities, &contour_data)?;
    let point_local_mm = viewport_uv_to_contour_plane_local_mm(viewport_uv, viewport)
        .ok_or(ContourDrawClickError::ProjectionFailed)?;

    let viewport_rect = world
        .get::<&Viewport>(viewport.viewport_entity)
        .map(|vp| vp.rect)
        .map_err(|_| ContourDrawClickError::ProjectionFailed)?;

    let loop_points_to_commit = {
        let mut editor = world
            .get::<&mut EditorState>(entities.editor)
            .map_err(|_| ContourDrawClickError::ToolNotActive)?;
        let reset_draft = editor
            .contour_draft
            .as_ref()
            .map(|draft| {
                draft.roi_entity != roi_entity
                    || !planes_match_for_slice(draft.plane, viewport.plane)
            })
            .unwrap_or(true);
        if reset_draft {
            editor.contour_draft = Some(ContourDraft {
                roi_entity,
                plane: viewport.plane,
                points: Vec::new(),
            });
        }

        let draft = editor
            .contour_draft
            .as_mut()
            .ok_or(ContourDrawClickError::ToolNotActive)?;
        let near_first = if let Some(first) = draft.points.first() {
            if let Some(first_uv) = contour_plane_local_mm_to_viewport_uv(first.local_mm, viewport)
            {
                let dx = (viewport_uv[0] - first_uv[0]) * viewport_rect[2];
                let dy = (viewport_uv[1] - first_uv[1]) * viewport_rect[3];
                (dx * dx + dy * dy).sqrt() <= LOOP_CLOSE_RADIUS_PX
            } else {
                false
            }
        } else {
            false
        };

        if near_first && !draft.points.is_empty() {
            if draft.points.len() < 3 {
                return Err(ContourDrawClickError::LoopNeedsThreePoints);
            }
            Some(draft.points.clone())
        } else {
            draft.points.push(ContourPoint {
                local_mm: point_local_mm,
            });
            return Ok(ContourDrawClickOutcome::PointAdded);
        }
    };

    if let Some(loop_points) = loop_points_to_commit {
        let mut next_contour_data = contour_data_for_active_roi(world, roi_entity)
            .ok_or(ContourDrawClickError::ActiveRoiNotContour)?;
        let loop_to_commit = ContourLoop {
            points: loop_points,
            is_closed: true,
        };
        if let Some(existing_slice) = next_contour_data
            .slices
            .iter_mut()
            .find(|slice| planes_match_for_slice(slice.plane, viewport.plane))
        {
            existing_slice.loops.push(loop_to_commit);
        } else {
            next_contour_data.slices.push(ContourSlice {
                plane: viewport.plane,
                loops: vec![loop_to_commit],
            });
        }

        roi_runtime::replace_contour_data(world, roi_entity, next_contour_data)
            .map_err(|_| ContourDrawClickError::CommitFailed)?;
        if let Ok(mut editor) = world.get::<&mut EditorState>(entities.editor) {
            editor.contour_draft = None;
        }
        Ok(ContourDrawClickOutcome::LoopCommitted)
    } else {
        Err(ContourDrawClickError::ToolNotActive)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{
        ContourData, InputState, RoiCacheKind, RoiJobKind, ViewportState, WindowSettings,
    };

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

    fn spawn_test_contour_roi(world: &mut World, family: PlaneFamily) -> hecs::Entity {
        world.spawn((Roi::new_contour(
            crate::components::RoiId(100),
            "Contour".to_string(),
            ContourData {
                active_plane_family: family,
                slices: Vec::new(),
            },
        ),))
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

    #[test]
    fn test_contour_draw_click_appends_draft_without_authoritative_mutation() {
        let mut world = World::new();
        let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
        let roi_entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial);
        {
            let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
            editor.active_roi = Some(roi_entity);
            editor.active_tool = EditorTool::ContourDraw;
        }

        let result = handle_contour_draw_click(&mut world, &entities, [0.4, 0.5]);
        assert_eq!(result, Ok(ContourDrawClickOutcome::PointAdded));

        let editor = world.get::<&EditorState>(entities.editor).unwrap();
        let draft = editor.contour_draft.as_ref().expect("draft");
        assert_eq!(draft.points.len(), 1);
        let roi = world.get::<&Roi>(roi_entity).unwrap();
        assert!(roi.contour_data().unwrap().slices.is_empty());
    }

    #[test]
    fn test_contour_draw_loop_closure_rejects_fewer_than_three_points() {
        let mut world = World::new();
        let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
        let roi_entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial);
        {
            let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
            editor.active_roi = Some(roi_entity);
            editor.active_tool = EditorTool::ContourDraw;
        }

        assert_eq!(
            handle_contour_draw_click(&mut world, &entities, [0.45, 0.45]),
            Ok(ContourDrawClickOutcome::PointAdded)
        );
        assert_eq!(
            handle_contour_draw_click(&mut world, &entities, [0.55, 0.45]),
            Ok(ContourDrawClickOutcome::PointAdded)
        );
        let close_result = handle_contour_draw_click(&mut world, &entities, [0.45, 0.45]);
        assert_eq!(
            close_result,
            Err(ContourDrawClickError::LoopNeedsThreePoints)
        );
    }

    #[test]
    fn test_contour_draw_loop_commit_adds_slice_loop_and_queues_voxel_rebuild() {
        let mut world = World::new();
        let entities = spawn_test_entities(&mut world, ViewMode::Axial, [0.0, 0.0, 0.0, 1.0], None);
        let roi_entity = spawn_test_contour_roi(&mut world, PlaneFamily::Axial);
        {
            let mut editor = world.get::<&mut EditorState>(entities.editor).unwrap();
            editor.active_roi = Some(roi_entity);
            editor.active_tool = EditorTool::ContourDraw;
        }

        assert_eq!(
            handle_contour_draw_click(&mut world, &entities, [0.40, 0.40]),
            Ok(ContourDrawClickOutcome::PointAdded)
        );
        assert_eq!(
            handle_contour_draw_click(&mut world, &entities, [0.58, 0.42]),
            Ok(ContourDrawClickOutcome::PointAdded)
        );
        assert_eq!(
            handle_contour_draw_click(&mut world, &entities, [0.52, 0.62]),
            Ok(ContourDrawClickOutcome::PointAdded)
        );
        let close_result = handle_contour_draw_click(&mut world, &entities, [0.40, 0.40]);
        assert_eq!(close_result, Ok(ContourDrawClickOutcome::LoopCommitted));

        let roi = world.get::<&Roi>(roi_entity).unwrap();
        let contour = roi.contour_data().unwrap();
        assert_eq!(contour.slices.len(), 1);
        assert_eq!(contour.slices[0].loops.len(), 1);
        assert!(contour.slices[0].loops[0].is_closed);
        assert_eq!(roi.job_state.queued, Some(RoiJobKind::RebuildVoxelCache));
        assert!(roi.is_cache_dirty(RoiCacheKind::Voxel));
        assert!(world
            .get::<&EditorState>(entities.editor)
            .unwrap()
            .contour_draft
            .is_none());
    }

    #[test]
    fn test_clear_contour_draft_when_tool_not_draw() {
        let mut world = World::new();
        let editor = world.spawn((EditorState {
            active_roi: Some(hecs::Entity::DANGLING),
            active_tool: EditorTool::Navigation,
            contour_draft: Some(ContourDraft {
                roi_entity: hecs::Entity::DANGLING,
                plane: PlaneDefinition {
                    family: PlaneFamily::Axial,
                    origin_mm: [0.0, 0.0, 0.0],
                    u_axis_mm: [1.0, 0.0, 0.0],
                    v_axis_mm: [0.0, 1.0, 0.0],
                    normal_mm: [0.0, 0.0, 1.0],
                },
                points: vec![],
            }),
        },));

        clear_contour_draft_if_inactive(&mut world, editor);
        assert!(world
            .get::<&EditorState>(editor)
            .unwrap()
            .contour_draft
            .is_none());
    }
}
