// Copies the cached 3D view into the window pixel for pixel. The cache has the window's size and
// the pass is limited to the 3D viewport, so a framebuffer pixel maps to the same cache texel.
@group(0) @binding(0) var cache: texture_2d<f32>;

struct VertexOut {
  @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_main(@location(0) position: vec3<f32>, @location(1) tex_coords: vec2<f32>) -> VertexOut {
  var out: VertexOut;
  out.position = vec4<f32>(position, 1.0);
  return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
  return textureLoad(cache, vec2<i32>(in.position.xy), 0);
}
