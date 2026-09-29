// One draw per (mesh part, viewport). `row0`/`row1` map the vertex position (x, y, z, 1) to the
// window-space NDC x and y, so the camera lives in the uniform and the mesh is uploaded once.
struct Draw {
  row0: vec4<f32>,
  row1: vec4<f32>,
  color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> draw: Draw;

struct VertexOut {
  @builtin(position) position: vec4<f32>,
  @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(@location(0) position: vec3<f32>) -> VertexOut {
  let p = vec4<f32>(position, 1.0);
  var out: VertexOut;
  out.position = vec4<f32>(dot(draw.row0, p), dot(draw.row1, p), 0.0, 1.0);
  out.color = draw.color;
  return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
  return in.color;
}
