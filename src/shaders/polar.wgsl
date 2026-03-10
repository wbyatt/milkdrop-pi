const PI: f32 = 3.14159265;
const TAU: f32 = 6.28318530;
const SPECTRUM_SIZE: u32 = 256u;
const MAX_RADIUS: f32 = 1.15;
const ASPECT: f32 = 1.5;
/// Full hue rotation period in seconds.
const COLOR_CYCLE_SECS: f32 = 20.0;

@group(0) @binding(0) var<storage, read> spectrum_left: array<f32>;
@group(0) @binding(1) var<storage, read> spectrum_right: array<f32>;

struct Uniforms {
    time: f32,
    rotation: f32,
};
@group(0) @binding(2) var<uniform> u: Uniforms;

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

// ---------------------------------------------------------------------------
// HSV → RGB (full saturation/value = neon)
// ---------------------------------------------------------------------------

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> vec3<f32> {
    let c = v * s;
    let hp = fract(h) * 6.0;
    let x = c * (1.0 - abs(hp % 2.0 - 1.0));
    let m = v - c;

    var rgb: vec3<f32>;
    if hp < 1.0 {
        rgb = vec3<f32>(c, x, 0.0);
    } else if hp < 2.0 {
        rgb = vec3<f32>(x, c, 0.0);
    } else if hp < 3.0 {
        rgb = vec3<f32>(0.0, c, x);
    } else if hp < 4.0 {
        rgb = vec3<f32>(0.0, x, c);
    } else if hp < 5.0 {
        rgb = vec3<f32>(x, 0.0, c);
    } else {
        rgb = vec3<f32>(c, 0.0, x);
    }
    return rgb + m;
}

// ---------------------------------------------------------------------------
// Spectrum sampling with circular interpolation
// ---------------------------------------------------------------------------

fn sample_left(idx_f: f32) -> f32 {
    let size_f = f32(SPECTRUM_SIZE);
    let wrapped = ((idx_f % size_f) + size_f) % size_f;
    let idx = u32(floor(wrapped));
    let next = (idx + 1u) % SPECTRUM_SIZE;
    let t = fract(wrapped);
    return mix(spectrum_left[idx], spectrum_left[next], t);
}

fn sample_right(idx_f: f32) -> f32 {
    let size_f = f32(SPECTRUM_SIZE);
    let wrapped = ((idx_f % size_f) + size_f) % size_f;
    let idx = u32(floor(wrapped));
    let next = (idx + 1u) % SPECTRUM_SIZE;
    let t = fract(wrapped);
    return mix(spectrum_right[idx], spectrum_right[next], t);
}

// ---------------------------------------------------------------------------
// Layer: polar beam traces with cycling complementary neon colors
// ---------------------------------------------------------------------------

fn beam_layer(uv: vec2<f32>) -> vec3<f32> {
    let centered = (uv - vec2<f32>(0.5, 0.5)) * vec2<f32>(ASPECT, 1.0);
    let r = length(centered);
    let theta = atan2(centered.y, centered.x);

    let theta_norm = fract((theta + PI + u.rotation) / TAU);

    // Left channel: frequency maps directly to theta.
    let left_idx = theta_norm * f32(SPECTRUM_SIZE);
    let left_amp = sample_left(left_idx) * MAX_RADIUS;
    let left_dist = abs(r - left_amp);

    // Right channel: 180° offset.
    let right_idx = fract(theta_norm + 0.5) * f32(SPECTRUM_SIZE);
    let right_amp = sample_right(right_idx) * MAX_RADIUS;
    let right_dist = abs(r - right_amp);

    // Continuously rotating complementary hues.
    let hue_a = fract(u.time / COLOR_CYCLE_SECS);
    let hue_b = fract(hue_a + 0.5);
    let color_a = hsv_to_rgb(hue_a, 1.0, 1.0);
    let color_b = hsv_to_rgb(hue_b, 1.0, 1.0);

    // Left beam: color_a core → color_b in the glow.
    let left_color_t = exp(-left_dist * left_dist * 12000.0);
    let left_brightness = exp(-left_dist * left_dist * 4000.0);
    let left_color = mix(color_b, color_a, left_color_t) * left_brightness;

    // Right beam: color_b core → color_a in the glow.
    let right_color_t = exp(-right_dist * right_dist * 12000.0);
    let right_brightness = exp(-right_dist * right_dist * 4000.0);
    let right_color = mix(color_a, color_b, right_color_t) * right_brightness;

    return left_color + right_color;
}

// ---------------------------------------------------------------------------
// Layer: faint euclidean grid
// ---------------------------------------------------------------------------

fn grid_layer(uv: vec2<f32>) -> vec3<f32> {
    let spacing = 0.055;
    let gx = fract(uv.x / spacing);
    let gy = fract(uv.y / spacing);
    let line_x = exp(-(gx - 0.5) * (gx - 0.5) * 4000.0);
    let line_y = exp(-(gy - 0.5) * (gy - 0.5) * 4000.0);
    let grid = (line_x + line_y) * 0.02;
    return vec3<f32>(0.10, 0.10, 0.12) * grid;
}

// ---------------------------------------------------------------------------
// Compositing
// ---------------------------------------------------------------------------

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = grid_layer(in.uv) + beam_layer(in.uv);
    return vec4<f32>(color, 1.0);
}
