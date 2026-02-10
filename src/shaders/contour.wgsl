// contour.wgsl — BASELINE: exact copy of Step 1 that worked
// Raw UV → clip space. No projection. No filtering.

struct Uniforms {
    zoom: f32,
    _pad0: f32,
    pan_x: f32,
    pan_y: f32,
    pivot_x: f32,
    pivot_y: f32,
    view_mode: u32,
    _pad1: u32,
    res_x: f32,
    res_y: f32,
    _pad2a: f32,
    _pad2b: f32,
    dim_x: u32,
    dim_y: u32,
    dim_z: u32,
    _pad3: u32,
    sp_x: f32,
    sp_y: f32,
    sp_z: f32,
    current_slice: i32,
}

struct ContourLine {
    p0: vec3<f32>,
    _pad0: f32,
    p1: vec3<f32>,
    _pad1: f32,
    color: vec4<f32>,
    plane_info: vec4<f32>, // x: view_mode, y: slice, z/w: unused
}

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var<storage, read> lines: array<ContourLine>;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

// Project volume UV (0-1) to screen UV (0-1).
// Inverse of main shader's screen→volume transform.
fn volume_to_screen(vol: vec3<f32>) -> vec2<f32> {
    // Step 1: Radiological flip — volume UV → "zoomed" UV
    var zoomed: vec2<f32>;
    if u.view_mode == 1u {        // Axial (XY)
        zoomed = vec2<f32>(1.0 - vol.x, 1.0 - vol.y);
    } else if u.view_mode == 2u { // Coronal (XZ)
        zoomed = vec2<f32>(1.0 - vol.x, 1.0 - vol.z);
    } else {                       // Sagittal (YZ)
        zoomed = vec2<f32>(1.0 - vol.y, 1.0 - vol.z);
    }

    // Step 2: Aspect correction K
    let dx = max(f32(u.dim_x), 1.0) * max(u.sp_x, 0.001);
    let dy = max(f32(u.dim_y), 1.0) * max(u.sp_y, 0.001);
    let dz = max(f32(u.dim_z), 1.0) * max(u.sp_z, 0.001);

    var slice_aspect = 1.0;
    if u.view_mode == 1u {
        slice_aspect = dx / dy;
    } else if u.view_mode == 2u {
        slice_aspect = dx / dz;
    } else {
        slice_aspect = dy / dz;
    }
    let screen_aspect = u.res_x / max(u.res_y, 1.0);
    let k = screen_aspect / slice_aspect;

    // Step 3: Apply inverse zoom+pan
    let pivot = vec2<f32>(0.5, 0.5);
    let pan = vec2<f32>(u.pan_x, u.pan_y);
    let rel = (zoomed - pivot - pan) * u.zoom;
    let screen_uv = rel / vec2<f32>(k, 1.0) + pivot;
    return screen_uv;
}

@vertex
fn vs_main(
    @builtin(vertex_index) vid: u32,
    @builtin(instance_index) iid: u32,
) -> VertexOutput {
    var out: VertexOutput;
    let line = lines[iid];

    // Step 1: Slice & View Mode Filtering
    // plane_info.x = view_mode, plane_info.y = slice
    if u32(line.plane_info.x) != u.view_mode || i32(line.plane_info.y) != u.current_slice {
        out.clip_position = vec4<f32>(0.0, 0.0, 0.0, 0.0);
        return out;
    }

    // Select endpoint: 0,1 = p0; 2,3 = p1
    var uv: vec2<f32>;
    // Select endpoint (0,1 → p0; 2,3 → p1)
    var vol_pos: vec3<f32>;
    if vid < 2u {
        vol_pos = line.p0;
    } else {
        vol_pos = line.p1;
    }

    // Step 2b: Apply full volume-to-screen projection
    let screen_uv = volume_to_screen(vol_pos);
    let screen_p0 = volume_to_screen(line.p0);
    let screen_p1 = volume_to_screen(line.p1);

    // Screen UV (0-1) → clip space (-1, +1), Y flipped
    let clip = vec2<f32>(screen_uv.x * 2.0 - 1.0, 1.0 - screen_uv.y * 2.0);

    // Perpendicular offset for line thickness
    let p0c = vec2<f32>(screen_p0.x * 2.0 - 1.0, 1.0 - screen_p0.y * 2.0);
    let p1c = vec2<f32>(screen_p1.x * 2.0 - 1.0, 1.0 - screen_p1.y * 2.0);
    let dir = p1c - p0c;
    let len = length(dir);

    var perp = vec2<f32>(0.0, 1.0);
    if len > 0.0001 {
        let n = dir / len;
        perp = vec2<f32>(-n.y, n.x);
    }

    let thickness = 3.0 / min(u.res_x, u.res_y);
    let side = select(-1.0, 1.0, vid % 2u == 0u);

    out.clip_position = vec4<f32>(clip + perp * thickness * side, 0.0, 1.0);
    out.color = line.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
