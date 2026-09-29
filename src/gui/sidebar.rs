use crate::components::*;
use crate::convert::PlaneFamily;
use crate::AppEvent;
use hecs::World;
use winit::event_loop::EventLoopProxy;

use crate::app::{roi, roi_runtime};
use crate::io::handlers;

/// One line saying what the ROI is stored as, and the one explicit conversion left in the UI.
///
/// There is no authority switch: editing tools convert the ROI when needed (`roi::ensure_editable`).
fn draw_roi_form(
    ui: &mut egui::Ui,
    world: &mut World,
    entities: &AppEntities,
    event_proxy: &EventLoopProxy<AppEvent>,
    roi_entity: hecs::Entity,
) {
    let Some((form, mesh_primary)) = world.get::<&Roi>(roi_entity).ok().map(|roi| {
        let form = match &roi.body {
            RoiBody::Voxel(_) => {
                "Voxels (read-only source; editing converts to contours)".to_string()
            }
            RoiBody::Contour(body) => {
                format!("Contours ({:?})", body.data.active_plane_family)
            }
            RoiBody::Mesh(_) => "Mesh".to_string(),
        };
        (form, matches!(roi.body, RoiBody::Mesh(_)))
    }) else {
        return;
    };
    ui.horizontal(|ui| {
        ui.label(format!("Stored as: {form}"));
        if !mesh_primary {
            return;
        }
        let voxel_status = roi::request_voxel_overlay_state(world, roi_entity);
        let rebuildable = matches!(
            voxel_status.state,
            roi::RepresentationRequestState::Stale | roi::RepresentationRequestState::Blocked
        );
        if ui
            .add_enabled(rebuildable, egui::Button::new("Build voxels").small())
            .on_hover_text(
                "Resample the mesh onto its reference voxel grid for the voxel overlay. This can be slow for large meshes.",
            )
            .clicked()
        {
            match roi::request_mesh_voxel_cache_rebuild(world, roi_entity) {
                Ok(()) => {
                    handlers::set_status_message(
                        world,
                        entities,
                        "Voxel-cache rebuild queued; mesh contours remain direct.".to_string(),
                    );
                    let _ = event_proxy.send_event(AppEvent::RebuildBindGroups);
                }
                Err(error) => handlers::set_status_message(
                    world,
                    entities,
                    format!("Voxel-cache rebuild request failed: {error:?}."),
                ),
            }
        }
    });
}

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
                    vd.spacing()[0],
                    vd.spacing()[1],
                    vd.spacing()[2]
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
                roi_runtime::roi_voxel_stats(world, e),
            ));
        }

        let active_roi = world
            .get::<&EditorState>(entities.editor)
            .ok()
            .and_then(|e| e.active_roi);
        let mut new_active_roi = active_roi;

        if ui.small_button("New contour ROI").clicked() {
            match roi_runtime::create_empty_contour_roi(world, entities.editor, PlaneFamily::Axial)
            {
                Ok(entity) => {
                    new_active_roi = Some(entity);
                    handlers::set_status_message(
                        world,
                        entities,
                        "Created contour ROI.".to_string(),
                    );
                    let _ = event_proxy.send_event(AppEvent::RebuildBindGroups);
                }
                Err(err) => handlers::set_status_message(world, entities, err),
            }
        }

        if let Some(roi_entity) = new_active_roi {
            draw_roi_form(ui, world, entities, event_proxy, roi_entity);
        }

        let active_mesh_roi = new_active_roi.filter(|entity| {
            world.get::<&Roi>(*entity).is_ok_and(|roi| {
                roi.primary_representation() == PrimaryRepresentation::Mesh
            })
        });
        if let Some(mesh_entity) = active_mesh_roi {
            let has_mesh_preview = world
                .get::<&Roi>(mesh_entity)
                .is_ok_and(|roi| roi.mesh_edit_preview().is_some());
            let (mut brush_radius_mm, mut brush_strength) = world
                .get::<&EditorState>(entities.editor)
                .map(|editor| (editor.mesh_brush_radius_mm, editor.mesh_brush_strength))
                .unwrap_or((12.0, 1.0));
            let radius_changed = ui
                .add(egui::Slider::new(&mut brush_radius_mm, 1.0..=50.0).text("Minimum radius mm"))
                .on_hover_text("The affected surface area grows during long drags. This is the minimum radius; disconnected surfaces remain unaffected.")
                .changed();
            let strength_changed = ui
                .add(egui::Slider::new(&mut brush_strength, 0.1..=2.0).text("Strength"))
                .changed();
            if radius_changed || strength_changed {
                if let Ok(mut editor) = world.get::<&mut EditorState>(entities.editor) {
                    editor.mesh_brush_radius_mm = brush_radius_mm;
                    editor.mesh_brush_strength = brush_strength;
                }
            }
            ui.horizontal(|ui| {
                if has_mesh_preview {
                    if ui.small_button("Commit").clicked() {
                        if let Ok(mut input) = world.get::<&mut InputState>(entities.input) {
                            input.mesh_move_pending_commit = false;
                        }
                        match roi::commit_mesh_edit_preview(world, entities.editor) {
                            Ok(()) => {
                                handlers::set_status_message(
                                    world,
                                    entities,
                                    "Committed mesh edit; build voxels explicitly when needed."
                                        .to_string(),
                                );
                                ctx.request_repaint();
                            }
                            Err(error) => handlers::set_status_message(
                                world,
                                entities,
                                format!("Mesh commit failed: {error:?}."),
                            ),
                        }
                    }
                    if ui.small_button("Cancel").clicked() {
                        if let Ok(mut input) = world.get::<&mut InputState>(entities.input) {
                            input.mesh_move_pending_commit = false;
                        }
                        match roi::cancel_mesh_edit_preview(world, entities.editor) {
                            Ok(()) => {
                                handlers::set_status_message(
                                    world,
                                    entities,
                                    "Cancelled mesh edit preview.".to_string(),
                                );
                                ctx.request_repaint();
                            }
                            Err(error) => handlers::set_status_message(
                                world,
                                entities,
                                format!("Mesh preview cancel failed: {error:?}."),
                            ),
                        }
                    }
                }
            });
        }

        for (entity, name, mut visible, mut opacity, stats) in layers {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.radio_value(&mut new_active_roi, Some(entity), "");
                    if ui.checkbox(&mut visible, "").changed() {
                        let allow_visibility = if visible {
                            roi_runtime::can_enable_roi_visibility(world, entity)
                        } else {
                            true
                        };

                        if allow_visibility {
                            if let Ok(mut roi) = world.get::<&mut Roi>(entity) {
                                roi.metadata.is_visible = visible;
                            }
                            let _ = event_proxy.send_event(AppEvent::RebuildBindGroups);
                        } else {
                            visible = false;
                            if let Ok(mut roi) = world.get::<&mut Roi>(entity) {
                                roi.metadata.is_visible = false;
                            }
                            handlers::set_status_message(
                                world,
                                entities,
                                "Only two ROI overlays can be visible at once in the current renderer."
                                    .to_string(),
                            );
                        }
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

            let show_contour_point_controls = Some(entity) == new_active_roi
                && world.get::<&Roi>(entity).is_ok_and(|roi| {
                    roi.primary_representation() == PrimaryRepresentation::Contour
                })
                && world
                    .get::<&EditorState>(entities.editor)
                    .is_ok_and(|editor| editor.active_tool == EditorTool::ContourSelect);
            if show_contour_point_controls {
                ui.horizontal(|ui| {
                if ui
                    .small_button("Insert Point")
                    .on_hover_text("Insert a point in the selected loop.")
                    .clicked()
                {
                    let mouse_uv = world
                        .get::<&InputState>(entities.input)
                        .map(|input| input.mouse_uv)
                        .unwrap_or([0.5, 0.5]);
                    match crate::systems::insert_point_into_selected_loop(world, entities, mouse_uv)
                    {
                        Ok(()) => {
                            handlers::set_status_message(
                                world,
                                entities,
                                "Inserted contour point.".to_string(),
                            );
                            ctx.request_repaint();
                        }
                        Err(err) => handlers::set_status_message(
                            world,
                            entities,
                            format!("Insert point failed: {err:?}"),
                        ),
                    }
                }

                if ui
                    .small_button("Delete Selected")
                    .on_hover_text("Delete selected point or loop.")
                    .clicked()
                {
                    match crate::systems::delete_selected_contour_element(world, entities) {
                        Ok(()) => {
                            handlers::set_status_message(
                                world,
                                entities,
                                "Deleted selected contour element.".to_string(),
                            );
                            ctx.request_repaint();
                        }
                        Err(err) => handlers::set_status_message(
                            world,
                            entities,
                            format!("Delete failed: {err:?}"),
                        ),
                    }
                }
                });
            }
        }

        if new_active_roi != active_roi {
            if let Ok(mut input) = world.get::<&mut InputState>(entities.input) {
                input.contour_move_pending_commit = false;
                input.mesh_move_pending_commit = false;
            }
            crate::systems::clear_contour_draft_for_roi_change(
                world,
                entities.editor,
                new_active_roi,
            );
            crate::systems::clear_contour_selection_for_roi_change(
                world,
                entities.editor,
                new_active_roi,
            );
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
        ui.label("RMB: Rotate (3D / Oblique)");
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
