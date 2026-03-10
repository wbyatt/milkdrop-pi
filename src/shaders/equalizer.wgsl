const NUM_BARS: u32 = 32u;
const NUM_SEGMENTS: u32 = 16u;
const WAVEFORM_SIZE: u32 = 1024u;
const RHYTHM_SIZE: u32 = 256u;

@group(0) @binding(0) var<storage, read> spectrum: array<f32>;
@group(0) @binding(1) var<storage, read> waveform: array<f32>;
@group(0) @binding(2) var<storage, read> rhythm: array<f32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Fullscreen triangle — covers the viewport in a single draw call.
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
// Layer: EQ bars
// ---------------------------------------------------------------------------

fn eq_layer(uv: vec2<f32>) -> vec3<f32> {
    let bar_f = uv.x * f32(NUM_BARS);
    let bar_index = min(u32(floor(bar_f)), NUM_BARS - 1u);
    let magnitude = spectrum[bar_index];
    let local_x = fract(bar_f);

    let seg_f = uv.y * f32(NUM_SEGMENTS);
    let seg_index = u32(floor(seg_f));
    let local_y = fract(seg_f);
    let seg_top = f32(seg_index + 1u) / f32(NUM_SEGMENTS);
    let lit = step(seg_top, magnitude);

    let dx = abs(local_x - 0.5);
    let dy = abs(local_y - 0.5);
    let cell = step(dx, 0.35) * step(dy, 0.38) * lit;

    // Cyan gradient: cool teal at bottom, bright cyan at top.
    let cyan = mix(
        vec3<f32>(0.00, 0.35, 0.45),
        vec3<f32>(0.10, 0.85, 0.90),
        uv.y,
    );

    let glow_x = exp(-dx * dx * 14.0);
    let above = max(uv.y - magnitude, 0.0);
    let glow_cap = exp(-above * above * 400.0);
    let bloom = glow_x * step(above, 0.0) * magnitude * 0.18
              + glow_x * glow_cap * 0.10;
    let glow_tint = vec3<f32>(0.00, 0.22, 0.28);

    return cyan * cell + glow_tint * bloom;
}

// ---------------------------------------------------------------------------
// Layer: waveform (neon pink oscilloscope trace)
// ---------------------------------------------------------------------------

const WAVE_CENTER: f32 = 0.81;
const WAVE_AMPLITUDE: f32 = 0.25;

fn waveform_layer(uv: vec2<f32>) -> vec3<f32> {
    let x_scaled = uv.x * f32(WAVEFORM_SIZE - 1u);
    let idx = u32(floor(x_scaled));
    let t = fract(x_scaled);
    let s0 = waveform[min(idx, WAVEFORM_SIZE - 1u)];
    let s1 = waveform[min(idx + 1u, WAVEFORM_SIZE - 1u)];
    let sample = mix(s0, s1, t);

    let wave_y = WAVE_CENTER + sample * WAVE_AMPLITUDE;
    let dist = abs(uv.y - wave_y);

    let core = exp(-dist * dist * 30000.0);
    let glow = exp(-dist * dist * 4000.0) * 0.2;

    let neon_pink = vec3<f32>(1.0, 0.08, 0.58);
    return neon_pink * (core + glow);
}

// ---------------------------------------------------------------------------
// Layer: rhythm background — radial sonar ripples + global color breathing
// ---------------------------------------------------------------------------

// Sample the rhythm envelope at a normalized position t in [0, 1].
fn rhythm_at(t: f32) -> f32 {
    let buf_pos = t * f32(RHYTHM_SIZE - 1u);
    let idx = u32(floor(buf_pos));
    let frac = fract(buf_pos);
    let e0 = rhythm[min(idx, RHYTHM_SIZE - 1u)];
    let e1 = rhythm[min(idx + 1u, RHYTHM_SIZE - 1u)];
    return mix(e0, e1, frac);
}

fn rhythm_bg_layer(uv: vec2<f32>) -> vec3<f32> {
    // Radial distance from screen center (aspect-corrected for ~16:10).
    let centered = (uv - vec2<f32>(0.5, 0.5)) * vec2<f32>(1.5, 1.0);
    let dist = length(centered);
    let radius_norm = saturate(dist / 0.85);

    // Map radius to rhythm timeline: center = now, edge = ~4s ago.
    let envelope = rhythm_at(1.0 - radius_norm);

    // Thin concentric ring lines — vector wireframe aesthetic.
    let ring_spacing = 0.025;
    let ring_phase = fract(dist / ring_spacing);
    let ring_line = exp(-(ring_phase - 0.5) * (ring_phase - 0.5) * 300.0);

    // Rings are visible only where the envelope is active, fading at edges.
    let fade = 1.0 - radius_norm * radius_norm;
    let ring_brightness = envelope * ring_line * fade * 0.6;

    // Warm amber rings emerging from the black.
    let ring_color = vec3<f32>(0.35, 0.09, 0.01);
    return ring_color * ring_brightness;
}

fn rhythm_pulse() -> f32 {
    // Most recent envelope value drives global background breathing.
    return rhythm[RHYTHM_SIZE - 1u];
}

// ---------------------------------------------------------------------------
// Compositing
// ---------------------------------------------------------------------------

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // True black at rest. CRT black is the void — let it breathe.
    let pulse = rhythm_pulse();
    let bg = vec3<f32>(0.0, 0.0, 0.0);

    let color = bg
        + rhythm_bg_layer(in.uv)
        + eq_layer(in.uv)
        + waveform_layer(in.uv);

    return vec4<f32>(color, 1.0);
}
