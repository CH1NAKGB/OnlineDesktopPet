struct VertexOutput { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32>, };
@group(0) @binding(0) var<uniform> animation: vec4<f32>;
@vertex fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var output: VertexOutput;
    output.position = vec4<f32>(positions[index], 0.0, 1.0);
    // Window coordinates start at the upper left; invert the GPU Y axis for upright art.
    output.uv = vec2<f32>(positions[index].x * 0.5 + 0.5, 0.5 - positions[index].y * 0.5);
    return output;
}
fn ellipse(point: vec2<f32>, center: vec2<f32>, radius: vec2<f32>) -> f32 { return 1.0 - smoothstep(0.94, 1.0, length((point - center) / radius)); }
fn layer(base: vec4<f32>, color: vec4<f32>, mask: f32) -> vec4<f32> { return mix(base, color, mask * color.a); }
@fragment fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let petting = clamp(animation.y, 0.0, 1.0);
    let skin = round(clamp(animation.z, 0.0, 2.0));
    let idle_bob = sin(animation.x * 4.0) * 0.012;
    let happy_bob = abs(sin(animation.x * 12.0)) * 0.035 * petting;
    let point = input.uv + vec2<f32>(0.0, idle_bob + happy_bob);
    var color = vec4<f32>(0.0);
    let body = select(vec4<f32>(0.92, 0.58, 0.18, 1.0), select(vec4<f32>(0.36, 0.58, 0.84, 1.0), vec4<f32>(0.52, 0.29, 0.15, 1.0), skin > 1.5), skin > 0.5);
    let head = select(vec4<f32>(1.00, 0.83, 0.39, 1.0), select(vec4<f32>(0.66, 0.82, 1.00, 1.0), vec4<f32>(0.78, 0.55, 0.34, 1.0), skin > 1.5), skin > 0.5);
    color = layer(color, body, ellipse(point, vec2<f32>(0.50, 0.60), vec2<f32>(0.31, 0.27)));
    color = layer(color, head, ellipse(point, vec2<f32>(0.44, 0.45), vec2<f32>(0.25, 0.26)));
    color = layer(color, mix(body, vec4<f32>(0.96, 0.67, 0.15, 1.0), 0.35), ellipse(point, vec2<f32>(0.64, 0.66), vec2<f32>(0.17, 0.12)));
    color = layer(color, vec4<f32>(0.88, 0.17, 0.10, 1.0), ellipse(point, vec2<f32>(0.43, 0.22), vec2<f32>(0.07, 0.10)));
    color = layer(color, vec4<f32>(0.88, 0.17, 0.10, 1.0), ellipse(point, vec2<f32>(0.51, 0.23), vec2<f32>(0.07, 0.11)));
    color = layer(color, vec4<f32>(0.18, 0.11, 0.06, 1.0), ellipse(point, vec2<f32>(0.37, 0.42), vec2<f32>(0.035, 0.045)));
    color = layer(color, vec4<f32>(0.18, 0.11, 0.06, 1.0), ellipse(point, vec2<f32>(0.51, 0.42), vec2<f32>(0.035, 0.045)));
    color = layer(color, vec4<f32>(0.98, 0.47, 0.08, 1.0), ellipse(point, vec2<f32>(0.44, 0.51), vec2<f32>(0.08, 0.05)));
    return color;
}
