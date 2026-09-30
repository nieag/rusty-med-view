use super::*;

#[test]
fn test_uniform_stride_covers_current_uniform_size() {
    let uniform_size = std::mem::size_of::<Uniforms>() as u64;
    assert!(UNIFORM_STRIDE >= uniform_size);
    assert_eq!(UNIFORM_STRIDE % UNIFORM_ALIGNMENT, 0);
}

#[test]
fn test_eight_overlay_uniform_array_matches_wgsl_alignment() {
    assert_eq!(std::mem::size_of::<VoxelOverlayUniform>(), 80);
    assert_eq!(std::mem::size_of::<Uniforms>(), 816);
    assert_eq!(std::mem::offset_of!(Uniforms, voxel_overlays), 48);
    assert_eq!(std::mem::offset_of!(Uniforms, window_params), 688);
    assert_eq!(std::mem::offset_of!(Uniforms, oblique_origin_uv), 752);
    assert_eq!(std::mem::offset_of!(Uniforms, oblique_u_dir_length), 768);
    assert_eq!(std::mem::offset_of!(Uniforms, oblique_v_dir_length), 784);
    assert_eq!(std::mem::offset_of!(Uniforms, voxel_overlays) % 16, 0);
    assert_eq!(std::mem::offset_of!(Uniforms, window_params) % 16, 0);
    assert_eq!(std::mem::offset_of!(Uniforms, oblique_origin_uv) % 16, 0);
    assert_eq!(std::mem::offset_of!(Uniforms, oblique_u_dir_length) % 16, 0);
    assert_eq!(std::mem::offset_of!(Uniforms, oblique_v_dir_length) % 16, 0);
}

#[test]
fn test_uniform_buffer_size_covers_all_viewport_slots() {
    let last_viewport_offset = (MAX_VIEWPORTS - 1) * UNIFORM_STRIDE;
    let uniform_size = std::mem::size_of::<Uniforms>() as u64;
    let buffer_size = UNIFORM_STRIDE * MAX_VIEWPORTS;
    assert!(last_viewport_offset + uniform_size <= buffer_size);
}

#[test]
fn test_main_shader_and_eight_overlay_bindings_validate() {
    let Some((_adapter, device, _queue)) = crate::render::test_device() else {
        return;
    };

    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let layout = create_bind_group_layout(&device);
    let _pipeline = create_render_pipeline(&device, &layout, wgpu::TextureFormat::Rgba8Unorm);
    let error = pollster::block_on(device.pop_error_scope());

    assert!(error.is_none(), "main shader validation failed: {error:?}");
}

#[test]
fn test_camera_motion_is_still_at_rest_moving_on_change_and_settles() {
    let mut motion = CameraMotion::default();
    let viewport = hecs::Entity::DANGLING;
    let still = [0.0_f32; CAMERA_KEY_LEN];
    let start = Instant::now() + std::time::Duration::from_secs(10);
    let second = std::time::Duration::from_secs(1);

    assert!(
        !motion.is_moving(viewport, still, start),
        "a first sight is not motion"
    );
    assert!(!motion.is_moving(viewport, still, start + second));

    // Any component of the key counts, e.g. the windowing changing.
    let mut cursor_moved = still;
    cursor_moved[8] = 0.2;
    let t = start + 2 * second;
    assert!(
        motion.is_moving(viewport, cursor_moved, t),
        "a change is motion"
    );
    assert!(motion.is_moving(
        viewport,
        cursor_moved,
        t + std::time::Duration::from_millis(100)
    ));
    assert!(
        !motion.is_moving(
            viewport,
            cursor_moved,
            t + CAMERA_SETTLE + std::time::Duration::from_millis(1)
        ),
        "still for the settle time returns to full quality"
    );

    let mut rotated = cursor_moved;
    rotated[3] = 0.3;
    assert!(motion.is_moving(viewport, rotated, t + 3 * second));
}

fn validate_wgsl(name: &str, source: &str) -> naga::Module {
    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|error| panic!("{name} does not parse:\n{}", error.emit_to_string(source)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap_or_else(|error| panic!("{name} does not validate: {error:?}"));
    module
}

#[test]
fn test_shaders_parse_and_validate_with_their_entry_points() {
    let main = validate_wgsl("shader.wgsl", include_str!("../../shaders/shader.wgsl"));
    let entries: Vec<_> = main.entry_points.iter().map(|e| e.name.as_str()).collect();
    for entry in ["vs_main", "fs_main", "fs_march_3d", "fs_overlay_3d"] {
        assert!(
            entries.contains(&entry),
            "shader.wgsl lacks {entry}: {entries:?}"
        );
    }
    validate_wgsl(
        "mesh_overlay.wgsl",
        include_str!("../../shaders/mesh_overlay.wgsl"),
    );
    validate_wgsl("blit_3d.wgsl", include_str!("../../shaders/blit_3d.wgsl"));
    validate_wgsl(
        "contour_overlay.wgsl",
        include_str!("../../shaders/contour_overlay.wgsl"),
    );
}

/// Renders one 2D view of an empty volume with the real shader and returns, in pixels, where the
/// crosshair's horizontal and vertical lines landed (the rows and columns they cover most).
fn shader_crosshair_position(
    mode: ViewMode,
    cursor_uv: [f32; 3],
    zoom: f32,
    pan: [f32; 2],
    dims: [u32; 3],
    spacing: [f32; 3],
) -> Option<[f32; 2]> {
    const WIDTH: u32 = 448; // 448 * 4 bytes is a multiple of the 256-byte copy row alignment
    const HEIGHT: u32 = 300;
    let (_adapter, device, queue) = crate::render::test_device()?;
    let (_volume, volume_view, volume_sampler) =
        crate::io::volume::create_dummy_r32_texture(&device, &queue);
    let (_dummy, dummy_view, _) = crate::io::volume::create_dummy_r8_texture(&device, &queue);
    let (_lut, lut_view) = crate::io::volume::create_default_colormap(&device, &queue);
    let layout = create_bind_group_layout(&device);
    let uniform_buffer = create_uniform_buffer(&device);
    let bind_group = create_scene_bind_group(
        &device,
        &layout,
        &SceneTextureViews {
            volume_view: &volume_view,
            volume_sampler: &volume_sampler,
            uniform_buffer: &uniform_buffer,
            overlay_views: [&dummy_view; MAX_VOXEL_OVERLAY_SLOTS],
            overlay_lut: &lut_view,
        },
    );
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let pipeline = create_render_pipeline(&device, &layout, format);
    let (vertex_buffer, index_buffer, index_count) = create_geometry_buffers(&device);

    let mut uniforms: Uniforms = bytemuck::Zeroable::zeroed();
    uniforms.cursor_pos = [cursor_uv[0], cursor_uv[1], cursor_uv[2], 0.0];
    uniforms.volume_dims = [dims[0], dims[1], dims[2], 0];
    uniforms.volume_spacing = [spacing[0], spacing[1], spacing[2], 0.0];
    uniforms.window_params = [40.0, 400.0, -1000.0, 1000.0];
    uniforms.resolution = [WIDTH as f32, HEIGHT as f32];
    uniforms.pan = pan;
    uniforms.zoom_pivot = [0.5, 0.5];
    uniforms.rotation = [0.0, 0.0, 0.0, 1.0];
    uniforms.zoom = zoom;
    uniforms.view_mode = mode as u32;
    uniforms.ray_steps = crate::systems::FULL_RAY_STEPS;
    queue.write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("crosshair test target"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("crosshair test readback"),
        size: (WIDTH * 4 * HEIGHT) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("crosshair test pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        pass.set_bind_group(0, &bind_group, &[0]);
        pass.draw_indexed(0..index_count, 0, 0..1);
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 4),
                rows_per_image: Some(HEIGHT),
            },
        },
        wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let pixels = readback.slice(..).get_mapped_range().to_vec();

    // Line colours by axis: X red, Y green, Z blue. The horizontal line is the axis across the
    // screen and the vertical one the axis up it.
    let (horizontal_channel, vertical_channel) = match mode {
        ViewMode::Axial => (0, 1),
        ViewMode::Coronal => (0, 2),
        ViewMode::Sagittal => (1, 2),
        _ => return None,
    };
    let pixel = |x: u32, y: u32| {
        let i = ((y * WIDTH + x) * 4) as usize;
        [pixels[i] as i32, pixels[i + 1] as i32, pixels[i + 2] as i32]
    };
    // The lines are thinner than a pixel, so each pixel gets only a fraction of their colour.
    // Weight every pixel by how far it leans toward the line colour over the grey background and
    // take the centre of mass across the image.
    let mut red_rows = vec![0f32; HEIGHT as usize];
    let mut green_columns = vec![0f32; WIDTH as usize];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let rgb = pixel(x, y);
            // How far the pixel leans toward one channel's line colour over the other two.
            let lean = |c: usize| {
                (0..3)
                    .filter(|o| *o != c)
                    .map(|o| rgb[c] - rgb[o])
                    .min()
                    .unwrap_or(0)
                    .max(0) as f32
            };
            red_rows[y as usize] += lean(horizontal_channel);
            green_columns[x as usize] += lean(vertical_channel);
        }
    }
    let centre = |weights: &[f32]| {
        let total: f32 = weights.iter().sum();
        (total > 50.0).then(|| {
            weights
                .iter()
                .enumerate()
                .map(|(i, w)| (i as f32 + 0.5) * w)
                .sum::<f32>()
                / total
        })
    };
    Some([centre(&green_columns)?, centre(&red_rows)?])
}

/// The Rust mapping that picking, contour editing, and the egui markers use must put a point where
/// the shader draws the crosshair, for every 2D view, zoom, and pan.
#[test]
fn test_shader_crosshair_agrees_with_the_rust_viewport_mapping() {
    let dims = [180_u32, 180, 125];
    let spacing = [2.0_f32, 2.0, 3.0];
    let geometry =
        crate::model::VoxelGeometry::new(dims, spacing, [0.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap();
    let cursor_uv = [0.3_f32, 0.6, 0.45];
    let cases = [
        (
            ViewMode::Axial,
            crate::convert::PlaneFamily::Axial,
            1.0,
            [0.0, 0.0],
        ),
        (
            ViewMode::Axial,
            crate::convert::PlaneFamily::Axial,
            1.7,
            [0.05, -0.03],
        ),
        (
            ViewMode::Coronal,
            crate::convert::PlaneFamily::Coronal,
            1.0,
            [0.0, 0.0],
        ),
        (
            ViewMode::Coronal,
            crate::convert::PlaneFamily::Coronal,
            1.3,
            [-0.04, 0.02],
        ),
        (
            ViewMode::Sagittal,
            crate::convert::PlaneFamily::Sagittal,
            1.0,
            [0.0, 0.0],
        ),
        (
            ViewMode::Sagittal,
            crate::convert::PlaneFamily::Sagittal,
            1.5,
            [0.03, 0.05],
        ),
    ];
    for (mode, family, zoom, pan) in cases {
        let Some(shader_px) = shader_crosshair_position(mode, cursor_uv, zoom, pan, dims, spacing)
        else {
            // No GPU adapter, or the crosshair is not visible at all (which fails below).
            if crate::render::test_device().is_none() {
                return;
            }
            panic!("{mode:?} zoom {zoom}: no crosshair found in the rendered image");
        };
        let plane =
            crate::convert::orthogonal_plane_from_volume_uv(family, cursor_uv, geometry).unwrap();
        let mapping = crate::convert::ViewportMapping {
            zoom,
            pan,
            pivot: [0.5, 0.5],
            screen_aspect: 448.0 / 300.0,
        };
        let uv =
            crate::convert::volume_uv_to_viewport_uv(cursor_uv, plane, geometry, mapping).unwrap();
        let rust_px = [uv[0] * 448.0, uv[1] * 300.0];
        assert!(
            (shader_px[0] - rust_px[0]).abs() < 2.0 && (shader_px[1] - rust_px[1]).abs() < 2.0,
            "{mode:?} zoom {zoom} pan {pan:?}: shader draws the crosshair at {shader_px:?} px, \
             the Rust mapping says {rust_px:?}"
        );
    }
}
