use std::cell::RefCell;

use crate::analysis::{AnalysisFrame, NUM_BANDS};
use crate::render::Visualization;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const HISTOGRAM_W: u32 = 512;
const HISTOGRAM_H: u32 = 384;
const NUM_POINTS: u32 = 131_072; // 128K parallel points
const WORKGROUP_SIZE: u32 = 256;

const NUM_TRANSFORMS: usize = 4;

const DECAY_WORKGROUPS: u32 = (HISTOGRAM_W * HISTOGRAM_H + WORKGROUP_SIZE - 1) / WORKGROUP_SIZE;
const ITERATE_WORKGROUPS: u32 = (NUM_POINTS + WORKGROUP_SIZE - 1) / WORKGROUP_SIZE;

// Audio mapping
const EMA_FAST: f32 = 0.04;
const EMA_SLOW: f32 = 0.015;
const LFO_SPEED: f32 = 0.12; // radians per second
const LFO_AMPLITUDE: f32 = 0.05;
const BEAT_ONSET_RATIO: f32 = 1.3;
const BEAT_AVG_ALPHA: f32 = 0.08;
const PALETTE_KICK_STRENGTH: f32 = 0.15;
const PALETTE_KICK_DECAY: f32 = 0.92;

/// Band ranges for each transform.
const BAND_RANGES: [(usize, usize); 4] = [(0, 4), (4, 12), (12, 22), (22, 32)];

// ---------------------------------------------------------------------------
// FlameParams — the mathematical flame definition
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct TransformParams {
    affine: [f32; 6], // a, b, c, d, e, f
    weight: f32,
    color_index: f32,
    variation_weights: [f32; 6], // linear, sinusoidal, spherical, swirl, horseshoe, polar
}

#[derive(Clone)]
struct CameraParams {
    x: f32,
    y: f32,
    zoom: f32,
    rotation: f32,
}

#[derive(Clone)]
struct FlameParams {
    transforms: [TransformParams; NUM_TRANSFORMS],
    camera: CameraParams,
    palette_phase: f32,
    brightness: f32,
    gamma: f32,
}

// ---------------------------------------------------------------------------
// Genomes — curated base affine + variation sets that define macro shape.
// Each genome specifies 4 transforms (one per frequency band).
// The driver lerps between genomes on a fixed clock.
// ---------------------------------------------------------------------------

/// Per-transform base shape: affine coefficients and variation weight profile.
#[derive(Clone)]
struct TransformGenome {
    affine: [f32; 6],
    variations: [f32; 6], // linear, sinusoidal, spherical, swirl, horseshoe, polar
}

/// A complete flame macro shape — 4 transforms.
#[derive(Clone)]
struct Genome {
    transforms: [TransformGenome; NUM_TRANSFORMS],
}

/// Seconds per genome.  Fixed clock — not audio-derived.
const GENOME_PERIOD: f32 = 12.0;
/// Fraction of the period spent morphing to the next genome.
const MORPH_FRACTION: f32 = 0.35;

const GENOMES: &[Genome] = &[
    // 0: Spiral — rotation-dominant, the classic
    Genome {
        transforms: [
            TransformGenome { affine: [ 0.70, -0.15,  0.00,  0.15,  0.70,  0.00], variations: [0.15, 0.00, 0.45, 0.35, 0.00, 0.05] },
            TransformGenome { affine: [-0.50,  0.25,  0.30,  0.25,  0.50, -0.20], variations: [0.30, 0.10, 0.10, 0.40, 0.05, 0.05] },
            TransformGenome { affine: [ 0.35,  0.45, -0.15, -0.45,  0.35,  0.10], variations: [0.15, 0.35, 0.05, 0.05, 0.25, 0.15] },
            TransformGenome { affine: [ 0.25, -0.35,  0.10,  0.35,  0.25, -0.05], variations: [0.10, 0.40, 0.00, 0.00, 0.10, 0.40] },
        ],
    },
    // 1: Fern / branching — strong translations, scale-down, asymmetric
    Genome {
        transforms: [
            TransformGenome { affine: [ 0.85,  0.04,  0.00, -0.04,  0.85,  0.16], variations: [0.60, 0.00, 0.20, 0.20, 0.00, 0.00] },
            TransformGenome { affine: [ 0.20, -0.26,  0.00,  0.23,  0.22,  0.16], variations: [0.40, 0.20, 0.00, 0.30, 0.10, 0.00] },
            TransformGenome { affine: [-0.15,  0.28,  0.00,  0.26,  0.24,  0.04], variations: [0.40, 0.15, 0.10, 0.20, 0.15, 0.00] },
            TransformGenome { affine: [ 0.00,  0.00,  0.00,  0.00,  0.16,  0.00], variations: [0.80, 0.00, 0.00, 0.00, 0.00, 0.20] },
        ],
    },
    // 2: Nebula — spherical + sinusoidal dominant, diffuse cloud
    Genome {
        transforms: [
            TransformGenome { affine: [ 0.60,  0.00,  0.10,  0.00,  0.60, -0.10], variations: [0.10, 0.30, 0.40, 0.10, 0.05, 0.05] },
            TransformGenome { affine: [-0.50,  0.00, -0.15,  0.00,  0.50,  0.20], variations: [0.10, 0.40, 0.30, 0.05, 0.10, 0.05] },
            TransformGenome { affine: [ 0.45,  0.10,  0.20, -0.10,  0.45, -0.15], variations: [0.05, 0.35, 0.35, 0.10, 0.05, 0.10] },
            TransformGenome { affine: [ 0.30, -0.10, -0.10,  0.10,  0.30,  0.05], variations: [0.10, 0.25, 0.30, 0.05, 0.10, 0.20] },
        ],
    },
    // 3: Starburst — polar + horseshoe, radial spokes
    Genome {
        transforms: [
            TransformGenome { affine: [ 0.55,  0.00,  0.00,  0.00,  0.55,  0.00], variations: [0.10, 0.10, 0.10, 0.05, 0.25, 0.40] },
            TransformGenome { affine: [ 0.00,  0.55,  0.15, -0.55,  0.00, -0.15], variations: [0.05, 0.15, 0.05, 0.10, 0.30, 0.35] },
            TransformGenome { affine: [ 0.40, -0.40,  0.00,  0.40,  0.40,  0.00], variations: [0.15, 0.20, 0.05, 0.10, 0.20, 0.30] },
            TransformGenome { affine: [ 0.30,  0.00,  0.10,  0.00,  0.30, -0.10], variations: [0.05, 0.30, 0.10, 0.05, 0.15, 0.35] },
        ],
    },
    // 4: Butterfly — reflections + moderate rotation, bilateral
    Genome {
        transforms: [
            TransformGenome { affine: [-0.65,  0.10,  0.00,  0.10,  0.65,  0.00], variations: [0.30, 0.20, 0.20, 0.20, 0.05, 0.05] },
            TransformGenome { affine: [ 0.60, -0.20,  0.15,  0.20,  0.60, -0.10], variations: [0.25, 0.15, 0.15, 0.30, 0.10, 0.05] },
            TransformGenome { affine: [ 0.35,  0.35, -0.10, -0.35,  0.35,  0.10], variations: [0.20, 0.30, 0.10, 0.10, 0.15, 0.15] },
            TransformGenome { affine: [ 0.20,  0.00,  0.05,  0.00, -0.30,  0.00], variations: [0.15, 0.35, 0.10, 0.05, 0.10, 0.25] },
        ],
    },
    // 5: Vortex — heavy swirl, tight center, expanding arms
    Genome {
        transforms: [
            TransformGenome { affine: [ 0.50, -0.40,  0.00,  0.40,  0.50,  0.00], variations: [0.10, 0.05, 0.15, 0.60, 0.05, 0.05] },
            TransformGenome { affine: [ 0.60,  0.30,  0.20, -0.30,  0.60, -0.10], variations: [0.15, 0.10, 0.10, 0.50, 0.10, 0.05] },
            TransformGenome { affine: [-0.40,  0.50, -0.10, -0.50, -0.40,  0.15], variations: [0.10, 0.10, 0.20, 0.45, 0.05, 0.10] },
            TransformGenome { affine: [ 0.30,  0.00,  0.00,  0.00,  0.30,  0.00], variations: [0.20, 0.05, 0.25, 0.40, 0.00, 0.10] },
        ],
    },
    // 6: Crystal — Sierpinski-like, tight scale-down with spread translations
    Genome {
        transforms: [
            TransformGenome { affine: [ 0.50,  0.00,  0.50,  0.00,  0.50,  0.00], variations: [0.60, 0.15, 0.10, 0.05, 0.05, 0.05] },
            TransformGenome { affine: [ 0.50,  0.00, -0.50,  0.00,  0.50,  0.00], variations: [0.55, 0.20, 0.10, 0.05, 0.05, 0.05] },
            TransformGenome { affine: [ 0.50,  0.00,  0.00,  0.00,  0.50,  0.50], variations: [0.55, 0.10, 0.15, 0.10, 0.05, 0.05] },
            TransformGenome { affine: [ 0.40,  0.10,  0.00, -0.10,  0.40, -0.50], variations: [0.50, 0.15, 0.10, 0.10, 0.10, 0.05] },
        ],
    },
    // 7: Jellyfish — sinusoidal + spherical, drooping organic tendrils
    Genome {
        transforms: [
            TransformGenome { affine: [ 0.60,  0.00,  0.00,  0.10,  0.70, -0.30], variations: [0.10, 0.45, 0.30, 0.05, 0.05, 0.05] },
            TransformGenome { affine: [ 0.50,  0.15,  0.20, -0.15,  0.50,  0.20], variations: [0.15, 0.40, 0.25, 0.10, 0.05, 0.05] },
            TransformGenome { affine: [-0.45,  0.20, -0.10,  0.20,  0.45,  0.15], variations: [0.10, 0.40, 0.30, 0.10, 0.05, 0.05] },
            TransformGenome { affine: [ 0.30,  0.00,  0.00,  0.00,  0.25,  0.10], variations: [0.20, 0.35, 0.20, 0.05, 0.10, 0.10] },
        ],
    },
];

fn lerp_genome(a: &Genome, b: &Genome, t: f32) -> Genome {
    let mut out = a.clone();
    for i in 0..NUM_TRANSFORMS {
        for j in 0..6 {
            out.transforms[i].affine[j] =
                a.transforms[i].affine[j] + (b.transforms[i].affine[j] - a.transforms[i].affine[j]) * t;
            out.transforms[i].variations[j] =
                a.transforms[i].variations[j] + (b.transforms[i].variations[j] - a.transforms[i].variations[j]) * t;
        }
    }
    out
}

fn default_params() -> FlameParams {
    let g = &GENOMES[0];
    FlameParams {
        transforms: [
            TransformParams {
                affine: g.transforms[0].affine,
                weight: 0.30,
                color_index: 0.0,
                variation_weights: g.transforms[0].variations,
            },
            TransformParams {
                affine: g.transforms[1].affine,
                weight: 0.30,
                color_index: 0.25,
                variation_weights: g.transforms[1].variations,
            },
            TransformParams {
                affine: g.transforms[2].affine,
                weight: 0.25,
                color_index: 0.55,
                variation_weights: g.transforms[2].variations,
            },
            TransformParams {
                affine: g.transforms[3].affine,
                weight: 0.15,
                color_index: 0.80,
                variation_weights: g.transforms[3].variations,
            },
        ],
        camera: CameraParams {
            x: 0.0,
            y: 0.0,
            zoom: 0.4,
            rotation: 0.0,
        },
        palette_phase: 0.0,
        brightness: 0.07,
        gamma: 2.2,
    }
}

// ---------------------------------------------------------------------------
// Spring — position + velocity with frame-rate-independent damping.
// Underdamped tuning gives overshoot: values surge past the target and
// oscillate back, producing the sweepy inertial motion we want.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Spring {
    pos: f32,
    vel: f32,
}

impl Spring {
    fn new(initial: f32) -> Self {
        Self {
            pos: initial,
            vel: 0.0,
        }
    }

    /// Advance the spring toward `target`.
    /// `stiffness`: how hard it pulls toward target (higher = snappier).
    /// `damping`:   per-frame velocity retention at 60fps (0.96 = bouncy, 0.90 = settled).
    fn update(&mut self, target: f32, dt: f32, stiffness: f32, damping: f32) {
        let force = (target - self.pos) * stiffness;
        self.vel += force * dt;
        // Frame-rate-independent damping: at 60fps each frame multiplies by `damping`.
        self.vel *= damping.powf(dt * 60.0);
        self.pos += self.vel * dt;
    }
}

// ---------------------------------------------------------------------------
// FlameDriver — audio-to-parameter mapping
// ---------------------------------------------------------------------------

struct FlameDriver {
    // EMA-smoothed band energy for weights / variation modulation (should be smooth)
    smooth_band_energy: [f32; NUM_TRANSFORMS],
    smooth_total_energy: f32,
    smooth_centroid: f32,
    // Peak-hold envelopes: fast attack, slow release.  Feed the springs.
    peak_energy: [f32; NUM_TRANSFORMS],
    peak_bass: f32,
    // Springs for scale and camera (give inertia + overshoot)
    scale_springs: [Spring; NUM_TRANSFORMS],
    zoom_spring: Spring,
    rotation_spring: Spring,
    drift_x_spring: Spring,
    drift_y_spring: Spring,
    // Slow wander phase for camera XY base drift
    wander_phase: f32,
    // LFO + beat + palette state
    lfo_phase: [f32; NUM_TRANSFORMS],
    beat_avg: f32,
    beat_prev: f32,
    beat_last_tick: f32,
    palette_kick: f32,
    prev_elapsed: f32,
}

/// Peak-hold release rate: how fast the envelope decays back down between beats.
/// 0.97 at 60fps → half-life ≈ 0.38s, holds the peak long enough for the spring
/// to swing through it.
const PEAK_RELEASE: f32 = 0.97;

/// Spring tuning for per-transform scale.
/// stiffness 14 + damping 0.96 → natural period ~1.7s, damping ratio ~0.3.
/// Strongly underdamped: ~35% overshoot on first swing.
const SCALE_STIFFNESS: f32 = 14.0;
const SCALE_DAMPING: f32 = 0.96;

/// Spring tuning for camera zoom — slightly more damped so the global
/// perspective shift feels weighty rather than bouncy.
const ZOOM_STIFFNESS: f32 = 10.0;
const ZOOM_DAMPING: f32 = 0.94;

impl FlameDriver {
    fn new(params: &FlameParams) -> Self {
        Self {
            smooth_band_energy: [0.0; NUM_TRANSFORMS],
            smooth_total_energy: 0.0,
            smooth_centroid: 0.5,
            peak_energy: [0.0; NUM_TRANSFORMS],
            peak_bass: 0.0,
            scale_springs: [
                Spring::new(1.0),
                Spring::new(1.0),
                Spring::new(1.0),
                Spring::new(1.0),
            ],
            zoom_spring: Spring::new(params.camera.zoom),
            rotation_spring: Spring::new(0.0),
            drift_x_spring: Spring::new(0.0),
            drift_y_spring: Spring::new(0.0),
            wander_phase: 0.0,
            lfo_phase: [0.0; NUM_TRANSFORMS],
            beat_avg: 0.0,
            beat_prev: 0.0,
            beat_last_tick: 0.0,
            palette_kick: 0.0,
            prev_elapsed: 0.0,
        }
    }

    fn drive(&mut self, frame: &AnalysisFrame, genome: &Genome, params: &mut FlameParams) {
        let dt = frame.elapsed - self.prev_elapsed;
        self.prev_elapsed = frame.elapsed;
        if dt <= 0.0 || dt > 0.5 {
            return;
        }

        // --- Band energy ---

        let mut range_energy = [0.0f32; NUM_TRANSFORMS];
        for (i, &(lo, hi)) in BAND_RANGES.iter().enumerate() {
            let sum: f32 = frame.bands[lo..hi].iter().sum();
            range_energy[i] = sum / (hi - lo) as f32;
        }

        // EMA for smooth values (weights, variation modulation)
        for i in 0..NUM_TRANSFORMS {
            self.smooth_band_energy[i] =
                EMA_FAST * range_energy[i] + (1.0 - EMA_FAST) * self.smooth_band_energy[i];
        }

        // Peak-hold envelopes: instant attack, slow release
        for i in 0..NUM_TRANSFORMS {
            if range_energy[i] > self.peak_energy[i] {
                self.peak_energy[i] = range_energy[i];
            } else {
                self.peak_energy[i] *= PEAK_RELEASE.powf(dt * 60.0);
            }
        }

        let bass_avg: f32 = frame.bands[..4].iter().sum::<f32>() / 4.0;
        if bass_avg > self.peak_bass {
            self.peak_bass = bass_avg;
        } else {
            self.peak_bass *= PEAK_RELEASE.powf(dt * 60.0);
        }

        // Total energy + centroid (EMA, these should be smooth)
        let total: f32 = frame.bands.iter().sum();
        self.smooth_total_energy =
            EMA_SLOW * total + (1.0 - EMA_SLOW) * self.smooth_total_energy;

        let centroid = if total > 0.001 {
            let weighted_sum: f32 = frame
                .bands
                .iter()
                .enumerate()
                .map(|(i, &e)| i as f32 * e)
                .sum();
            weighted_sum / (total * (NUM_BANDS - 1) as f32)
        } else {
            0.5
        };
        self.smooth_centroid = EMA_SLOW * centroid + (1.0 - EMA_SLOW) * self.smooth_centroid;

        // --- Beat detection → palette kick ---

        self.beat_avg = BEAT_AVG_ALPHA * bass_avg + (1.0 - BEAT_AVG_ALPHA) * self.beat_avg;
        let beat_threshold = self.beat_avg * BEAT_ONSET_RATIO;
        let was_below = self.beat_prev < beat_threshold;
        let is_above = bass_avg >= beat_threshold;
        self.beat_prev = bass_avg;

        if was_below && is_above && (frame.elapsed - self.beat_last_tick) >= 0.15 {
            self.beat_last_tick = frame.elapsed;
            self.palette_kick += PALETTE_KICK_STRENGTH;
        }
        self.palette_kick *= PALETTE_KICK_DECAY;
        params.palette_phase += self.palette_kick * dt * 10.0;

        // --- Per-transform scale via springs ---
        // Peak energy → spring target.  The spring overshoots, giving each band
        // that dramatic forward/back sweep.

        for i in 0..NUM_TRANSFORMS {
            let target_scale = 1.0 + self.peak_energy[i] * 3.0;
            self.scale_springs[i].update(target_scale, dt, SCALE_STIFFNESS, SCALE_DAMPING);

            let scale = self.scale_springs[i].pos.max(0.2); // floor to prevent collapse
            self.lfo_phase[i] += dt * LFO_SPEED * (1.0 + i as f32 * 0.3);
            let lfo_drift = self.lfo_phase[i].sin() * LFO_AMPLITUDE;

            for j in 0..6 {
                let base = genome.transforms[i].affine[j];
                let drift = if j < 2 { lfo_drift } else { 0.0 };
                params.transforms[i].affine[j] =
                    (base * scale + drift).clamp(-2.5, 2.5);
            }
        }

        // --- Transform weights (smooth, not springy) ---

        for i in 0..NUM_TRANSFORMS {
            params.transforms[i].weight = (0.025 + self.smooth_band_energy[i] * 2.0).max(0.025);
        }

        // --- Variation profiles from genome, modulated by energy + centroid ---

        let centroid = self.smooth_centroid;
        for i in 0..NUM_TRANSFORMS {
            let energy = self.smooth_band_energy[i];
            for v in 0..6 {
                params.transforms[i].variation_weights[v] =
                    genome.transforms[i].variations[v] * (1.0 + energy * 0.5);
            }
            params.transforms[i].variation_weights[5] += centroid * 0.08;
            params.transforms[i].variation_weights[2] += (1.0 - centroid) * 0.08;
        }

        // --- Brightness (smooth) ---

        params.brightness = 0.05 + self.smooth_total_energy * 0.01;

        // --- Camera via springs: zoom, rotation, XY drift ---

        // Zoom: bass peaks push in, spring overshoots giving punch-in / pull-back.
        let zoom_target = 0.30 + self.peak_bass * 0.5;
        self.zoom_spring.update(zoom_target, dt, ZOOM_STIFFNESS, ZOOM_DAMPING);
        params.camera.zoom = self.zoom_spring.pos.clamp(0.15, 0.8);

        // Rotation: rhythm_rotation provides a slow accumulating angle.
        // Spring it so beats torque the camera and it coasts with inertia.
        // Stronger multiplier so the rotation is clearly visible.
        let rot_target = frame.rhythm_rotation * 0.4;
        self.rotation_spring.update(rot_target, dt, 6.0, 0.97);
        params.camera.rotation = self.rotation_spring.pos;

        // XY drift: slow circular wander as a base, spectral balance pushes
        // the center — bass-heavy pulls one direction, treble another.
        // Gives parallax as the flame slides across the frame.
        self.wander_phase += dt * 0.15;
        let wander_x = self.wander_phase.cos() * 0.15;
        let wander_y = (self.wander_phase * 0.7).sin() * 0.1; // different rate → Lissajous

        let spectral_push = (self.smooth_centroid - 0.5) * 0.3;
        let energy_push = self.smooth_total_energy * 0.02;

        let x_target = wander_x + spectral_push + energy_push;
        let y_target = wander_y - spectral_push * 0.5;
        self.drift_x_spring.update(x_target, dt, 4.0, 0.97);
        self.drift_y_spring.update(y_target, dt, 4.0, 0.97);
        params.camera.x = self.drift_x_spring.pos;
        params.camera.y = self.drift_y_spring.pos;
    }
}

// ---------------------------------------------------------------------------
// GPU-side struct layouts (must match WGSL)
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuTransform {
    affine: [f32; 6],
    weight: f32,
    color_index: f32,
    variations: [f32; 6],
    _pad: [f32; 2],
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct ComputeUniforms {
    camera_x: f32,
    camera_y: f32,
    camera_zoom: f32,
    camera_rotation: f32,
    frame_seed: u32,
    decay_factor: f32,
    histogram_w: u32,
    histogram_h: u32,
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct RenderUniforms {
    brightness: f32,
    gamma: f32,
    histogram_w: u32,
    histogram_h: u32,
}

// ---------------------------------------------------------------------------
// Mutable state behind RefCell
// ---------------------------------------------------------------------------

struct GenomeSequencer {
    index: usize,
    clock: f32, // seconds into current genome
}

impl GenomeSequencer {
    fn new() -> Self {
        Self { index: 0, clock: 0.0 }
    }

    /// Advance the clock and return the current (possibly morphing) genome.
    fn advance(&mut self, dt: f32) -> Genome {
        self.clock += dt;
        if self.clock >= GENOME_PERIOD {
            self.clock -= GENOME_PERIOD;
            self.index = (self.index + 1) % GENOMES.len();
        }

        let morph_start = GENOME_PERIOD * (1.0 - MORPH_FRACTION);
        if self.clock >= morph_start {
            let t = (self.clock - morph_start) / (GENOME_PERIOD * MORPH_FRACTION);
            let smoothed = t * t * (3.0 - 2.0 * t); // smoothstep
            let next = (self.index + 1) % GENOMES.len();
            lerp_genome(&GENOMES[self.index], &GENOMES[next], smoothed)
        } else {
            GENOMES[self.index].clone()
        }
    }
}

struct FlameState {
    params: FlameParams,
    driver: FlameDriver,
    sequencer: GenomeSequencer,
    needs_reinit: bool,
    frame_seed: u32,
}

impl FlameState {
    fn new() -> Self {
        let params = default_params();
        let driver = FlameDriver::new(&params);
        Self {
            params,
            driver,
            sequencer: GenomeSequencer::new(),
            needs_reinit: false,
            frame_seed: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Palette generation
// ---------------------------------------------------------------------------

fn generate_palette() -> Vec<[f32; 4]> {
    // Cool palette with a narrow yellow accent for contrast.
    let stops: &[(f32, [f32; 3])] = &[
        (0.0,  [0.9, 0.0, 0.7]),    // magenta
        (0.25, [0.15, 0.2, 1.0]),    // deep blue
        (0.45, [0.0, 0.5, 0.6]),     // teal
        (0.55, [0.9, 0.85, 0.1]),    // yellow (narrow 10% band)
        (0.65, [0.0, 0.5, 0.6]),     // back to teal
        (0.80, [0.6, 0.1, 0.9]),     // violet
        (1.0,  [0.9, 0.0, 0.7]),     // magenta (wrap)
    ];

    (0..256)
        .map(|i| {
            let t = i as f32 / 255.0;
            // Find surrounding stops
            let mut lo = 0;
            for s in 1..stops.len() {
                if stops[s].0 >= t {
                    lo = s - 1;
                    break;
                }
            }
            let hi = lo + 1;
            let span = stops[hi].0 - stops[lo].0;
            let frac = if span > 0.0 {
                (t - stops[lo].0) / span
            } else {
                0.0
            };
            let c0 = stops[lo].1;
            let c1 = stops[hi].1;
            [
                c0[0] + (c1[0] - c0[0]) * frac,
                c0[1] + (c1[1] - c0[1]) * frac,
                c0[2] + (c1[2] - c0[2]) * frac,
                1.0,
            ]
        })
        .collect()
}

fn apply_palette_phase(base: &[[f32; 4]], phase: f32) -> Vec<[f32; 4]> {
    let n = base.len();
    (0..n)
        .map(|i| {
            let shifted = (i as f32 / n as f32 + phase).fract();
            let pos = shifted * (n - 1) as f32;
            let lo = pos as usize;
            let hi = (lo + 1).min(n - 1);
            let frac = pos - lo as f32;
            let c0 = base[lo];
            let c1 = base[hi];
            [
                c0[0] + (c1[0] - c0[0]) * frac,
                c0[1] + (c1[1] - c0[1]) * frac,
                c0[2] + (c1[2] - c0[2]) * frac,
                1.0,
            ]
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Random initial points
// ---------------------------------------------------------------------------

fn generate_initial_points() -> Vec<[f32; 4]> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::time::SystemTime;

    let mut hasher = DefaultHasher::new();
    SystemTime::now().hash(&mut hasher);
    let mut seed = hasher.finish();

    (0..NUM_POINTS)
        .map(|_| {
            let xor = |s: &mut u64| {
                *s ^= *s << 13;
                *s ^= *s >> 7;
                *s ^= *s << 17;
                *s
            };
            let x = (xor(&mut seed) as f32 / u64::MAX as f32) * 2.0 - 1.0;
            let y = (xor(&mut seed) as f32 / u64::MAX as f32) * 2.0 - 1.0;
            let c = (xor(&mut seed) as f32 / u64::MAX as f32).clamp(0.0, 1.0);
            [x, y, c, 0.0]
        })
        .collect()
}

// ---------------------------------------------------------------------------
// FractalFlame — GPU resources + orchestration
// ---------------------------------------------------------------------------

pub struct FractalFlame {
    decay_pipeline: wgpu::ComputePipeline,
    iterate_pipeline: wgpu::ComputePipeline,
    render_pipeline: wgpu::RenderPipeline,
    compute_bind_group: wgpu::BindGroup,
    render_bind_group: wgpu::BindGroup,
    histogram_buffer: wgpu::Buffer,
    point_state_buffer: wgpu::Buffer,
    transform_buffer: wgpu::Buffer,
    compute_uniform_buffer: wgpu::Buffer,
    render_uniform_buffer: wgpu::Buffer,
    palette_buffer: wgpu::Buffer,
    base_palette: Vec<[f32; 4]>,
    state: RefCell<FlameState>,
}

impl FractalFlame {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Self {
        let compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flame_compute"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/flame_compute.wgsl").into(),
            ),
        });

        let render_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flame_render"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/flame_render.wgsl").into(),
            ),
        });

        // --- Buffers ---

        let histogram_size = (HISTOGRAM_W * HISTOGRAM_H * 2 * 4) as u64;
        let histogram_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flame_histogram"),
            size: histogram_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&histogram_buffer, 0, &vec![0u8; histogram_size as usize]);

        let point_state_size = (NUM_POINTS as u64) * 16;
        let point_state_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flame_points"),
            size: point_state_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let initial_points = generate_initial_points();
        queue.write_buffer(
            &point_state_buffer,
            0,
            bytemuck::cast_slice(&initial_points),
        );

        let transform_size = (NUM_TRANSFORMS * std::mem::size_of::<GpuTransform>()) as u64;
        let transform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flame_transforms"),
            size: transform_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let compute_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flame_compute_uniforms"),
            size: std::mem::size_of::<ComputeUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let render_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flame_render_uniforms"),
            size: std::mem::size_of::<RenderUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let palette_size = (256 * 16) as u64; // 256 × vec4<f32>
        let palette_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flame_palette"),
            size: palette_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let base_palette = generate_palette();
        queue.write_buffer(&palette_buffer, 0, bytemuck::cast_slice(&base_palette));

        // --- Compute bind group ---

        let compute_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("flame_compute"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flame_compute"),
            layout: &compute_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: compute_uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: transform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: histogram_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: point_state_buffer.as_entire_binding(),
                },
            ],
        });

        // --- Render bind group ---

        let render_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("flame_render"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
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

        let render_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flame_render"),
            layout: &render_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: histogram_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: render_uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: palette_buffer.as_entire_binding(),
                },
            ],
        });

        // --- Pipelines ---

        let compute_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("flame_compute"),
                bind_group_layouts: &[&compute_bgl],
                push_constant_ranges: &[],
            });

        let decay_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("flame_decay"),
            layout: Some(&compute_pipeline_layout),
            module: &compute_shader,
            entry_point: Some("decay"),
            compilation_options: Default::default(),
            cache: None,
        });

        let iterate_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("flame_iterate"),
            layout: Some(&compute_pipeline_layout),
            module: &compute_shader,
            entry_point: Some("iterate"),
            compilation_options: Default::default(),
            cache: None,
        });

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("flame_render"),
                bind_group_layouts: &[&render_bgl],
                push_constant_ranges: &[],
            });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("flame_render"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &render_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &render_shader,
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
            decay_pipeline,
            iterate_pipeline,
            render_pipeline,
            compute_bind_group,
            render_bind_group,
            histogram_buffer,
            point_state_buffer,
            transform_buffer,
            compute_uniform_buffer,
            render_uniform_buffer,
            palette_buffer,
            base_palette,
            state: RefCell::new(FlameState::new()),
        }
    }

    fn write_uniforms_and_dispatch(&self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let state = self.state.borrow();

        // Write transform data
        let gpu_transforms: Vec<GpuTransform> = state
            .params
            .transforms
            .iter()
            .map(|t| GpuTransform {
                affine: t.affine,
                weight: t.weight,
                color_index: t.color_index,
                variations: t.variation_weights,
                _pad: [0.0; 2],
            })
            .collect();
        queue.write_buffer(
            &self.transform_buffer,
            0,
            bytemuck::cast_slice(&gpu_transforms),
        );

        // Write compute uniforms
        let compute_uniforms = ComputeUniforms {
            camera_x: state.params.camera.x,
            camera_y: state.params.camera.y,
            camera_zoom: state.params.camera.zoom,
            camera_rotation: state.params.camera.rotation,
            frame_seed: state.frame_seed,
            decay_factor: 0.75,
            histogram_w: HISTOGRAM_W,
            histogram_h: HISTOGRAM_H,
        };
        queue.write_buffer(
            &self.compute_uniform_buffer,
            0,
            bytemuck::cast_slice(&[compute_uniforms]),
        );

        // Write render uniforms
        let render_uniforms = RenderUniforms {
            brightness: state.params.brightness,
            gamma: state.params.gamma,
            histogram_w: HISTOGRAM_W,
            histogram_h: HISTOGRAM_H,
        };
        queue.write_buffer(
            &self.render_uniform_buffer,
            0,
            bytemuck::cast_slice(&[render_uniforms]),
        );

        // Write palette with phase shift
        let shifted_palette =
            apply_palette_phase(&self.base_palette, state.params.palette_phase);
        queue.write_buffer(
            &self.palette_buffer,
            0,
            bytemuck::cast_slice(&shifted_palette),
        );

        drop(state);

        // Dispatch compute work
        let mut encoder = device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("flame_compute"),
            });

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("flame_decay"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.decay_pipeline);
            pass.set_bind_group(0, &self.compute_bind_group, &[]);
            pass.dispatch_workgroups(DECAY_WORKGROUPS, 1, 1);
        }

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("flame_iterate"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.iterate_pipeline);
            pass.set_bind_group(0, &self.compute_bind_group, &[]);
            pass.dispatch_workgroups(ITERATE_WORKGROUPS, 1, 1);
        }

        queue.submit(std::iter::once(encoder.finish()));
    }

    fn reinit(&self, queue: &wgpu::Queue) {
        let histogram_size = (HISTOGRAM_W * HISTOGRAM_H * 2 * 4) as usize;
        queue.write_buffer(&self.histogram_buffer, 0, &vec![0u8; histogram_size]);

        let initial_points = generate_initial_points();
        queue.write_buffer(
            &self.point_state_buffer,
            0,
            bytemuck::cast_slice(&initial_points),
        );
    }
}

impl Visualization for FractalFlame {
    fn on_activate(&mut self) {
        let mut state = self.state.borrow_mut();
        state.params = default_params();
        state.driver = FlameDriver::new(&state.params);
        state.sequencer = GenomeSequencer::new();
        state.needs_reinit = true;
    }

    fn update(&self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &AnalysisFrame) {
        {
            let mut state = self.state.borrow_mut();

            if state.needs_reinit {
                state.needs_reinit = false;
                drop(state);
                self.reinit(queue);
                let mut state = self.state.borrow_mut();
                state.frame_seed = state.frame_seed.wrapping_add(1);
                let dt = frame.elapsed - state.driver.prev_elapsed;
                let genome = state.sequencer.advance(dt.max(0.0).min(0.5));
                let mut params = state.params.clone();
                state.driver.drive(frame, &genome, &mut params);
                state.params = params;
            } else {
                state.frame_seed = state.frame_seed.wrapping_add(1);
                let dt = frame.elapsed - state.driver.prev_elapsed;
                let genome = state.sequencer.advance(dt.max(0.0).min(0.5));
                let mut params = state.params.clone();
                state.driver.drive(frame, &genome, &mut params);
                state.params = params;
            }
        }

        self.write_uniforms_and_dispatch(device, queue);
    }

    fn render<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        pass.set_pipeline(&self.render_pipeline);
        pass.set_bind_group(0, &self.render_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
