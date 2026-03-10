use crate::analysis::{AnalysisFrame, SPECTRUM_SIZE};
use crate::render::Visualization;

struct Buffers {
    spectrum_left: wgpu::Buffer,
    spectrum_right: wgpu::Buffer,
    uniforms: wgpu::Buffer,
}

pub struct Polar {
    pipeline: wgpu::RenderPipeline,
    buffers: Buffers,
    bind_group: wgpu::BindGroup,
}

impl Polar {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("polar"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/polar.wgsl").into(),
            ),
        });

        let buffers = Buffers {
            spectrum_left: create_storage_buffer(device, "spectrum_left", SPECTRUM_SIZE),
            spectrum_right: create_storage_buffer(device, "spectrum_right", SPECTRUM_SIZE),
            uniforms: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("polar_uniforms"),
                size: 16, // vec4 alignment: time + 3 padding
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        };

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("polar"),
                entries: &[
                    storage_binding_entry(0),
                    storage_binding_entry(1),
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
            label: Some("polar"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffers.spectrum_left.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buffers.spectrum_right.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffers.uniforms.as_entire_binding(),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("polar"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("polar"),
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

impl Visualization for Polar {
    fn update(&self, queue: &wgpu::Queue, frame: &AnalysisFrame) {
        queue.write_buffer(
            &self.buffers.spectrum_left,
            0,
            bytemuck::cast_slice(&frame.spectrum_left),
        );
        queue.write_buffer(
            &self.buffers.spectrum_right,
            0,
            bytemuck::cast_slice(&frame.spectrum_right),
        );
        queue.write_buffer(
            &self.buffers.uniforms,
            0,
            bytemuck::cast_slice(&[frame.elapsed, frame.rhythm_rotation, 0.0_f32, 0.0]),
        );
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
