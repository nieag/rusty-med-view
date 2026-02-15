//! Segment management GUI panel.
//!
//! Provides UI for creating, viewing, and managing contour-based segments.

use crate::components::*;
use crate::systems::segment_system::SegmentManager;
use hecs::World;

/// Draw the segment management panel in a side panel or window.
pub fn draw_segment_panel(ui: &mut egui::Ui, world: &mut World, entities: &AppEntities) {
    ui.heading("📐 Segments");
    ui.separator();

    // Get segment manager mutably
    let mut segment_count = 0;
    let mut active_idx = None;
    let mut segment_names: Vec<(usize, String, [f32; 4], bool)> = Vec::new();
    let mut contour_count = 0;
    let mut active_sdf_status = "No active segment".to_string();

    if let Ok(mgr) = world.get::<&SegmentManager>(entities.segments) {
        segment_count = mgr.len();
        active_idx = mgr.active_segment;
        for (i, seg) in mgr.segments.iter().enumerate() {
            segment_names.push((i, seg.name.clone(), seg.color, seg.visible));
            contour_count += seg.contours.count();
        }
        if let Some(active) = mgr.active_segment.and_then(|idx| mgr.segments.get(idx)) {
            active_sdf_status = if active.sdf_dirty {
                "Rebuilding".to_string()
            } else if active.sdf.is_some() {
                "Up-to-date".to_string()
            } else if active.has_contours() {
                "Stale".to_string()
            } else {
                "No contours".to_string()
            };
        }
    }

    // Add segment button
    ui.horizontal(|ui| {
        if ui.button("➕ Add Segment").clicked() {
            if let Ok(mut mgr) = world.get::<&mut SegmentManager>(entities.segments) {
                let idx = mgr.len();
                let colors = [
                    [1.0, 0.3, 0.3, 0.8], // Red
                    [0.3, 1.0, 0.3, 0.8], // Green
                    [0.3, 0.3, 1.0, 0.8], // Blue
                    [1.0, 1.0, 0.3, 0.8], // Yellow
                    [1.0, 0.3, 1.0, 0.8], // Magenta
                    [0.3, 1.0, 1.0, 0.8], // Cyan
                ];
                let color = colors[idx % colors.len()];
                mgr.add_segment(&format!("Segment {}", idx + 1), color);
            }
        }
    });

    ui.separator();

    if segment_count == 0 {
        ui.label("No segments. Click ➕ to add one.");
        return;
    }

    // Segment list
    egui::ScrollArea::vertical()
        .max_height(200.0)
        .show(ui, |ui| {
            let mut to_remove = None;
            let mut to_activate = None;

            for (idx, name, color, visible) in &segment_names {
                let is_active = active_idx == Some(*idx);

                ui.horizontal(|ui| {
                    // Color indicator
                    let color32 = egui::Color32::from_rgba_unmultiplied(
                        (color[0] * 255.0) as u8,
                        (color[1] * 255.0) as u8,
                        (color[2] * 255.0) as u8,
                        255,
                    );
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 2.0, color32);

                    // Name with selection
                    let label = if is_active {
                        format!("▶ {}", name)
                    } else {
                        name.clone()
                    };

                    if ui.selectable_label(is_active, &label).clicked() {
                        to_activate = Some(*idx);
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Delete button
                        if ui
                            .small_button("🗑")
                            .on_hover_text("Delete segment")
                            .clicked()
                        {
                            to_remove = Some(*idx);
                        }

                        // Visibility toggle
                        let vis_icon = if *visible { "👁" } else { "👁‍🗨" };
                        if ui
                            .small_button(vis_icon)
                            .on_hover_text("Toggle visibility")
                            .clicked()
                        {
                            if let Ok(mut mgr) = world.get::<&mut SegmentManager>(entities.segments)
                            {
                                if let Some(seg) = mgr.segments.get_mut(*idx) {
                                    seg.visible = !seg.visible;
                                }
                            }
                        }
                    });
                });
            }

            // Process actions after iteration
            if let Some(idx) = to_remove {
                if let Ok(mut mgr) = world.get::<&mut SegmentManager>(entities.segments) {
                    mgr.remove_segment(idx);
                }
            }
            if let Some(idx) = to_activate {
                if let Ok(mut mgr) = world.get::<&mut SegmentManager>(entities.segments) {
                    mgr.active_segment = Some(idx);
                }
            }
        });

    ui.separator();

    // Stats
    ui.label(format!("Total contours: {}", contour_count));
    ui.label(format!("SDF: {}", active_sdf_status));

    if let Ok(perf) = world.get::<&SegPerfConfig>(entities.seg_perf) {
        ui.separator();
        ui.label("Segmentation Perf");
        ui.label("Live updates: on (immediate)");
        ui.label(format!("Resolution Scale: {:.2}", perf.resolution_scale));
        ui.label(format!(
            "Last update: SDF {:.1} ms, Mesh {:.1} ms, Queue {}",
            perf.last_sdf_ms, perf.last_mesh_ms, perf.queue_depth
        ));
    }

    ui.separator();
    ui.label("SDF Preview");
    if let Ok(mut preview) = world.get::<&mut SdfPreviewState>(entities.sdf_preview) {
        // Keep 3D surface preview always enabled for continuous mesh inspection.
        preview.show_3d_surface = true;
        ui.checkbox(&mut preview.enabled, "Show SDF Preview");
        ui.add(egui::Slider::new(&mut preview.opacity, 0.0..=1.0).text("Opacity"));
        ui.add(egui::Slider::new(&mut preview.value_window_mm, 0.5..=32.0).text("Window (mm)"));
        ui.checkbox(&mut preview.show_zero_isoline, "Zero Isoline");
        ui.label("3D Surface Preview: always on");
    }

    // Instructions
    if active_idx.is_some() {
        ui.separator();
        ui.small("Select ✏ Contour tool, then draw on 2D views.");
    }
}
