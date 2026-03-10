use std::sync::Arc;
use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::analysis::AnalysisFrame;
use crate::transition::Compositor;

/// A swappable visualization that draws into a render pass.
///
/// The renderer owns GPU infrastructure (device, surface, present). The visualization
/// owns its own pipeline, buffers, and bind groups, and knows how to draw itself.
pub trait Visualization {
    /// Upload per-frame analysis data to GPU buffers.
    fn update(&self, queue: &wgpu::Queue, frame: &AnalysisFrame);

    /// Issue draw commands into the active render pass.
    fn render<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>);

    /// Called when the window resizes. Default: no-op (UV-based shaders don't need this).
    fn resize(&mut self, _width: u32, _height: u32) {}
}

/// GPU infrastructure: device, surface, encoder, present.
/// Knows nothing about what is being drawn — that's the Visualization's job.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
}

impl Renderer {
    pub fn new(window: Arc<Window>) -> Self {
        pollster::block_on(Self::init(window))
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
    }

    pub fn render(&self, viz: &dyn Visualization, frame: &AnalysisFrame) {
        viz.update(&self.queue, frame);

        let output = match self.surface.get_current_texture() {
            Ok(tex) => tex,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            Err(e) => {
                log::error!("surface error: {}", e);
                return;
            }
        };

        let view = output.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("visualization"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });

            viz.render(&mut pass);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
    }

    /// Renders two visualizations blended through the compositor during a transition.
    pub fn render_transition(
        &self,
        viz_a: &dyn Visualization,
        viz_b: &dyn Visualization,
        frame: &AnalysisFrame,
        compositor: &Compositor,
        mix: f32,
    ) {
        viz_a.update(&self.queue, frame);
        viz_b.update(&self.queue, frame);

        let output = match self.surface.get_current_texture() {
            Ok(tex) => tex,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            Err(e) => {
                log::error!("surface error: {}", e);
                return;
            }
        };

        let output_view = output.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("transition_a"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: compositor.view_a(),
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            viz_a.render(&mut pass);
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("transition_b"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: compositor.view_b(),
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            viz_b.render(&mut pass);
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("crossfade"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            compositor.composite(&self.queue, &mut pass, mix);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
    }

    async fn init(window: Arc<Window>) -> Self {
        let size = window.inner_size();

        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window).unwrap();

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .expect("no compatible GPU adapter");

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default(), None)
            .await
            .expect("failed to create GPU device");

        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats[0];

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: select_present_mode(&caps),
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 1,
        };
        surface.configure(&device, &config);

        Self {
            device,
            queue,
            surface,
            config,
        }
    }
}

fn select_present_mode(caps: &wgpu::SurfaceCapabilities) -> wgpu::PresentMode {
    let preferred = [wgpu::PresentMode::Mailbox, wgpu::PresentMode::AutoVsync];
    for mode in preferred {
        if caps.present_modes.contains(&mode) {
            log::info!("present mode: {:?}", mode);
            return mode;
        }
    }
    caps.present_modes[0]
}
