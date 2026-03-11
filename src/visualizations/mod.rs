pub mod equalizer;
pub mod flame;
pub mod gol;
pub mod polar;
pub mod terrain;

pub use equalizer::Equalizer;
pub use flame::FractalFlame;
pub use gol::GameOfLife;
pub use polar::Polar;
pub use terrain::Terrain;

use crate::render::Visualization;

struct Entry {
    name: &'static str,
    create: fn(&wgpu::Device, &wgpu::Queue, wgpu::TextureFormat) -> Box<dyn Visualization>,
}

const REGISTRY: &[Entry] = &[
    Entry {
        name: "equalizer",
        create: |device, _queue, format| Box::new(Equalizer::new(device, format)),
    },
    Entry {
        name: "gol",
        create: |device, _queue, format| Box::new(GameOfLife::new(device, format)),
    },
    Entry {
        name: "polar",
        create: |device, _queue, format| Box::new(Polar::new(device, format)),
    },
    Entry {
        name: "terrain",
        create: |device, _queue, format| Box::new(Terrain::new(device, format)),
    },
    Entry {
        name: "flame",
        create: |device, queue, format| Box::new(FractalFlame::new(device, queue, format)),
    },
];

pub fn available_names() -> Vec<&'static str> {
    REGISTRY.iter().map(|e| e.name).collect()
}

/// Creates all registered visualizations and returns (vizzes, names, enabled).
/// If `enabled_names` is non-empty, only those are initially enabled.
pub fn create_all(
    enabled_names: &[String],
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
) -> (Vec<Box<dyn Visualization>>, Vec<&'static str>, Vec<bool>) {
    // Validate requested names
    for name in enabled_names {
        if !REGISTRY.iter().any(|e| e.name == name) {
            let available: Vec<_> = available_names();
            panic!(
                "unknown visualization '{}'. available: {}",
                name,
                available.join(", ")
            );
        }
    }

    let names: Vec<&'static str> = REGISTRY.iter().map(|e| e.name).collect();
    let enabled: Vec<bool> = REGISTRY
        .iter()
        .map(|e| enabled_names.is_empty() || enabled_names.iter().any(|n| n == e.name))
        .collect();
    let vizzes: Vec<Box<dyn Visualization>> = REGISTRY
        .iter()
        .map(|e| (e.create)(device, queue, format))
        .collect();

    (vizzes, names, enabled)
}
