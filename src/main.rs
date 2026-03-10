mod analysis;
mod audio;
mod cli;
mod render;
mod transition;
mod visualizations;

use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Fullscreen, Window, WindowAttributes, WindowId};

use analysis::SpectrumAnalyzer;
use audio::{AudioCapture, AudioReceiver};
use cli::Args;
use render::{Renderer, Visualization};
use transition::Compositor;

const TRANSITION_SECS: f32 = 3.0;

fn main() {
    env_logger::init();
    let args = <Args as clap::Parser>::parse();
    let event_loop = EventLoop::new().expect("failed to create event loop");
    let mut app = App {
        args,
        state: None,
    };
    event_loop.run_app(&mut app).expect("event loop error");
}

struct App {
    args: Args,
    state: Option<AppState>,
}

struct AppState {
    window: Arc<Window>,
    _capture: AudioCapture,
    receiver: AudioReceiver,
    analyzer: SpectrumAnalyzer,
    renderer: Renderer,
    compositor: Compositor,
    cycle: VisualizationCycle,
    sample_buffer: Vec<f32>,
}

struct VisualizationCycle {
    entries: Vec<Box<dyn Visualization>>,
    index: usize,
    last_switch: Instant,
    duration: Duration,
    transition: Option<TransitionState>,
}

struct TransitionState {
    from_index: usize,
    started: Instant,
}

impl VisualizationCycle {
    fn new(entries: Vec<Box<dyn Visualization>>, duration_secs: u64) -> Self {
        Self {
            entries,
            index: 0,
            last_switch: Instant::now(),
            duration: Duration::from_secs(duration_secs),
            transition: None,
        }
    }

    fn current(&self) -> &dyn Visualization {
        self.entries[self.index].as_ref()
    }

    fn current_mut(&mut self) -> &mut dyn Visualization {
        self.entries[self.index].as_mut()
    }

    fn viz(&self, index: usize) -> &dyn Visualization {
        self.entries[index].as_ref()
    }

    fn advance_if_due(&mut self) {
        if self.entries.len() <= 1 {
            return;
        }

        // Complete any finished transition
        if let Some(trans) = &self.transition {
            if trans.started.elapsed().as_secs_f32() >= TRANSITION_SECS {
                self.transition = None;
            }
            return; // don't start a new transition during one
        }

        if self.last_switch.elapsed() >= self.duration {
            let from = self.index;
            self.index = (self.index + 1) % self.entries.len();
            self.transition = Some(TransitionState {
                from_index: from,
                started: Instant::now(),
            });
            self.last_switch = Instant::now();
            log::info!("transitioning to visualization {}/{}", self.index + 1, self.entries.len());
        }
    }

    /// Returns (from_index, eased_mix) if a transition is active.
    fn transition_mix(&self) -> Option<(usize, f32)> {
        let trans = self.transition.as_ref()?;
        let t = (trans.started.elapsed().as_secs_f32() / TRANSITION_SECS).clamp(0.0, 1.0);
        let eased = t * t * (3.0 - 2.0 * t); // smoothstep
        Some((trans.from_index, eased))
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }

        let attrs = WindowAttributes::default()
            .with_title("milkdrop-pi")
            .with_inner_size(PhysicalSize::new(800u32, 600));
        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .expect("failed to create window"),
        );

        let (capture, receiver, audio_config) = AudioCapture::start();
        let analyzer = SpectrumAnalyzer::new(&audio_config);
        let renderer = Renderer::new(window.clone());

        let size = window.inner_size();
        let compositor = Compositor::new(renderer.device(), renderer.format(), size.width.max(1), size.height.max(1));

        let viz_names = self.args.viz_names();
        let entries = visualizations::create(&viz_names, renderer.device(), renderer.format());
        let cycle = VisualizationCycle::new(entries, self.args.duration);

        self.state = Some(AppState {
            window,
            _capture: capture,
            receiver,
            analyzer,
            renderer,
            compositor,
            cycle,
            sample_buffer: Vec::with_capacity(audio_config.sample_rate as usize),
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = &mut self.state else {
            return;
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key: Key::Named(NamedKey::F11),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => toggle_fullscreen(&state.window),
            WindowEvent::Resized(size) => {
                state.renderer.resize(size);
                state.compositor.resize(state.renderer.device(), size.width.max(1), size.height.max(1));
                state.cycle.current_mut().resize(size.width, size.height);
            }
            WindowEvent::RedrawRequested => {
                state.receiver.drain(&mut state.sample_buffer);
                let frame = state.analyzer.process(&state.sample_buffer);

                if let Some((from_idx, mix)) = state.cycle.transition_mix() {
                    let from = state.cycle.viz(from_idx);
                    let to = state.cycle.current();
                    state.renderer.render_transition(from, to, frame, &state.compositor, mix);
                } else {
                    state.renderer.render(state.cycle.current(), frame);
                }

                state.sample_buffer.clear();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(state) = &mut self.state {
            state.cycle.advance_if_due();
            state.window.request_redraw();
        }
    }
}

fn toggle_fullscreen(window: &Window) {
    let next = match window.fullscreen() {
        Some(_) => None,
        None => Some(Fullscreen::Borderless(None)),
    };
    window.set_fullscreen(next);
}
