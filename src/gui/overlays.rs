use crate::components::*;
use crate::overlay::OverlayManager;
use crate::AppEvent;
use hecs::{Entity, World};
use winit::event_loop::EventLoopProxy;

pub fn draw_viewport_overlays(
    ctx: &egui::Context,
    world: &mut World,
    entities: &AppEntities,
    event_proxy: &EventLoopProxy<AppEvent>,
    central_rect: egui::Rect,
    vps: &[(Entity, ViewMode, egui::Rect)],
    active_viewport_entity: Option<Entity>,
    volume_info: Option<[u32; 3]>,
) {
    let mut cursor_pos = [0.0, 0.0, 0.0];
    if let Ok(t) = world.get::<&Transform>(entities.cursor) {
        cursor_pos = t.position;
    }

    let x0 = central_rect.min.x;
    let y0 = central_rect.min.y;
    let hw = central_rect.width() / 2.0;
    let hh = central_rect.height() / 2.0;

    let mut gizmo_rotation = [0.0f32, 0.0, 0.0, 1.0];
    let mut data_orientation = [0.0f32, 0.0, 0.0, 1.0];
    for (_, vol) in world.query::<&VolumeData>().with::<&MainVolumeTag>().iter() {
        data_orientation = vol.orientation;
    }

    for (_, (vp, vs)) in world.query::<(&Viewport, &ViewportState)>().iter() {
        if vp.mode == ViewMode::ThreeD {
            gizmo_rotation =
                crate::util::orientation::compose_view_rotation(data_orientation, vs.user_rotation);
        }
    }

    // --- Viewport Separation Lines ---
    let active_protocol = {
        world
            .get::<&ProtocolState>(entities.protocol)
            .map(|p| p.active_protocol.clone())
            .unwrap_or_else(|_| "Standard 2x2".to_string())
    };

    if active_protocol == "Standard 2x2" {
        let painter = ctx.layer_painter(egui::LayerId::background());
        let border_color = egui::Color32::from_gray(60);
        painter.line_segment(
            [
                egui::pos2(x0, y0 + hh),
                egui::pos2(x0 + central_rect.width(), y0 + hh),
            ],
            (2.0, border_color),
        );
        painter.line_segment(
            [
                egui::pos2(x0 + hw, y0),
                egui::pos2(x0 + hw, y0 + central_rect.height()),
            ],
            (2.0, border_color),
        );
    } else if active_protocol == "Clinical Triple" {
        let painter = ctx.layer_painter(egui::LayerId::background());
        let border_color = egui::Color32::from_gray(60);
        painter.line_segment(
            [
                egui::pos2(x0 + hw, y0),
                egui::pos2(x0 + hw, y0 + central_rect.height()),
            ],
            (2.0, border_color),
        );
        painter.line_segment(
            [
                egui::pos2(x0 + hw, y0 + hh),
                egui::pos2(x0 + central_rect.width(), y0 + hh),
            ],
            (2.0, border_color),
        );
    }

    let vol_dims = volume_info.unwrap_or([0, 0, 0]);

    // 1. Draw Viewport Labels and Gizmos
    for (e, mode, rect) in vps.iter() {
        let rx0 = rect.min.x;
        let ry0 = rect.min.y;
        let rhw = rect.width() / 2.0;
        let rhh = rect.height() / 2.0;
        let is_active = Some(*e) == active_viewport_entity;

        let mut label_res: Option<egui::Response> = None;

        egui::Area::new(egui::Id::new("overlay").with(e))
            .fixed_pos([rx0 + 10.0, ry0 + 10.0])
            .interactable(true)
            .show(ctx, |ui| {
                if mode == &ViewMode::ThreeD {
                    label_res = Some(draw_label(ui, "3D View", is_active));
                    if let Ok(w) = world.get::<&VolumeWindowing>(entities.volume_windowing) {
                        ui.label(format!("W/L: {:.0} / {:.0}", w.width, w.center));
                    }
                } else if mode == &ViewMode::Axial {
                    let slice_z = (cursor_pos[2] * vol_dims[2] as f32).floor() as u32;
                    label_res = Some(draw_label(ui, "Axial (Top)", is_active));
                    ui.label(format!("Slice: {} / {}", slice_z, vol_dims[2]));
                    if let Ok(w) = world.get::<&VolumeWindowing>(entities.volume_windowing) {
                        ui.label(format!("W/L: {:.0} / {:.0}", w.width, w.center));
                    }

                    let labels = crate::util::orientation::get_viewport_anatomical_labels(*mode);
                    marker(ui, labels[2], egui::pos2(rx0 + rhw, ry0 + 15.0)); // Top
                    marker(
                        ui,
                        labels[3],
                        egui::pos2(rx0 + rhw, ry0 + rect.height() - 15.0),
                    ); // Bottom
                    marker(ui, labels[0], egui::pos2(rx0 + 15.0, ry0 + rhh)); // Left
                    marker(
                        ui,
                        labels[1],
                        egui::pos2(rx0 + rect.width() - 15.0, ry0 + rhh),
                    ); // Right
                } else if mode == &ViewMode::Coronal {
                    let slice_y = (cursor_pos[1] * vol_dims[1] as f32).floor() as u32;
                    label_res = Some(draw_label(ui, "Coronal (Front)", is_active));
                    ui.label(format!("Slice: {} / {}", slice_y, vol_dims[1]));
                    if let Ok(w) = world.get::<&VolumeWindowing>(entities.volume_windowing) {
                        ui.label(format!("W/L: {:.0} / {:.0}", w.width, w.center));
                    }
                    let labels = crate::util::orientation::get_viewport_anatomical_labels(*mode);
                    marker(ui, labels[2], egui::pos2(rx0 + rhw, ry0 + 15.0));
                    marker(
                        ui,
                        labels[3],
                        egui::pos2(rx0 + rhw, ry0 + rect.height() - 15.0),
                    );
                    marker(ui, labels[0], egui::pos2(rx0 + 15.0, ry0 + rhh));
                    marker(
                        ui,
                        labels[1],
                        egui::pos2(rx0 + rect.width() - 15.0, ry0 + rhh),
                    );
                } else if mode == &ViewMode::Sagittal {
                    let slice_x = (cursor_pos[0] * vol_dims[0] as f32).floor() as u32;
                    label_res = Some(draw_label(ui, "Sagittal (Side)", is_active));
                    ui.label(format!("Slice: {} / {}", slice_x, vol_dims[0]));
                    if let Ok(w) = world.get::<&VolumeWindowing>(entities.volume_windowing) {
                        ui.label(format!("W/L: {:.0} / {:.0}", w.width, w.center));
                    }
                    let labels = crate::util::orientation::get_viewport_anatomical_labels(*mode);
                    marker(ui, labels[2], egui::pos2(rx0 + rhw, ry0 + 15.0));
                    marker(
                        ui,
                        labels[3],
                        egui::pos2(rx0 + rhw, ry0 + rect.height() - 15.0),
                    );
                    marker(ui, labels[0], egui::pos2(rx0 + 15.0, ry0 + rhh));
                    marker(
                        ui,
                        labels[1],
                        egui::pos2(rx0 + rect.width() - 15.0, ry0 + rhh),
                    );
                }
            });

        if let Some(res) = label_res {
            if res.double_clicked() {
                let _ = event_proxy.send_event(AppEvent::ToggleMaximize(*e));
                ctx.request_repaint();
            }
            let dnd_id = egui::Id::new("viewport_dnd");
            if res.drag_started() {
                ctx.memory_mut(|mem| mem.data.insert_temp(dnd_id, *e));
            }
            if res.drag_stopped() {
                if let Some(source_e) = ctx.memory(|mem| mem.data.get_temp::<Entity>(dnd_id)) {
                    let drop_pos = ctx.input(|i| i.pointer.interact_pos());
                    if let Some(pos) = drop_pos {
                        for (target_e, _, target_rect) in vps.iter() {
                            if target_rect.contains(pos) && target_e != &source_e {
                                let _ = event_proxy
                                    .send_event(AppEvent::SwapViewports(source_e, *target_e));
                                ctx.request_repaint();
                                break;
                            }
                        }
                    }
                }
            }
        }

        if mode == &ViewMode::ThreeD {
            let gizmo_rect = egui::Rect::from_center_size(
                egui::pos2(rx0 + 80.0, ry0 + rect.height() - 80.0),
                egui::vec2(120.0, 120.0),
            );
            egui::Area::new("gizmo_3d".into())
                .fixed_pos(gizmo_rect.min)
                .interactable(false)
                .show(ctx, |ui| {
                    let view_quat = super::gizmo::quat_from_array(gizmo_rotation);
                    super::gizmo::draw_gizmo(ui, gizmo_rect, view_quat);
                });
        }
    }

    // 2. Draw Annotation Markers and Segmentation Contours (layered)
    egui::Area::new("annotations_layer".into())
        .fixed_pos(central_rect.min)
        .interactable(true)
        .show(ctx, |ui| {
            let mut vd_query = world.query::<&VolumeData>().with::<&MainVolumeTag>();
            let vol_data = vd_query.iter().next().map(|(_, vd)| vd);

            if let (Ok(mut state), Ok(mut overlay), Some(vd)) = (
                world.get::<&mut AnnotationState>(entities.annotations),
                world.get::<&mut OverlayManager>(entities.overlay),
                vol_data,
            ) {
                let focused_id = state.focused_id;
                let items = &mut state.annotations;

                let cursor_vec = glam::Vec3::from(cursor_pos);

                let mut clicked_id = None;
                for (e, mode, rect) in vps {
                    if let Ok(vs) = world.get::<&ViewportState>(*e) {
                        // --- Draw Annotations ---
                        if let Some(id) = draw_annotations(
                            ui,
                            items,
                            &vs,
                            vd,
                            data_orientation,
                            *rect,
                            *mode,
                            &mut overlay,
                            focused_id,
                            cursor_vec,
                        ) {
                            clicked_id = Some(id);
                        }

                        // --- Draw Segmentation Contours ---
                        if mode != &ViewMode::ThreeD {
                            let mut seg_query = world.query::<&Segmentation>();
                            for (_, seg) in seg_query.iter() {
                                if !seg.is_visible {
                                    continue;
                                }

                                let current_slice = match mode {
                                    ViewMode::Axial => {
                                        (cursor_pos[2] * vol_dims[2] as f32).floor() as i32
                                    }
                                    ViewMode::Coronal => {
                                        (cursor_pos[1] * vol_dims[1] as f32).floor() as i32
                                    }
                                    ViewMode::Sagittal => {
                                        (cursor_pos[0] * vol_dims[0] as f32).floor() as i32
                                    }
                                    _ => 0,
                                };

                                if let Some(slice_contours) =
                                    seg.contour_set.get_slice(*mode, current_slice)
                                {
                                    draw_contours_2d(
                                        ui.painter(),
                                        slice_contours,
                                        &vs,
                                        *rect,
                                        vol_dims,
                                        vd.spacing,
                                        data_orientation,
                                        *mode,
                                    );
                                }
                            }
                        }
                    }
                }
                if let Some(id) = clicked_id {
                    let _ = event_proxy.send_event(AppEvent::FocusAnnotation(id));
                }
            }
        });
}

fn draw_label(ui: &mut egui::Ui, text: &str, is_active: bool) -> egui::Response {
    egui::Frame::new()
        .fill(egui::Color32::from_black_alpha(180))
        .corner_radius(4.0)
        .inner_margin(4.0)
        .show(ui, |ui| {
            ui.add(
                egui::Button::new(
                    egui::RichText::new(text)
                        .color(if is_active {
                            egui::Color32::GREEN
                        } else {
                            egui::Color32::WHITE
                        })
                        .size(14.0)
                        .strong(),
                )
                .fill(egui::Color32::TRANSPARENT)
                .sense(egui::Sense::click_and_drag()),
            )
        })
        .inner
}

fn marker(ui: &mut egui::Ui, text: &str, pos: egui::Pos2) {
    ui.painter().text(
        pos,
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(16.0),
        egui::Color32::from_gray(200),
    );
}

fn draw_annotations(
    ui: &mut egui::Ui,
    annotations: &mut [Annotation],
    view: &ViewportState,
    vol: &VolumeData,
    data_orientation: [f32; 4],
    rect: egui::Rect,
    mode: ViewMode,
    overlay: &mut OverlayManager,
    focused_id: Option<uuid::Uuid>,
    cursor_pos: glam::Vec3,
) -> Option<uuid::Uuid> {
    let aspect_ratios = vol.aspect_ratios();

    if vol.dimensions[0] == 0 {
        return None;
    }

    let mut clicked_id = None;

    let viewport_idx = match mode {
        ViewMode::ThreeD => 0,
        ViewMode::Axial => 1,
        ViewMode::Coronal => 2,
        ViewMode::Sagittal => 3,
    };

    for (idx, ann) in annotations.iter_mut().enumerate() {
        if let Some(plane) = crate::util::orientation::SlicePlane::from_mode(mode) {
            let axis = plane.depth_axis(data_orientation);
            let ann_depth = ann.world_pos[axis];
            let current_depth = cursor_pos[axis];

            if (ann_depth - current_depth).abs() > 0.005 {
                continue;
            }
        }

        if let Some(screen_pos) = world_to_screen(
            ann.world_pos,
            viewport_idx,
            view.zoom,
            view.pan,
            view.pivot,
            view.user_rotation,
            data_orientation,
            aspect_ratios,
            rect,
        ) {
            let sense = if viewport_idx > 0 {
                egui::Sense::click_and_drag()
            } else {
                egui::Sense::click()
            };
            let id = ui.make_persistent_id(format!("ann_{}_{}", viewport_idx, ann.id));

            let point_rect = egui::Rect::from_center_size(screen_pos, egui::vec2(24.0, 24.0));
            let response = ui.interact(point_rect, id, sense);

            if response.clicked() {
                clicked_id = Some(ann.id);
            }

            if viewport_idx > 0 && response.dragged() {
                overlay.dragging_idx = Some(idx);
                overlay.dragging_viewport = viewport_idx as u32;

                if let Some(mouse_pos) = ui.ctx().pointer_latest_pos() {
                    let zoom = view.zoom;
                    let pan = view.pan;
                    let pivot = view.pivot;

                    let screen_w = rect.width();
                    let screen_h = rect.height();
                    let screen_aspect = if screen_h > 0.0 {
                        screen_w / screen_h
                    } else {
                        1.0
                    };

                    if let Some(plane) =
                        crate::util::orientation::SlicePlane::from_viewport(viewport_idx as u32)
                    {
                        let slice_aspect = plane.slice_aspect(aspect_ratios, data_orientation);
                        let k = screen_aspect / slice_aspect;

                        let ndc_x = (mouse_pos.x - rect.min.x) / rect.width();
                        let ndc_y = (mouse_pos.y - rect.min.y) / rect.height();

                        overlay.mouse_screen_uv = [ndc_x, ndc_y];

                        let world_u = ((ndc_x - pivot[0]) * k / zoom) + pivot[0] + pan[0];
                        let world_v = ((ndc_y - pivot[1]) / zoom) + pivot[1] + pan[1];

                        let depth_axis = plane.depth_axis(data_orientation);
                        let vol_pos = plane.screen_uv_to_volume(
                            [world_u, world_v],
                            ann.world_pos[depth_axis],
                            data_orientation,
                        );
                        ann.world_pos = glam::Vec3::from(vol_pos);
                    }

                    ann.world_pos = ann.world_pos.clamp(glam::Vec3::ZERO, glam::Vec3::ONE);
                }
            } else if response.drag_stopped() {
                overlay.dragging_idx = None;
            }

            let draw_pos = if response.dragged() {
                ui.ctx().pointer_latest_pos().unwrap_or(screen_pos)
            } else {
                world_to_screen(
                    ann.world_pos,
                    viewport_idx,
                    view.zoom,
                    view.pan,
                    view.pivot,
                    view.user_rotation,
                    data_orientation,
                    aspect_ratios,
                    rect,
                )
                .unwrap_or(screen_pos)
            };

            let is_focused = focused_id == Some(ann.id);
            let is_hovered = response.hovered();

            ui.painter().circle_stroke(
                draw_pos,
                if is_focused {
                    8.0
                } else if is_hovered {
                    6.0
                } else {
                    4.0
                },
                egui::Stroke::new(
                    if is_focused {
                        3.0
                    } else if is_hovered {
                        2.5
                    } else {
                        2.0
                    },
                    if is_focused {
                        egui::Color32::from_rgb(255, 100, 100)
                    } else if is_hovered {
                        egui::Color32::from_rgb(255, 200, 200)
                    } else {
                        egui::Color32::WHITE
                    },
                ),
            );

            ui.painter().text(
                draw_pos + egui::vec2(8.0, -8.0),
                egui::Align2::LEFT_BOTTOM,
                &ann.label,
                egui::FontId::proportional(if is_focused {
                    16.0
                } else if is_hovered {
                    15.0
                } else {
                    14.0
                }),
                if is_focused {
                    egui::Color32::from_rgb(255, 100, 100)
                } else if is_hovered {
                    egui::Color32::from_rgb(255, 200, 200)
                } else {
                    egui::Color32::WHITE
                },
            );
        }
    }
    clicked_id
}

fn world_to_screen(
    pos: glam::Vec3,
    viewport_idx: usize,
    zoom: f32,
    pan: [f32; 2],
    pivot: [f32; 2],
    rotation: [f32; 4],
    data_orientation: [f32; 4],
    aspect_ratios: [f32; 3],
    rect: egui::Rect,
) -> Option<egui::Pos2> {
    let screen_aspect = if rect.height() > 0.0 {
        rect.width() / rect.height()
    } else {
        1.0
    };

    if let Some([ndc_x, ndc_y]) = crate::render::geometry::world_to_ndc(
        pos,
        viewport_idx,
        zoom,
        pan,
        pivot,
        rotation,
        data_orientation,
        aspect_ratios,
        screen_aspect,
    ) {
        if (!(0.0..=1.0).contains(&ndc_x) || !(0.0..=1.0).contains(&ndc_y)) && viewport_idx > 0 {
            return None;
        }

        Some(egui::Pos2::new(
            rect.min.x + ndc_x * rect.width(),
            rect.min.y + ndc_y * rect.height(),
        ))
    } else {
        None
    }
}

fn draw_contours_2d(
    painter: &egui::Painter,
    slice: &crate::segmentation::SliceContours,
    view: &ViewportState,
    rect: egui::Rect,
    vol_dims: [u32; 3],
    vol_spacing: [f32; 3],
    data_orientation: [f32; 4],
    mode: ViewMode,
) {
    // TODO: This function couples the segmentation rendering to the egui UI layer (epaint).
    // In a future Level 3 architecture, we should move this to a custom wgpu pipeline
    // using a neutral tessellator (like lyon) to support UI-agnostic rendering and
    // better 3D depth integration.

    let zoom = view.zoom;
    let pan = view.pan;
    let pivot = view.pivot;

    let plane = if let Some(p) = crate::util::orientation::SlicePlane::from_mode(mode) {
        p
    } else {
        return;
    };

    let screen_w = rect.width();
    let screen_h = rect.height();
    let screen_aspect = if screen_h > 0.0 {
        screen_w / screen_h
    } else {
        1.0
    };

    let vol_aspects = [
        vol_dims[0] as f32 * vol_spacing[0],
        vol_dims[1] as f32 * vol_spacing[1],
        vol_dims[2] as f32 * vol_spacing[2],
    ];

    let slice_aspect = plane.slice_aspect(vol_aspects, data_orientation);
    let k = screen_aspect / slice_aspect;

    for contour in &slice.contours {
        let mut points = Vec::new();
        for p in &contour.points {
            // Map 2D slice UV to 3D volume UV for the projection API
            let vol_uv = match plane {
                crate::util::orientation::SlicePlane::Axial => [p.x, p.y, 0.0],
                crate::util::orientation::SlicePlane::Coronal => [p.x, 0.0, p.y],
                crate::util::orientation::SlicePlane::Sagittal => [0.0, p.x, p.y],
            };

            let [ndc_u, ndc_v] = plane.volume_to_screen_uv(vol_uv, data_orientation);

            let ndc_x = (ndc_u - pivot[0] - pan[0]) * zoom / k + pivot[0];
            let ndc_y = (ndc_v - pivot[1] - pan[1]) * zoom + pivot[1];

            points.push(egui::pos2(
                rect.min.x + ndc_x * rect.width(),
                rect.min.y + ndc_y * rect.height(),
            ));
        }

        if points.len() > 1 {
            let stroke = egui::Stroke::new(1.5, egui::Color32::from_rgb(0, 255, 255));

            if contour.is_closed {
                let first = points[0];

                // Manually draw to avoid any egui closed_line weirdness
                let mut p_loop = points.clone();
                p_loop.push(first);
                painter.add(egui::Shape::line(p_loop, stroke));
            } else {
                painter.add(egui::Shape::line(points, stroke));
            }
        }
    }
}
