use rustfft::num_complex::Complex;
use rustfft::FftPlanner;
use std::sync::Arc;
use std::time::Instant;

use crate::audio::AudioConfig;

pub const NUM_BANDS: usize = 32;
pub const WAVEFORM_SIZE: usize = 1024;
pub const RHYTHM_SIZE: usize = 256;
pub const SPECTRUM_SIZE: usize = 256;

/// EMA rise coefficient — near-instant onset for tight audio-visual sync.
const ATTACK: f32 = 0.95;
/// EMA fall coefficient — bars decay slowly (VU meter feel).
const DECAY: f32 = 0.2;
/// Floor in dB for magnitude normalization.
const DB_FLOOR: f32 = -60.0;
/// Rhythm envelope low-pass cutoff in Hz (~3 Hz captures beat pulses).
const RHYTHM_CUTOFF_HZ: f32 = 3.0;
/// Render rate assumption for rhythm filter coefficient.
const RENDER_FPS: f32 = 60.0;
/// Rotation speed: tuned so typical music (~0.1 avg RMS) gives ~π/2 per 2s (quarter turn per measure at ~120 BPM).
const ROTATION_SPEED: f32 = 8.0;

/// All per-frame analysis results, ready for any visualization.
pub struct AnalysisFrame {
    pub bands: [f32; NUM_BANDS],
    pub waveform: [f32; WAVEFORM_SIZE],
    pub rhythm: [f32; RHYTHM_SIZE],
    pub spectrum_left: [f32; SPECTRUM_SIZE],
    pub spectrum_right: [f32; SPECTRUM_SIZE],
    /// Seconds since the analyzer was created. Available to all visualizations.
    pub elapsed: f32,
    /// Accumulated rotation angle (radians) driven by rhythmic energy.
    pub rhythm_rotation: f32,
}

pub struct SpectrumAnalyzer {
    fft: Arc<dyn rustfft::Fft<f32>>,
    fft_size: usize,
    sample_rate: u32,
    channels: u16,
    window: Vec<f32>,
    fft_buffer: Vec<Complex<f32>>,
    frame: AnalysisFrame,
    rhythm_state: RhythmState,
    rotation_accumulator: f32,
    start_time: Instant,
}

struct RhythmState {
    envelope: f32,
    alpha: f32,
    history: [f32; RHYTHM_SIZE],
    write_idx: usize,
}

impl RhythmState {
    fn new() -> Self {
        let alpha = 1.0 - (-2.0 * std::f32::consts::PI * RHYTHM_CUTOFF_HZ / RENDER_FPS).exp();
        Self {
            envelope: 0.0,
            alpha,
            history: [0.0; RHYTHM_SIZE],
            write_idx: 0,
        }
    }

    fn push(&mut self, rms: f32) {
        self.envelope = self.alpha * rms + (1.0 - self.alpha) * self.envelope;
        self.history[self.write_idx] = self.envelope;
        self.write_idx = (self.write_idx + 1) % RHYTHM_SIZE;
    }

    fn read_into(&self, out: &mut [f32; RHYTHM_SIZE]) {
        for i in 0..RHYTHM_SIZE {
            out[i] = self.history[(self.write_idx + i) % RHYTHM_SIZE];
        }
    }
}

impl SpectrumAnalyzer {
    pub fn new(config: &AudioConfig) -> Self {
        let fft_size = compute_fft_size(config.sample_rate);
        log::info!("FFT size: {} ({} Hz sample rate)", fft_size, config.sample_rate);

        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(fft_size);
        let window = hann_window(fft_size);

        Self {
            fft,
            fft_size,
            sample_rate: config.sample_rate,
            channels: config.channels,
            window,
            fft_buffer: vec![Complex::default(); fft_size],
            frame: AnalysisFrame {
                bands: [0.0; NUM_BANDS],
                waveform: [0.0; WAVEFORM_SIZE],
                rhythm: [0.0; RHYTHM_SIZE],
                spectrum_left: [0.0; SPECTRUM_SIZE],
                spectrum_right: [0.0; SPECTRUM_SIZE],
                elapsed: 0.0,
                rhythm_rotation: 0.0,
            },
            rhythm_state: RhythmState::new(),
            rotation_accumulator: 0.0,
            start_time: Instant::now(),
        }
    }

    /// Processes raw interleaved audio samples into all analysis outputs.
    /// When no new samples are available, holds the previous frame unchanged.
    pub fn process(&mut self, samples: &[f32]) -> &AnalysisFrame {
        self.frame.elapsed = self.start_time.elapsed().as_secs_f32();
        if samples.is_empty() {
            return &self.frame;
        }

        let mono = mix_to_mono(samples, self.channels);
        let (left, right) = split_channels(samples, self.channels);

        self.update_bands(&mono);
        self.update_waveform(&mono);
        self.update_rhythm(&mono);
        self.update_stereo_spectra(&left, &right);

        &self.frame
    }

    fn update_bands(&mut self, mono: &[f32]) {
        self.fill_fft_buffer(mono);
        self.fft.process(&mut self.fft_buffer);
        let magnitudes = compute_magnitudes(&self.fft_buffer, self.fft_size);
        let mut raw_bands = [0.0; NUM_BANDS];
        map_frequencies(&magnitudes, self.sample_rate, self.fft_size, &mut raw_bands);
        smooth(&mut self.frame.bands, &raw_bands);
    }

    fn update_waveform(&mut self, mono: &[f32]) {
        fill_waveform(&mut self.frame.waveform, mono);
    }

    fn update_rhythm(&mut self, mono: &[f32]) {
        let rms = compute_rms(mono);
        self.rhythm_state.push(rms);
        self.rhythm_state.read_into(&mut self.frame.rhythm);

        let dt = 1.0 / RENDER_FPS;
        self.rotation_accumulator += self.rhythm_state.envelope * ROTATION_SPEED * dt;
        self.frame.rhythm_rotation = self.rotation_accumulator;
    }

    fn update_stereo_spectra(&mut self, left: &[f32], right: &[f32]) {
        let left_mags = self.compute_channel_magnitudes(left);
        let mut raw_left = [0.0; SPECTRUM_SIZE];
        map_frequencies(&left_mags, self.sample_rate, self.fft_size, &mut raw_left);
        smooth(&mut self.frame.spectrum_left, &raw_left);

        let right_mags = self.compute_channel_magnitudes(right);
        let mut raw_right = [0.0; SPECTRUM_SIZE];
        map_frequencies(&right_mags, self.sample_rate, self.fft_size, &mut raw_right);
        smooth(&mut self.frame.spectrum_right, &raw_right);
    }

    fn compute_channel_magnitudes(&mut self, channel: &[f32]) -> Vec<f32> {
        self.fill_fft_buffer(channel);
        self.fft.process(&mut self.fft_buffer);
        compute_magnitudes(&self.fft_buffer, self.fft_size)
    }

    fn fill_fft_buffer(&mut self, samples: &[f32]) {
        let take = samples.len().min(self.fft_size);
        let offset = self.fft_size.saturating_sub(take);

        for c in &mut self.fft_buffer {
            *c = Complex::default();
        }

        for i in 0..take {
            let src = samples.len() - take + i;
            let dst = offset + i;
            self.fft_buffer[dst].re = samples[src] * self.window[dst];
        }
    }
}

// ---------------------------------------------------------------------------
// Pure functions — each operates at a single level of abstraction.
// ---------------------------------------------------------------------------

fn compute_fft_size(sample_rate: u32) -> usize {
    let samples_per_frame = (sample_rate as f32 / 60.0).ceil() as usize;
    samples_per_frame.next_power_of_two()
}

fn hann_window(size: usize) -> Vec<f32> {
    use std::f32::consts::PI;
    (0..size)
        .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / (size - 1) as f32).cos()))
        .collect()
}

fn mix_to_mono(samples: &[f32], channels: u16) -> Vec<f32> {
    if channels == 1 {
        return samples.to_vec();
    }
    let ch = channels as usize;
    samples
        .chunks_exact(ch)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

fn split_channels(samples: &[f32], channels: u16) -> (Vec<f32>, Vec<f32>) {
    if channels < 2 {
        return (samples.to_vec(), samples.to_vec());
    }
    let ch = channels as usize;
    let mut left = Vec::with_capacity(samples.len() / ch);
    let mut right = Vec::with_capacity(samples.len() / ch);
    for frame in samples.chunks_exact(ch) {
        left.push(frame[0]);
        right.push(frame[1]);
    }
    (left, right)
}

fn compute_magnitudes(fft_output: &[Complex<f32>], fft_size: usize) -> Vec<f32> {
    let scale = 1.0 / fft_size as f32;
    fft_output[..fft_size / 2]
        .iter()
        .map(|c| (c.re * c.re + c.im * c.im).sqrt() * scale)
        .collect()
}

/// Maps linear FFT bins into logarithmically-spaced perceptual frequency bands.
/// Works for any output size (32 bands for EQ, 256 for polar spectrum, etc.).
fn map_frequencies(magnitudes: &[f32], sample_rate: u32, fft_size: usize, output: &mut [f32]) {
    let num_bins = output.len();
    let min_freq = 20.0_f32;
    let max_freq = (sample_rate as f32 / 2.0).min(20_000.0);
    let bin_width = sample_rate as f32 / fft_size as f32;

    for i in 0..num_bins {
        let f_lo = min_freq * (max_freq / min_freq).powf(i as f32 / num_bins as f32);
        let f_hi = min_freq * (max_freq / min_freq).powf((i + 1) as f32 / num_bins as f32);

        let bin_lo = (f_lo / bin_width) as usize;
        let bin_hi = ((f_hi / bin_width) as usize + 1).min(magnitudes.len());
        let bin_lo = bin_lo.min(bin_hi.saturating_sub(1));

        if bin_lo < bin_hi {
            let sum: f32 = magnitudes[bin_lo..bin_hi].iter().sum();
            let avg = sum / (bin_hi - bin_lo) as f32;
            let db = 20.0 * (avg + 1e-10).log10();
            output[i] = ((db - DB_FLOOR) / -DB_FLOOR).clamp(0.0, 1.0);
        } else {
            output[i] = 0.0;
        }
    }
}

/// EMA smoothing: fast attack, slow decay. Works on any equal-length slices.
fn smooth(smoothed: &mut [f32], raw: &[f32]) {
    for i in 0..smoothed.len() {
        let alpha = if raw[i] > smoothed[i] { ATTACK } else { DECAY };
        smoothed[i] = alpha * raw[i] + (1.0 - alpha) * smoothed[i];
    }
}

fn compute_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

fn fill_waveform(out: &mut [f32; WAVEFORM_SIZE], mono: &[f32]) {
    if mono.is_empty() {
        out.fill(0.0);
        return;
    }
    for i in 0..WAVEFORM_SIZE {
        let src = i * mono.len() / WAVEFORM_SIZE;
        out[i] = mono[src];
    }
}
