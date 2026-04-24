use crate::components::*;
use crate::AppEvent;
use hecs::World;
use winit::event_loop::EventLoopProxy;

use crate::app::roi_runtime;

pub fn draw_sidebar(
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    world: &mut World,
    entities: &AppEntities,
    event_proxy: &EventLoopProxy<AppEvent>,
    volume_info: Option<[u32; 3]>,
) {
    ui.add_space(8.0);

    ui.collapsing("📁 Protocol", |ui| {
        if let Ok(proto) = world.get::<&ProtocolState>(entities.protocol) {
            let mut selected = proto.active_protocol.clone();
            let registry = crate::render::protocols::get_protocol_registry();
            egui::ComboBox::from_label("Active Protocol")
                .selected_text(&selected)
                .show_ui(ui, |ui| {
                    for p in registry {
                        if ui
                            .selectable_value(&mut selected, p.name.clone(), &p.name)
                            .clicked()
                        {
                            ctx.request_repaint();
                        }
                    }
                });

            if selected != proto.active_protocol {
                let _ = event_proxy.send_event(AppEvent::SwitchProtocol(selected));
                ctx.request_repaint();
            }
        }
    });

    ui.separator();

    // --- Volume Info ---
    ui.collapsing("📊 Volume Info", |ui| {
        if let Some(dims) = volume_info {
            ui.label(format!("Dimensions: {}×{}×{}", dims[0], dims[1], dims[2]));
            if let Some((_, vd)) = world
                .query::<&VolumeData>()
                .with::<&MainVolumeTag>()
                .iter()
                .next()
            {
                ui.label(format!(
                    "Spacing: {:.2}×{:.2}×{:.2} mm",
                    vd.spacing[0], vd.spacing[1], vd.spacing[2]
                ));
                ui.label(format!(
                    "Range: {:.0} to {:.0} HU",
                    vd.intensity_range[0], vd.intensity_range[1]
                ));
            }
        } else {
            ui.label("No volume loaded");
        }
    });

    ui.separator();

    // --- Windowing / Contrast (Detailed) ---
    ui.collapsing("🌓 Windowing", |ui| {
        if let Ok(mut windowing) = world.get::<&mut VolumeWindowing>(entities.volume_windowing) {
            ui.label("Center (HU)");
            ui.add(egui::Slider::new(&mut windowing.center, -1024.0..=3071.0).show_value(true));

            ui.label("Width (HU)");
            ui.add(egui::Slider::new(&mut windowing.width, 1.0..=4096.0).show_value(true));

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Brain").clicked() {
                    windowing.center = 40.0;
                    windowing.width = 80.0;
                }
                if ui.button("Default").clicked() {
                    windowing.center = 40.0;
                    windowing.width = 400.0;
                }
            });
        }
    });

    ui.separator();

    // --- Layer Control ---
    ui.collapsing("📚 Layers", |ui| {
        let mut layers: Vec<(
            hecs::Entity,
            String,
            bool,
            f32,
            Option<roi_runtime::VoxelRoiStats>,
        )> = Vec::new();
        for (e, (roi, settings)) in world.query::<(&Roi, &LayerSettings)>().iter() {
            layers.push((
                e,
                roi.metadata.name.clone(),
                roi.metadata.is_visible,
                settings.opacity,
                roi_runtime::voxel_roi_stats(world, e),
            ));
        }

        let active_roi = world
            .get::<&EditorState>(entities.editor)
            .ok()
            .and_then(|e| e.active_roi);
        let mut new_active_roi = active_roi;

        for (entity, name, mut visible, mut opacity, stats) in layers {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.radio_value(&mut new_active_roi, Some(entity), "");
                    if ui.checkbox(&mut visible, "").changed() {
                        if let Ok(mut roi) = world.get::<&mut Roi>(entity) {
                            roi.metadata.is_visible = visible;
                        }
                        let _ = event_proxy.send_event(AppEvent::RebuildBindGroups);
                    }
                    ui.label(name);
                });
                if let Some(stats) = stats {
                    ui.label(format!(
                        "{} voxels, {:.2} mm^3",
                        stats.occupied_voxels, stats.volume_mm3
                    ));
                }
                if visible
                    && ui
                        .add(egui::Slider::new(&mut opacity, 0.0..=1.0).text("Opacity"))
                        .changed()
                {
                    if let Ok(mut set) = world.get::<&mut LayerSettings>(entity) {
                        set.opacity = opacity;
                    }
                }
            });
        }

        if new_active_roi != active_roi {
            if let Ok(mut editor) = world.get::<&mut EditorState>(entities.editor) {
                editor.active_roi = new_active_roi;
                let _ = event_proxy.send_event(AppEvent::RebuildBindGroups);
            }
        }
    });

    ui.separator();

    // --- Annotations ---
    ui.collapsing("📍 Annotations", |ui| {
        if ui.button("➕ Add at Cursor").clicked() {
            let mut current_pos = glam::Vec3::ZERO;
            if let Ok(t) = world.get::<&Transform>(entities.cursor) {
                current_pos = glam::Vec3::from(t.position);
            }

            if let Ok(mut state) = world.get::<&mut AnnotationState>(entities.annotations) {
                let next_idx = state.annotations.len() + 1;
                let new_id = uuid::Uuid::new_v4();
                state.annotations.push(Annotation {
                    id: new_id,
                    world_pos: current_pos,
                    label: format!("Note {}", next_idx),
                    note: String::new(),
                    comments: vec![],
                });
                state.focused_id = Some(new_id);
                state.show_right_sidebar = true;
            }
        }

        if ui.button("📁 View All Notes").clicked() {
            if let Ok(mut state) = world.get::<&mut AnnotationState>(entities.annotations) {
                state.focused_id = None;
                state.show_right_sidebar = true;
            }
        }

        ui.separator();

        if let Ok(mut state) = world.get::<&mut AnnotationState>(entities.annotations) {
            if !state.annotations.is_empty() {
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        let mut to_focus = None;
                        for ann in &state.annotations {
                            let is_focused = state.focused_id == Some(ann.id);
                            if ui
                                .selectable_label(is_focused, format!("📍 {}", ann.label))
                                .clicked()
                            {
                                to_focus = Some(ann.id);
                            }
                        }
                        if let Some(id) = to_focus {
                            state.focused_id = Some(id);
                            state.show_right_sidebar = true;
                        }
                    });
            } else {
                ui.label(
                    egui::RichText::new("No notes yet.")
                        .size(10.0)
                        .color(egui::Color32::GRAY),
                );
            }
        }
    });

    ui.separator();
    ui.collapsing("⌨ Controls", |ui| {
        ui.label("LMB: Set crosshair");
        ui.label("MMB: Pan");
        ui.label("RMB: Rotate (3D)");
        ui.label("Scroll: Zoom / Slice");
        ui.label("Ctrl+Scroll: 2D Zoom");
    });

    ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
        ui.separator();
        if let Some(hu) = crate::systems::get_hu_at_mouse(world, entities) {
            ui.label(format!("HU at cursor: {:.0}", hu));
        } else {
            ui.label("HU at cursor: --");
        }
    });
}
