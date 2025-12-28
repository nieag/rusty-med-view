// src/main.rs

// 1. Declare Modules
mod components;
mod geometry;
mod gui;
mod systems;
mod volume;

// 2. Imports
use components::*; // Import our data structs (Transform, CameraRig, etc.)
use hecs::World;
use std::sync::Arc;
use wgpu::util::DeviceExt;
use winit::{event::*, event_loop::EventLoop, window::WindowBuilder};

fn main() {
    // --- 1. Window & WGPU Setup ---
    let event_loop = EventLoop::new().unwrap();
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("Medical Viewer - ECS Refactor")
            .build(&event_loop)
            .unwrap(),
    );

    let instance = wgpu::Instance::default();
    let surface = instance.create_surface(window.clone()).unwrap();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: Some(&surface),
        force_fallback_adapter: false,
    }))
    .unwrap();

    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
            .unwrap();

    let surface_caps = surface.get_capabilities(&adapter);
    let surface_format = surface_caps.formats[0];
    let mut config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: surface_format,
        width: window.inner_size().width,
        height: window.inner_size().height,
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: surface_caps.alpha_modes[0],
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    surface.configure(&device, &config);

    // --- 2. Initialize Data & ECS ---
    let mut world = World::new();

    // Generate Volume (CPU + GPU)
    let (_, diffuse_view, diffuse_sampler, voxel_data) =
        volume::create_voxel_texture(&device, &queue);
    
    world.spawn((VolumeData {
        size: 64, // Must match volume::create_voxel_texture size
        densities: voxel_data,
    },));

    // Spawn Camera Rig
    world.spawn((CameraRig {
        radius: 3.5,
        speed: 1.0,
        start_time: std::time::Instant::now(),
    },));

    // Spawn 3D Cursor (Center of Volume)
    world.spawn((
        Transform {
            position: [0.5, 0.5, 0.5],
            rotation: [0.0, 0.0, 0.0],
        },
        CursorTag,
    ));

    // Spawn Window Settings
    let settings_entity = world.spawn((WindowSettings {
        width: config.width,
        height: config.height,
    },));

    // Spawn Input State
    world.spawn((InputState {
        last_mouse_pos: [0.0, 0.0],
        mouse_uv: [0.0, 0.0],
        active_viewport: 0,
        modifiers: winit::keyboard::ModifiersState::empty(),
        is_dragging: false,
        drag_start_pos: [0.0, 0.0],
        drag_start_pan: [0.0, 0.0],
    },));

    // Spawn View State (zoom levels and pan offsets per viewport)
    world.spawn((ViewState {
        zoom: [3.5, 1.0, 1.0, 1.0],
        pan: [[0.0, 0.0]; 4],
    },));
    // --- 3. Pipeline Setup ---

    // Create Uniform Buffer (Allocation only, no data yet)
    // We need 4 slots (one per viewport), aligned to 256 bytes.
    let uniform_alignment = 256;
    let uniform_buffer_size = (uniform_alignment * 4) as u64;
    let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Uniform Buffer"),
        size: uniform_buffer_size,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    // Create Bind Group Layout
    let texture_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                // Binding 0: 3D Texture
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D3,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                // Binding 1: Sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // Binding 2: Uniforms (Dynamic Offset)
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true, // <--- IMPORTANT
                        min_binding_size: Some(
                            std::num::NonZeroU64::new(std::mem::size_of::<Uniforms>() as u64)
                                .unwrap(),
                        ),
                    },
                    count: None,
                },
            ],
            label: Some("texture_bind_group_layout"),
        });

    // Create Bind Group
    let diffuse_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &texture_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&diffuse_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&diffuse_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniform_buffer,
                    offset: 0,
                    size: Some(
                        std::num::NonZeroU64::new(std::mem::size_of::<Uniforms>() as u64).unwrap(),
                    ),
                }),
            },
        ],
        label: Some("diffuse_bind_group"),
    });

    // Load Shader
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/shader.wgsl").into()),
    });

    // Create Pipeline
    let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Render Pipeline Layout"),
        bind_group_layouts: &[&texture_bind_group_layout],
        push_constant_ranges: &[],
    });

    let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Render Pipeline"),
        layout: Some(&render_pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: "vs_main",
            buffers: &[geometry::Vertex::desc()], // Use descriptor from geometry.rs
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: "fs_main",
            targets: &[Some(wgpu::ColorTargetState {
                format: config.format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Back),
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
    });

    // Create Geometry Buffers (from geometry.rs)
    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Vertex Buffer"),
        contents: bytemuck::cast_slice(geometry::QUAD_VERTICES),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Index Buffer"),
        contents: bytemuck::cast_slice(geometry::QUAD_INDICES),
        usage: wgpu::BufferUsages::INDEX,
    });
    let num_indices = geometry::QUAD_INDICES.len() as u32;

    let mut gui = gui::Gui::new(&device, config.format, &event_loop);

    // --- 4. Event Loop ---
    let _ = event_loop.run(move |event, window_target| match event {
        Event::WindowEvent { event, .. } => {
            if gui.handle_event(&window, &event) {
                return;
            }

            match event {
                WindowEvent::CloseRequested => window_target.exit(),

                // NEW: Track Mouse Movement
                WindowEvent::CursorMoved { position, .. } => {
                    systems::sys_update_mouse(&mut world, position.x, position.y);
                }

                // NEW: Handle Mouse Buttons
                WindowEvent::MouseInput { button, state, .. } => {
                    systems::sys_handle_mouse_button(&mut world, button, state);
                }

                // NEW: Track Scrolling
                WindowEvent::MouseWheel { delta, .. } => {
                    // Normalize delta (LineDelta is usually 1.0, PixelDelta varies)
                    let y_delta = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y,
                        MouseScrollDelta::PixelDelta(pos) => (pos.y * 0.001) as f32,
                    };

                    if y_delta != 0.0 {
                        systems::sys_handle_input_scroll(&mut world, y_delta);
                        window.request_redraw(); // Force a redraw so we see the update instantly
                    }
                }

                // Track keyboard modifiers (Ctrl, Shift, Alt)
                WindowEvent::ModifiersChanged(modifiers) => {
                    systems::sys_update_modifiers(&mut world, modifiers.state());
                }

                WindowEvent::Resized(size) => {
                    // Resize WGPU Surface
                    config.width = size.width;
                    config.height = size.height;
                    surface.configure(&device, &config);

                    // Update WindowSettings in ECS
                    // We use query_one to find the specific entity we created earlier
                    let mut query = world
                        .query_one::<&mut WindowSettings>(settings_entity)
                        .unwrap();
                    if let Some(settings) = query.get() {
                        settings.width = size.width;
                        settings.height = size.height;
                    }
                }

                WindowEvent::RedrawRequested => {
                    // --- A. PREPARE DATA (Systems) ---

                    // We need to pack 4 structs into one byte array, with 256-byte alignment padding.
                    let mut all_uniforms_bytes = Vec::with_capacity(1024);

                    for mode in 0..4 {
                        // Call the logic system (Defined in systems.rs)
                        let u = systems::sys_prepare_render_data(&mut world, mode);

                        // Serialize and Pad
                        all_uniforms_bytes.extend_from_slice(bytemuck::cast_slice(&[u]));
                        while all_uniforms_bytes.len() % 256 != 0 {
                            all_uniforms_bytes.push(0);
                        }
                    }

                    // Upload to GPU
                    queue.write_buffer(&uniform_buffer, 0, &all_uniforms_bytes);
                    gui.prepare(&window, &world);

                    // --- B. RENDER FRAME ---
                    let frame = surface.get_current_texture().unwrap();
                    let view = frame
                        .texture
                        .create_view(&wgpu::TextureViewDescriptor::default());

                    let mut encoder =
                        device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("Render Encoder"),
                        });

                    {
                        let mut render_pass =
                            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("Render Pass"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: &view,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(wgpu::Color {
                                            r: 0.1,
                                            g: 0.1,
                                            b: 0.1,
                                            a: 1.0,
                                        }),
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                depth_stencil_attachment: None,
                                timestamp_writes: None,
                                occlusion_query_set: None,
                            });

                        render_pass.set_pipeline(&render_pipeline);
                        render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                        render_pass
                            .set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint16);

                        let hw = config.width / 2;
                        let hh = config.height / 2;

                        // --- 1. Top-Left: 3D X-Ray (Mode 0) ---
                        // Y starts at 0.0 (Top)
                        render_pass.set_viewport(0.0, 0.0, hw as f32, hh as f32, 0.0, 1.0);
                        render_pass.set_bind_group(0, &diffuse_bind_group, &[0]);
                        render_pass.draw_indexed(0..num_indices, 0, 0..1);

                        // --- 2. Top-Right: Top Slice/XY (Mode 1) ---
                        // X starts at hw (Right), Y starts at 0.0 (Top)
                        render_pass.set_viewport(hw as f32, 0.0, hw as f32, hh as f32, 0.0, 1.0);
                        render_pass.set_bind_group(0, &diffuse_bind_group, &[256]);
                        render_pass.draw_indexed(0..num_indices, 0, 0..1);

                        // --- 3. Bottom-Left: Front Slice/XZ (Mode 2) ---
                        // X starts at 0.0 (Left), Y starts at hh (Bottom)
                        render_pass.set_viewport(0.0, hh as f32, hw as f32, hh as f32, 0.0, 1.0);
                        render_pass.set_bind_group(0, &diffuse_bind_group, &[512]);
                        render_pass.draw_indexed(0..num_indices, 0, 0..1);

                        // --- 4. Bottom-Right: Side Slice/YZ (Mode 3) ---
                        // X starts at hw (Right), Y starts at hh (Bottom)
                        render_pass
                            .set_viewport(hw as f32, hh as f32, hw as f32, hh as f32, 0.0, 1.0);
                        render_pass.set_bind_group(0, &diffuse_bind_group, &[768]);
                        render_pass.draw_indexed(0..num_indices, 0, 0..1);
                    }
                    let screen_descriptor = egui_wgpu::ScreenDescriptor {
                        size_in_pixels: [config.width, config.height],
                        pixels_per_point: window.scale_factor() as f32,
                    };

                    gui.render(&device, &queue, &mut encoder, &view, &screen_descriptor);

                    queue.submit(std::iter::once(encoder.finish()));
                    frame.present();
                }
                _ => {}
            }
        }

        Event::AboutToWait => {
            systems::sys_handle_mouse_drag(&mut world);
            window.request_redraw();
        }
        _ => {}
    });
}
