// Fractal flame compute shader — chaos game iteration + histogram decay.
// Two entry points: `decay` (per-pixel fade) and `iterate` (per-point chaos game).

struct ComputeUniforms {
    camera_x: f32,
    camera_y: f32,
    camera_zoom: f32,
    camera_rotation: f32,
    frame_seed: u32,
    decay_factor: f32,
    histogram_w: u32,
    histogram_h: u32,
};

struct Transform {
    affine: array<f32, 6>,
    weight: f32,
    color_index: f32,
    variations: array<f32, 6>,
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var<uniform> u: ComputeUniforms;
@group(0) @binding(1) var<storage, read> transforms: array<Transform>;
@group(0) @binding(2) var<storage, read_write> histogram: array<atomic<u32>>;
@group(0) @binding(3) var<storage, read_write> points: array<vec4<f32>>;

const NUM_TRANSFORMS: u32 = 4u;
const ITERATIONS: u32 = 40u;
const PI: f32 = 3.14159265;

// ---------------------------------------------------------------------------
// PCG hash
// ---------------------------------------------------------------------------

fn pcg(input: u32) -> u32 {
    var state = input * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn hash_to_float(h: u32) -> f32 {
    return f32(h) / 4294967295.0;
}

// ---------------------------------------------------------------------------
// Variation functions
// ---------------------------------------------------------------------------

fn v_linear(x: f32, y: f32) -> vec2<f32> {
    return vec2(x, y);
}

fn v_sinusoidal(x: f32, y: f32) -> vec2<f32> {
    return vec2(sin(x), sin(y));
}

fn v_spherical(x: f32, y: f32) -> vec2<f32> {
    let r2 = x * x + y * y + 1e-10;
    return vec2(x / r2, y / r2);
}

fn v_swirl(x: f32, y: f32) -> vec2<f32> {
    let r2 = x * x + y * y;
    let s = sin(r2);
    let c = cos(r2);
    return vec2(x * s - y * c, x * c + y * s);
}

fn v_horseshoe(x: f32, y: f32) -> vec2<f32> {
    let r = sqrt(x * x + y * y) + 1e-10;
    return vec2((x - y) * (x + y) / r, 2.0 * x * y / r);
}

fn v_polar(x: f32, y: f32) -> vec2<f32> {
    let theta = atan2(y, x);
    let r = sqrt(x * x + y * y);
    return vec2(theta / PI, r - 1.0);
}

fn apply_variations(t: Transform, x: f32, y: f32) -> vec2<f32> {
    var result = vec2(0.0, 0.0);
    result += t.variations[0] * v_linear(x, y);
    result += t.variations[1] * v_sinusoidal(x, y);
    result += t.variations[2] * v_spherical(x, y);
    result += t.variations[3] * v_swirl(x, y);
    result += t.variations[4] * v_horseshoe(x, y);
    result += t.variations[5] * v_polar(x, y);
    return result;
}

// ---------------------------------------------------------------------------
// Weighted transform selection
// ---------------------------------------------------------------------------

fn select_transform(rand_val: f32) -> u32 {
    var total = 0.0;
    for (var i = 0u; i < NUM_TRANSFORMS; i++) {
        total += transforms[i].weight;
    }
    let r = rand_val * total;
    var cum = 0.0;
    for (var i = 0u; i < NUM_TRANSFORMS - 1u; i++) {
        cum += transforms[i].weight;
        if r < cum {
            return i;
        }
    }
    return NUM_TRANSFORMS - 1u;
}

// ---------------------------------------------------------------------------
// Camera projection
// ---------------------------------------------------------------------------

fn project(x: f32, y: f32) -> vec2<i32> {
    let tx = x - u.camera_x;
    let ty = y - u.camera_y;
    let cos_r = cos(u.camera_rotation);
    let sin_r = sin(u.camera_rotation);
    let rx = tx * cos_r - ty * sin_r;
    let ry = tx * sin_r + ty * cos_r;
    let sx = rx * u.camera_zoom + 0.5;
    let sy = ry * u.camera_zoom + 0.5;
    return vec2<i32>(i32(sx * f32(u.histogram_w)), i32(sy * f32(u.histogram_h)));
}

// ---------------------------------------------------------------------------
// Decay: fade histogram by decay_factor. One thread per pixel.
// ---------------------------------------------------------------------------

@compute @workgroup_size(256)
fn decay(@builtin(global_invocation_id) gid: vec3<u32>) {
    let pixel_count = u.histogram_w * u.histogram_h;
    if gid.x >= pixel_count {
        return;
    }

    let idx = gid.x * 2u;
    let count = atomicLoad(&histogram[idx]);
    let new_count = u32(f32(count) * u.decay_factor);
    atomicStore(&histogram[idx], new_count);

    let color_sum = atomicLoad(&histogram[idx + 1u]);
    let new_color = u32(f32(color_sum) * u.decay_factor);
    atomicStore(&histogram[idx + 1u], new_color);
}

// ---------------------------------------------------------------------------
// Iterate: chaos game. One thread per point, 20 iterations.
// ---------------------------------------------------------------------------

@compute @workgroup_size(256)
fn iterate(@builtin(global_invocation_id) gid: vec3<u32>) {
    let point_idx = gid.x;
    if point_idx >= arrayLength(&points) {
        return;
    }

    var p = points[point_idx];
    var x = p.x;
    var y = p.y;
    var color = p.z;

    for (var iter = 0u; iter < ITERATIONS; iter++) {
        // Hash for this iteration — deterministic per (point, iter) so the
        // transform sequence is stable across frames.  Visual evolution comes
        // from smoothly-changing audio-driven transform parameters, not from
        // reshuffling the random sequence every frame.
        let h = pcg(point_idx * ITERATIONS + iter);
        let rand_val = hash_to_float(h);

        // Select weighted transform
        let ti = select_transform(rand_val);
        let t = transforms[ti];

        // Apply affine: new_x = a*x + b*y + c, new_y = d*x + e*y + f
        let ax = t.affine[0] * x + t.affine[1] * y + t.affine[2];
        let ay = t.affine[3] * x + t.affine[4] * y + t.affine[5];

        // Apply variation mix
        let v = apply_variations(t, ax, ay);
        x = v.x;
        y = v.y;

        // Clamp to prevent divergence
        x = clamp(x, -10.0, 10.0);
        y = clamp(y, -10.0, 10.0);

        // Blend color toward transform's color
        color = (color + t.color_index) * 0.5;

        // Project to histogram pixel
        let pixel = project(x, y);
        if pixel.x >= 0 && pixel.x < i32(u.histogram_w) &&
           pixel.y >= 0 && pixel.y < i32(u.histogram_h) {
            let pidx = u32(pixel.y) * u.histogram_w + u32(pixel.x);
            atomicAdd(&histogram[pidx * 2u], 1u);
            atomicAdd(&histogram[pidx * 2u + 1u], u32(color * 1000.0));
        }
    }

    // Write updated point state
    points[point_idx] = vec4(x, y, color, 0.0);
}
