use crate::app::{annotations, roi_runtime};
use crate::components::*;
use crate::AppEvent;
use hecs::World;
use winit::event_loop::EventLoopProxy;

const NOTE_PREVIEW_MAX_CHARS: usize = 64;

/// Shortens a note for the annotation list without splitting a multi-byte character.
fn note_preview(note: &str) -> String {
    if note.chars().count() > NOTE_PREVIEW_MAX_CHARS {
        let kept: String = note.chars().take(NOTE_PREVIEW_MAX_CHARS - 3).collect();
        format!("{kept}...")
    } else {
        note.to_string()
    }
}

pub fn draw_discussion_sidebar(
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    world: &mut World,
    session: &mut Session,
    event_proxy: &EventLoopProxy<AppEvent>,
) {
    ui.heading("💬 Discussion");
    ui.separator();

    let mut to_delete = None;
    let mut to_locate = None;
    let mut comment_to_add = None;
    let mut new_focus = None;
    let mut back_to_list = false;

    let focused = session
        .annotations
        .focused_id
        .and_then(|id| annotations::find_annotation(world, id).map(|entity| (id, entity)));

    if let Some((focused_id, entity)) = focused {
        let anchor_mm = world.get::<&Anchor>(entity).map(|anchor| anchor.world_mm);
        ui.horizontal(|ui| {
            if ui.button("⬅").on_hover_text("Back to List").clicked() {
                back_to_list = true;
            }
            if let Ok(mut label) = world.get::<&mut Label>(entity) {
                ui.add(
                    egui::TextEdit::singleline(&mut label.0).font(egui::FontId::proportional(20.0)),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("🗑").on_hover_text("Delete").clicked() {
                    to_delete = Some(focused_id);
                }
                if ui.button("🎯").on_hover_text("Locate").clicked() {
                    to_locate = anchor_mm.ok();
                }
            });
        });

        ui.add_space(4.0);
        ui.label(egui::RichText::new("Notes").strong());
        if let Ok(mut note) = world.get::<&mut Text>(entity) {
            ui.add(
                egui::TextEdit::multiline(&mut note.0)
                    .hint_text("Add clinical observation notes...")
                    .desired_rows(5)
                    .desired_width(ui.available_width()),
            );
        }
        if let Ok(provenance) = world.get::<&Provenance>(entity) {
            ui.label(
                egui::RichText::new(format!("Created by {}", provenance.author))
                    .size(10.0)
                    .color(egui::Color32::GRAY),
            );
        }

        ui.separator();
        ui.label(egui::RichText::new("thread").strong());

        let comments = annotations::comments_of(world, focused_id);
        egui::ScrollArea::vertical()
            .max_height(300.0)
            .show(ui, |ui| {
                for comment in &comments {
                    let response = ui
                        .group(|ui| {
                            ui.label(
                                egui::RichText::new(&comment.author)
                                    .strong()
                                    .color(egui::Color32::LIGHT_BLUE),
                            );
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
                if comments.is_empty() {
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
            let res =
                ui.add(egui::TextEdit::singleline(&mut input_text).hint_text("Type a reply..."));
            if ((res.lost_focus() && ctx.input(|i| i.key_pressed(egui::Key::Enter)))
                || ui.button("Send").clicked())
                && !input_text.is_empty()
            {
                comment_to_add = Some((focused_id, input_text.clone()));
                input_text.clear();
            }
            ctx.memory_mut(|mem| mem.data.insert_temp(dnd_id, input_text));
        });
    } else if session.annotations.focused_id.is_some() {
        // The focused annotation no longer exists.
        back_to_list = true;
    } else {
        let rows = annotations::annotation_rows(world);
        ui.vertical(|ui| {
            ui.add_space(8.0);
            if rows.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(50.0);
                    ui.label("No annotations yet.");
                    ui.label("Click \"Add at Cursor\" in the left panel.");
                });
            } else {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for row in &rows {
                        let response = ui
                            .group(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(&row.label).strong());
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui.button("🗑").on_hover_text("Delete").clicked() {
                                                to_delete = Some(row.id);
                                            }
                                            if ui.button("🎯").on_hover_text("Locate").clicked() {
                                                to_locate = Some(row.anchor_mm);
                                            }
                                        },
                                    );
                                });
                                let preview = note_preview(&row.note);
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
                            new_focus = Some(row.id);
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
        session.annotations.focused_id = None;
    }
    if let Some(id) = new_focus {
        session.annotations.focused_id = Some(id);
    }

    // Apply pending actions
    if let Some(id) = to_delete {
        let _ = event_proxy.send_event(AppEvent::DeleteAnnotation(id));
    }
    if let (Some(world_mm), Some(geometry)) = (to_locate, roi_runtime::main_volume_geometry(world))
    {
        session.cursor.position = crate::convert::world_mm_to_volume_uv(world_mm, geometry);
    }
    if let Some((id, text)) = comment_to_add {
        let _ = event_proxy.send_event(AppEvent::AddComment(id, text));
    }

    ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
        if ui.button("Close Sidebar").clicked() {
            session.annotations.show_right_sidebar = false;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_note_preview_keeps_short_notes_unchanged() {
        assert_eq!(note_preview("short note"), "short note");
    }

    #[test]
    fn test_note_preview_truncates_ascii_notes_with_ellipsis() {
        let preview = note_preview(&"a".repeat(100));

        assert_eq!(preview.chars().count(), NOTE_PREVIEW_MAX_CHARS);
        assert!(preview.ends_with("..."));
    }

    #[test]
    fn test_note_preview_does_not_panic_on_multibyte_characters() {
        // 60 ASCII bytes then a 2-byte character straddling the old byte-61 cut.
        let note = format!("{}æøå{}", "a".repeat(60), "b".repeat(40));
        let preview = note_preview(&note);

        assert_eq!(preview.chars().count(), NOTE_PREVIEW_MAX_CHARS);
        assert!(preview.ends_with("..."));
        assert!(note_preview(&"😀".repeat(100)).ends_with("..."));
    }
}
