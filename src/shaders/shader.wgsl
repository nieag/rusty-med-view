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
    // ---
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

// --- GPU Overlay Primitives ---
const MAX_OVERLAY_PRIMITIVES: u32 = 64u;
const PRIMITIVE_CIRCLE: u32 = 0u;
const PRIMITIVE_RING: u32 = 1u;
const PRIMITIVE_LINE: u32 = 2u;

struct OverlayPrimitive {
    world_pos: vec4<f32>,     // xyz = position, w = unused
    color: vec4<f32>,         // rgba
    params: vec4<f32>,        // radius, thickness, viewport_mask, kind
    secondary_pos: vec4<f32>, // for lines: end point
};

@group(0) @binding(7) var<storage, read> overlay_primitives: array<OverlayPrimitive, 64>;

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

// Apply windowing transform to intensity value
// Maps [center - width/2, center + width/2] -> [0, 1]
fn apply_window(value: f32, center: f32, width: f32) -> f32 {
    if width <= 0.0 { return 0.5; }
    let low = center - width * 0.5;
    return clamp((value - low) / width, 0.0, 1.0);
}
// Note: Gizmo is now rendered via egui for proper font rendering

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
        let pan = uniforms.pan;
        // Enforce center-based pivot logic matching systems.rs
        // We ignore uniforms.zoom_pivot potentially, or assume it's 0.5. 
        // Let's use 0.5 explicit to match new system logic.
        let pivot = vec2<f32>(0.5, 0.5); 
        
        // Aspect Correction
        // 1. Screen Aspect
        let screen_aspect = uniforms.resolution.x / uniforms.resolution.y;

        // 2. Slice Aspect
        let spacing = uniforms.volume_spacing.xyz;
        let dims = vec3<f32>(uniforms.volume_dims.xyz);
        let vol_aspects = vec3<f32>(
            (dims.x * spacing.x) / (dims.y * spacing.y), // xy
            (dims.x * spacing.x) / (dims.z * spacing.z), // xz
            (dims.y * spacing.y) / (dims.z * spacing.z)  // yz
        );
        // Note: vol_aspects logic from systems.rs:
        // Axial(1): Ratio = X/Y = aspect[0]/aspect[1] -> This assumes aspect_ratios() returns physical unit per pixel?
        // systems.rs: vol.aspect_ratios() returns [d*s, d*s, d*s]. 
        // No, vol.aspect_ratios() in systems is Dimensions * Spacing.
        // So vol_aspects[0] is Width(mm), vol_aspects[1] is Height(mm).
        // Ratio = Width / Height.
        
        // Recompute here:
        // Axial (XY)
        let phys_x = dims.x * spacing.x;
        let phys_y = dims.y * spacing.y;
        let phys_z = dims.z * spacing.z;

        var slice_aspect = 1.0;
        if uniforms.view_mode == 1u { slice_aspect = phys_x / phys_y; } else if uniforms.view_mode == 2u { slice_aspect = phys_x / phys_z; } else if uniforms.view_mode == 3u { slice_aspect = phys_y / phys_z; } // Y over Z

        // Correction K
        let k = screen_aspect / slice_aspect;
        
        // Map Screen UV to "Corrected" relative coord
        // (uv - 0.5) * vec2(k, 1.0)
        let centered_uv = (uv - pivot) * vec2<f32>(k, 1.0);
        
        // Apply Zoom and Pan
        let zoomed_uv = centered_uv / zoom + pivot + pan;

        if any(zoomed_uv < vec2<f32>(0.0)) || any(zoomed_uv > vec2<f32>(1.0)) {
            final_color = vec4<f32>(0.05, 0.05, 0.05, 1.0);
            draw_crosshair = false;
        } else {
            if uniforms.view_mode == 1u { // XY
                sample_pos = vec3<f32>(zoomed_uv.x, zoomed_uv.y, cursor.z);
                // Crosshair calc inverse
                // zoomed_uv = (uv - 0.5)*k/zoom + 0.5 + pan
                // uv - 0.5 = (zoomed_uv - 0.5 - pan) * zoom / k
                // uv = ... + 0.5
                let rel_pos = (cursor.xy - pan - pivot) * zoom;
                crosshair_screen_pos = (rel_pos / vec2<f32>(k, 1.0)) + pivot;
            } else if uniforms.view_mode == 2u { // XZ
                sample_pos = vec3<f32>(zoomed_uv.x, cursor.y, zoomed_uv.y);
                let rel_pos = (cursor.xz - pan - pivot) * zoom;
                crosshair_screen_pos = (rel_pos / vec2<f32>(k, 1.0)) + pivot;
            } else if uniforms.view_mode == 3u { // YZ
                sample_pos = vec3<f32>(cursor.x, zoomed_uv.x, zoomed_uv.y);
                let rel_pos = (cursor.yz - pan - pivot) * zoom;
                crosshair_screen_pos = (rel_pos / vec2<f32>(k, 1.0)) + pivot;
            }

            // 1. Sample Main Volume and apply windowing
            let raw_intensity = textureSample(t_diffuse, s_diffuse, sample_pos).r;
            let windowed = apply_window(raw_intensity, uniforms.window_params.x, uniforms.window_params.y);
            final_color = vec4<f32>(windowed, windowed, windowed, 1.0);
            
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
        // --- MODE 0: 3D X-RAY (Volume-Based Rotation) ---
        // Camera is FIXED, volume rotates
        let zoom = uniforms.zoom;
        let pivot = uniforms.zoom_pivot;
        let pan = uniforms.pan;
        let zoomed_uv = (in.uv - pivot) / zoom + pivot + pan;
        let screen_pos = vec2<f32>((zoomed_uv.x - 0.5) * aspect, zoomed_uv.y - 0.5);

        // --- Volume Rotation Matrix from Quaternion ---
        // This represents the volume's orientation in world space
        let q = uniforms.rotation;
        
        // Convert quaternion to rotation matrix
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

        // Rotation matrix (column-major for WGSL mat3x3)
        // This rotates FROM object space TO world space
        let rot_mat = mat3x3<f32>(
            vec3<f32>(1.0 - (yy + zz), xy + wz, xz - wy),
            vec3<f32>(xy - wz, 1.0 - (xx + zz), yz + wx),
            vec3<f32>(xz + wy, yz - wx, 1.0 - (xx + yy))
        );
        
        // Inverse rotation (transpose for orthogonal matrix)
        // This rotates FROM world space TO object space
        let inv_rot_mat = transpose(rot_mat);
        
        // Fixed camera position in world space
        let radius = 3.5;
        let cam_pos = vec3<f32>(0.0, 0.0, -radius);
        let forward = vec3<f32>(0.0, 0.0, 1.0);
        let right = vec3<f32>(1.0, 0.0, 0.0);
        let up = vec3<f32>(0.0, 1.0, 0.0);

        // Ray direction in world space
        let ray_dir_world = normalize(forward + right * screen_pos.x + up * screen_pos.y);
        
        // Transform ray into object space for volume intersection
        let cam_pos_obj = inv_rot_mat * cam_pos;
        let ray_dir_obj = inv_rot_mat * ray_dir_world;

        // Calculate aspect ratios for raymarching box
        let dims = vec3<f32>(uniforms.volume_dims.xyz);
        let spacing = uniforms.volume_spacing.xyz;
        let physical_size = dims * spacing;
        let max_dim_vol = max(max(physical_size.x, physical_size.y), physical_size.z);
        let aspect_ratio_vol = physical_size / max_dim_vol;

        // Project Cursor (cursor is in object space UV, transform to world for projection)
        let cursor_obj = (uniforms.cursor_pos.xyz - 0.5) * aspect_ratio_vol;
        let cursor_world = rot_mat * cursor_obj;
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

        // Raymarching in object space
        let box_min = -0.5 * aspect_ratio_vol;
        let box_max = 0.5 * aspect_ratio_vol;
        let t_hit = intersectAABB(cam_pos_obj, ray_dir_obj, box_min, box_max);

        if t_hit.x > t_hit.y || t_hit.y < 0.0 {
            final_color = vec4<f32>(0.05, 0.05, 0.05, 1.0);
        } else {
            let start_pos = cam_pos_obj + ray_dir_obj * max(t_hit.x, 0.0);
            let total_dist = t_hit.y - max(t_hit.x, 0.0);
            let steps = 128;
            let step_size = total_dist / f32(steps);
            var current_pos = start_pos;
            
            // Accumulators
            var acc_density = 0.0;
            var acc_color = vec3<f32>(0.0);

            for (var i = 0; i < steps; i++) {
                let tex_coord = (current_pos / aspect_ratio_vol) + 0.5;
                
                // 1. Sample Volume (Density) with windowing
                let raw_voxel = textureSampleLevel(t_diffuse, s_diffuse, tex_coord, 0.0).r;
                let voxel = apply_window(raw_voxel, uniforms.window_params.x, uniforms.window_params.y);
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
                    overlay_color = mix(overlay_color, o2.rgb, o2.a);
                    overlay_alpha = max(overlay_alpha, o2.a);
                }
                
                // Composite
                var weight = density;
                var sample_rgb = vec3<f32>(voxel);
                var final_rgb = mix(sample_rgb, overlay_color, overlay_alpha);

                if overlay_alpha > 0.0 {
                    weight = 1.0 - acc_density;
                    final_rgb = overlay_color;
                }

                acc_color += final_rgb * weight;
                acc_density += weight;

                if acc_density >= 1.0 { break; }
                current_pos += ray_dir_obj * step_size;
            }
            final_color = vec4<f32>(acc_color + 0.05, 1.0);
        }
        // Gizmo is now rendered via egui, not shader
    }

    // Crosshair
    if draw_crosshair {
        let ch_alpha = get_crosshair_alpha(in.uv, crosshair_screen_pos, aspect);
        let ch_color = vec3<f32>(0.0, 1.0, 0.0);
        final_color = vec4<f32>(mix(final_color.rgb, ch_color, ch_alpha * 0.6), 1.0);
    }

    // --- Overlay Primitives ---
    // Viewport mask bits: 1=3D(0), 2=Axial(1), 4=Coronal(2), 8=Sagittal(3)
    let viewport_bit = 1u << uniforms.view_mode;

    for (var i = 0u; i < uniforms.overlay_primitive_count; i++) {
        if i >= MAX_OVERLAY_PRIMITIVES { break; }

        let prim = overlay_primitives[i];
        let viewport_mask = u32(prim.params.z);
        
        // Skip if not visible in this viewport
        if (viewport_mask & viewport_bit) == 0u { continue; }

        let kind = u32(prim.params.w);
        let radius = prim.params.x;
        let prim_color = prim.color;
        
        // Get world position (use mouse if this is the dragged primitive)
        var world_pos = prim.world_pos.xyz;
        var use_mouse_pos = false;
        if uniforms.overlay_dragging_idx == i {
            use_mouse_pos = true;
        }
        
        // Project to screen UV based on viewport mode
        var screen_pos = vec2<f32>(-10.0, -10.0);
        var visible = false;

        if uniforms.view_mode > 0u {
            // --- 2D Viewport projection ---
            let zoom = uniforms.zoom;
            let pan = uniforms.pan;
            let pivot = vec2<f32>(0.5, 0.5);
            
            // Calculate aspect correction
            let screen_aspect_ratio = uniforms.resolution.x / uniforms.resolution.y;
            let spacing = uniforms.volume_spacing.xyz;
            let dims = vec3<f32>(uniforms.volume_dims.xyz);
            let phys_x = dims.x * spacing.x;
            let phys_y = dims.y * spacing.y;
            let phys_z = dims.z * spacing.z;

            var slice_aspect = 1.0;
            var uv = vec2<f32>(0.0);

            if uniforms.view_mode == 1u {
                // Axial (XY)
                slice_aspect = phys_x / phys_y;
                if use_mouse_pos {
                    uv = uniforms.overlay_mouse_uv;
                } else {
                    uv = world_pos.xy;
                }
            } else if uniforms.view_mode == 2u {
                // Coronal (XZ)
                slice_aspect = phys_x / phys_z;
                if use_mouse_pos {
                    uv = uniforms.overlay_mouse_uv;
                } else {
                    uv = vec2<f32>(world_pos.x, world_pos.z);
                }
            } else if uniforms.view_mode == 3u {
                // Sagittal (YZ)
                slice_aspect = phys_y / phys_z;
                if use_mouse_pos {
                    uv = uniforms.overlay_mouse_uv;
                } else {
                    uv = vec2<f32>(world_pos.y, world_pos.z);
                }
            }

            let k = screen_aspect_ratio / slice_aspect;
            
            // Project: world UV -> screen UV
            // Inverse of: volume_uv = ((screen_uv - 0.5) * k / zoom) + 0.5 + pan
            // screen_uv = ((volume_uv - 0.5 - pan) * zoom / k) + 0.5
            let rel_pos = (uv - pan - pivot) * zoom;
            screen_pos = (rel_pos / vec2<f32>(k, 1.0)) + pivot;
            visible = screen_pos.x >= 0.0 && screen_pos.x <= 1.0 && screen_pos.y >= 0.0 && screen_pos.y <= 1.0;
        } else {
            // --- 3D Viewport projection (similar to crosshair) ---
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

            let dims = vec3<f32>(uniforms.volume_dims.xyz);
            let spacing = uniforms.volume_spacing.xyz;
            let physical_size = dims * spacing;
            let max_dim_vol = max(max(physical_size.x, physical_size.y), physical_size.z);
            let aspect_ratio_vol = physical_size / max_dim_vol;

            let pos_obj = (world_pos - 0.5) * aspect_ratio_vol;
            let pos_world = rot_mat * pos_obj;

            let cam_pos = vec3<f32>(0.0, 0.0, -3.5);
            let forward = vec3<f32>(0.0, 0.0, 1.0);
            let right = vec3<f32>(1.0, 0.0, 0.0);
            let up = vec3<f32>(0.0, 1.0, 0.0);

            let to_prim = pos_world - cam_pos;
            let dist_z = dot(to_prim, forward);
            if dist_z > 0.0 {
                let dist_x = dot(to_prim, right);
                let dist_y = dot(to_prim, up);
                let proj_u = (dist_x / dist_z) / aspect;
                let proj_v = dist_y / dist_z;
                let p_uv = vec2<f32>(proj_u + 0.5, proj_v + 0.5);

                let zoom = uniforms.zoom;
                let pan = uniforms.pan;
                let pivot = uniforms.zoom_pivot;
                screen_pos = (p_uv - pan - pivot) * zoom + pivot;
                visible = true;
            }
        }

        if !visible { continue; }
        
        // Distance from current pixel to primitive center (in UV space)
        let dist = length(in.uv - screen_pos);
        
        // Convert radius from world units to screen units (approximate)
        let screen_radius = radius * uniforms.zoom * 0.02; // Scale factor

        if kind == PRIMITIVE_CIRCLE {
            // Filled circle with soft edge
            let edge_softness = 0.003;
            let alpha_circle = 1.0 - smoothstep(screen_radius - edge_softness, screen_radius + edge_softness, dist);
            if alpha_circle > 0.0 {
                final_color = vec4<f32>(mix(final_color.rgb, prim_color.rgb, alpha_circle * prim_color.a), 1.0);
            }
        } else if kind == PRIMITIVE_RING {
            // Hollow ring
            let thickness = prim.params.y * uniforms.zoom * 0.02;
            let inner_dist = abs(dist - screen_radius);
            let edge_softness = 0.002;
            let alpha_ring = 1.0 - smoothstep(thickness - edge_softness, thickness + edge_softness, inner_dist);
            if alpha_ring > 0.0 {
                final_color = vec4<f32>(mix(final_color.rgb, prim_color.rgb, alpha_ring * prim_color.a), 1.0);
            }
        }
        // Line rendering would go here for PRIMITIVE_LINE
    }

    return final_color;
}
