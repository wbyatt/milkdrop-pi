pub mod equalizer;
pub mod gol;
pub mod polar;
pub mod terrain;

pub use equalizer::Equalizer;
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
];

pub fn available_names() -> Vec<&'static str> {
    REGISTRY.iter().map(|e| e.name).collect()
}

pub fn create(
    names: &[String],
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
) -> Vec<Box<dyn Visualization>> {
    let entries: Vec<&Entry> = if names.is_empty() {
        REGISTRY.iter().collect()
    } else {
        names
            .iter()
            .map(|name| {
                REGISTRY
                    .iter()
                    .find(|e| e.name == name)
                    .unwrap_or_else(|| {
                        let available: Vec<_> = available_names();
                        panic!(
                            "unknown visualization '{}'. available: {}",
                            name,
                            available.join(", ")
                        );
                    })
            })
            .collect()
    };

    entries
        .iter()
        .map(|e| (e.create)(device, queue, format))
        .collect()
}
