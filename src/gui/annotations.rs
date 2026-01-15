use crate::components::*;
use crate::AppEvent;
use hecs::World;
use winit::event_loop::EventLoopProxy;

pub fn draw_discussion_sidebar(
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    world: &mut World,
    entities: &AppEntities,
    event_proxy: &EventLoopProxy<AppEvent>,
) {
    ui.heading("💬 Discussion");
    ui.separator();

    let mut to_delete = None;
    let mut to_locate = None;
    let mut comment_to_add = None;
    let mut new_focus = None;
    let mut back_to_list = false;

    if let Ok(mut state) = world.get::<&mut AnnotationState>(entities.annotations) {
        if let Some(focused_id) = state.focused_id {
            let ann_idx = state.annotations.iter().position(|a| a.id == focused_id);
            if let Some(idx) = ann_idx {
                let ann = &mut state.annotations[idx];
                ui.horizontal(|ui| {
                    if ui.button("⬅").on_hover_text("Back to List").clicked() {
                        back_to_list = true;
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut ann.label)
                            .font(egui::FontId::proportional(20.0)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("🗑").on_hover_text("Delete").clicked() {
                            to_delete = Some(ann.id);
                        }
                        if ui.button("🎯").on_hover_text("Locate").clicked() {
                            to_locate = Some(ann.world_pos);
                        }
                    });
                });

                ui.add_space(4.0);
                ui.label(egui::RichText::new("Notes").strong());
                ui.add(
                    egui::TextEdit::multiline(&mut ann.note)
                        .hint_text("Add clinical observation notes...")
                        .desired_rows(5)
                        .desired_width(ui.available_width()),
                );

                ui.separator();
                ui.label(egui::RichText::new("thread").strong());

                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for comment in &ann.comments {
                            let response = ui
                                .group(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new(&comment.author)
                                                .strong()
                                                .color(egui::Color32::LIGHT_BLUE),
                                        );
                                        ui.label(
                                            egui::RichText::new("Just now")
                                                .size(10.0)
                                                .color(egui::Color32::GRAY),
                                        );
                                    });
                                    ui.label(&comment.text);
                                })
                                .response;

                            if response.hovered() {
                                ui.painter().rect_filled(
                                    response.rect,
                                    2.0,
                                    egui::Color32::from_white_alpha(10),
                                );
                            }
                        }
                        if ann.comments.is_empty() {
                            ui.label(
                                egui::RichText::new("No comments yet.")
                                    .italics()
                                    .color(egui::Color32::GRAY),
                            );
                        }
                    });

                ui.separator();
                ui.horizontal(|ui| {
                    let dnd_id = egui::Id::new("comment_input");
                    let mut input_text =
                        ctx.memory(|mem| mem.data.get_temp::<String>(dnd_id).unwrap_or_default());
                    let res = ui.add(
                        egui::TextEdit::singleline(&mut input_text).hint_text("Type a reply..."),
                    );
                    if (res.lost_focus() && ctx.input(|i| i.key_pressed(egui::Key::Enter)))
                        || ui.button("Send").clicked()
                    {
                        if !input_text.is_empty() {
                            comment_to_add = Some((focused_id, input_text.clone()));
                            input_text.clear();
                        }
                    }
                    ctx.memory_mut(|mem| mem.data.insert_temp(dnd_id, input_text));
                });
            } else {
                back_to_list = true;
            }
        } else {
            ui.vertical(|ui| {
                ui.add_space(8.0);
                if state.annotations.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(50.0);
                        ui.label("No annotations yet.");
                        ui.label("Click \"Add at Cursor\" in the left panel.");
                    });
                } else {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for ann in &state.annotations {
                            let response = ui
                                .group(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new(&ann.label).strong());
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if ui.button("🗑").on_hover_text("Delete").clicked()
                                                {
                                                    to_delete = Some(ann.id);
                                                }
                                                if ui.button("🎯").on_hover_text("Locate").clicked()
                                                {
                                                    to_locate = Some(ann.world_pos);
                                                }
                                            },
                                        );
                                    });
                                    let preview = if ann.note.len() > 64 {
                                        format!("{}...", &ann.note[..61])
                                    } else {
                                        ann.note.clone()
                                    };
                                    if !preview.is_empty() {
                                        ui.label(
                                            egui::RichText::new(preview)
                                                .size(12.0)
                                                .color(egui::Color32::GRAY),
                                        );
                                    } else {
                                        ui.label(
                                            egui::RichText::new("No notes yet...")
                                                .italics()
                                                .color(egui::Color32::DARK_GRAY)
                                                .size(11.0),
                                        );
                                    }
                                })
                                .response
                                .interact(egui::Sense::click());

                            if response.clicked() {
                                new_focus = Some(ann.id);
                            }
                            if response.hovered() {
                                ui.painter().rect_filled(
                                    response.rect,
                                    4.0,
                                    egui::Color32::from_white_alpha(15),
                                );
                            }
                        }
                    });
                }
            });
        }

        if back_to_list {
            state.focused_id = None;
        }
        if let Some(id) = new_focus {
            state.focused_id = Some(id);
        }
    }

    // Apply pending actions
    if let Some(id) = to_delete {
        let _ = event_proxy.send_event(AppEvent::DeleteAnnotation(id));
    }
    if let Some(pos) = to_locate {
        if let Ok(mut t) = world.get::<&mut Transform>(entities.cursor) {
            t.position = pos.into();
        }
    }
    if let Some((id, text)) = comment_to_add {
        let _ = event_proxy.send_event(AppEvent::AddComment(id, text));
    }

    ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
        if ui.button("Close Sidebar").clicked() {
            if let Ok(mut state) = world.get::<&mut AnnotationState>(entities.annotations) {
                state.show_right_sidebar = false;
            }
        }
    });
}
