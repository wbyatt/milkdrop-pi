use std::cell::RefCell;

use crate::analysis::{AnalysisFrame, NUM_BANDS, RHYTHM_SIZE};
use crate::render::Visualization;

const GRID_W: usize = 80;
const GRID_H: usize = 60;
const CELL_COUNT: usize = GRID_W * GRID_H;

/// Initial fraction of cells that are alive.
const INITIAL_DENSITY: f64 = 0.30;
/// Minimum seconds between ticks for any given frequency bin.
const MIN_TICK_INTERVAL: f32 = 0.15;
/// Base camera drift speed (UV units per second — 1.0 = one full grid width/sec).
const DRIFT_BASE: f32 = 0.015;
/// Additional drift speed from musical energy.
const DRIFT_MUSIC: f32 = 0.01;
/// How quickly the camera heading turns toward the target angle.
/// Lower = longer sweeps before direction changes.
const HEADING_SLEW: f32 = 0.3;
/// How much a bin's energy must exceed its moving average to trigger a tick.
const ONSET_RATIO: f32 = 1.25;
/// EMA coefficient for the per-bin moving average.
const AVG_ALPHA: f32 = 0.08;
/// Total energy below this counts as silence.
const SILENCE_THRESHOLD: f32 = 0.01;
/// Seconds of continuous silence before we arm the respawn.
const SILENCE_DURATION: f32 = 1.0;
/// Grid density below this triggers respawn when sound returns.
const RESPAWN_DENSITY: f32 = 0.05;
/// Fraction of dead cells to randomly revive on respawn.
const RESPAWN_FILL: f64 = 0.25;

// ---------------------------------------------------------------------------
// Grid — owns cell state and boundary logic.
// ---------------------------------------------------------------------------

struct Grid {
    width: usize,
    height: usize,
    cells: Vec<bool>,
}

impl Grid {
    fn new_random(width: usize, height: usize) -> Self {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        use std::time::SystemTime;

        let mut hasher = DefaultHasher::new();
        SystemTime::now().hash(&mut hasher);
        let mut seed = hasher.finish();

        let cells = (0..width * height)
            .map(|_| {
                // xorshift64
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                (seed % 100) < (INITIAL_DENSITY * 100.0) as u64
            })
            .collect();

        Self {
            width,
            height,
            cells,
        }
    }

    fn density(&self) -> f32 {
        let alive = self.cells.iter().filter(|&&c| c).count();
        alive as f32 / self.cells.len() as f32
    }

    /// Randomly set dead cells alive at the given fill rate.
    fn seed_random(&mut self, fill: f64) {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        use std::time::SystemTime;

        let mut hasher = DefaultHasher::new();
        SystemTime::now().hash(&mut hasher);
        let mut seed = hasher.finish();

        for cell in &mut self.cells {
            if !*cell {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                if (seed % 100) < (fill * 100.0) as u64 {
                    *cell = true;
                }
            }
        }
    }

    fn alive(&self, x: usize, y: usize) -> bool {
        self.cells[y * self.width + x]
    }

    /// Fetch cell state with boundary handling. Currently toroidal wrap.
    fn get(&self, x: i32, y: i32) -> bool {
        let wx = x.rem_euclid(self.width as i32) as usize;
        let wy = y.rem_euclid(self.height as i32) as usize;
        self.cells[wy * self.width + wx]
    }

    fn neighbor_count(&self, x: usize, y: usize) -> u8 {
        let ix = x as i32;
        let iy = y as i32;
        let mut count = 0u8;
        for dy in -1..=1i32 {
            for dx in -1..=1i32 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                if self.get(ix + dx, iy + dy) {
                    count += 1;
                }
            }
        }
        count
    }

    /// Advance only the cells whose bin fired. All other cells keep their state.
    /// Evaluates GOL rules against the current (pre-step) grid for consistency.
    fn partial_step(&self, cell_bins: &[usize], bin_fired: &[bool]) -> Grid {
        let mut next = self.cells.clone();
        for y in 0..self.height {
            for x in 0..self.width {
                let idx = y * self.width + x;
                if !bin_fired[cell_bins[idx]] {
                    continue;
                }
                let n = self.neighbor_count(x, y);
                next[idx] = if self.alive(x, y) {
                    n == 2 || n == 3
                } else {
                    n == 3
                };
            }
        }
        Grid {
            width: self.width,
            height: self.height,
            cells: next,
        }
    }
}

// ---------------------------------------------------------------------------
// Spatial bin mapping — precomputed per-cell frequency bin from radial position.
// ---------------------------------------------------------------------------

fn build_cell_bins(width: usize, height: usize) -> Vec<usize> {
    (0..height)
        .flat_map(|y| {
            (0..width).map(move |x| {
                let nx = (x as f32 / width as f32) * 2.0 - 1.0;
                let ny = (y as f32 / height as f32) * 2.0 - 1.0;
                let r = (nx * nx + ny * ny).sqrt();
                let mapped = r.sqrt();
                ((mapped * NUM_BANDS as f32) as usize).min(NUM_BANDS - 1)
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Per-bin onset detection.
// ---------------------------------------------------------------------------

struct BinOnsets {
    avg_energy: [f32; NUM_BANDS],
    prev_energy: [f32; NUM_BANDS],
    last_tick: [f32; NUM_BANDS],
}

impl BinOnsets {
    fn new() -> Self {
        Self {
            avg_energy: [0.0; NUM_BANDS],
            prev_energy: [0.0; NUM_BANDS],
            last_tick: [0.0; NUM_BANDS],
        }
    }

    /// Returns which bins fired this frame.
    fn detect(&mut self, bands: &[f32; NUM_BANDS], elapsed: f32) -> [bool; NUM_BANDS] {
        let mut fired = [false; NUM_BANDS];
        for i in 0..NUM_BANDS {
            let energy = bands[i];
            self.avg_energy[i] = AVG_ALPHA * energy + (1.0 - AVG_ALPHA) * self.avg_energy[i];

            let threshold = self.avg_energy[i] * ONSET_RATIO;
            let was_below = self.prev_energy[i] < threshold;
            let is_above = energy >= threshold;
            self.prev_energy[i] = energy;

            if was_below && is_above && (elapsed - self.last_tick[i]) >= MIN_TICK_INTERVAL {
                self.last_tick[i] = elapsed;
                fired[i] = true;
            }
        }
        fired
    }
}

// ---------------------------------------------------------------------------
// Connected component labeling — flood fill with diagonal adjacency.
// ---------------------------------------------------------------------------

struct ComponentInfo {
    sum_x: f32,
    sum_y: f32,
    count: u32,
}

/// Labels connected groups of live cells. Returns (labels, component_infos).
/// Labels are 0 for dead cells, 1+ for component ID.
fn label_components(grid: &Grid) -> (Vec<u32>, Vec<ComponentInfo>) {
    let n = grid.width * grid.height;
    let mut labels = vec![0u32; n];
    let mut infos = Vec::new();
    let mut stack = Vec::new();
    let mut current_label = 0u32;

    for y in 0..grid.height {
        for x in 0..grid.width {
            let idx = y * grid.width + x;
            if !grid.alive(x, y) || labels[idx] != 0 {
                continue;
            }

            current_label += 1;
            let mut info = ComponentInfo {
                sum_x: 0.0,
                sum_y: 0.0,
                count: 0,
            };

            stack.push((x, y));
            while let Some((cx, cy)) = stack.pop() {
                let ci = cy * grid.width + cx;
                if labels[ci] != 0 || !grid.alive(cx, cy) {
                    continue;
                }
                labels[ci] = current_label;
                info.sum_x += cx as f32;
                info.sum_y += cy as f32;
                info.count += 1;

                for dy in -1..=1i32 {
                    for dx in -1..=1i32 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        let nx = cx as i32 + dx;
                        let ny = cy as i32 + dy;
                        // For component labeling, don't wrap — only label
                        // screen-contiguous groups.
                        if nx >= 0
                            && nx < grid.width as i32
                            && ny >= 0
                            && ny < grid.height as i32
                        {
                            let ni = ny as usize * grid.width + nx as usize;
                            if labels[ni] == 0 && grid.alive(nx as usize, ny as usize) {
                                stack.push((nx as usize, ny as usize));
                            }
                        }
                    }
                }
            }

            infos.push(info);
        }
    }

    (labels, infos)
}

/// Map component centroid to a frequency bin radially: bass at center, treble at edges.
fn component_freq_bin(info: &ComponentInfo, grid_width: usize, grid_height: usize) -> usize {
    let n = info.count.max(1) as f32;
    let cx = info.sum_x / n;
    let cy = info.sum_y / n;
    // Normalize to [-1, 1] from center.
    let nx = (cx / grid_width as f32) * 2.0 - 1.0;
    let ny = (cy / grid_height as f32) * 2.0 - 1.0;
    // Radial distance from center. Allow >1.0 so corner groups
    // map past the last bin — clipping gives a sense of fullness.
    let r = (nx * nx + ny * ny).sqrt();
    // Square root to counteract area growth — spreads bands evenly across the grid.
    let mapped = r.sqrt();
    ((mapped * NUM_BANDS as f32) as usize).min(NUM_BANDS - 1)
}

// ---------------------------------------------------------------------------
// Simulation state
// ---------------------------------------------------------------------------

struct BeatDetector {
    avg: f32,
    prev: f32,
    last_tick: f32,
}

impl BeatDetector {
    fn new() -> Self {
        Self { avg: 0.0, prev: 0.0, last_tick: 0.0 }
    }

    fn detect(&mut self, frame: &AnalysisFrame) -> bool {
        let envelope = frame.rhythm[RHYTHM_SIZE - 1];
        self.avg = AVG_ALPHA * envelope + (1.0 - AVG_ALPHA) * self.avg;
        let threshold = self.avg * ONSET_RATIO;
        let was_below = self.prev < threshold;
        let is_above = envelope >= threshold;
        self.prev = envelope;

        if was_below && is_above && (frame.elapsed - self.last_tick) >= MIN_TICK_INTERVAL {
            self.last_tick = frame.elapsed;
            true
        } else {
            false
        }
    }
}

struct GolState {
    grid: Grid,
    cell_bins: Vec<usize>,
    cell_brightness: Vec<f32>,
    bin_onsets: BinOnsets,
    beat: BeatDetector,
    camera_x: f32,
    camera_y: f32,
    heading: f32,
    prev_elapsed: f32,
    silence_start: Option<f32>,
}

impl GolState {
    fn new() -> Self {
        Self {
            grid: Grid::new_random(GRID_W, GRID_H),
            cell_bins: build_cell_bins(GRID_W, GRID_H),
            cell_brightness: vec![0.0; CELL_COUNT],
            bin_onsets: BinOnsets::new(),
            beat: BeatDetector::new(),
            camera_x: 0.0,
            camera_y: 0.0,
            heading: 0.0,
            prev_elapsed: 0.0,
            silence_start: None,
        }
    }

    fn update_camera(&mut self, frame: &AnalysisFrame) {
        let dt = frame.elapsed - self.prev_elapsed;
        self.prev_elapsed = frame.elapsed;
        if dt <= 0.0 || dt > 0.5 {
            return;
        }

        // Slew heading toward rhythm rotation for sweepy direction changes.
        let target_heading = frame.rhythm_rotation;
        let delta = wrap_angle(target_heading - self.heading);
        self.heading += delta * HEADING_SLEW * dt;

        // Move.
        let dx = self.heading.cos();
        let dy = self.heading.sin();
        let bass: f32 = frame.bands[..4].iter().sum::<f32>() / 4.0;
        let speed = DRIFT_BASE + bass * DRIFT_MUSIC;
        self.camera_x += dx * speed * dt;
        self.camera_y += dy * speed * dt;
        self.camera_x = self.camera_x.rem_euclid(1.0);
        self.camera_y = self.camera_y.rem_euclid(1.0);
    }

    fn check_respawn(&mut self, frame: &AnalysisFrame) {
        let total_energy: f32 = frame.bands.iter().sum();
        let is_silent = total_energy < SILENCE_THRESHOLD;

        if is_silent {
            if self.silence_start.is_none() {
                self.silence_start = Some(frame.elapsed);
            }
        } else {
            // Sound returned — check if we were silent long enough.
            if let Some(start) = self.silence_start {
                let was_silent = (frame.elapsed - start) >= SILENCE_DURATION;
                if was_silent && self.grid.density() < RESPAWN_DENSITY {
                    self.grid.seed_random(RESPAWN_FILL);
                    log::info!("respawned cells after silence");
                }
            }
            self.silence_start = None;
        }
    }

    fn tick(&mut self, frame: &AnalysisFrame) {
        self.check_respawn(frame);

        // Global beat → everyone steps.
        if self.beat.detect(frame) {
            let all_fired = [true; NUM_BANDS];
            self.grid = self.grid.partial_step(&self.cell_bins, &all_fired);
            return;
        }

        // Per-bin onsets → regional steps.
        let fired = self.bin_onsets.detect(&frame.bands, frame.elapsed);
        if fired.iter().any(|&f| f) {
            self.grid = self.grid.partial_step(&self.cell_bins, &fired);
        }
    }

    fn update_brightness(&mut self, frame: &AnalysisFrame) {
        let (labels, infos) = label_components(&self.grid);

        for i in 0..CELL_COUNT {
            let label = labels[i];
            if label == 0 {
                self.cell_brightness[i] = 0.0;
            } else {
                let info = &infos[(label - 1) as usize];
                let bin = component_freq_bin(info, GRID_W, GRID_H);
                let energy = frame.bands[bin];
                self.cell_brightness[i] = 0.08 + energy * 0.92;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// GPU structs
// ---------------------------------------------------------------------------

/// Normalize angle delta to [-PI, PI].
fn wrap_angle(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    grid_w: u32,
    grid_h: u32,
    camera_x: f32,
    camera_y: f32,
}

// ---------------------------------------------------------------------------
// Visualization
// ---------------------------------------------------------------------------

pub struct GameOfLife {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    cell_buffer: wgpu::Buffer,
    state: RefCell<GolState>,
}

impl GameOfLife {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gol"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/gol.wgsl").into()),
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gol_uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let cell_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gol_cells"),
            size: (CELL_COUNT * std::mem::size_of::<f32>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("gol"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gol"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: cell_buffer.as_entire_binding(),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gol"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gol"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Self {
            pipeline,
            bind_group,
            uniform_buffer,
            cell_buffer,
            state: RefCell::new(GolState::new()),
        }
    }
}

impl Visualization for GameOfLife {
    fn on_activate(&mut self) {
        let mut state = self.state.borrow_mut();
        *state = GolState::new();
    }

    fn update(&self, _device: &wgpu::Device, queue: &wgpu::Queue, frame: &AnalysisFrame) {
        let mut state = self.state.borrow_mut();

        state.update_camera(frame);
        state.tick(frame);
        state.update_brightness(frame);

        let uniforms = Uniforms {
            grid_w: GRID_W as u32,
            grid_h: GRID_H as u32,
            camera_x: state.camera_x,
            camera_y: state.camera_y,
        };

        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));
        queue.write_buffer(
            &self.cell_buffer,
            0,
            bytemuck::cast_slice(&state.cell_brightness),
        );
    }

    fn render<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
