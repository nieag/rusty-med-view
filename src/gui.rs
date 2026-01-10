use crate::components::*;
use crate::overlay::OverlayManager;
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
                                if visible
                                    && ui
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

                    // --- Annotations ---
                    ui.collapsing("Annotations", |ui| {
                        // "Add" Button
                        if ui.button("➕ Add Annotation").clicked() {
                            // Logic to add annotation at current cursor position
                            // We need to access AnnotationState (mutable) and Cursor Position
                            // Cursor Position is in `cursor_pos` variable (Transform)
                            // But we need to write to AnnotationState.

                            // Collect queries outside closure to avoid ownership issues?
                            // Actually we are inside `show`, we can query mutable world.
                            // But we need to be careful about borrowing.

                            // Let's grab the current cursor pos from the world first
                            let mut current_pos = glam::Vec3::ZERO;
                            for (_, (t, _)) in world.query::<(&Transform, &CursorTag)>().iter() {
                                current_pos = glam::Vec3::from(t.position);
                            }

                            for (_, state) in world.query_mut::<&mut AnnotationState>() {
                                state.annotations.push(Annotation {
                                    world_pos: current_pos,
                                    label: "New".to_string(),
                                });
                            }
                        }

                        ui.separator();

                        // List Annotations
                        let mut to_delete = None;
                        let mut to_locate = None;

                        // We iterate mutably over AnnotationState to allow text editing
                        for (_, state) in world.query_mut::<&mut AnnotationState>() {
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

                        // Handle "Locate" Action
                        if let Some(pos) = to_locate {
                            for (_, (t, _)) in world.query_mut::<(&mut Transform, &CursorTag)>() {
                                t.position = pos.into();
                            }
                            // Center 2D views on this position
                            for (_, view) in world.query_mut::<&mut ViewState>() {
                                // Axial: center on x, y
                                view.pan[1] = [pos.x - 0.5, pos.y - 0.5];
                                // Coronal: center on x, z
                                view.pan[2] = [pos.x - 0.5, pos.z - 0.5];
                                // Sagittal: center on y, z
                                view.pan[3] = [pos.y - 0.5, pos.z - 0.5];
                            }
                        }
                    });

                    ui.separator();

                    // --- Status & Instructions ---
                    if let Some(msg) = &status_msg {
                        ui.label(egui::RichText::new(msg).color(egui::Color32::LIGHT_BLUE));
                    }
                    if let VolumeLoadingState::Loading = loading_state {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Processing...");
                        });
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

            // Get rotation for 3D gizmo
            let mut gizmo_rotation = [0.0f32, 0.0, 0.0, 1.0]; // Identity quaternion
            for (_, view) in world.query::<&ViewState>().iter() {
                gizmo_rotation = view.rotation[0]; // 3D view rotation
            }

            egui::Area::new("overlay_3d".into())
                .fixed_pos([x0 + 10.0, y0 + 10.0])
                .interactable(false) // CRITICAL: Allow mouse to fall through to WGPU
                .show(ctx, |ui| draw_label(ui, "3D View", active_viewport == 0));

            // 3D Orientation Gizmo using glam-based module
            let gizmo_rect = egui::Rect::from_center_size(
                egui::pos2(x0 + 80.0, y0 + hh - 80.0),
                egui::vec2(120.0, 120.0),
            );

            egui::Area::new("gizmo_3d".into())
                .fixed_pos(gizmo_rect.min)
                .interactable(false)
                .show(ctx, |ui| {
                    let view_quat = crate::gizmo::quat_from_array(gizmo_rotation);
                    crate::gizmo::draw_gizmo(ui, gizmo_rect, view_quat);
                });

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

            // --- Draw Annotations ---
            egui::Area::new("annotations_layer".into())
                .fixed_pos(central_rect.min)
                .interactable(false)
                .show(ctx, |ui| {
                    let mut ann_query = world.query::<&mut AnnotationState>();
                    let mut vs_query = world.query::<&ViewState>();
                    let mut vd_query = world.query::<&VolumeData>();
                    let mut overlay_query = world.query::<&mut OverlayManager>();

                    if let (Some((_, state)), Some((_, vs)), Some((_, vd)), Some((_, overlay))) = (
                        ann_query.iter().next(),
                        vs_query.iter().next(),
                        vd_query.iter().next(),
                        overlay_query.iter().next(),
                    ) {
                        let items = &mut state.annotations;

                        // Viewport 0
                        draw_annotations(
                            ui,
                            items,
                            vs,
                            vd,
                            egui::Rect::from_min_size(egui::pos2(x0, y0), egui::vec2(hw, hh)),
                            0,
                            overlay,
                        );
                        // Viewport 1
                        draw_annotations(
                            ui,
                            items,
                            vs,
                            vd,
                            egui::Rect::from_min_size(egui::pos2(x0 + hw, y0), egui::vec2(hw, hh)),
                            1,
                            overlay,
                        );
                        // Viewport 2
                        draw_annotations(
                            ui,
                            items,
                            vs,
                            vd,
                            egui::Rect::from_min_size(egui::pos2(x0, y0 + hh), egui::vec2(hw, hh)),
                            2,
                            overlay,
                        );
                        // Viewport 3
                        draw_annotations(
                            ui,
                            items,
                            vs,
                            vd,
                            egui::Rect::from_min_size(
                                egui::pos2(x0 + hw, y0 + hh),
                                egui::vec2(hw, hh),
                            ),
                            3,
                            overlay,
                        );
                    }
                });
        });

        // Update input state flag so other systems (like paint) know egui is using the pointer
        for (_, input) in world.query_mut::<&mut InputState>() {
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

// --- Annotation Helpers ---

fn draw_annotations(
    ui: &mut egui::Ui,
    annotations: &mut [Annotation],
    view: &ViewState,
    vol: &VolumeData,
    rect: egui::Rect,
    viewport_idx: usize,
    overlay: &mut OverlayManager,
) {
    let aspect_ratios = vol.aspect_ratios();

    // Safety check for empty dimensions
    if vol.dimensions[0] == 0 {
        return;
    }

    for (idx, ann) in annotations.iter_mut().enumerate() {
        if let Some(screen_pos) =
            world_to_screen(ann.world_pos, viewport_idx, view, aspect_ratios, rect)
        {
            // Draw marker as an interactive widget
            let sense = if viewport_idx > 0 {
                egui::Sense::drag()
            } else {
                egui::Sense::hover()
            };
            // We use a predefined ID to track state across frames
            let id = ui.make_persistent_id(format!("ann_{}_{}", viewport_idx, idx));

            // Allocate space for the interaction
            let point_rect = egui::Rect::from_center_size(screen_pos, egui::vec2(16.0, 16.0));
            let response = ui.interact(point_rect, id, sense);

            // Handle Dragging using ABSOLUTE position (not deltas) for minimal lag
            if viewport_idx > 0 && response.dragged() {
                // Update OverlayManager for GPU rendering with zero lag
                overlay.dragging_idx = Some(idx);
                overlay.dragging_viewport = viewport_idx as u32;

                // Get current mouse position directly from egui
                if let Some(mouse_pos) = ui.ctx().pointer_latest_pos() {
                    // Convert screen position to world coordinates directly
                    // This is the inverse of world_to_screen for 2D views

                    let zoom = view.zoom[viewport_idx];
                    let pan = view.pan[viewport_idx];
                    let pivot = view.pivot[viewport_idx];

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

                    // Convert screen pos to NDC (0..1 within viewport rect)
                    let ndc_x = (mouse_pos.x - rect.min.x) / rect.width();
                    let ndc_y = (mouse_pos.y - rect.min.y) / rect.height();

                    // Store screen UV for shader (within this viewport)
                    overlay.mouse_screen_uv = [ndc_x, ndc_y];

                    // Invert the world_to_screen projection:
                    // ndc_x = ((u - pivot[0] - pan[0]) * zoom / k) + pivot[0]
                    // Solving for u: u = ((ndc_x - pivot[0]) * k / zoom) + pivot[0] + pan[0]
                    let world_u = ((ndc_x - pivot[0]) * k / zoom) + pivot[0] + pan[0];
                    let world_v = ((ndc_y - pivot[1]) / zoom) + pivot[1] + pan[1];

                    // Apply to appropriate axes, keeping the slice axis unchanged
                    match viewport_idx {
                        1 => {
                            // Axial (x, y) - z is slice axis
                            ann.world_pos.x = world_u;
                            ann.world_pos.y = world_v;
                        }
                        2 => {
                            // Coronal (x, z) - y is slice axis
                            ann.world_pos.x = world_u;
                            ann.world_pos.z = world_v;
                        }
                        3 => {
                            // Sagittal (y, z) - x is slice axis
                            ann.world_pos.y = world_u;
                            ann.world_pos.z = world_v;
                        }
                        _ => {}
                    };

                    // Clamp to volume bounds
                    ann.world_pos = ann.world_pos.clamp(glam::Vec3::ZERO, glam::Vec3::ONE);
                }
            } else if response.drag_stopped() {
                // Clear drag state when drag ends
                overlay.dragging_idx = None;
            }

            // Calculate text draw position
            let draw_pos = if response.dragged() {
                // During drag, use mouse position for text too
                ui.ctx().pointer_latest_pos().unwrap_or(screen_pos)
            } else {
                world_to_screen(ann.world_pos, viewport_idx, view, aspect_ratios, rect)
                    .unwrap_or(screen_pos)
            };

            // GPU draws the circle now - egui only draws text label
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
    view: &ViewState,
    aspect_ratios: [f32; 3],
    rect: egui::Rect,
) -> Option<egui::Pos2> {
    let screen_aspect = if rect.height() > 0.0 {
        rect.width() / rect.height()
    } else {
        1.0
    };

    if let Some([ndc_x, ndc_y]) =
        crate::geometry::world_to_ndc(pos, viewport_idx, view, aspect_ratios, screen_aspect)
    {
        // Clip
        if !(0.0..=1.0).contains(&ndc_x) || !(0.0..=1.0).contains(&ndc_y) {
            // Re-apply clipping for non-3D views if needed
            if viewport_idx > 0 {
                return None;
            }
        }

        Some(egui::Pos2::new(
            rect.min.x + ndc_x * rect.width(),
            rect.min.y + ndc_y * rect.height(),
        ))
    } else {
        None
    }
}
