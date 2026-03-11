// Conway's Game of Life — grid lines + frequency-pulsing live cells.
// Camera offset creates the illusion of flying over an infinite grid.

struct Uniforms {
    grid_w: u32,
    grid_h: u32,
    camera_x: f32,
    camera_y: f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
// Per-cell brightness: 0.0 = dead, >0.0 = alive (modulated by frequency bin).
@group(0) @binding(1) var<storage, read> cells: array<f32>;

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

// Modular wrap that handles negative values correctly.
fn wrap(val: i32, size: u32) -> u32 {
    return u32(((val % i32(size)) + i32(size)) % i32(size));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let grid = vec2<f32>(f32(u.grid_w), f32(u.grid_h));
    let camera = vec2<f32>(u.camera_x, u.camera_y);

    // Offset UV by camera position and map to cell coordinates.
    let cell_coord = (in.uv + camera) * grid;
    let cell_raw = vec2<i32>(floor(cell_coord));
    let cell_frac = fract(cell_coord);

    // Wrap cell index into the toroidal grid.
    let cx = wrap(cell_raw.x, u.grid_w);
    let cy = wrap(cell_raw.y, u.grid_h);
    let brightness = cells[cy * u.grid_w + cx];

    // Grid lines: per-axis antialiasing with Nyquist fadeout.
    // Use each axis's own derivative for correct AA width.
    let px = fwidth(cell_coord.x);
    let py = fwidth(cell_coord.y);

    // Distance to nearest grid line per axis.
    let gx = min(cell_frac.x, 1.0 - cell_frac.x);
    let gy = min(cell_frac.y, 1.0 - cell_frac.y);

    // Smoothstep AA per axis, width scaled to pixel footprint.
    let line_x = smoothstep(px, 0.0, gx);
    let line_y = smoothstep(py, 0.0, gy);

    // Fade out grid lines when cells approach pixel size (moire prevention).
    // pixels_per_cell < ~3 → grid pattern can't be resolved cleanly.
    let fade_x = smoothstep(1.5, 3.0, 1.0 / px);
    let fade_y = smoothstep(1.5, 3.0, 1.0 / py);

    let grid_line = max(line_x * fade_x, line_y * fade_y);
    let grid_dist = min(gx, gy);
    let grid_glow = exp(-grid_dist * grid_dist * 800.0) * 0.08 * max(fade_x, fade_y);

    // Cell fill: interior of live cells.
    let cell_fill = brightness * (1.0 - grid_line);

    let green = vec3(0.0, 1.0, 0.3);
    let grid_intensity = (grid_line * 0.12 + grid_glow);
    let color = green * (cell_fill + grid_intensity);

    return vec4(color, 1.0);
}
