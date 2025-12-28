struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coords: vec2<f32>,
}
struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
struct Uniforms {
    resolution: vec2<f32>,
    time: f32,
    view_mode: u32, // 0=3D, 1=Top(XY), 2=Front(XZ), 3=Side(YZ)
    cursor_pos: vec4<f32>
};

@group(0) @binding(0) var t_diffuse: texture_3d<f32>;
@group(0) @binding(1) var s_diffuse: sampler;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;

@vertex
fn vs_main(model: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(model.position, 1.0);
    out.uv = model.tex_coords;
    return out;
}

// Update helper to accept a specific target position
fn get_crosshair_alpha(uv: vec2<f32>, center: vec2<f32>, aspect: f32) -> f32 {
    let thickness = 0.002;
    let blur = 0.001;

    // Compare UV against the requested 'center' instead of fixed 0.5
    let dist_x = abs(uv.x - center.x) * aspect;
    let dist_y = abs(uv.y - center.y);

    let line_x = 1.0 - smoothstep(thickness, thickness + blur, dist_x);
    let line_y = 1.0 - smoothstep(thickness, thickness + blur, dist_y);

    return max(line_x, line_y);
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

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var final_color = vec4<f32>(0.0);
    let aspect = uniforms.resolution.x / uniforms.resolution.y;

    // Where should the crosshair be drawn on the screen? (UV 0.0 to 1.0)
    var crosshair_screen_pos = vec2<f32>(-10.0, -10.0); // Default off-screen
    var draw_crosshair = false;

    // --- MODE 1,2,3: 2D SLICES ---
    if (uniforms.view_mode > 0u) {
        let uv = in.uv;
        var sample_pos = vec3<f32>(0.0);
        let cursor = uniforms.cursor_pos.xyz;

        // Map cursor 3D position to 2D Screen UVs for each view
        if (uniforms.view_mode == 1u) {
            // Top View (XY)
            sample_pos = vec3<f32>(uv.x, uv.y, cursor.z);
            crosshair_screen_pos = cursor.xy;
        }
        if (uniforms.view_mode == 2u) {
            // Front View (XZ)
            sample_pos = vec3<f32>(uv.x, cursor.y, uv.y);
            crosshair_screen_pos = vec2<f32>(cursor.x, cursor.z);
        }
        if (uniforms.view_mode == 3u) {
            // Side View (YZ)
            sample_pos = vec3<f32>(cursor.x, uv.x, uv.y);
            crosshair_screen_pos = vec2<f32>(cursor.y, cursor.z);
        }

        final_color = textureSample(t_diffuse, s_diffuse, sample_pos);
        draw_crosshair = true;

    } else {
        // --- MODE 0: 3D X-RAY WITH PROJECTED CURSOR ---

        // 1. Camera Setup
        let uv = (in.uv - 0.5);
        let screen_pos = vec2<f32>(uv.x * aspect, uv.y);

        let radius = 3.5;
        let cam_pos = vec3<f32>(0.0, 0.0, -radius);
        let cam_target = vec3<f32>(0.0, 0.0, 0.0);

        let forward = normalize(cam_target - cam_pos);
        let right = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), forward));
        let up = cross(forward, right);
        let ray_dir = normalize(forward + right * screen_pos.x + up * screen_pos.y);

        // 2. PROJECT CURSOR ONTO SCREEN (The New Math)
        let cursor_world = uniforms.cursor_pos.xyz - 0.5;
        let to_cursor = cursor_world - cam_pos;
        let dist_z = dot(to_cursor, forward);

        // Only draw if the cursor is in front of the camera
        if (dist_z > 0.0) {
            // Project onto Camera Plane
            let dist_x = dot(to_cursor, right);
            let dist_y = dot(to_cursor, up);

            // Perspective Divide: (x / z, y / z)
            // We must reverse the aspect ratio math we did for ray_dir
            let screen_u = (dist_x / dist_z) / aspect;
            let screen_v = (dist_y / dist_z);

            // Map back to 0..1 UV space
            crosshair_screen_pos = vec2<f32>(screen_u + 0.5, screen_v + 0.5);
            draw_crosshair = true;
        }

        // 3. Standard Raymarching (Simplified for brevity)
        let t_hit = intersectAABB(cam_pos, ray_dir, vec3<f32>(-0.5), vec3<f32>(0.5));

        if (t_hit.x > t_hit.y || t_hit.y < 0.0) {
             final_color = vec4<f32>(0.05, 0.05, 0.05, 1.0);
        } else {
            let start_pos = cam_pos + ray_dir * max(t_hit.x, 0.0);
            let total_dist = t_hit.y - max(t_hit.x, 0.0);
            let steps = 128;
            let step_size = total_dist / f32(steps);

            var current_pos = start_pos;
            var acc_density = 0.0;
            var acc_color = vec3<f32>(0.0);

            for (var i = 0; i < steps; i++) {
                let tex_coord = current_pos + 0.5;
                let voxel = textureSampleLevel(t_diffuse, s_diffuse, tex_coord, 0.0);
                if (voxel.a > 0.0) {
                     let weight = voxel.a * 0.05;
                     acc_color += voxel.rgb * weight;
                     acc_density += weight;
                }
                if (acc_density >= 1.0) { break; }
                current_pos += ray_dir * step_size;
            }
            final_color = vec4<f32>(acc_color + 0.05, 1.0);
        }
    }

    // --- DRAW CROSSHAIR ---
    if (draw_crosshair) {
        let ch_alpha = get_crosshair_alpha(in.uv, crosshair_screen_pos, aspect);
        // Make the 3D crosshair slightly brighter/distinct
        let ch_color = vec3<f32>(0.0, 1.0, 0.0);
        final_color = vec4<f32>(mix(final_color.rgb, ch_color, ch_alpha * 0.6), 1.0);
    }

    return final_color;
}
