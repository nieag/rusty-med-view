struct Uniforms {
    zoom: f32,
    _pad0: f32,
    pan: vec2<f32>,
    pivot: vec2<f32>,
    view_mode: u32,
    show_zero_isoline: u32,
    resolution: vec2<f32>,
    volume_dims: vec3<u32>,
    current_slice: i32,
    volume_spacing: vec3<f32>,
    opacity: f32,
    sdf_dims: vec3<u32>,
    _pad1: u32,
    value_window_mm: f32,
    _pad2: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(1) @binding(0) var t_sdf: texture_3d<f32>;
@group(1) @binding(1) var s_sdf: sampler;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) screen_uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    var out: VsOut;
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0)
    );
    let p = pos[vid];
    out.clip = vec4<f32>(p, 0.0, 1.0);
    out.screen_uv = vec2<f32>(0.5 * (p.x + 1.0), 1.0 - 0.5 * (p.y + 1.0));
    return out;
}

fn screen_to_volume_uv(screen_uv: vec2<f32>, view_mode: u32) -> vec3<f32> {
    let dx = max(f32(u.volume_dims.x), 1.0) * max(u.volume_spacing.x, 0.001);
    let dy = max(f32(u.volume_dims.y), 1.0) * max(u.volume_spacing.y, 0.001);
    let dz = max(f32(u.volume_dims.z), 1.0) * max(u.volume_spacing.z, 0.001);

    var slice_aspect = 1.0;
    if view_mode == 1u {
        slice_aspect = dx / dy;
    } else if view_mode == 2u {
        slice_aspect = dx / dz;
    } else {
        slice_aspect = dy / dz;
    }

    let screen_aspect = u.resolution.x / max(u.resolution.y, 1.0);
    let k = screen_aspect / slice_aspect;

    let rel = (screen_uv - u.pivot) * vec2<f32>(k, 1.0);
    let zoomed = rel / max(u.zoom, 0.0001) + u.pivot + u.pan;

    var vol = vec3<f32>(0.0);
    if view_mode == 1u {
        vol.x = 1.0 - zoomed.x;
        vol.y = 1.0 - zoomed.y;
        vol.z = (f32(u.current_slice) + 0.5) / max(f32(u.volume_dims.z), 1.0);
    } else if view_mode == 2u {
        vol.x = 1.0 - zoomed.x;
        vol.z = 1.0 - zoomed.y;
        vol.y = (f32(u.current_slice) + 0.5) / max(f32(u.volume_dims.y), 1.0);
    } else {
        vol.y = 1.0 - zoomed.x;
        vol.z = 1.0 - zoomed.y;
        vol.x = (f32(u.current_slice) + 0.5) / max(f32(u.volume_dims.x), 1.0);
    }

    return vol;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if u.view_mode == 0u {
        return vec4<f32>(0.0);
    }

    let uvw = screen_to_volume_uv(in.screen_uv, u.view_mode);
    if any(uvw < vec3<f32>(0.0)) || any(uvw > vec3<f32>(1.0)) {
        return vec4<f32>(0.0);
    }

    let dims = vec3<i32>(u.sdf_dims);
    if any(dims <= vec3<i32>(0)) {
        return vec4<f32>(0.0);
    }

    let clamped_uvw = clamp(uvw, vec3<f32>(0.0), vec3<f32>(0.999999));
    let coord = vec3<i32>(floor(clamped_uvw * vec3<f32>(u.sdf_dims)));
    let sdf = textureLoad(t_sdf, coord, 0).r;

    if sdf > 1.0e19 {
        return vec4<f32>(0.0);
    }

    let w = max(u.value_window_mm, 0.0001);
    let t = clamp(sdf / w, -1.0, 1.0);

    let inside = vec3<f32>(0.18, 0.55, 0.95);
    let outside = vec3<f32>(0.95, 0.48, 0.22);
    let neutral = vec3<f32>(0.88, 0.88, 0.88);

    var color = neutral;
    if t < 0.0 {
        color = mix(neutral, inside, -t);
    } else {
        color = mix(neutral, outside, t);
    }

    var alpha = u.opacity;
    if u.show_zero_isoline != 0u {
        let band = fwidth(sdf) + (0.08 * w);
        let iso = 1.0 - smoothstep(0.0, band, abs(sdf));
        color = mix(color, vec3<f32>(1.0, 1.0, 0.3), iso);
        alpha = max(alpha, iso * 0.8);
    }

    return vec4<f32>(color, alpha);
}
