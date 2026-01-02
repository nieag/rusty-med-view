use crate::components::*;
use crate::{file_dialog, nifti_loader, systems};
use egui_wgpu::Renderer;
use egui_winit::State;
use hecs::World;
use std::sync::mpsc::Sender;
use wgpu::{Device, TextureFormat};
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
            None, // Added 6th argument (max_inner_size)
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
        volume_sender: Sender<Result<LoadResult, nifti_loader::LoadError>>,
    ) {
        let raw_input = self.state.take_egui_input(window);

        let full_output = self.context.run(raw_input, |ctx| {
            // 1. Data Collection from ECS
            let mut active_viewport = 99;
            let mut status_msg = None;
            let mut loading_state = VolumeLoadingState::Ready;
            let mut volume_info = None;

            for (_, inp) in world.query::<&InputState>().iter() {
                active_viewport = inp.active_viewport;
            }
            for (_, gui_state) in world.query::<&GuiState>().iter() {
                status_msg = gui_state.status_message.clone();
            }
            for (_, state) in world.query::<&VolumeLoadingState>().iter() {
                loading_state = state.clone();
            }
            for (_, vol) in world.query::<&VolumeData>().iter() {
                volume_info = Some(vol.dimensions);
            }

            // 2. Sidebar Implementation
            egui::SidePanel::left("left_panel")
                .resizable(true)
                .default_width(250.0)
                .show(ctx, |ui| {
                    ui.heading("Medical Viewer");
                    ui.separator();

                    // --- Data Loading ---
                    ui.collapsing("Data Loading", |ui| {
                        if ui.button("📂 Load Main Volume (NIfTI)").clicked() {
                            // Update status message
                            for (_, gui_state) in world.query_mut::<&mut GuiState>() {
                                gui_state.status_message = Some("Loading...".to_string());
                            }
                            // Spawn file picker DIRECTLY from button click (required for WASM user gesture)
                            let sender = volume_sender.clone();
                            file_dialog::spawn_file_picker(move |result| {
                                if let Some((_filename, data)) = result {
                                    let load_result = nifti_loader::load_nifti_from_bytes(&data)
                                        .map(LoadResult::Volume);
                                    let _ = sender.send(load_result);
                                }
                            });
                        }
                        if let Some(dims) = volume_info {
                            ui.label(format!("Volume: {}x{}x{}", dims[0], dims[1], dims[2]));
                        } else {
                            ui.label("No volume loaded");
                        }

                        ui.separator();
                        ui.label("Overlays");
                        if ui.button("📂 Load Label (Slot 1)").clicked() {
                            // Update status message
                            for (_, gui_state) in world.query_mut::<&mut GuiState>() {
                                gui_state.status_message = Some("Loading Labelmap...".to_string());
                            }
                            // Spawn file picker DIRECTLY from button click (required for WASM user gesture)
                            let sender = volume_sender.clone();
                            file_dialog::spawn_file_picker(move |result| {
                                if let Some((filename, data)) = result {
                                    let load_result =
                                        nifti_loader::load_label_from_bytes(&data, filename)
                                            .map(LoadResult::Label);
                                    let _ = sender.send(load_result);
                                }
                            });
                        }
                    });

                    ui.separator();

                    // --- Windowing / Contrast Controls (HU-based) ---
                    ui.collapsing("Windowing", |ui| {
                        let mut windowing_query = world.query::<&mut VolumeWindowing>();
                        if let Some((_, windowing)) = windowing_query.iter().next() {
                            ui.label("Window Center (HU)");
                            ui.add(
                                egui::Slider::new(&mut windowing.center, -1024.0..=3071.0)
                                    .show_value(true),
                            );

                            ui.label("Window Width (HU)");
                            ui.add(
                                egui::Slider::new(&mut windowing.width, 1.0..=4096.0)
                                    .show_value(true),
                            );

                            ui.separator();
                            ui.label("Presets:");
                            ui.horizontal(|ui| {
                                if ui.button("Soft Tissue").clicked() {
                                    windowing.center = 40.0;
                                    windowing.width = 400.0;
                                }
                                if ui.button("Lung").clicked() {
                                    windowing.center = -600.0;
                                    windowing.width = 1500.0;
                                }
                            });
                            ui.horizontal(|ui| {
                                if ui.button("Bone").clicked() {
                                    windowing.center = 400.0;
                                    windowing.width = 2000.0;
                                }
                                if ui.button("Brain").clicked() {
                                    windowing.center = 40.0;
                                    windowing.width = 80.0;
                                }
                            });
                        } else {
                            ui.label("No windowing settings available");
                        }
                    });

                    ui.separator();

                    // --- Toolbox (Label Editor) ---
                    ui.collapsing("Toolbox", |ui| {
                        // Access EditorState
                        let mut editor_state_query = world.query::<&mut EditorState>();
                        if let Some((_, editor)) = editor_state_query.iter().next() {
                            ui.label("Active Tool");
                            ui.horizontal(|ui| {
                                ui.radio_value(
                                    &mut editor.active_tool,
                                    EditorTool::Navigation,
                                    "Nav",
                                );
                                ui.radio_value(&mut editor.active_tool, EditorTool::Brush, "Brush");
                                ui.radio_value(
                                    &mut editor.active_tool,
                                    EditorTool::Eraser,
                                    "Erase",
                                );
                            });

                            if editor.active_tool != EditorTool::Navigation {
                                ui.separator();
                                ui.label(format!("Brush Size: {:.1}", editor.brush_size));
                                ui.add(
                                    egui::Slider::new(&mut editor.brush_size, 1.0..=20.0)
                                        .text("px"),
                                );

                                ui.label("Label ID");
                                ui.add(egui::Slider::new(&mut editor.active_label_index, 1..=10));
                            }
                        }
                    });

                    ui.separator();

                    // --- Layer Control ---
                    ui.collapsing("Layers", |ui| {
                        let mut layers: Vec<(hecs::Entity, String, bool, f32)> = Vec::new();
                        for (e, (seg, settings)) in
                            world.query::<(&Segmentation, &LayerSettings)>().iter()
                        {
                            layers.push((e, seg.name.clone(), seg.is_visible, settings.opacity));
                        }

                        // New Layer Button
                        if ui.button("➕ Create New Layer").clicked() {
                            // Need to trigger creation. Can't do it here easily since we need Device/Queue.
                            // Set a flag in GuiState?
                            for (_, gui_state) in world.query_mut::<&mut GuiState>() {
                                gui_state.load_label_requested = true; // Temporary hijack for "Create" logic
                                                                       // Actually create logic needs to differentiate Load vs Create.
                                                                       // Let's rely on checking this flag in Lib.rs and creating a default one.
                                                                       // Or better, add `create_label_requested` to GuiState.
                                                                       // For now, let's leave it as "TODO" or implement properly next step.
                                                                       // Let's implement active layer selection first.
                            }
                        }

                        // Collect EditorState to update active layer
                        let mut active_layer = None;
                        for (_, editor) in world.query::<&EditorState>().iter() {
                            active_layer = editor.active_layer;
                        }
                        let mut new_active_layer = active_layer;

                        for (entity, name, mut visible, mut opacity) in layers {
                            ui.group(|ui| {
                                ui.horizontal(|ui| {
                                    // Radio button for "Active Layer"
                                    ui.radio_value(&mut new_active_layer, Some(entity), "");

                                    if ui.checkbox(&mut visible, "").changed() {
                                        if let Ok(mut seg) =
                                            world.query_one::<&mut Segmentation>(entity)
                                        {
                                            if let Some(s) = seg.get() {
                                                s.is_visible = visible;
                                            }
                                        }
                                    }
                                    ui.label(name);
                                });
                                if visible {
                                    if ui
                                        .add(
                                            egui::Slider::new(&mut opacity, 0.0..=1.0)
                                                .text("Opacity"),
                                        )
                                        .changed()
                                    {
                                        if let Ok(mut set) =
                                            world.query_one::<&mut LayerSettings>(entity)
                                        {
                                            if let Some(s) = set.get() {
                                                s.opacity = opacity;
                                            }
                                        }
                                    }
                                }
                            });
                        }

                        // Write back active layer selection
                        if new_active_layer != active_layer {
                            for (_, editor) in world.query_mut::<&mut EditorState>() {
                                editor.active_layer = new_active_layer;
                            }
                        }
                    });

                    ui.separator();

                    // --- Status & Instructions ---
                    if let Some(msg) = &status_msg {
                        ui.label(egui::RichText::new(msg).color(egui::Color32::LIGHT_BLUE));
                    }
                    match loading_state {
                        VolumeLoadingState::Loading => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label("Processing...");
                            });
                        }
                        _ => {}
                    }

                    ui.separator();
                    ui.collapsing("Controls", |ui| {
                        ui.label("Left Mouse: Crosshair / Paint");
                        ui.label("Middle Mouse: Pan");
                        ui.label("Right Mouse: Rotate (3D)");
                        ui.label("Scroll: Zoom / Slice");
                        ui.label("Ctrl+Scroll: 2D Zoom");
                    });

                    // --- Always-visible HU Readout at bottom ---
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                        ui.separator();
                        if let Some(hu) = systems::get_hu_at_mouse(world) {
                            ui.label(format!("HU at cursor: {:.0}", hu));
                        } else {
                            ui.label("HU at cursor: --");
                        }
                    });
                });

            // 3. Viewport Rect Calculation
            // We SHUT DOWN the CentralPanel because it consumes mouse events.
            // Instead, we just read the remaining space using `available_rect()`.
            let central_rect = ctx.available_rect();

            let pixels_per_point = ctx.pixels_per_point();
            for (_, settings) in world.query_mut::<&mut WindowSettings>() {
                settings.viewport_rect = [
                    central_rect.min.x * pixels_per_point,
                    central_rect.min.y * pixels_per_point,
                    central_rect.width() * pixels_per_point,
                    central_rect.height() * pixels_per_point,
                ];
            }

            // 4. Overlays (Floating on top of Central Area)
            let mut cursor_pos = [0.0, 0.0, 0.0];
            for (_, (t, _)) in world.query::<(&Transform, &CursorTag)>().iter() {
                cursor_pos = t.position;
            }

            let draw_label = |ui: &mut egui::Ui, text: &str, is_active: bool| {
                ui.add(egui::Label::new(
                    egui::RichText::new(text)
                        .color(if is_active {
                            egui::Color32::GREEN
                        } else {
                            egui::Color32::WHITE
                        })
                        .size(16.0)
                        .strong(),
                ));
            };

            // Calculate relative positions for overlays based on the 4 viewports within central_rect
            let hw = central_rect.width() / 2.0;
            let hh = central_rect.height() / 2.0;
            let x0 = central_rect.min.x;
            let y0 = central_rect.min.y;

            egui::Area::new("overlay_3d".into())
                .fixed_pos([x0 + 10.0, y0 + 10.0])
                .interactable(false) // CRITICAL: Allow mouse to fall through to WGPU
                .show(ctx, |ui| draw_label(ui, "3D View", active_viewport == 0));

            egui::Area::new("overlay_xy".into())
                .fixed_pos([x0 + hw + 10.0, y0 + 10.0])
                .interactable(false)
                .show(ctx, |ui| {
                    draw_label(ui, "Axial (Top)", active_viewport == 1);
                    ui.label(format!("Slice Z: {:.2}", cursor_pos[2]));
                });

            egui::Area::new("overlay_xz".into())
                .fixed_pos([x0 + 10.0, y0 + hh + 10.0])
                .interactable(false)
                .show(ctx, |ui| {
                    draw_label(ui, "Coronal (Front)", active_viewport == 2);
                    ui.label(format!("Slice Y: {:.2}", cursor_pos[1]));
                });

            egui::Area::new("overlay_yz".into())
                .fixed_pos([x0 + hw + 10.0, y0 + hh + 10.0])
                .interactable(false)
                .show(ctx, |ui| {
                    draw_label(ui, "Sagittal (Side)", active_viewport == 3);
                    ui.label(format!("Slice X: {:.2}", cursor_pos[0]));
                });
        });

        self.output = Some(full_output);
    }

    pub fn render(
        &mut self,
        device: &Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        screen_descriptor: &egui_wgpu::ScreenDescriptor,
    ) {
        // Retrieve the output we saved in prepare()
        let output = self
            .output
            .take()
            .expect("Gui::prepare() must be called before Gui::render()");

        // Generate the geometry
        let tessellation = self
            .context
            .tessellate(output.shapes, self.context.pixels_per_point());

        // Update textures (font atlas)
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
        // Cleanup textures that are no longer needed
        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }
    }
}
