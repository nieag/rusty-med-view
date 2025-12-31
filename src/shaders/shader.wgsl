struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coords: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

struct Uniforms {
    cursor_pos: vec4<f32>,
    volume_dims: vec4<u32>,
    volume_spacing: vec4<f32>,
    overlay_opacities: vec4<f32>,
    resolution: vec2<f32>,
    mouse_uv: vec2<f32>,
    pan: vec2<f32>,
    zoom_pivot: vec2<f32>,
    rotation: vec4<f32>, // quaternion [x, y, z, w]
    zoom: f32,
    time: f32,
    view_mode: u32,
    overlay_flags: u32,
};

@group(0) @binding(0) var t_diffuse: texture_3d<f32>;
@group(0) @binding(1) var s_diffuse: sampler; // Trilinear sampler
@group(0) @binding(2) var<uniform> uniforms: Uniforms;

// Overlay 1
@group(0) @binding(3) var t_label1: texture_3d<u32>;
@group(0) @binding(4) var t_lut1: texture_1d<f32>;

// Overlay 2
@group(0) @binding(5) var t_label2: texture_3d<u32>;
@group(0) @binding(6) var t_lut2: texture_1d<f32>;

@vertex
fn vs_main(model: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(model.position, 1.0);
    out.uv = model.tex_coords;
    return out;
}

// Helper: Rotates a point
fn rotateY(a: f32) -> mat3x3<f32> {
    let c = cos(a);
    let s = sin(a);
    return mat3x3<f32>(vec3<f32>(c, 0.0, -s), vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(s, 0.0, c));
}

// Ray-Box Intersection
fn intersectAABB(rayOrigin: vec3<f32>, rayDir: vec3<f32>, boxMin: vec3<f32>, boxMax: vec3<f32>) -> vec2<f32> {
    let tMin = (boxMin - rayOrigin) / rayDir;
    let tMax = (boxMax - rayOrigin) / rayDir;
    let t1 = min(tMin, tMax);
    let t2 = max(tMin, tMax);
    let tNear = max(max(t1.x, t1.y), t1.z);
    let tFar = min(min(t2.x, t2.y), t2.z);
    return vec2<f32>(tNear, tFar);
}

// Helper: Get label color from specific overlay slot
fn get_overlay_color(
    tex: texture_3d<u32>,
    lut: texture_1d<f32>,
    uvw: vec3<f32>,
    opacity: f32,
    s_sampler: sampler,
    force_solid: bool
) -> vec4<f32> {
    // Note: We use the same sampler as the main volume for coordinate consistency, 
    // but textures are UINT so no filtering happens on the fetch itself (nearest).
    // Actually, wgpu doesn't allow filtering for UINT textures.
    // We must use `textureLoad` with integer coordinates.

    let dims = vec3<f32>(uniforms.volume_dims.xyz);
    let coords = vec3<i32>(floor(uvw * dims));
    
    // Bounds check
    if any(coords < vec3<i32>(0)) || any(coords >= vec3<i32>(uniforms.volume_dims.xyz)) {
        return vec4<f32>(0.0);
    }

    let label_id = textureLoad(tex, coords, 0).r;

    if label_id == 0u {
        return vec4<f32>(0.0);
    }
    
    // Lookup color
    // We assume LUT is 256 pixels
    let color = textureLoad(lut, i32(label_id) % 256, 0);

    if force_solid {
        return vec4<f32>(color.rgb, 1.0);
    }
    return vec4<f32>(color.rgb, color.a * opacity);
}

// Compute Crosshair alpha
fn get_crosshair_alpha(uv: vec2<f32>, center: vec2<f32>, aspect: f32) -> f32 {
    let thickness = 0.002;
    let blur = 0.001;
    let dist_x = abs(uv.x - center.x) * aspect;
    let dist_y = abs(uv.y - center.y);
    let line_x = 1.0 - smoothstep(thickness, thickness + blur, dist_x);
    let line_y = 1.0 - smoothstep(thickness, thickness + blur, dist_y);
    return max(line_x, line_y);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var final_color = vec4<f32>(0.0);
    let aspect = uniforms.resolution.x / uniforms.resolution.y;

    var crosshair_screen_pos = vec2<f32>(-10.0, -10.0);
    var draw_crosshair = false;

    // --- MODE 1,2,3: 2D SLICES ---
    if uniforms.view_mode > 0u {
        let uv = in.uv;
        var sample_pos = vec3<f32>(0.0);
        let cursor = uniforms.cursor_pos.xyz;

        let zoom = uniforms.zoom;
        let pivot = uniforms.zoom_pivot;
        let pan = uniforms.pan;
        let zoomed_uv = (uv - pivot) / zoom + pivot + pan;

        if any(zoomed_uv < vec2<f32>(0.0)) || any(zoomed_uv > vec2<f32>(1.0)) {
            final_color = vec4<f32>(0.05, 0.05, 0.05, 1.0);
            draw_crosshair = false;
        } else {
            if uniforms.view_mode == 1u { // XY
                sample_pos = vec3<f32>(zoomed_uv.x, zoomed_uv.y, cursor.z);
                crosshair_screen_pos = (cursor.xy - pan - pivot) * zoom + pivot;
            } else if uniforms.view_mode == 2u { // XZ
                sample_pos = vec3<f32>(zoomed_uv.x, cursor.y, zoomed_uv.y);
                crosshair_screen_pos = (cursor.xz - pan - pivot) * zoom + pivot;
            } else if uniforms.view_mode == 3u { // YZ
                sample_pos = vec3<f32>(cursor.x, zoomed_uv.x, zoomed_uv.y);
                crosshair_screen_pos = (cursor.yz - pan - pivot) * zoom + pivot;
            }

            // 1. Sample Main Volume
            final_color = textureSample(t_diffuse, s_diffuse, sample_pos);
            
            // 2. Blend Overlays (if texture not dummy)
            // We can't easily detect "dummy" in shader, so we rely on uniforms or just data being 0
            
            // Overlay 1
            let col1 = get_overlay_color(t_label1, t_lut1, sample_pos, uniforms.overlay_opacities.x, s_diffuse, false);
            if col1.a > 0.0 {
                // Alpha blend: SrcAlpha, OneMinusSrcAlpha
                final_color = vec4<f32>(mix(final_color.rgb, col1.rgb, col1.a), 1.0);
            }
            
            // Overlay 2
            // Overlay 2
            let col2 = get_overlay_color(t_label2, t_lut2, sample_pos, uniforms.overlay_opacities.y, s_diffuse, false);
            if col2.a > 0.0 {
                final_color = vec4<f32>(mix(final_color.rgb, col2.rgb, col2.a), 1.0);
            }

            draw_crosshair = true;
        }
    } else {
        // --- MODE 0: 3D X-RAY ---
        // (Simplified Camera setup same as before)
        let zoom = uniforms.zoom;
        let pivot = uniforms.zoom_pivot;
        let pan = uniforms.pan;
        let zoomed_uv = (in.uv - pivot) / zoom + pivot + pan;
        let screen_pos = vec2<f32>((zoomed_uv.x - 0.5) * aspect, zoomed_uv.y - 0.5);

        // --- Quaternion-based Camera ---
        // Rotation quaternion stored as vec4(x, y, z, w)
        let q = uniforms.rotation;
        
        // Convert quaternion to rotation matrix
        // The camera looks at origin from +Z with this orientation
        let x2 = q.x + q.x;
        let y2 = q.y + q.y;
        let z2 = q.z + q.z;
        let xx = q.x * x2;
        let xy = q.x * y2;
        let xz = q.x * z2;
        let yy = q.y * y2;
        let yz = q.y * z2;
        let zz = q.z * z2;
        let wx = q.w * x2;
        let wy = q.w * y2;
        let wz = q.w * z2;

        // Rotation matrix from quaternion (column-major for WGSL mat3x3)
        let rot_mat = mat3x3<f32>(
            vec3<f32>(1.0 - (yy + zz), xy + wz, xz - wy),
            vec3<f32>(xy - wz, 1.0 - (xx + zz), yz + wx),
            vec3<f32>(xz + wy, yz - wx, 1.0 - (xx + yy))
        );
        
        // Camera position: rotate base position (0, 0, radius) by quaternion
        let radius = 3.5;
        let base_cam_pos = vec3<f32>(0.0, 0.0, -radius);
        let cam_pos = rot_mat * base_cam_pos;

        let cam_target = vec3<f32>(0.0, 0.0, 0.0);
        let forward = normalize(cam_target - cam_pos);
        
        // Right and Up are derived from rotated basis
        // We rotate the standard basis vectors
        let base_right = vec3<f32>(1.0, 0.0, 0.0);
        let base_up = vec3<f32>(0.0, 1.0, 0.0);
        let right = rot_mat * base_right;
        let up = rot_mat * base_up;

        let ray_dir = normalize(forward + right * screen_pos.x + up * screen_pos.y);

        // Calculate aspect ratios for raymarching box
        let dims = vec3<f32>(uniforms.volume_dims.xyz);
        let spacing = uniforms.volume_spacing.xyz;
        let physical_size = dims * spacing;
        let max_dim_vol = max(max(physical_size.x, physical_size.y), physical_size.z);
        let aspect_ratio_vol = physical_size / max_dim_vol;

        // Project Cursor
        let cursor_world = (uniforms.cursor_pos.xyz - 0.5) * aspect_ratio_vol;
        let to_cursor = cursor_world - cam_pos;
        let dist_z = dot(to_cursor, forward);
        if dist_z > 0.0 {
            let dist_x = dot(to_cursor, right);
            let dist_y = dot(to_cursor, up);
            let screen_u = (dist_x / dist_z) / aspect;
            let screen_v = (dist_y / dist_z);
            let p_uv = vec2<f32>(screen_u + 0.5, screen_v + 0.5);
            crosshair_screen_pos = (p_uv - pan - pivot) * zoom + pivot;
            draw_crosshair = true;
        }

        // Raymarching
        let box_min = -0.5 * aspect_ratio_vol;
        let box_max = 0.5 * aspect_ratio_vol;
        let t_hit = intersectAABB(cam_pos, ray_dir, box_min, box_max);

        if t_hit.x > t_hit.y || t_hit.y < 0.0 {
            final_color = vec4<f32>(0.05, 0.05, 0.05, 1.0);
        } else {
            let start_pos = cam_pos + ray_dir * max(t_hit.x, 0.0);
            let total_dist = t_hit.y - max(t_hit.x, 0.0);
            let steps = 128;
            let step_size = total_dist / f32(steps);
            var current_pos = start_pos;
            
            // Accumulators
            var acc_density = 0.0;
            var acc_color = vec3<f32>(0.0);

            for (var i = 0; i < steps; i++) {
                let tex_coord = (current_pos / aspect_ratio_vol) + 0.5;
                
                // 1. Sample Volume (Density)
                let voxel = textureSampleLevel(t_diffuse, s_diffuse, tex_coord, 0.0).r;
                let density = voxel * 0.05; 
                
                // 2. Sample Overlays (Color)
                var overlay_color = vec3<f32>(0.0);
                var overlay_alpha = 0.0;

                let o1 = get_overlay_color(t_label1, t_lut1, tex_coord, uniforms.overlay_opacities.x, s_diffuse, true);
                if o1.a > 0.0 {
                    overlay_color = mix(overlay_color, o1.rgb, o1.a);
                    overlay_alpha = max(overlay_alpha, o1.a);
                }

                let o2 = get_overlay_color(t_label2, t_lut2, tex_coord, uniforms.overlay_opacities.y, s_diffuse, true);
                if o2.a > 0.0 {
                    overlay_color = mix(overlay_color, o2.rgb, o2.a); // Simple mix
                    overlay_alpha = max(overlay_alpha, o2.a);
                }
                
                // Composite
                // If overlay is present, it contributes to color
                // Volume density contributes to grayscale

                var weight = density;
                var sample_rgb = vec3<f32>(voxel);
                var final_rgb = mix(sample_rgb, overlay_color, overlay_alpha);

                if overlay_alpha > 0.0 {
                    // Force solid appearance in 3D: Use full remaining budget
                    weight = 1.0 - acc_density;
                    final_rgb = overlay_color;
                }
                
                // Accumulate
                // This is a simplified "Addtive-ish" blending for X-Ray
                // Improvement: Standard Front-to-Back compositing
                // alpha = (1 - acc_alpha) * sample_alpha
                // col += alpha * sample_col
                
                // Using existing additive style for now to match style
                acc_color += final_rgb * weight;
                acc_density += weight;

                if acc_density >= 1.0 { break; }
                current_pos += ray_dir * step_size;
            }
            final_color = vec4<f32>(acc_color + 0.05, 1.0);
        }
    }

    // Crosshair
    if draw_crosshair {
        let ch_alpha = get_crosshair_alpha(in.uv, crosshair_screen_pos, aspect);
        let ch_color = vec3<f32>(0.0, 1.0, 0.0);
        final_color = vec4<f32>(mix(final_color.rgb, ch_color, ch_alpha * 0.6), 1.0);
    }

    return final_color;
}
