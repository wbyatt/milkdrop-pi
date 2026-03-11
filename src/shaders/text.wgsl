struct GlyphInstance {
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) uv_offset: vec2<f32>,
    @location(3) uv_size: vec2<f32>,
    @location(4) color: vec4<f32>,
}

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn vs_main(
    @builtin(vertex_index) vi: u32,
    instance: GlyphInstance,
) -> VsOut {
    // Two-triangle quad: 0,1,2, 2,1,3
    let index = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
    );

    let local = index[vi];
    let ndc = instance.pos + local * instance.size;
    let uv = instance.uv_offset + local * instance.uv_size;

    var out: VsOut;
    out.position = vec4(ndc.x, ndc.y, 0.0, 1.0);
    out.uv = uv;
    out.color = instance.color;
    return out;
}

@group(0) @binding(0) var atlas_tex: texture_2d<f32>;
@group(0) @binding(1) var atlas_sampler: sampler;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let alpha = textureSample(atlas_tex, atlas_sampler, in.uv).r;
    return vec4(in.color.rgb, in.color.a * alpha);
}
