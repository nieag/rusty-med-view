// Contour Line Shader
// Renders 2D line segments for contour visualization
// Uses instancing - each instance is a line segment

struct Uniforms {
    // View transform
    zoom: f32,
    pan: vec2<f32>,
    pivot: vec2<f32>,
    
    // Viewport info
    view_mode: u32,     // 0=3D, 1=Axial, 2=Coronal, 3=Sagittal
    resolution: vec2<f32>,
    
    // Volume info
    volume_dims: vec3<u32>,
    volume_spacing: vec3<f32>,
    
    // Slice info
    current_slice: i32,  // Current slice index for this view mode
    
    _padding: vec3<f32>,
}

struct ContourLine {
    // Start point (in volume UV coordinates 0-1)
    p0: vec3<f32>,
    _pad0: f32,
    // End point (in volume UV coordinates 0-1)
    p1: vec3<f32>,
    _pad1: f32,
    // Color RGBA
    color: vec4<f32>,
    // Plane info: x=view_mode (1=axial,2=coronal,3=sagittal), y=slice_index, z=_pad, w=_pad
    plane_info: vec4<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var<storage, read> lines: array<ContourLine>;

struct VertexInput {
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

// Transform volume UV to screen UV for 2D views
// This is the INVERSE of the main shader's screen-to-volume transform
fn volume_to_screen(pos: vec3<f32>, view_mode: u32) -> vec2<f32> {
    var volume_uv: vec2<f32>;
    
    if view_mode == 1u { // Axial
        // Radiological: flip X for right-on-left
        volume_uv = vec2<f32>(1.0 - pos.x, 1.0 - pos.y);
    } else if view_mode == 2u { // Coronal
        volume_uv = vec2<f32>(1.0 - pos.x, 1.0 - pos.z);
    } else { // Sagittal
        volume_uv = vec2<f32>(1.0 - pos.y, 1.0 - pos.z);
    }
    
    // Calculate aspect correction K (same as main shader)
    let dims = vec3<f32>(uniforms.volume_dims);
    let spacing = uniforms.volume_spacing;
    var slice_aspect = 1.0;
    if view_mode == 1u {
        slice_aspect = (dims.x * spacing.x) / (dims.y * spacing.y);
    } else if view_mode == 2u {
        slice_aspect = (dims.x * spacing.x) / (dims.z * spacing.z);
    } else {
        slice_aspect = (dims.y * spacing.y) / (dims.z * spacing.z);
    }
    
    let screen_aspect = uniforms.resolution.x / uniforms.resolution.y;
    let k = screen_aspect / slice_aspect;
    
    let zoom = uniforms.zoom;
    let pan = uniforms.pan;
    let pivot = vec2<f32>(0.5, 0.5); // Match main shader's forced center pivot
    
    // Main shader: screen_uv -> volume_uv
    //   centered_uv = (screen_uv - pivot) * vec2(k, 1.0)
    //   volume_uv = centered_uv / zoom + pivot + pan
    //
    // Inverse: volume_uv -> screen_uv
    //   centered_uv = (volume_uv - pivot - pan) * zoom
    //   screen_uv = centered_uv / vec2(k, 1.0) + pivot
    
    let centered = (volume_uv - pivot - pan) * zoom;
    let screen_uv = centered / vec2<f32>(k, 1.0) + pivot;
    
    return screen_uv;
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    
    let line = lines[in.instance_index];
    
    // Select endpoint based on vertex index (0,1 = p0, 2,3 = p1)
    var pos: vec3<f32>;
    if in.vertex_index < 2u {
        pos = line.p0;
    } else {
        pos = line.p1;
    }
    
    // Apply radiological flip: volume UV (0-1) -> screen UV with flip
    // Radiological: patient right on screen left
    let flipped_pos = vec2<f32>(1.0 - pos.x, 1.0 - pos.y);
    
    // Convert to clip space (-1 to 1)
    let clip = flipped_pos * 2.0 - 1.0;
    
    // Calculate line direction for thickness
    let p0_flip = vec2<f32>(1.0 - line.p0.x, 1.0 - line.p0.y);
    let p1_flip = vec2<f32>(1.0 - line.p1.x, 1.0 - line.p1.y);
    let line_dir = normalize(p1_flip - p0_flip);
    let perp = vec2<f32>(-line_dir.y, line_dir.x);
    
    // Offset for line thickness (in clip space units)
    let offset_dir = select(-1.0, 1.0, in.vertex_index % 2u == 0u);
    let thickness = 0.004; // ~2 pixels
    
    out.clip_position = vec4<f32>(clip + perp * thickness * offset_dir, 0.0, 1.0);
    out.color = vec4<f32>(1.0, 0.0, 1.0, 1.0); // MAGENTA for now
    
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
