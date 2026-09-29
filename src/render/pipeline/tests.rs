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
    assert_eq!(std::mem::size_of::<Uniforms>(), 832);
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
    let instance = wgpu::Instance::default();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        compatible_surface: None,
        force_fallback_adapter: true,
    })) else {
        return;
    };
    let Ok((device, _queue)) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
    else {
        return;
    };

    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let layout = create_bind_group_layout(&device);
    let _pipeline = create_render_pipeline(&device, &layout, wgpu::TextureFormat::Rgba8Unorm);
    let error = pollster::block_on(device.pop_error_scope());

    assert!(error.is_none(), "main shader validation failed: {error:?}");
}
