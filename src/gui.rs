use crate::components::*;
// use egui::Window;
use egui_wgpu::Renderer;
use egui_winit::State;
use hecs::World;
use wgpu::{Device, TextureFormat};
use winit::window::Window as WinitWindow;

pub struct Gui {
    pub context: egui::Context,
    state: State,
    renderer: Renderer,
    // We must store the output frame between 'prepare' and 'render'
    output: Option<egui::FullOutput>,
    // Track if user requested to load a file
    pub load_requested: bool,
    // Current loading status message
    pub status_message: Option<String>,
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
            load_requested: false,
            status_message: None,
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

    pub fn prepare(&mut self, window: &WinitWindow, world: &World) {
        // Reset load request each frame
        self.load_requested = false;

        let raw_input = self.state.take_egui_input(window);

        // Use Cell for interior mutability in closure
        let load_clicked = std::cell::Cell::new(false);
        let status_msg = self.status_message.clone();

        // CAPTURE THE OUTPUT HERE
        let full_output = self.context.run(raw_input, |ctx| {
            // 1. Get Data from ECS
            let mut cursor_pos = [0.0, 0.0, 0.0];
            let mut width = 100.0;
            let mut height = 100.0;
            let mut active_viewport = 99;

            // 1. Get the Scale Factor (DPI)
            let pixels_per_point = window.scale_factor() as f32;
            for (_, (t, _)) in world.query::<(&Transform, &CursorTag)>().iter() {
                cursor_pos = t.position;
            }
            for (_, set) in world.query::<&WindowSettings>().iter() {
                width = (set.width as f32) / pixels_per_point;
                height = (set.height as f32) / pixels_per_point;
            }
            for (_, inp) in world.query::<&InputState>().iter() {
                active_viewport = inp.active_viewport;
            }

            let hw = width / 2.0;
            let hh = height / 2.0;

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

            // 3. Draw Overlays
            egui::Area::new("overlay_3d".into())
                .fixed_pos([10.0, 10.0])
                .show(ctx, |ui| {
                    draw_label(ui, "3D View", active_viewport == 0);
                    ui.add_space(4.0);
                    if ui.button("📂 Load NIfTI...").clicked() {
                        load_clicked.set(true);
                    }
                    if let Some(msg) = &status_msg {
                        ui.label(msg);
                    }
                });

            egui::Area::new("overlay_xy".into())
                .fixed_pos([hw + 10.0, 10.0])
                .show(ctx, |ui| {
                    draw_label(ui, "Axial (Top)", active_viewport == 1);
                    ui.label(format!("Slice Z: {:.2}", cursor_pos[2]));
                });

            egui::Area::new("overlay_xz".into())
                .fixed_pos([10.0, hh + 10.0])
                .show(ctx, |ui| {
                    draw_label(ui, "Coronal (Front)", active_viewport == 2);
                    ui.label(format!("Slice Y: {:.2}", cursor_pos[1]));
                });

            egui::Area::new("overlay_yz".into())
                .fixed_pos([hw + 10.0, hh + 10.0])
                .show(ctx, |ui| {
                    draw_label(ui, "Sagittal (Side)", active_viewport == 3);
                    ui.label(format!("Slice X: {:.2}", cursor_pos[0]));
                });
        });

        // Copy button click state to self after closure
        self.load_requested = load_clicked.get();

        // Store it for the render step
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
