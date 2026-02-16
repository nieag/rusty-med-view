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
    cursor_pos: vec3<f32>,
    _pad4: f32,
    rotation: vec4<f32>,
    _padding: vec4<f32>,
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
    } else if u.view_mode == 4u { // Oblique (rotation-driven slice)
        let q = u.rotation;
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
        let rot = mat3x3<f32>(
            vec3<f32>(1.0 - (yy + zz), xy + wz, xz - wy),
            vec3<f32>(xy - wz, 1.0 - (xx + zz), yz + wx),
            vec3<f32>(xz + wy, yz - wx, 1.0 - (xx + yy))
        );
        let u_axis = normalize(rot * vec3<f32>(1.0, 0.0, 0.0));
        let v_axis = normalize(rot * vec3<f32>(0.0, 1.0, 0.0));
        let dims = vec3<f32>(f32(u.dim_x), f32(u.dim_y), f32(u.dim_z));
        let sp = vec3<f32>(u.sp_x, u.sp_y, u.sp_z);
        let phys = max(dims * sp, vec3<f32>(0.001));
        let p_mm = (vol - 0.5) * phys;
        let c_mm = (vec3<f32>(u.cursor_pos.x, u.cursor_pos.y, u.cursor_pos.z) - 0.5) * phys;
        let du = dot(p_mm - c_mm, u_axis);
        let dv = dot(p_mm - c_mm, v_axis);
        let lu = max(length(u_axis * phys), 1e-3);
        let lv = max(length(v_axis * phys), 1e-3);
        zoomed = vec2<f32>(0.5 - du / lu, 0.5 - dv / lv);
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
    } else if u.view_mode == 4u {
        let q = u.rotation;
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
        let rot = mat3x3<f32>(
            vec3<f32>(1.0 - (yy + zz), xy + wz, xz - wy),
            vec3<f32>(xy - wz, 1.0 - (xx + zz), yz + wx),
            vec3<f32>(xz + wy, yz - wx, 1.0 - (xx + yy))
        );
        let u_axis = normalize(rot * vec3<f32>(1.0, 0.0, 0.0));
        let v_axis = normalize(rot * vec3<f32>(0.0, 1.0, 0.0));
        let lu = max(length(vec3<f32>(dx, dy, dz) * u_axis), 1e-3);
        let lv = max(length(vec3<f32>(dx, dy, dz) * v_axis), 1e-3);
        slice_aspect = lu / lv;
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
    } else if u.view_mode == 4u { // Oblique
        depth_axis = 2u;
        dim_depth = 1u;
    } else { // Sagittal (YZ), Depth X
        depth_axis = 0u;
        dim_depth = u.dim_x;
    }
    let slice_uv = (f32(u.current_slice) + 0.5) / f32(max(dim_depth, 1u));

    let source_view_mode = u32(line.plane_info.x);
    var p0_uv = line.p0;
    var p1_uv = line.p1;
    var is_intersection = false;

    // Render contours only in their own viewport mode/slice.
    // Cross-plane intersection markers are intentionally disabled now that
    // derived slice isolines are rendered as the canonical cross-view contour.
    if source_view_mode != u.view_mode {
        out.clip_position = vec4<f32>(0.0, 0.0, 0.0, 0.0);
        return out;
    }
    if u.view_mode != 4u && i32(line.plane_info.y) != u.current_slice {
        out.clip_position = vec4<f32>(0.0, 0.0, 0.0, 0.0);
        return out;
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
