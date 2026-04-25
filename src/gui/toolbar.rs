use crate::components::*;
use crate::io::handlers;
use crate::{file_dialog, nifti_loader, AppEvent};
use hecs::World;
use winit::event_loop::EventLoopProxy;

fn set_editor_tool(
    world: &mut World,
    entities: &AppEntities,
    requested_tool: EditorTool,
) -> Result<(), String> {
    if requested_tool == EditorTool::Navigation {
        let mut editor = world
            .get::<&mut EditorState>(entities.editor)
            .map_err(|_| "Missing editor state".to_string())?;
        editor.active_tool = EditorTool::Navigation;
        editor.contour_draft = None;
        editor.contour_selection = None;
        return Ok(());
    }

    let active_roi = world
        .get::<&EditorState>(entities.editor)
        .map_err(|_| "Missing editor state".to_string())?
        .active_roi
        .ok_or_else(|| "Select a contour ROI before using contour tools.".to_string())?;

    let roi = world
        .get::<&Roi>(active_roi)
        .map_err(|_| "Active ROI is missing from the scene.".to_string())?;
    if roi.primary_representation != PrimaryRepresentation::Contour {
        return Err(
            "Active ROI is not contour-primary. Create/select a contour ROI first.".to_string(),
        );
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
    Ok(())
}

pub fn draw_toolbar(
    _ctx: &egui::Context,
    ui: &mut egui::Ui,
    world: &mut World,
    entities: &AppEntities,
    event_proxy: &EventLoopProxy<AppEvent>,
    status_msg: Option<String>,
    windowing_active: bool,
) {
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
            ("Contour Edit", EditorTool::ContourSelect),
            ("Contour Draw", EditorTool::ContourDraw),
        ] {
            if ui.selectable_label(active_tool == tool, label).clicked() {
                if let Err(message) = set_editor_tool(world, entities, tool) {
                    handlers::set_status_message(world, entities, message);
                }
            }
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

            if let Some(msg) = status_msg {
                ui.label(egui::RichText::new(msg).color(egui::Color32::LIGHT_BLUE));
            }
        });
    });
}
