@group(0) @binding(0) var tex_a: texture_2d<f32>;
@group(0) @binding(1) var tex_b: texture_2d<f32>;
@group(0) @binding(2) var tex_sampler: sampler;

struct Uniforms {
    mix_factor: f32,
};
@group(0) @binding(3) var<uniform> u: Uniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) id: u32) -> VertexOutput {
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    var out: VertexOutput;
    let p = pos[id];
    out.position = vec4<f32>(p, 0.0, 1.0);
    out.uv = (p + 1.0) * 0.5;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Flip V: our UV has y=0 at bottom, but texture sampling has y=0 at top.
    let sample_uv = vec2<f32>(in.uv.x, 1.0 - in.uv.y);
    let a = textureSample(tex_a, tex_sampler, sample_uv);
    let b = textureSample(tex_b, tex_sampler, sample_uv);
    return mix(a, b, u.mix_factor);
}
