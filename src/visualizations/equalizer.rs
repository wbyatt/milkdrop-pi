use crate::analysis::{AnalysisFrame, NUM_BANDS, RHYTHM_SIZE, WAVEFORM_SIZE};
use crate::render::Visualization;

struct Buffers {
    spectrum: wgpu::Buffer,
    waveform: wgpu::Buffer,
    rhythm: wgpu::Buffer,
}

pub struct Equalizer {
    pipeline: wgpu::RenderPipeline,
    buffers: Buffers,
    bind_group: wgpu::BindGroup,
}

impl Equalizer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("equalizer"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/equalizer.wgsl").into(),
            ),
        });

        let buffers = Buffers {
            spectrum: create_storage_buffer(device, "spectrum", NUM_BANDS),
            waveform: create_storage_buffer(device, "waveform", WAVEFORM_SIZE),
            rhythm: create_storage_buffer(device, "rhythm", RHYTHM_SIZE),
        };

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("equalizer"),
                entries: &[
                    storage_binding_entry(0),
                    storage_binding_entry(1),
                    storage_binding_entry(2),
                ],
            });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("equalizer"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffers.spectrum.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buffers.waveform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffers.rhythm.as_entire_binding(),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("equalizer"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("equalizer"),
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
        }
    }
}

impl Visualization for Equalizer {
    fn update(&self, queue: &wgpu::Queue, frame: &AnalysisFrame) {
        queue.write_buffer(&self.buffers.spectrum, 0, bytemuck::cast_slice(&frame.bands));
        queue.write_buffer(&self.buffers.waveform, 0, bytemuck::cast_slice(&frame.waveform));
        queue.write_buffer(&self.buffers.rhythm, 0, bytemuck::cast_slice(&frame.rhythm));
    }

    fn render<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn create_storage_buffer(device: &wgpu::Device, label: &str, count: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (count * std::mem::size_of::<f32>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn storage_binding_entry(index: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: index,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
