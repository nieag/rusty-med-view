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
