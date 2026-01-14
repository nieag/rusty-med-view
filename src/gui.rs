use crate::components::*;
use crate::overlay::OverlayManager;
use crate::{file_dialog, nifti_loader, systems, AppEvent};
use egui_wgpu::Renderer;
use egui_winit::State;
use hecs::World;
use wgpu::{Device, TextureFormat};
use winit::event_loop::EventLoopProxy;
use winit::window::Window as WinitWindow;

pub struct Gui {
    pub context: egui::Context,
    state: State,
    renderer: Renderer,
    // We must store the output frame between 'prepare' and 'render'
    output: Option<egui::FullOutput>,
}

impl Gui {
    pub fn new(device: &Device, format: TextureFormat, window: &WinitWindow) -> Self {
        let context = egui::Context::default();

        let state = State::new(
            context.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );

        let renderer = Renderer::new(device, format, egui_wgpu::RendererOptions::default());

        Self {
            context,
            state,
            renderer,
            output: None,
        }
    }

    pub fn handle_event(
        &mut self,
        window: &WinitWindow,
        event: &winit::event::WindowEvent,
    ) -> bool {
        let response = self.state.on_window_event(window, event);
        response.consumed
    }

    pub fn prepare(
        &mut self,
        window: &WinitWindow,
        world: &mut World,
        entities: &AppEntities,
        event_proxy: EventLoopProxy<AppEvent>,
    ) {
        let raw_input = self.state.take_egui_input(window);

        let full_output = self.context.run(raw_input, |ctx| {
            // 1. Data Collection from ECS via AppEntities
            let (active_viewport_entity, status_msg, volume_info, windowing_active) = {
                let active_viewport_entity = world
                    .get::<&InputState>(entities.input)
                    .map(|i| i.active_viewport)
                    .unwrap_or(None);
                let status_msg = world
                    .get::<&GuiState>(entities.gui_state)
                    .map(|g| g.status_message.clone())
                    .unwrap_or(None);
                let (volume_info, windowing_active) = {
                    let mut query = world.query::<&VolumeData>().with::<&MainVolumeTag>();
                    query
                        .iter()
                        .next()
                        .map(|(_, vd)| {
                            let dims = if vd.dimensions == [0, 0, 0] {
                                None
                            } else {
                                Some(vd.dimensions)
                            };
                            (dims, dims.is_some())
                        })
                        .unwrap_or((None, false))
                };
                (
                    active_viewport_entity,
                    status_msg,
                    volume_info,
                    windowing_active,
                )
            };

            // 1.5 Top Toolbar
            egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
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
                                let load_result = nifti_loader::load_nifti_from_bytes(&data)
                                    .map(LoadResult::Volume);
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
                                    nifti_loader::load_label_from_bytes(&data, filename)
                                        .map(LoadResult::Label);
                                let _ = proxy.send_event(AppEvent::VolumeLoaded(load_result));
                            }
                        });
                    }

                    ui.separator();

                    // --- Tool Selection ---
                    if let Ok(mut editor) = world.get::<&mut EditorState>(entities.editor) {
                        ui.label("Tool:");
                        ui.radio_value(&mut editor.active_tool, EditorTool::Navigation, "🖱 Nav");
                        ui.radio_value(&mut editor.active_tool, EditorTool::Brush, "🖌 Brush");
                        ui.radio_value(&mut editor.active_tool, EditorTool::Eraser, "⌫ Erase");

                        if editor.active_layer.is_none()
                            && editor.active_tool != EditorTool::Navigation
                        {
                            editor.active_tool = EditorTool::Navigation;
                        }
                    }

                    ui.separator();

                    // --- Presets (Quick Access) ---
                    if windowing_active {
                        if let Ok(mut windowing) =
                            world.get::<&mut VolumeWindowing>(entities.volume_windowing)
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

                    if let Some(msg) = status_msg {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(msg).color(egui::Color32::LIGHT_BLUE));
                        });
                    }
                });
            });

            // 2. Sidebar Implementation (Metadata & Detailed Controls)
            egui::SidePanel::left("left_panel")
                .resizable(true)
                .default_width(220.0)
                .show(ctx, |ui| {
                    ui.add_space(8.0);

                    ui.collapsing("📁 Protocol", |ui| {
                        if let Ok(proto) = world.get::<&ProtocolState>(entities.protocol) {
                            let mut selected = proto.active_protocol.clone();
                            let registry = crate::protocols::get_protocol_registry();
                            egui::ComboBox::from_label("Active Protocol")
                                .selected_text(&selected)
                                .show_ui(ui, |ui| {
                                    for p in registry {
                                        if ui
                                            .selectable_value(
                                                &mut selected,
                                                p.name.clone(),
                                                &p.name,
                                            )
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
                        if let Ok(mut windowing) =
                            world.get::<&mut VolumeWindowing>(entities.volume_windowing)
                        {
                            ui.label("Center (HU)");
                            ui.add(
                                egui::Slider::new(&mut windowing.center, -1024.0..=3071.0)
                                    .show_value(true),
                            );

                            ui.label("Width (HU)");
                            ui.add(
                                egui::Slider::new(&mut windowing.width, 1.0..=4096.0)
                                    .show_value(true),
                            );

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

                    // --- Toolbox (Brush Settings) ---
                    if let Ok(editor) = world.get::<&EditorState>(entities.editor) {
                        if editor.active_tool != EditorTool::Navigation {
                            ui.collapsing("🖌 Brush Settings", |ui| {
                                let mut editor =
                                    world.get::<&mut EditorState>(entities.editor).unwrap();
                                ui.label(format!("Size: {:.1} px", editor.brush_size));
                                ui.add(egui::Slider::new(&mut editor.brush_size, 1.0..=20.0));

                                ui.label("Label ID");
                                ui.add(egui::Slider::new(&mut editor.active_label_index, 1..=10));
                            });
                            ui.separator();
                        }
                    }

                    // --- Layer Control ---
                    ui.collapsing("📚 Layers", |ui| {
                        let mut layers: Vec<(hecs::Entity, String, bool, f32)> = Vec::new();
                        for (e, (seg, settings)) in
                            world.query::<(&Segmentation, &LayerSettings)>().iter()
                        {
                            layers.push((e, seg.name.clone(), seg.is_visible, settings.opacity));
                        }

                        if ui.button("➕ Create New Layer").clicked() {
                            let _ = event_proxy.send_event(AppEvent::CreateNewLayer);
                        }

                        let active_layer = world
                            .get::<&EditorState>(entities.editor)
                            .ok()
                            .and_then(|e| e.active_layer);
                        let mut new_active_layer = active_layer;

                        for (entity, name, mut visible, mut opacity) in layers {
                            ui.group(|ui| {
                                ui.horizontal(|ui| {
                                    ui.radio_value(&mut new_active_layer, Some(entity), "");
                                    if ui.checkbox(&mut visible, "").changed() {
                                        if let Ok(mut seg) = world.get::<&mut Segmentation>(entity)
                                        {
                                            seg.is_visible = visible;
                                        }
                                    }
                                    ui.label(name);
                                });
                                if visible
                                    && ui
                                        .add(
                                            egui::Slider::new(&mut opacity, 0.0..=1.0)
                                                .text("Opacity"),
                                        )
                                        .changed()
                                {
                                    if let Ok(mut set) = world.get::<&mut LayerSettings>(entity) {
                                        set.opacity = opacity;
                                    }
                                }
                            });
                        }

                        if new_active_layer != active_layer {
                            if let Ok(mut editor) = world.get::<&mut EditorState>(entities.editor) {
                                editor.active_layer = new_active_layer;
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

                            if let Ok(mut state) =
                                world.get::<&mut AnnotationState>(entities.annotations)
                            {
                                state.annotations.push(Annotation {
                                    world_pos: current_pos,
                                    label: "New".to_string(),
                                });
                            }
                        }

                        ui.separator();

                        let mut to_delete = None;
                        let mut to_locate = None;

                        {
                            if let Ok(mut state) =
                                world.get::<&mut AnnotationState>(entities.annotations)
                            {
                                for (i, ann) in state.annotations.iter_mut().enumerate() {
                                    ui.horizontal(|ui| {
                                        if ui.button("🎯").on_hover_text("Locate").clicked() {
                                            to_locate = Some(ann.world_pos);
                                        }
                                        ui.text_edit_singleline(&mut ann.label);
                                        if ui.button("🗑").clicked() {
                                            to_delete = Some(i);
                                        }
                                    });
                                }

                                if let Some(idx) = to_delete {
                                    state.annotations.remove(idx);
                                }
                            }
                        }

                        if let Some(pos) = to_locate {
                            if let Ok(mut t) = world.get::<&mut Transform>(entities.cursor) {
                                t.position = pos.into();
                            }
                            for (_, (vp, vs)) in
                                world.query_mut::<(&Viewport, &mut ViewportState)>()
                            {
                                vs.pan = match vp.mode {
                                    ViewMode::Axial => [pos.x - 0.5, pos.y - 0.5],
                                    ViewMode::Coronal => [pos.x - 0.5, pos.z - 0.5],
                                    ViewMode::Sagittal => [pos.y - 0.5, pos.z - 0.5],
                                    _ => vs.pan,
                                };
                            }
                        }
                    });

                    ui.separator();
                    ui.collapsing("⌨ Controls", |ui| {
                        ui.label("LMB: Crosshair / Paint");
                        ui.label("MMB: Pan");
                        ui.label("RMB: Rotate (3D)");
                        ui.label("Scroll: Zoom / Slice");
                        ui.label("Ctrl+Scroll: 2D Zoom");
                    });

                    ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                        ui.separator();
                        if let Some(hu) = systems::get_hu_at_mouse(world, entities) {
                            ui.label(format!("HU at cursor: {:.0}", hu));
                        } else {
                            ui.label("HU at cursor: --");
                        }
                    });
                });

            // 3. Viewport Rect Calculation (Data-Driven by ViewportLayout)
            let central_rect = ctx.available_rect();
            let pixels_per_point = ctx.pixels_per_point();

            let x0 = central_rect.min.x;
            let y0 = central_rect.min.y;
            let cw = central_rect.width();
            let ch = central_rect.height();

            let mut vps = Vec::new();
            for (e, (vp, layout, _)) in
                world.query_mut::<(&mut Viewport, &ViewportLayout, &ViewportState)>()
            {
                let rel = layout.relative_rect;
                let rect = egui::Rect::from_min_size(
                    egui::pos2(x0 + rel[0] * cw, y0 + rel[1] * ch),
                    egui::vec2(rel[2] * cw, rel[3] * ch),
                );

                vp.rect = [
                    rect.min.x * pixels_per_point,
                    rect.min.y * pixels_per_point,
                    rect.width() * pixels_per_point,
                    rect.height() * pixels_per_point,
                ];
                vps.push((e, vp.mode, rect));
            }

            for (_, settings) in world.query_mut::<&mut WindowSettings>() {
                settings.viewport_rect = [
                    x0 * pixels_per_point,
                    y0 * pixels_per_point,
                    central_rect.width() * pixels_per_point,
                    central_rect.height() * pixels_per_point,
                ];
            }

            // 4. Overlays
            let mut cursor_pos = [0.0, 0.0, 0.0];
            if let Ok(t) = world.get::<&Transform>(entities.cursor) {
                cursor_pos = t.position;
            }

            let draw_label = |ui: &mut egui::Ui, text: &str, is_active: bool| -> egui::Response {
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
            };

            let hw = central_rect.width() / 2.0;
            let hh = central_rect.height() / 2.0;
            let x0 = central_rect.min.x;
            let y0 = central_rect.min.y;

            let mut gizmo_rotation = [0.0f32, 0.0, 0.0, 1.0];
            for (_, (vp, vs)) in world.query::<(&Viewport, &ViewportState)>().iter() {
                if vp.mode == ViewMode::ThreeD {
                    gizmo_rotation = vs.rotation;
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
                // Vertical line at 50% width
                painter.line_segment(
                    [
                        egui::pos2(x0 + hw, y0),
                        egui::pos2(x0 + hw, y0 + central_rect.height()),
                    ],
                    (2.0, border_color),
                );
                // Horizontal line on the right half at 50% height
                painter.line_segment(
                    [
                        egui::pos2(x0 + hw, y0 + hh),
                        egui::pos2(x0 + central_rect.width(), y0 + hh),
                    ],
                    (2.0, border_color),
                );
            }

            let vol_dims = volume_info.unwrap_or([0, 0, 0]);

            // Anatomical markers helper
            let marker = |ui: &mut egui::Ui, text: &str, pos: egui::Pos2| {
                ui.painter().text(
                    pos,
                    egui::Align2::CENTER_CENTER,
                    text,
                    egui::FontId::proportional(16.0),
                    egui::Color32::from_gray(200),
                );
            };

            for (e, mode, rect) in vps.clone() {
                let rx0 = rect.min.x;
                let ry0 = rect.min.y;
                let rhw = rect.width() / 2.0;
                let rhh = rect.height() / 2.0;
                let is_active = Some(e) == active_viewport_entity;

                let mut label_res: Option<egui::Response> = None;

                egui::Area::new(egui::Id::new("overlay").with(e))
                    .fixed_pos([rx0 + 10.0, ry0 + 10.0])
                    .interactable(true)
                    .show(ctx, |ui| match mode {
                        ViewMode::ThreeD => {
                            label_res = Some(draw_label(ui, "3D View", is_active));
                            if let Ok(w) = world.get::<&VolumeWindowing>(entities.volume_windowing)
                            {
                                ui.label(format!("W/L: {:.0} / {:.0}", w.width, w.center));
                            }
                        }
                        ViewMode::Axial => {
                            let slice_z = (cursor_pos[2] * vol_dims[2] as f32).round() as u32;
                            label_res = Some(draw_label(ui, "Axial (Top)", is_active));
                            if let Some((curr, total)) = Some((slice_z, vol_dims[2])) {
                                ui.label(format!("Slice: {} / {}", curr, total));
                            }
                            if let Ok(w) = world.get::<&VolumeWindowing>(entities.volume_windowing)
                            {
                                ui.label(format!("W/L: {:.0} / {:.0}", w.width, w.center));
                            }

                            marker(ui, "A", egui::pos2(rx0 + rhw, ry0 + 15.0));
                            marker(ui, "P", egui::pos2(rx0 + rhw, ry0 + rect.height() - 15.0));
                            marker(ui, "R", egui::pos2(rx0 + 15.0, ry0 + rhh));
                            marker(ui, "L", egui::pos2(rx0 + rect.width() - 15.0, ry0 + rhh));
                        }
                        ViewMode::Coronal => {
                            let slice_y = (cursor_pos[1] * vol_dims[1] as f32).round() as u32;
                            label_res = Some(draw_label(ui, "Coronal (Front)", is_active));
                            if let Some((curr, total)) = Some((slice_y, vol_dims[1])) {
                                ui.label(format!("Slice: {} / {}", curr, total));
                            }
                            if let Ok(w) = world.get::<&VolumeWindowing>(entities.volume_windowing)
                            {
                                ui.label(format!("W/L: {:.0} / {:.0}", w.width, w.center));
                            }
                            marker(ui, "S", egui::pos2(rx0 + rhw, ry0 + 15.0));
                            marker(ui, "I", egui::pos2(rx0 + rhw, ry0 + rect.height() - 15.0));
                            marker(ui, "R", egui::pos2(rx0 + 15.0, ry0 + rhh));
                            marker(ui, "L", egui::pos2(rx0 + rect.width() - 15.0, ry0 + rhh));
                        }
                        ViewMode::Sagittal => {
                            let slice_x = (cursor_pos[0] * vol_dims[0] as f32).round() as u32;
                            label_res = Some(draw_label(ui, "Sagittal (Side)", is_active));
                            if let Some((curr, total)) = Some((slice_x, vol_dims[0])) {
                                ui.label(format!("Slice: {} / {}", curr, total));
                            }
                            if let Ok(w) = world.get::<&VolumeWindowing>(entities.volume_windowing)
                            {
                                ui.label(format!("W/L: {:.0} / {:.0}", w.width, w.center));
                            }
                            marker(ui, "S", egui::pos2(rx0 + rhw, ry0 + 15.0));
                            marker(ui, "I", egui::pos2(rx0 + rhw, ry0 + rect.height() - 15.0));
                            marker(ui, "A", egui::pos2(rx0 + 15.0, ry0 + rhh));
                            marker(ui, "P", egui::pos2(rx0 + rect.width() - 15.0, ry0 + rhh));
                        }
                    });

                if let Some(res) = label_res {
                    if res.double_clicked() {
                        let _ = event_proxy.send_event(AppEvent::ToggleMaximize(e));
                        ctx.request_repaint();
                    }
                    let dnd_id = egui::Id::new("viewport_dnd");
                    if res.drag_started() {
                        ctx.memory_mut(|mem| mem.data.insert_temp(dnd_id, e));
                    }
                    if res.drag_stopped() {
                        if let Some(source_e) =
                            ctx.memory(|mem| mem.data.get_temp::<hecs::Entity>(dnd_id))
                        {
                            // Find which viewport we dropped onto
                            let drop_pos = ctx.input(|i| i.pointer.interact_pos());
                            if let Some(pos) = drop_pos {
                                for (target_e, _, target_rect) in vps.clone() {
                                    if target_rect.contains(pos) && target_e != source_e {
                                        let _ = event_proxy.send_event(AppEvent::SwapViewports(
                                            source_e, target_e,
                                        ));
                                        ctx.request_repaint();
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }

                if mode == ViewMode::ThreeD {
                    let gizmo_rect = egui::Rect::from_center_size(
                        egui::pos2(rx0 + 80.0, ry0 + rect.height() - 80.0),
                        egui::vec2(120.0, 120.0),
                    );
                    egui::Area::new("gizmo_3d".into())
                        .fixed_pos(gizmo_rect.min)
                        .interactable(false)
                        .show(ctx, |ui| {
                            let view_quat = crate::gizmo::quat_from_array(gizmo_rotation);
                            crate::gizmo::draw_gizmo(ui, gizmo_rect, view_quat);
                        });
                }
            }

            // --- Instruction Overlay if empty ---
            if volume_info.is_none() {
                egui::Area::new("instructions".into())
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(ctx, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space(20.0);
                            ui.label(
                                egui::RichText::new("No data loaded")
                                    .size(32.0)
                                    .strong()
                                    .color(egui::Color32::from_gray(180)),
                            );
                            ui.label(
                                egui::RichText::new(
                                    "Use \u{1f4c2} Load Main Volume (NIfTI) to get started",
                                )
                                .size(18.0)
                                .color(egui::Color32::from_gray(140)),
                            );
                        });
                    });
            }

            // --- Draw Annotations ---
            egui::Area::new("annotations_layer".into())
                .fixed_pos(central_rect.min)
                .interactable(false)
                .show(ctx, |ui| {
                    let mut vd_query = world.query::<&VolumeData>().with::<&MainVolumeTag>();
                    let vol_data = vd_query.iter().next().map(|(_, vd)| vd);

                    if let (Ok(mut state), Ok(mut overlay), Some(vd)) = (
                        world.get::<&mut AnnotationState>(entities.annotations),
                        world.get::<&mut OverlayManager>(entities.overlay),
                        vol_data,
                    ) {
                        let items = &mut state.annotations;

                        // Loop over all viewports to draw annotations in each
                        for (e, mode, rect) in vps {
                            if let Ok(vs) = world.get::<&ViewportState>(e) {
                                draw_annotations(ui, items, &vs, vd, rect, mode, &mut overlay);
                            }
                        }
                    }
                });
        });

        // Update input state flag
        if let Ok(mut input) = world.get::<&mut InputState>(entities.input) {
            input.egui_wants_input =
                self.context.wants_pointer_input() || self.context.is_using_pointer();
        }

        self.output = Some(full_output);
    }

    pub fn render(
        &mut self,
        device: &Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        screen_descriptor: &egui_wgpu::ScreenDescriptor,
    ) -> std::time::Duration {
        let output = self
            .output
            .take()
            .expect("Gui::prepare() must be called before Gui::render()");

        let repaint_after = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .expect("Missing root viewport output")
            .repaint_delay;

        let tessellation = self
            .context
            .tessellate(output.shapes, self.context.pixels_per_point());

        for (id, delta) in &output.textures_delta.set {
            self.renderer.update_texture(device, queue, *id, delta);
        }

        self.renderer
            .update_buffers(device, queue, encoder, &tessellation, screen_descriptor);
        {
            let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Egui Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            self.renderer.render(
                &mut render_pass.forget_lifetime(),
                &tessellation,
                screen_descriptor,
            );
        }
        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }

        repaint_after
    }
}

// --- Annotation Helpers ---

fn draw_annotations(
    ui: &mut egui::Ui,
    annotations: &mut [Annotation],
    view: &ViewportState,
    vol: &VolumeData,
    rect: egui::Rect,
    mode: ViewMode,
    overlay: &mut OverlayManager,
) {
    let aspect_ratios = vol.aspect_ratios();

    if vol.dimensions[0] == 0 {
        return;
    }

    let viewport_idx = match mode {
        ViewMode::ThreeD => 0,
        ViewMode::Axial => 1,
        ViewMode::Coronal => 2,
        ViewMode::Sagittal => 3,
    };

    for (idx, ann) in annotations.iter_mut().enumerate() {
        if let Some(screen_pos) = world_to_screen(
            ann.world_pos,
            viewport_idx,
            view.zoom,
            view.pan,
            view.pivot,
            view.rotation,
            aspect_ratios,
            rect,
        ) {
            let sense = if viewport_idx > 0 {
                egui::Sense::drag()
            } else {
                egui::Sense::hover()
            };
            let id = ui.make_persistent_id(format!("ann_{}_{}", viewport_idx, idx));

            let point_rect = egui::Rect::from_center_size(screen_pos, egui::vec2(16.0, 16.0));
            let response = ui.interact(point_rect, id, sense);

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

                    let slice_aspect = match viewport_idx {
                        1 => aspect_ratios[0] / aspect_ratios[1], // Axial (X/Y)
                        2 => aspect_ratios[0] / aspect_ratios[2], // Coronal (X/Z)
                        3 => aspect_ratios[1] / aspect_ratios[2], // Sagittal (Y/Z)
                        _ => 1.0,
                    };
                    let k = screen_aspect / slice_aspect;

                    let ndc_x = (mouse_pos.x - rect.min.x) / rect.width();
                    let ndc_y = (mouse_pos.y - rect.min.y) / rect.height();

                    overlay.mouse_screen_uv = [ndc_x, ndc_y];

                    let world_u = ((ndc_x - pivot[0]) * k / zoom) + pivot[0] + pan[0];
                    let world_v = ((ndc_y - pivot[1]) / zoom) + pivot[1] + pan[1];

                    if let Some(plane) =
                        crate::orientation::SlicePlane::from_viewport(viewport_idx as u32)
                    {
                        let vol_pos = plane.screen_uv_to_volume(
                            [world_u, world_v],
                            ann.world_pos[plane.depth_axis()],
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
                    view.rotation,
                    aspect_ratios,
                    rect,
                )
                .unwrap_or(screen_pos)
            };

            ui.painter().text(
                draw_pos + egui::vec2(8.0, -8.0),
                egui::Align2::LEFT_BOTTOM,
                &ann.label,
                egui::FontId::proportional(14.0),
                egui::Color32::WHITE,
            );
        }
    }
}

fn world_to_screen(
    pos: glam::Vec3,
    viewport_idx: usize,
    zoom: f32,
    pan: [f32; 2],
    pivot: [f32; 2],
    rotation: [f32; 4],
    aspect_ratios: [f32; 3],
    rect: egui::Rect,
) -> Option<egui::Pos2> {
    let screen_aspect = if rect.height() > 0.0 {
        rect.width() / rect.height()
    } else {
        1.0
    };

    if let Some([ndc_x, ndc_y]) = crate::geometry::world_to_ndc(
        pos,
        viewport_idx,
        zoom,
        pan,
        pivot,
        rotation,
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
