// mesh.wgsl - Mesh rendering shader for segmentation surfaces
//
// Renders triangle meshes with Phong shading for 3D views.

// ============================================================================
// Vertex Input/Output
// ============================================================================

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
}

// ============================================================================
// Uniforms
// ============================================================================

struct MeshUniforms {
    model_matrix: mat4x4<f32>,
    view_proj_matrix: mat4x4<f32>,
    normal_matrix: mat4x4<f32>,  // Inverse transpose of model for normals
    color: vec4<f32>,            // Base color (RGBA)
    camera_pos: vec4<f32>,       // Camera position in world space
    light_dir: vec4<f32>,        // Directional light direction (normalized)
    light_color: vec4<f32>,      // Light color and intensity
    ambient: vec4<f32>,          // Ambient light color
}

@group(0) @binding(0) var<uniform> uniforms: MeshUniforms;

// ============================================================================
// Vertex Shader
// ============================================================================

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    
    // Transform position to world space
    let world_pos = uniforms.model_matrix * vec4<f32>(input.position, 1.0);
    output.world_position = world_pos.xyz;
    
    // Transform to clip space
    output.clip_position = uniforms.view_proj_matrix * world_pos;
    
    // Transform normal to world space (using normal matrix)
    output.world_normal = normalize((uniforms.normal_matrix * vec4<f32>(input.normal, 0.0)).xyz);
    
    return output;
}

// ============================================================================
// Fragment Shader - Phong Shading
// ============================================================================

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let N = normalize(input.world_normal);
    let L = normalize(-uniforms.light_dir.xyz);  // Light direction (toward light)
    let V = normalize(uniforms.camera_pos.xyz - input.world_position);
    let H = normalize(L + V);  // Half vector for specular
    
    // Diffuse lighting
    let NdotL = max(dot(N, L), 0.0);
    let diffuse = uniforms.color.rgb * uniforms.light_color.rgb * NdotL;
    
    // Specular lighting (Blinn-Phong)
    let NdotH = max(dot(N, H), 0.0);
    let shininess = 32.0;
    let specular = uniforms.light_color.rgb * pow(NdotH, shininess) * 0.5;
    
    // Ambient lighting
    let ambient = uniforms.color.rgb * uniforms.ambient.rgb;
    
    // Final color
    let final_color = ambient + diffuse + specular;
    
    return vec4<f32>(final_color, uniforms.color.a);
}

// ============================================================================
// Alternative: Flat Shading (using face normals)
// ============================================================================

@fragment
fn fs_flat(input: VertexOutput) -> @location(0) vec4<f32> {
    // Compute face normal from derivatives
    let dx = dpdx(input.world_position);
    let dy = dpdy(input.world_position);
    let N = normalize(cross(dx, dy));
    
    let L = normalize(-uniforms.light_dir.xyz);
    let NdotL = max(dot(N, L), 0.0);
    
    let diffuse = uniforms.color.rgb * uniforms.light_color.rgb * NdotL;
    let ambient = uniforms.color.rgb * uniforms.ambient.rgb;
    
    return vec4<f32>(ambient + diffuse, uniforms.color.a);
}
