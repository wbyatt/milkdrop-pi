# milkdrop-pi

A global Windows audio visualizer built in Rust. Captures system audio via WASAPI loopback, performs spectral analysis, and renders shader-driven visuals via wgpu — targeting a broken, staticky, color-inaccurate 1980s CRT.

## Architecture

- **Audio capture** → cpal (WASAPI loopback)
- **Analysis** → rustfft / spectral analysis
- **Rendering** → wgpu + WGSL shaders, winit windowing
- Threads communicate via lock-free ring buffers

## Audio Pipeline

- **Sample rate is determined at runtime** by querying the default output device. All downstream sizing derives from this value.
- **FFT is frame-locked:** one FFT per render frame (~60fps). Window size = samples accumulated in one frame period, zero-padded to the next power of 2.
  - 44.1kHz → 1024, 48kHz → 1024, 96kHz → 2048, 192kHz → 4096
- Frequency resolution is secondary to temporal smoothness. If bass resolution becomes an issue, zero-pad more aggressively or add 50% window overlap — but not preemptively.

## Design Principles

**Single Responsibility is the only hard rule.** Every software entity — function, struct, module, file — must operate at a single level of abstraction. If a function mixes orchestration with implementation detail, split it. If a module owns two unrelated concerns, separate them.

Downstream principles (encapsulation, open/closed, Liskov substitution, dependency injection, etc.) are heuristics in service of single responsibility, not rules of their own. Apply them when they reduce mixed abstraction levels; skip them when they'd add indirection without clarity.

Concretely:
- A function either coordinates other functions OR does leaf-level work, never both.
- A struct owns one secret (internal representation, resource handle, algorithm state). If it owns two, split it.
- A module groups entities that share an abstraction level and concern. Cross-cutting glue gets its own module.
- Prefer passing capabilities in (dependency injection) over reaching out to globals, but only when it clarifies responsibility boundaries.

## Display Target

The output display is a broken, staticky, color-inaccurate 1980s CRT. This is a feature, not a constraint. Design choices should work *with* the lo-fi aesthetic:

- Embrace banding, bloom, phosphor trails, and imprecise color
- Favor bold shapes and high-contrast patterns over fine detail
- Consider CRT-native artifacts (scanlines, convergence error, flicker) as compositional elements
- Subtlety reads as absence on this display — go vivid
