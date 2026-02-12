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

    // Determine depth axis for current view
    var depth_axis: u32;
    var dim_depth: u32;
    if u.view_mode == 1u { // Axial (XY), Depth Z
        depth_axis = 2u;
        dim_depth = u.dim_z;
    } else if u.view_mode == 2u { // Coronal (XZ), Depth Y
        depth_axis = 1u;
        dim_depth = u.dim_y;
    } else { // Sagittal (YZ), Depth X
        depth_axis = 0u;
        dim_depth = u.dim_x;
    }
    let slice_uv = (f32(u.current_slice) + 0.5) / f32(max(dim_depth, 1u));

    let source_view_mode = u32(line.plane_info.x);
    var p0_uv = line.p0;
    var p1_uv = line.p1;
    var is_intersection = false;

    if source_view_mode == u.view_mode {
        // Standard in-plane rendering: Only show if slice matches
        if i32(line.plane_info.y) != u.current_slice {
            out.clip_position = vec4<f32>(0.0, 0.0, 0.0, 0.0);
            return out;
        }
    } else {
        // Cross-plane intersection rendering
        let d0 = p0_uv[depth_axis] - slice_uv;
        let d1 = p1_uv[depth_axis] - slice_uv;

        if d0 * d1 > 0.0 {
            // Both points on same side of slice plane, no intersection
            out.clip_position = vec4<f32>(0.0, 0.0, 0.0, 0.0);
            return out;
        }

        // Calculate intersection point p_int
        let denom = p1_uv[depth_axis] - p0_uv[depth_axis];
        var t = 0.5;
        if abs(denom) > 1e-6 {
            t = (slice_uv - p0_uv[depth_axis]) / denom;
        }
        let p_int = p0_uv + (p1_uv - p0_uv) * t;
        
        // For intersection, we draw a very small segment around p_int 
        // to give the line thickness logic something to work with.
        // We nudge it slightly in one of the other axes.
        let nudge_axis = (depth_axis + 1u) % 3u;
        p0_uv = p_int;
        p1_uv = p_int;
        p1_uv[nudge_axis] += 0.005; // 0.5% nudge to create a "dot"
        is_intersection = true;
    }

    // Select endpoint for current vertex
    var vol_pos: vec3<f32>;
    if vid < 2u {
        vol_pos = p0_uv;
    } else {
        vol_pos = p1_uv;
    }

    // Apply projection
    let screen_uv = volume_to_screen(vol_pos);
    let screen_p0 = volume_to_screen(p0_uv);
    let screen_p1 = volume_to_screen(p1_uv);

    let clip = vec2<f32>(screen_uv.x * 2.0 - 1.0, 1.0 - screen_uv.y * 2.0);
    let p0c = vec2<f32>(screen_p0.x * 2.0 - 1.0, 1.0 - screen_p0.y * 2.0);
    let p1c = vec2<f32>(screen_p1.x * 2.0 - 1.0, 1.0 - screen_p1.y * 2.0);
    
    let dir = p1c - p0c;
    let len = length(dir);
    var perp = vec2<f32>(0.0, 1.0);
    if len > 0.0001 {
        let n = dir / len;
        perp = vec2<f32>(-n.y, n.x);
    }

    var thickness = 3.0 / min(u.res_x, u.res_y);
    if is_intersection {
        thickness = 5.0 / min(u.res_x, u.res_y); // Slightly thicker for intersections
    }
    let side = select(-1.0, 1.0, vid % 2u == 0u);

    out.clip_position = vec4<f32>(clip + perp * thickness * side, 0.0, 1.0);
    out.color = line.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
