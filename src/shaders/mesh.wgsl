struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
}

struct Uniforms {
    cursor_pos: vec4<f32>,
    volume_dims: vec4<u32>,
    volume_spacing: vec4<f32>,
    overlay_opacities: vec4<f32>,
    window_params: vec4<f32>,        // [center, width, data_min, data_max]
    resolution: vec2<f32>,
    mouse_uv: vec2<f32>,
    pan: vec2<f32>,
    zoom_pivot: vec2<f32>,
    rotation: vec4<f32>, // quaternion [x, y, z, w]
    // Overlay primitive fields
    overlay_mouse_uv: vec2<f32>,     // Mouse position for dragged primitive
    overlay_primitive_count: u32,    // Number of active primitives
    overlay_dragging_idx: u32,       // Index being dragged (0xFFFFFFFF = none)
    // Brush preview
    brush_preview: vec4<f32>,        // [brush_size, active, viewport, _]
    brush_center_voxel: vec4<f32>,   // [voxel_x, voxel_y, voxel_z, valid]
    // --- Orientation Support ---
    volume_orientation_quat: vec4<f32>,
    slice_axis_mapping: vec4<u32>,
    slice_axis_flips: vec4<u32>,
    // ---
    zoom: f32,
    view_mode: u32,
    overlay_flags: u32,
    _padding: u32,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

@vertex
fn vs_main(model: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    
    // 1. Calculate volume aspect ratio (matching shader.wgsl)
    let dims = vec3<f32>(uniforms.volume_dims.xyz);
    let spacing = uniforms.volume_spacing.xyz;
    let physical_size = dims * spacing;
    let max_dim_vol = max(max(physical_size.x, physical_size.y), physical_size.z);
    let aspect_ratio_vol = physical_size / max_dim_vol;

    // 2. Quaternion to rotation matrix (Object -> World)
    let q = uniforms.rotation;
    let x2 = q.x + q.x; let y2 = q.y + q.y; let z2 = q.z + q.z;
    let xx = q.x * x2; let xy = q.x * y2; let xz = q.x * z2;
    let yy = q.y * y2; let yz = q.y * z2; let zz = q.z * z2;
    let wx = q.w * x2; let wy = q.w * y2; let wz = q.w * z2;
    let rot_mat = mat3x3<f32>(
        vec3<f32>(1.0 - (yy + zz), xy + wz, xz - wy),
        vec3<f32>(xy - wz, 1.0 - (xx + zz), yz + wx),
        vec3<f32>(xz + wy, yz - wx, 1.0 - (xx + yy))
    );

    // 3. Object space to world space
    let obj_pos = (model.position - 0.5) * aspect_ratio_vol;
    let world_pos = rot_mat * obj_pos;

    // 4. Fixed camera configuration (matching shader.wgsl)
    let radius = 3.5;
    let cam_pos = vec3<f32>(0.0, 0.0, -radius);
    let forward = vec3<f32>(0.0, 0.0, 1.0);
    let right = vec3<f32>(1.0, 0.0, 0.0);
    let up = vec3<f32>(0.0, 1.0, 0.0);

    let to_pos = world_pos - cam_pos;
    let dist_z = dot(to_pos, forward);
    let dist_x = dot(to_pos, right);
    let dist_y = dot(to_pos, up);

    // 5. Radiological Projection (matching shader.wgsl)
    let screen_aspect = uniforms.resolution.x / uniforms.resolution.y;
    let raw_u = -(dist_x / dist_z) / screen_aspect;
    let raw_v = -(dist_y / dist_z);
    let p_uv = vec2<f32>(raw_u + 0.5, raw_v + 0.5);

    // 6. Apply Zoom/Pan/Pivot
    let zoom = uniforms.zoom;
    let pan = uniforms.pan;
    let pivot = vec2<f32>(0.5, 0.5);
    let final_uv = (p_uv - pan - pivot) * zoom + pivot;

    // 7. Final NDC coordinates
    out.clip_position = vec4<f32>(
        (final_uv.x - 0.5) * 2.0,
        (0.5 - final_uv.y) * 2.0,
        (dist_z - 1.0) / 10.0, // Linear depth in 0..1 range
        1.0
    );

    out.world_pos = world_pos;
    out.normal = rot_mat * model.normal;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let light_dir = normalize(vec3<f32>(0.5, 0.5, -1.0));
    let normal = normalize(in.normal);
    let diffuse = max(dot(normal, light_dir), 0.2);

    let color = vec3<f32>(0.0, 0.8, 1.0); // Cyan mesh
    return vec4<f32>(color * diffuse, 1.0);
}
