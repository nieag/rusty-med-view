pub mod annotations;
pub mod gizmo;
pub mod overlays;
pub mod segments;
pub mod sidebar;
pub mod toolbar;

use crate::components::*;
use crate::AppEvent;
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
            // 1. Data Collection
            let (status_msg, volume_info, windowing_active, active_viewport_entity) = {
                let status_msg = world
                    .get::<&GuiState>(entities.gui_state)
                    .map(|g| g.status_message.clone())
                    .unwrap_or(None);

                let volume_info = {
                    let mut query = world.query::<&VolumeData>().with::<&MainVolumeTag>();
                    query.iter().next().and_then(|(_, vd)| {
                        if vd.dimensions == [0, 0, 0] {
                            None
                        } else {
                            Some(vd.dimensions)
                        }
                    })
                };

                let windowing_active = volume_info.is_some();

                let active_viewport_entity = world
                    .get::<&InputState>(entities.input)
                    .map(|i| i.active_viewport)
                    .unwrap_or(None);

                (
                    status_msg,
                    volume_info,
                    windowing_active,
                    active_viewport_entity,
                )
            };

            // 2. UI Layout
            egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
                toolbar::draw_toolbar(
                    ctx,
                    ui,
                    world,
                    entities,
                    &event_proxy,
                    status_msg,
                    windowing_active,
                );
            });

            egui::SidePanel::left("left_panel")
                .resizable(true)
                .default_width(220.0)
                .show(ctx, |ui| {
                    sidebar::draw_sidebar(ctx, ui, world, entities, &event_proxy, volume_info);
                });

            let show_right_sidebar = world
                .get::<&AnnotationState>(entities.annotations)
                .map(|s| s.show_right_sidebar)
                .unwrap_or(false);

            if show_right_sidebar {
                egui::SidePanel::right("discussion_panel")
                    .resizable(true)
                    .default_width(320.0)
                    .show(ctx, |ui| {
                        annotations::draw_discussion_sidebar(
                            ctx,
                            ui,
                            world,
                            entities,
                            &event_proxy,
                        );
                    });
            }

            // 3. Viewport Rect Calculation
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
            overlays::draw_viewport_overlays(
                ctx,
                world,
                entities,
                &event_proxy,
                central_rect,
                &vps,
                active_viewport_entity,
                volume_info,
            );

            // 5. Input Synchronization
            if let Ok(mut input) = world.get::<&mut InputState>(entities.input) {
                input.egui_wants_input = ctx.wants_pointer_input() || ctx.is_using_pointer();
            }
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
