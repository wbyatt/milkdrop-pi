const PI: f32 = 3.14159265;
const TAU: f32 = 6.28318530;
const SPECTRUM_SIZE: u32 = 256u;
const WAVEFORM_SIZE: u32 = 1024u;
const NUM_RIDGES: u32 = 40u;

// --- Layout ---
const HORIZON: f32 = 0.42;
const ASPECT: f32 = 1.5;

// --- Terrain ---
const MAX_DEPTH: f32 = 18.0;
const HEIGHT_SCALE: f32 = 0.20;
const PERSPECTIVE_SCALE: f32 = 0.3;
const X_SPACING: f32 = 1.2;
const LINE_WIDTH: f32 = 0.0015;

// --- Day/night cycle ---
const CYCLE_PERIOD: f32 = 60.0;
const SUN_RADIUS: f32 = 0.09;
const SUN_SPOKES: f32 = 8.0;
const SCANLINE_PERIOD: f32 = 3.0;

@group(0) @binding(0) var<storage, read> terrain_history: array<f32>;
@group(0) @binding(1) var<storage, read> waveform: array<f32>;

struct Uniforms {
    time: f32,
    scroll_frac: f32,
    write_idx: f32,
    bass_energy: f32,
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
// Utilities
// ---------------------------------------------------------------------------

fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x * 0.1031, p.y * 0.1030, p.x * 0.0973));
    p3 = p3 + dot(p3, vec3<f32>(p3.y + 33.33, p3.z + 33.33, p3.x + 33.33));
    return fract((p3.x + p3.y) * p3.z);
}

/// Samples the waveform with a wide box blur for bass-only content.
/// The 16-tap kernel removes high frequencies, leaving smooth bass undulations.
fn sample_bass_waveform(x: f32) -> f32 {
    let idx_f = clamp(x, 0.0, 1.0) * f32(WAVEFORM_SIZE - 1u);
    let center = i32(floor(idx_f));
    let half_kernel = 16;
    var sum = 0.0;
    var count = 0.0;
    for (var k = -half_kernel; k <= half_kernel; k = k + 1) {
        let idx = clamp(center + k, 0, i32(WAVEFORM_SIZE - 1u));
        sum = sum + waveform[u32(idx)];
        count = count + 1.0;
    }
    return sum / count;
}

/// Reads height from the circular history buffer for a given ridge and x position.
fn sample_ridge(ridge_idx: u32, x: f32) -> f32 {
    let row = (u32(u.write_idx) + ridge_idx) % NUM_RIDGES;
    let col_f = clamp(x, 0.0, 1.0) * f32(SPECTRUM_SIZE - 1u);
    let col = u32(floor(col_f));
    let next_col = min(col + 1u, SPECTRUM_SIZE - 1u);
    let base = row * SPECTRUM_SIZE;
    return mix(terrain_history[base + col], terrain_history[base + next_col], fract(col_f));
}

/// Perspective depth for ridge i (0 = nearest/oldest, NUM_RIDGES-1 = farthest/newest).
fn ridge_depth(i: u32) -> f32 {
    let ridge_spacing = MAX_DEPTH / f32(NUM_RIDGES);
    return (f32(i) + 1.0 - u.scroll_frac) * ridge_spacing;
}

/// Displaced screen-Y for a ridge at a given x position.
fn ridge_y(i: u32, x: f32) -> f32 {
    let depth = ridge_depth(i);
    let base_y = HORIZON - PERSPECTIVE_SCALE / depth;
    let height = sample_ridge(i, x);
    let h_scaled = height * HEIGHT_SCALE / (depth * 0.5 + 1.0);
    return base_y - h_scaled;
}

// ---------------------------------------------------------------------------
// Layer: terrain (history-based wireframe with contour verticals)
// ---------------------------------------------------------------------------

fn terrain_layer(uv: vec2<f32>) -> vec3<f32> {
    if uv.y > HORIZON + LINE_WIDTH { return vec3<f32>(0.0, 0.0, 0.0); }

    let grid_color = vec3<f32>(0.0, 0.75, 0.55);
    var color = vec3<f32>(0.0, 0.0, 0.0);

    // --- Pass 1: Compute displaced Y for each ridge, draw horizontal lines ---
    var ry: array<f32, 40>;

    for (var i = 0u; i < NUM_RIDGES; i = i + 1u) {
        let y = ridge_y(i, uv.x);
        ry[i] = y;

        let dist = abs(uv.y - y);
        let line = 1.0 - smoothstep(0.0, LINE_WIDTH * 2.0, dist);
        let glow = exp(-dist * dist * 80000.0) * 0.08;

        let depth = ridge_depth(i);
        let fade = exp(-depth * 0.12) * smoothstep(0.0, 1.0, depth);
        color = color + grid_color * (line + glow) * fade;
    }

    // --- Pass 2: Vertical contour lines between adjacent ridges ---
    for (var i = 0u; i < NUM_RIDGES - 1u; i = i + 1u) {
        let y_near = ry[i];
        let y_far = ry[i + 1u];
        let y_lo = min(y_near, y_far);
        let y_hi = max(y_near, y_far);

        // Skip if pixel is outside this ridge pair's vertical span
        if uv.y < y_lo || uv.y > y_hi { continue; }

        // Interpolate depth between the two ridges at this y
        let t = clamp((uv.y - y_near) / (y_far - y_near + 0.0001), 0.0, 1.0);
        let depth = mix(ridge_depth(i), ridge_depth(i + 1u), t);

        // World-x at this interpolated depth
        let world_x = (uv.x - 0.5) * depth * ASPECT;

        // Distance to nearest grid x-line, converted to screen space
        let gx = fract(world_x / X_SPACING + 0.5);
        let gx_dist_screen = min(gx, 1.0 - gx) * X_SPACING / (depth * ASPECT);
        let x_line = 1.0 - smoothstep(0.0, LINE_WIDTH * 2.0, gx_dist_screen);
        let x_glow = exp(-gx_dist_screen * gx_dist_screen * 80000.0) * 0.08;

        let fade = exp(-depth * 0.12);
        color = color + grid_color * (x_line + x_glow) * fade;
    }

    return color;
}

// ---------------------------------------------------------------------------
// Layer: bass waveform at horizon (neon pink, drawn on top of everything)
// ---------------------------------------------------------------------------

fn bass_horizon(uv: vec2<f32>) -> vec3<f32> {
    let bass_sample = sample_bass_waveform(uv.x);
    let wave_y = HORIZON + bass_sample * 0.12;
    let wave_dist = abs(uv.y - wave_y);
    let wave_line = 1.0 - smoothstep(0.0, LINE_WIDTH * 2.0, wave_dist);
    let wave_glow = exp(-wave_dist * wave_dist * 80000.0) * 0.08;
    let neon_pink = vec3<f32>(1.0, 0.08, 0.58);
    return neon_pink * (wave_line + wave_glow);
}

// ---------------------------------------------------------------------------
// Sky disk rotation — stars and sun are pinned to a single rotating disk
// whose axis sits below the horizon.
// ---------------------------------------------------------------------------

/// Center of the sky disk rotation, well below the horizon.
const SKY_AXIS: vec2<f32> = vec2<f32>(0.5, 0.05);
/// Radius from axis to the sun's fixed position on the disk.
const SKY_RADIUS: f32 = 0.55;

/// Rotate a point around the sky axis by the cycle angle.
fn rotate_sky(uv: vec2<f32>, angle: f32) -> vec2<f32> {
    let centered = uv - SKY_AXIS;
    let c = cos(angle);
    let s = sin(angle);
    let rotated = vec2<f32>(
        centered.x * c - centered.y * s,
        centered.x * s + centered.y * c,
    );
    return rotated + SKY_AXIS;
}

/// The sun's fixed position on the disk (directly above the axis).
fn sun_disk_pos() -> vec2<f32> {
    return vec2<f32>(SKY_AXIS.x, SKY_AXIS.y + SKY_RADIUS);
}

// ---------------------------------------------------------------------------
// Layer: starfield (procedural, pinned to rotating sky disk)
// ---------------------------------------------------------------------------

fn star_layer(screen_uv: vec2<f32>, sky_uv: vec2<f32>, sun_pos: vec2<f32>) -> vec3<f32> {
    // Clip to visible sky (screen space), not disk space
    if screen_uv.y < HORIZON { return vec3<f32>(0.0, 0.0, 0.0); }

    // Fade stars based on proximity to the sun — directional illumination.
    // Near the sun: washed out. Far side: stays dark and starry.
    let to_sun = length((screen_uv - sun_pos) * vec2<f32>(ASPECT, 1.0));
    let sun_above = max(sun_pos.y - HORIZON, 0.0);
    let wash_radius = 0.2 + sun_above * 1.5; // at noon, wash reaches far
    let sun_fade = smoothstep(wash_radius * 0.4, wash_radius, to_sun);

    let cell_size = 0.02;
    let cell = floor(sky_uv / cell_size);
    let cell_hash = hash21(cell);

    if cell_hash < 0.88 { return vec3<f32>(0.0, 0.0, 0.0); }

    let star_offset = vec2<f32>(
        hash21(cell + vec2<f32>(17.0, 0.0)),
        hash21(cell + vec2<f32>(0.0, 31.0)),
    );
    let star_pos = (cell + 0.2 + star_offset * 0.6) * cell_size;
    let d = length((sky_uv - star_pos) * vec2<f32>(ASPECT, 1.0));

    // Map star to a spectrum frequency via its hash — bass stars pulse with kicks,
    // treble stars shimmer with hi-hats.
    let freq_hash = hash21(cell + vec2<f32>(89.0, 13.0));
    let spec_idx = u32(freq_hash * f32(SPECTRUM_SIZE - 1u));
    let newest_row = (u32(u.write_idx) + NUM_RIDGES - 1u) % NUM_RIDGES;
    let freq_energy = terrain_history[newest_row * SPECTRUM_SIZE + spec_idx];
    let music_boost = 0.3 + freq_energy * 2.0; // floor of 0.3, up to 2.3 at full energy

    let star_size = mix(0.0005, 0.0015, cell_hash) * (0.8 + freq_energy * 0.6);
    var brightness = exp(-d * d / (star_size * star_size));

    let twinkle = 0.7 + 0.3 * sin(u.time * (2.0 + cell_hash * 4.0) + cell_hash * TAU);
    brightness = brightness * twinkle * music_boost * sun_fade;

    // Subtle hue scatter: mostly white, tinted toward blue, yellow, or warm red
    let hue_hash = hash21(cell + vec2<f32>(53.0, 7.0));
    var star_color = vec3<f32>(0.9, 0.9, 1.0); // default cool white
    if hue_hash < 0.3 {
        star_color = vec3<f32>(0.7, 0.8, 1.0);  // blue-white
    } else if hue_hash < 0.55 {
        star_color = vec3<f32>(1.0, 0.95, 0.8);  // warm yellow-white
    } else if hue_hash < 0.7 {
        star_color = vec3<f32>(1.0, 0.85, 0.75); // soft orange-white
    }

    return star_color * brightness;
}

// ---------------------------------------------------------------------------
// Layer: cyberpunk sun (wireframe circle + spokes + scanline fill)
// ---------------------------------------------------------------------------

fn sun_layer(uv: vec2<f32>, sun_pos: vec2<f32>, pixel_y: f32) -> vec3<f32> {
    if uv.y < HORIZON - 0.02 { return vec3<f32>(0.0, 0.0, 0.0); }

    let sun_color = vec3<f32>(1.0, 0.4, 0.1);
    let horizon_fade = smoothstep(HORIZON - 0.02, HORIZON + 0.05, sun_pos.y);

    let to_sun = (uv - sun_pos) * vec2<f32>(ASPECT, 1.0);
    let sun_dist = length(to_sun);

    var color = vec3<f32>(0.0, 0.0, 0.0);

    // --- Large atmospheric glow (smooth gradient, no scanlines) ---
    // Fills the sky when sun is high, fades to black at edges
    let glow = exp(-sun_dist * sun_dist * 1.5) * 0.35;
    color = color + sun_color * glow * horizon_fade;

    // --- Wireframe sun body (only drawn near the sun) ---
    if sun_dist < SUN_RADIUS * 3.0 {
        let sun_theta = atan2(to_sun.y, to_sun.x);
        let scanline = step(0.34, fract(pixel_y / SCANLINE_PERIOD));

        var wire = 0.0;

        // Outer wireframe ring
        let ring_dist = abs(sun_dist - SUN_RADIUS);
        wire = wire + exp(-ring_dist * ring_dist * 200000.0);

        // Inner wireframe ring
        let inner_dist = abs(sun_dist - SUN_RADIUS * 0.55);
        wire = wire + exp(-inner_dist * inner_dist * 200000.0) * 0.5;

        // Radial spokes
        let spoke = abs(sin(sun_theta * SUN_SPOKES));
        let spoke_line = step(0.97, spoke);
        let spoke_fade = smoothstep(SUN_RADIUS * 2.0, SUN_RADIUS * 0.1, sun_dist);
        wire = wire + spoke_line * spoke_fade * 0.7;

        // Scanlined interior fill
        let interior = smoothstep(SUN_RADIUS, SUN_RADIUS - 0.008, sun_dist);
        wire = wire + interior * scanline * 0.5;

        // Bass-reactive pulse ring
        let bass = u.bass_energy;
        let pulse_r = SUN_RADIUS * (1.0 + bass * 0.4);
        let pulse_dist = abs(sun_dist - pulse_r);
        wire = wire + exp(-pulse_dist * pulse_dist * 80000.0) * bass;

        color = color + sun_color * wire * horizon_fade;
    }

    return color;
}

// ---------------------------------------------------------------------------
// Layer: horizon glow (tracks the sun's position on the rotating disk)
// ---------------------------------------------------------------------------

fn horizon_glow(uv: vec2<f32>, sun_pos: vec2<f32>, pixel_y: f32) -> vec3<f32> {
    // Glow intensity based on how close the sun is to the horizon
    let sun_above = sun_pos.y - HORIZON;
    let near_horizon = exp(-sun_above * sun_above * 60.0);
    let visible = smoothstep(-0.05, 0.02, sun_above);
    let glow_intensity = near_horizon * visible;

    if glow_intensity < 0.01 { return vec3<f32>(0.0, 0.0, 0.0); }

    let warm = vec3<f32>(0.8, 0.2, 0.4);
    let hot = vec3<f32>(1.0, 0.5, 0.1);

    let dist_from_horizon = abs(uv.y - HORIZON);
    // Glow is strongest near the sun's x-position
    let dx = (uv.x - sun_pos.x) * ASPECT;
    let lateral_fade = exp(-dx * dx * 4.0);
    let glow = exp(-dist_from_horizon * dist_from_horizon * 100.0) * lateral_fade;

    let scanline = 0.5 + 0.5 * step(0.34, fract(pixel_y / SCANLINE_PERIOD));

    let color = mix(warm, hot, exp(-dist_from_horizon * 20.0));

    return color * glow * glow_intensity * scanline * 0.4;
}

// ---------------------------------------------------------------------------
// Compositing
// ---------------------------------------------------------------------------

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Sky disk rotation angle
    let sky_angle = u.time / CYCLE_PERIOD * TAU;

    // Sun's current screen position (fixed on disk, rotated into view)
    let sun_pos = rotate_sky(sun_disk_pos(), sky_angle);

    // Stars use rotated UVs so the whole field rotates with the disk
    let sky_uv = rotate_sky(in.uv, -sky_angle);

    var color = vec3<f32>(0.0, 0.0, 0.0);

    color = color + star_layer(in.uv, sky_uv, sun_pos);
    color = color + sun_layer(in.uv, sun_pos, in.position.y);
    color = color + horizon_glow(in.uv, sun_pos, in.position.y);
    color = color + terrain_layer(in.uv);
    color = color + bass_horizon(in.uv);

    return vec4<f32>(color, 1.0);
}
