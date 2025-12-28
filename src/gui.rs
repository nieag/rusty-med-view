use crate::components::*;
use egui::Window;
use egui_wgpu::Renderer;
use egui_winit::State;
use hecs::World;
use wgpu::{Device, TextureFormat};
use winit::window::Window as WinitWindow;

pub struct Gui {
    pub context: egui::Context,
    state: State,
    renderer: Renderer,
    // NEW: We must store the output frame between 'prepare' and 'render'
    output: Option<egui::FullOutput>,
}

impl Gui {
    pub fn new(
        device: &Device,
        format: TextureFormat,
        event_loop: &winit::event_loop::EventLoopWindowTarget<()>,
    ) -> Self {
        let context = egui::Context::default();

        let state = State::new(
            context.clone(),
            egui::ViewportId::ROOT,
            event_loop,
            Some(1.0),
            None,
        );

        let renderer = Renderer::new(device, format, None, 1);

        Self {
            context,
            state,
            renderer,
            output: None, // Initialize as empty
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
        let raw_input = self.state.take_egui_input(window);

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
            for (_, set) in world.query::<(&WindowSettings)>().iter() {
                width = (set.width as f32) / pixels_per_point;
                height = (set.height as f32) / pixels_per_point;
            }
            for (_, inp) in world.query::<(&InputState)>().iter() {
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
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Egui Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            self.renderer
                .render(&mut render_pass, &tessellation, screen_descriptor);
        }
        // Cleanup textures that are no longer needed
        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }
    }
}
