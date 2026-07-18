use crate::components::*;
use crate::io::handlers;
use crate::{file_dialog, nifti_loader, AppEvent};
use hecs::World;
use winit::event_loop::EventLoopProxy;

fn apply_roi_history_action(
    world: &mut World,
    entities: &AppEntities,
    event_proxy: &EventLoopProxy<AppEvent>,
    undo: bool,
) {
    if let Ok(mut input) = world.get::<&mut InputState>(entities.input) {
        input.contour_move_pending_commit = false;
        input.mesh_move_pending_commit = false;
    }
    let result = if undo {
        crate::app::roi_runtime::undo_roi_edit(world, entities.editor)
    } else {
        crate::app::roi_runtime::redo_roi_edit(world, entities.editor)
    };
    match result {
        Ok(_) => {
            handlers::set_status_message(
                world,
                entities,
                if undo {
                    "Undid ROI edit.".to_string()
                } else {
                    "Redid ROI edit.".to_string()
                },
            );
            let _ = event_proxy.send_event(AppEvent::RebuildBindGroups);
        }
        Err(error) => handlers::set_status_message(
            world,
            entities,
            format!("{} failed: {error:?}.", if undo { "Undo" } else { "Redo" }),
        ),
    }
}

fn roi_history_shortcuts(ctx: &egui::Context) -> (bool, bool) {
    let wants_keyboard_input = ctx.wants_keyboard_input();
    ctx.input(|input| {
        if wants_keyboard_input || !input.modifiers.command {
            return (false, false);
        }
        (
            !input.modifiers.shift && input.key_pressed(egui::Key::Z),
            (input.modifiers.shift && input.key_pressed(egui::Key::Z))
                || input.key_pressed(egui::Key::Y),
        )
    })
}

fn set_editor_tool(
    world: &mut World,
    entities: &AppEntities,
    requested_tool: EditorTool,
) -> Result<(), String> {
    if requested_tool == EditorTool::Navigation {
        if let Ok(mut input) = world.get::<&mut InputState>(entities.input) {
            input.contour_move_pending_commit = false;
            input.mesh_move_pending_commit = false;
        }
        let mut editor = world
            .get::<&mut EditorState>(entities.editor)
            .map_err(|_| "Missing editor state".to_string())?;
        editor.active_tool = EditorTool::Navigation;
        editor.contour_draft = None;
        editor.contour_selection = None;
        editor.mesh_selection = None;
        let preview_roi = editor
            .contour_move_preview
            .take()
            .map(|preview| preview.roi_entity);
        let mesh_preview_roi = editor
            .mesh_edit_preview
            .take()
            .map(|preview| preview.roi_entity);
        drop(editor);
        if let Some(roi_entity) = preview_roi {
            if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
                roi.end_preview();
            }
        }
        if let Some(roi_entity) = mesh_preview_roi {
            if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
                roi.end_preview();
            }
        }
        return Ok(());
    }

    let active_roi = world
        .get::<&EditorState>(entities.editor)
        .map_err(|_| "Missing editor state".to_string())?
        .active_roi
        .ok_or_else(|| "Select an ROI before choosing an edit tool.".to_string())?;

    let roi = world
        .get::<&Roi>(active_roi)
        .map_err(|_| "Active ROI is missing from the scene.".to_string())?;
    if roi.metadata.is_locked {
        return Err("Active ROI is locked.".to_string());
    }
    match requested_tool {
        EditorTool::ContourSelect | EditorTool::ContourDraw
            if roi.primary_representation != PrimaryRepresentation::Contour =>
        {
            return Err(
                "Switch the active ROI to Contour authority before editing points or adding loops."
                    .to_string(),
            );
        }
        EditorTool::MeshDeform if roi.primary_representation != PrimaryRepresentation::Mesh => {
            return Err("Switch the active ROI to Mesh authority before deforming it.".to_string());
        }
        _ => {}
    }
    drop(roi);

    if let Ok(mut input) = world.get::<&mut InputState>(entities.input) {
        input.contour_move_pending_commit = false;
        input.mesh_move_pending_commit = false;
    }

    let mut editor = world
        .get::<&mut EditorState>(entities.editor)
        .map_err(|_| "Missing editor state".to_string())?;
    editor.active_tool = requested_tool;
    if requested_tool != EditorTool::ContourDraw {
        editor.contour_draft = None;
    }
    if requested_tool != EditorTool::ContourSelect {
        editor.contour_selection = None;
    }
    let preview_roi = if requested_tool != EditorTool::ContourSelect {
        editor
            .contour_move_preview
            .take()
            .map(|preview| preview.roi_entity)
    } else {
        None
    };
    if requested_tool != EditorTool::MeshDeform {
        editor.mesh_selection = None;
    }
    let mesh_preview_roi = if requested_tool != EditorTool::MeshDeform {
        editor
            .mesh_edit_preview
            .take()
            .map(|preview| preview.roi_entity)
    } else {
        None
    };
    drop(editor);
    for roi_entity in [preview_roi, mesh_preview_roi].into_iter().flatten() {
        if let Ok(mut roi) = world.get::<&mut Roi>(roi_entity) {
            roi.end_preview();
        }
    }
    Ok(())
}

pub fn draw_toolbar(
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    world: &mut World,
    entities: &AppEntities,
    event_proxy: &EventLoopProxy<AppEvent>,
    status_msg: Option<String>,
    windowing_active: bool,
) {
    let (undo_shortcut, redo_shortcut) = roi_history_shortcuts(ctx);
    if undo_shortcut {
        apply_roi_history_action(world, entities, event_proxy, true);
    } else if redo_shortcut {
        apply_roi_history_action(world, entities, event_proxy, false);
    }

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        ui.heading("🩺 Medical Viewer");
        ui.separator();

        // --- Data Loading ---
        ui.label("Data:");
        if ui
            .button("📂 Volume")
            .on_hover_text("Load Main Volume (NIfTI)")
            .clicked()
        {
            if let Ok(mut g) = world.get::<&mut GuiState>(entities.gui_state) {
                g.status_message = Some("Loading...".to_string());
            }
            let proxy = event_proxy.clone();
            file_dialog::spawn_file_picker(move |result| {
                if let Some((_filename, data)) = result {
                    let load_result =
                        nifti_loader::load_nifti_from_bytes(&data).map(LoadResult::Volume);
                    let _ = proxy.send_event(AppEvent::VolumeLoaded(load_result));
                }
            });
        }
        if ui
            .button("📂 Label")
            .on_hover_text("Load Labelmap")
            .clicked()
        {
            if let Ok(mut g) = world.get::<&mut GuiState>(entities.gui_state) {
                g.status_message = Some("Loading Labelmap...".to_string());
            }
            let proxy = event_proxy.clone();
            file_dialog::spawn_file_picker(move |result| {
                if let Some((filename, data)) = result {
                    let load_result =
                        nifti_loader::load_label_from_bytes(&data, filename).map(LoadResult::Label);
                    let _ = proxy.send_event(AppEvent::VolumeLoaded(load_result));
                }
            });
        }

        // --- Presets (Quick Access) ---
        if windowing_active {
            if let Ok(mut windowing) = world.get::<&mut VolumeWindowing>(entities.volume_windowing)
            {
                ui.label("Presets:");
                if ui.small_button("Soft").clicked() {
                    windowing.center = 40.0;
                    windowing.width = 400.0;
                }
                if ui.small_button("Lung").clicked() {
                    windowing.center = -600.0;
                    windowing.width = 1500.0;
                }
                if ui.small_button("Bone").clicked() {
                    windowing.center = 400.0;
                    windowing.width = 2000.0;
                }
            }
        }

        ui.separator();
        ui.label("Tool:");
        let active_tool = world
            .get::<&EditorState>(entities.editor)
            .map(|editor| editor.active_tool)
            .unwrap_or(EditorTool::Navigation);

        for (label, tool) in [
            ("Nav", EditorTool::Navigation),
            ("Edit Points", EditorTool::ContourSelect),
            ("Add Loop", EditorTool::ContourDraw),
            ("Deform Mesh", EditorTool::MeshDeform),
        ] {
            if ui.selectable_label(active_tool == tool, label).clicked() {
                if let Err(message) = set_editor_tool(world, entities, tool) {
                    handlers::set_status_message(world, entities, message);
                }
            }
        }

        ui.separator();
        let can_undo = crate::app::roi_runtime::can_undo_roi_edit(world, entities.editor);
        let can_redo = crate::app::roi_runtime::can_redo_roi_edit(world, entities.editor);
        if ui
            .add_enabled(can_undo, egui::Button::new("↶"))
            .on_hover_text("Undo ROI edit")
            .clicked()
        {
            apply_roi_history_action(world, entities, event_proxy, true);
        }
        if ui
            .add_enabled(can_redo, egui::Button::new("↷"))
            .on_hover_text("Redo ROI edit")
            .clicked()
        {
            apply_roi_history_action(world, entities, event_proxy, false);
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Ok(mut state) = world.get::<&mut AnnotationState>(entities.annotations) {
                let icon = if state.show_right_sidebar {
                    "📝"
                } else {
                    "🗒"
                };
                if ui
                    .selectable_label(state.show_right_sidebar, format!("{} Notes", icon))
                    .on_hover_text("Toggle Discussion Sidebar")
                    .clicked()
                {
                    state.show_right_sidebar = !state.show_right_sidebar;
                }
            }
        });
    });

    if let Some(msg) = status_msg {
        ui.label(egui::RichText::new(msg).color(egui::Color32::LIGHT_BLUE));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roi_history_shortcuts_do_not_reenter_egui_context_lock() {
        let ctx = egui::Context::default();

        assert_eq!(roi_history_shortcuts(&ctx), (false, false));
    }
}
