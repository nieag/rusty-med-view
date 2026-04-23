use crate::components::*;
use crate::{file_dialog, nifti_loader, AppEvent};
use hecs::World;
use winit::event_loop::EventLoopProxy;

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
