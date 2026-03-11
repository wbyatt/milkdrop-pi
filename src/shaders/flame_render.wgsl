// Fractal flame render shader — reads histogram, tone-maps with log-density,
// palette lookup, gamma correction.

struct RenderUniforms {
    brightness: f32,
    gamma: f32,
    histogram_w: u32,
    histogram_h: u32,
};

@group(0) @binding(0) var<storage, read> histogram: array<u32>;
@group(0) @binding(1) var<uniform> u: RenderUniforms;
@group(0) @binding(2) var<storage, read> palette: array<vec4<f32>>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) id: u32) -> VertexOutput {
    var pos = array<vec2<f32>, 3>(
        vec2(-1.0, -1.0),
        vec2( 3.0, -1.0),
        vec2(-1.0,  3.0),
    );
    var out: VertexOutput;
    let p = pos[id];
    out.position = vec4(p, 0.0, 1.0);
    out.uv = vec2((p.x + 1.0) * 0.5, 1.0 - (p.y + 1.0) * 0.5);
    return out;
}

// Read hit count for a histogram pixel, bounds-checked.
fn hits_at(x: u32, y: u32) -> u32 {
    if x >= u.histogram_w || y >= u.histogram_h {
        return 0u;
    }
    return histogram[(y * u.histogram_w + x) * 2u];
}

const NOISE_THRESHOLD: u32 = 3u;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let px = u32(in.uv.x * f32(u.histogram_w));
    let py = u32(in.uv.y * f32(u.histogram_h));

    if px >= u.histogram_w || py >= u.histogram_h {
        return vec4(0.0, 0.0, 0.0, 1.0);
    }

    let idx = (py * u.histogram_w + px) * 2u;
    let hit_count = histogram[idx];
    let color_sum = histogram[idx + 1u];

    // Density floor: pixels with very few hits are never structural.
    if hit_count < NOISE_THRESHOLD {
        return vec4(0.0, 0.0, 0.0, 1.0);
    }

    // Spatial despeckle: a pixel is noise if none of its 4 cardinal neighbors
    // have meaningful hits.  Structural flame pixels always have company.
    let n_up    = hits_at(px, py - 1u);
    let n_down  = hits_at(px, py + 1u);
    let n_left  = hits_at(px - 1u, py);
    let n_right = hits_at(px + 1u, py);
    let neighbor_max = max(max(n_up, n_down), max(n_left, n_right));

    if neighbor_max < NOISE_THRESHOLD {
        return vec4(0.0, 0.0, 0.0, 1.0);
    }

    // Average color index from accumulated fixed-point values
    let avg_color = f32(color_sum) / (f32(hit_count) * 1000.0);
    let clamped_color = clamp(avg_color, 0.0, 1.0);

    // Palette lookup with linear interpolation
    let palette_pos = clamped_color * 255.0;
    let lo = u32(floor(palette_pos));
    let hi = min(lo + 1u, 255u);
    let frac = palette_pos - f32(lo);
    let pal_color = mix(palette[lo], palette[hi], frac);

    // Log-density tone mapping — subtract the threshold so the visible
    // range starts at the noise floor rather than above it.
    let effective = f32(hit_count - 2u);
    let density = log2(1.0 + effective) * u.brightness;

    // Apply density to palette color
    var color = pal_color.rgb * density;

    // Gamma correction
    let inv_gamma = 1.0 / u.gamma;
    color = pow(clamp(color, vec3(0.0), vec3(1.0)), vec3(inv_gamma));

    return vec4(color, 1.0);
}
