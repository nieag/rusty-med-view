use crate::components::*;
use crate::convert::{
    oblique_plane_from_view_rotation, orthogonal_plane_from_volume_uv, PlaneFamily,
};
use crate::AppEvent;
use hecs::World;
use winit::event_loop::EventLoopProxy;

use crate::app::roi_runtime;
use crate::io::handlers;

fn insert_test_contour_loop(
    world: &mut World,
    entities: &AppEntities,
    roi_entity: hecs::Entity,
) -> Result<(), String> {
    let geometry = {
        let mut query = world.query::<&VolumeData>().with::<&MainVolumeTag>();
        let (_, volume) = query
            .iter()
            .next()
            .ok_or_else(|| "Load a main volume before inserting a test contour.".to_string())?;
        VoxelGeometry {
            dimensions: volume.dimensions,
            spacing: volume.spacing,
            origin: volume.origin,
            orientation: volume.orientation,
        }
    };

    let cursor_uv = world
        .get::<&Transform>(entities.cursor)
        .map(|cursor| cursor.position)
        .unwrap_or([0.5, 0.5, 0.5]);

    let mut contour_data = world
        .get::<&Roi>(roi_entity)
        .map_err(|_| "Active contour ROI is missing.".to_string())?
        .contour_data()
        .cloned()
        .ok_or_else(|| "Active ROI is not contour-primary.".to_string())?;

    let plane = match contour_data.active_plane_family {
        PlaneFamily::Axial | PlaneFamily::Coronal | PlaneFamily::Sagittal => {
            orthogonal_plane_from_volume_uv(contour_data.active_plane_family, cursor_uv, geometry)
                .ok_or_else(|| "Failed to resolve orthogonal contour plane.".to_string())?
        }
        PlaneFamily::Oblique => {
            let active_viewport = world
                .get::<&InputState>(entities.input)
                .ok()
                .and_then(|input| input.active_viewport)
                .ok_or_else(|| {
                    "Activate an oblique viewport before inserting an oblique test contour."
                        .to_string()
                })?;
            let viewport = world
                .get::<&Viewport>(active_viewport)
                .map_err(|_| "Active viewport is missing.".to_string())?;
            if viewport.mode != ViewMode::Oblique {
                return Err(
                    "Switch the active viewport to oblique before inserting oblique test contour."
                        .to_string(),
                );
            }
            let rotation = world
                .get::<&ViewportState>(active_viewport)
                .map_err(|_| "Active viewport state is missing.".to_string())?
                .user_rotation;
            oblique_plane_from_view_rotation(cursor_uv, rotation, geometry)
                .ok_or_else(|| "Failed to resolve oblique contour plane.".to_string())?
        }
    };

    let seeded_loop = ContourLoop {
        points: vec![
            ContourPoint {
                local_mm: [-12.0, -10.0],
            },
            ContourPoint {
                local_mm: [12.0, -10.0],
            },
            ContourPoint {
                local_mm: [0.0, 12.0],
            },
        ],
        is_closed: true,
    };

    contour_data.slices.push(ContourSlice {
        plane,
        loops: vec![seeded_loop],
    });

    roi_runtime::replace_contour_data(world, roi_entity, contour_data).map_err(
        |err| match err {
            roi_runtime::ContourMutationError::MissingRoi => {
                "Active contour ROI is missing.".to_string()
            }
            roi_runtime::ContourMutationError::NotContourRoi => {
                "Active ROI is not contour-primary.".to_string()
            }
        },
    )?;

    Ok(())
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

        ui.horizontal(|ui| {
            ui.label("Create contour ROI:");
            for (label, family) in [
                ("Axial", PlaneFamily::Axial),
                ("Coronal", PlaneFamily::Coronal),
                ("Sagittal", PlaneFamily::Sagittal),
                ("Oblique", PlaneFamily::Oblique),
            ] {
                if ui.small_button(label).clicked() {
                    match roi_runtime::create_empty_contour_roi(world, entities.editor, family) {
                        Ok(entity) => {
                            new_active_roi = Some(entity);
                            handlers::set_status_message(
                                world,
                                entities,
                                format!("Created contour ROI ({family:?})"),
                            );
                            let _ = event_proxy.send_event(AppEvent::RebuildBindGroups);
                        }
                        Err(err) => {
                            handlers::set_status_message(world, entities, err);
                        }
                    }
                }
            }
        });

        let mut active_contour_plane_family = new_active_roi.and_then(|entity| {
            world
                .get::<&Roi>(entity)
                .ok()
                .and_then(|roi| roi.contour_data().map(|contour| contour.active_plane_family))
        });

        if let (Some(entity), Some(current_family)) = (new_active_roi, active_contour_plane_family) {
            ui.horizontal(|ui| {
                ui.label("Contour plane family:");
                egui::ComboBox::from_id_salt("contour_plane_family")
                    .selected_text(format!("{current_family:?}"))
                    .show_ui(ui, |ui| {
                        for family in [
                            PlaneFamily::Axial,
                            PlaneFamily::Coronal,
                            PlaneFamily::Sagittal,
                            PlaneFamily::Oblique,
                        ] {
                            ui.selectable_value(
                                &mut active_contour_plane_family,
                                Some(family),
                                format!("{family:?}"),
                            );
                        }
                    });
            });

            if active_contour_plane_family != Some(current_family) {
                if let Some(new_family) = active_contour_plane_family {
                    if let Err(err) =
                        roi_runtime::set_active_contour_plane_family(world, entity, new_family)
                    {
                        let message = match err {
                            roi_runtime::ContourPlaneFamilySwitchError::MissingRoi => {
                                "Cannot set contour plane family: active ROI is missing.".to_string()
                            }
                            roi_runtime::ContourPlaneFamilySwitchError::NotContourRoi => {
                                "Cannot set contour plane family: active ROI is not contour-primary."
                                    .to_string()
                            }
                            roi_runtime::ContourPlaneFamilySwitchError::RequiresConversion => {
                                "Cannot switch contour plane family when contour loops already exist."
                                    .to_string()
                            }
                        };
                        handlers::set_status_message(world, entities, message);
                    }
                }
            }
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

            if ui
                .small_button("Insert test contour (dev)")
                .on_hover_text("Temporary helper: seed a closed contour loop on the active contour plane.")
                .clicked()
            {
                match insert_test_contour_loop(world, entities, entity) {
                    Ok(()) => {
                        handlers::set_status_message(
                            world,
                            entities,
                            "Inserted test contour loop for active ROI.".to_string(),
                        );
                        ctx.request_repaint();
                    }
                    Err(message) => handlers::set_status_message(world, entities, message),
                }
            }
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
