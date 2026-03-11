use std::cell::RefCell;

use crate::analysis::{AnalysisFrame, SPECTRUM_SIZE, WAVEFORM_SIZE};
use crate::render::Visualization;

const NUM_RIDGES: usize = 40;
const MAX_DEPTH: f32 = 18.0;
const BASE_SCROLL: f32 = 1.5;
const RHYTHM_SCROLL: f32 = 0.3;

struct Buffers {
    terrain_history: wgpu::Buffer,
    waveform: wgpu::Buffer,
    uniforms: wgpu::Buffer,
}

struct TerrainState {
    /// Flat circular buffer: NUM_RIDGES rows × SPECTRUM_SIZE columns.
    /// Each row is a frozen spectrum snapshot captured when that ridge entered.
    history: Vec<f32>,
    /// Next row to write (= oldest existing row).
    write_idx: usize,
    /// Total ridges born so far (for detecting new ridge crossings).
    prev_ridge_count: i64,
}

pub struct Terrain {
    pipeline: wgpu::RenderPipeline,
    buffers: Buffers,
    bind_group: wgpu::BindGroup,
    state: RefCell<TerrainState>,
}

impl Terrain {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("terrain"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/terrain.wgsl").into(),
            ),
        });

        let history_bytes = (NUM_RIDGES * SPECTRUM_SIZE * std::mem::size_of::<f32>()) as u64;

        let waveform_bytes = (WAVEFORM_SIZE * std::mem::size_of::<f32>()) as u64;

        let buffers = Buffers {
            terrain_history: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("terrain_history"),
                size: history_bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            waveform: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("terrain_waveform"),
                size: waveform_bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            uniforms: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("terrain_uniforms"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        };

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("terrain"),
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
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terrain"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffers.terrain_history.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buffers.waveform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffers.uniforms.as_entire_binding(),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terrain"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain"),
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
            buffers,
            bind_group,
            state: RefCell::new(TerrainState {
                history: vec![0.0; NUM_RIDGES * SPECTRUM_SIZE],
                write_idx: 0,
                prev_ridge_count: 0,
            }),
        }
    }
}

impl Visualization for Terrain {
    fn update(&self, _device: &wgpu::Device, queue: &wgpu::Queue, frame: &AnalysisFrame) {
        let mut state = self.state.borrow_mut();

        let scroll = frame.elapsed * BASE_SCROLL + frame.rhythm_rotation * RHYTHM_SCROLL;
        let ridge_spacing = MAX_DEPTH / NUM_RIDGES as f32;
        let s = scroll / ridge_spacing;
        let current_count = s.floor() as i64;

        let new_ridges = (current_count - state.prev_ridge_count)
            .max(0)
            .min(NUM_RIDGES as i64) as usize;

        if new_ridges > 0 {
            let row = compute_terrain_row(&frame.spectrum_left, &frame.spectrum_right);
            for _ in 0..new_ridges {
                let offset = state.write_idx * SPECTRUM_SIZE;
                state.history[offset..offset + SPECTRUM_SIZE].copy_from_slice(&row);
                state.write_idx = (state.write_idx + 1) % NUM_RIDGES;
            }
        }
        state.prev_ridge_count = current_count;

        queue.write_buffer(
            &self.buffers.terrain_history,
            0,
            bytemuck::cast_slice(&state.history),
        );
        queue.write_buffer(
            &self.buffers.waveform,
            0,
            bytemuck::cast_slice(&frame.waveform),
        );

        let bass: f32 = frame.spectrum_left[..8].iter().sum::<f32>()
            + frame.spectrum_right[..8].iter().sum::<f32>();

        queue.write_buffer(
            &self.buffers.uniforms,
            0,
            bytemuck::cast_slice(&[frame.elapsed, s.fract(), state.write_idx as f32, bass / 16.0]),
        );
    }

    fn render<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Mixes stereo spectrum into a single terrain row.
/// Bass (low indices) at center, treble at edges — symmetric mountain shapes.
fn compute_terrain_row(
    left: &[f32; SPECTRUM_SIZE],
    right: &[f32; SPECTRUM_SIZE],
) -> [f32; SPECTRUM_SIZE] {
    let mut row = [0.0f32; SPECTRUM_SIZE];
    for i in 0..SPECTRUM_SIZE {
        let x = i as f32 / (SPECTRUM_SIZE - 1) as f32;
        row[i] = if x < 0.5 {
            let t = 1.0 - x * 2.0;
            lerp_spectrum(left, t)
        } else {
            let t = (x - 0.5) * 2.0;
            lerp_spectrum(right, t)
        };
    }
    row
}

fn lerp_spectrum(spectrum: &[f32; SPECTRUM_SIZE], t: f32) -> f32 {
    let idx_f = t * (SPECTRUM_SIZE - 1) as f32;
    let idx = (idx_f as usize).min(SPECTRUM_SIZE - 2);
    let frac = idx_f - idx as f32;
    spectrum[idx] * (1.0 - frac) + spectrum[idx + 1] * frac
}
