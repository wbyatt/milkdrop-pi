mod atlas;
mod text;

use atlas::GlyphAtlas;
use text::{GlyphInstance, TextRenderer};
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, NamedKey};

pub enum OverlayAction {
    ToggleViz(usize),
    SetDuration(u64),
}

pub struct OverlayConfig {
    pub viz_names: Vec<&'static str>,
    pub viz_enabled: Vec<bool>,
    pub duration_secs: u64,
}

pub struct Overlay {
    visible: bool,
    cursor: usize,
    atlas: GlyphAtlas,
    text: TextRenderer,
}

impl Overlay {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let atlas = GlyphAtlas::new(device, queue);
        let text = TextRenderer::new(device, &atlas, format);
        Self {
            visible: false,
            cursor: 0,
            atlas,
            text,
        }
    }

    /// Returns an action if overlay state changed. Consumes the key if overlay is visible.
    /// Returns `(consumed, optional_action)`.
    pub fn handle_key(&mut self, event: &KeyEvent, config: &OverlayConfig) -> (bool, Option<OverlayAction>) {
        if event.state != ElementState::Pressed {
            return (false, None);
        }

        // Tilde toggles overlay visibility
        if let Key::Character(ch) = &event.logical_key {
            if ch.as_str() == "`" {
                self.visible = !self.visible;
                if self.visible {
                    self.cursor = self.cursor.min(config.viz_names.len().saturating_sub(1));
                }
                return (true, None);
            }
        }

        if !self.visible {
            return (false, None);
        }

        // When visible, consume all keys
        let action = match &event.logical_key {
            Key::Named(NamedKey::ArrowUp) => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
                None
            }
            Key::Named(NamedKey::ArrowDown) => {
                if self.cursor + 1 < config.viz_names.len() {
                    self.cursor += 1;
                }
                None
            }
            Key::Named(NamedKey::Space | NamedKey::Enter) => {
                Some(OverlayAction::ToggleViz(self.cursor))
            }
            Key::Named(NamedKey::ArrowLeft) => {
                let new_dur = config.duration_secs.saturating_sub(5).max(5);
                Some(OverlayAction::SetDuration(new_dur))
            }
            Key::Named(NamedKey::ArrowRight) => {
                let new_dur = config.duration_secs.saturating_add(5).min(600);
                Some(OverlayAction::SetDuration(new_dur))
            }
            _ => None,
        };

        (true, action)
    }

    pub fn prepare(&mut self, queue: &wgpu::Queue, config: &OverlayConfig, screen_w: u32, screen_h: u32) {
        if !self.visible {
            return;
        }

        let mut instances = Vec::with_capacity(512);
        let gw = self.atlas.glyph_width as f32;
        let gh = self.atlas.glyph_height as f32;
        let char_w = gw * 2.0 / screen_w as f32;
        let char_h = gh * 2.0 / screen_h as f32;

        // Panel dimensions in characters
        let panel_cols = 34u32;
        let panel_rows = (7 + config.viz_names.len()) as u32;

        let panel_w = panel_cols as f32 * char_w;
        let panel_h = panel_rows as f32 * char_h;
        let panel_x = -panel_w / 2.0;
        let panel_y = panel_h / 2.0; // top edge (NDC y-up)

        // Background quad using solid space glyph
        let (bg_uv_off, bg_uv_size) = self.atlas.glyph_uv(' ');
        instances.push(GlyphInstance {
            pos: [panel_x, panel_y],
            size: [panel_w, -panel_h], // negative height → downward
            uv_offset: bg_uv_off,
            uv_size: bg_uv_size,
            color: [0.0, 0.0, 0.0, 0.75],
        });

        let text_color = [0.0, 1.0, 0.4, 1.0]; // retro green
        let dim_color = [0.0, 0.6, 0.3, 0.7];
        let highlight_color = [1.0, 1.0, 0.2, 1.0];

        let mut row = 0usize;

        // Title line
        self.push_text(&mut instances, " MILKDROP CONFIG", panel_x, panel_y, char_w, char_h, row, text_color);
        let close_text = "` close ";
        let close_col = (panel_cols as usize).saturating_sub(close_text.len());
        self.push_text_at_col(&mut instances, close_text, panel_x, panel_y, char_w, char_h, row, close_col, dim_color);
        row += 2;

        // Section header
        self.push_text(&mut instances, " VISUALIZATIONS", panel_x, panel_y, char_w, char_h, row, dim_color);
        row += 1;

        // Viz entries
        for (i, name) in config.viz_names.iter().enumerate() {
            let is_selected = i == self.cursor;
            let marker = if is_selected { " > " } else { "   " };
            let check = if config.viz_enabled[i] { "[x] " } else { "[ ] " };
            let line = format!("{}{}{}", marker, check, name);
            let color = if is_selected { highlight_color } else { text_color };
            self.push_text(&mut instances, &line, panel_x, panel_y, char_w, char_h, row, color);
            row += 1;
        }

        row += 1;

        // Duration section
        self.push_text(&mut instances, " CYCLE DURATION", panel_x, panel_y, char_w, char_h, row, dim_color);
        row += 1;
        let dur_line = format!("   < {}s >", config.duration_secs);
        self.push_text(&mut instances, &dur_line, panel_x, panel_y, char_w, char_h, row, text_color);

        self.text.prepare(queue, &instances);
    }

    pub fn draw<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        if !self.visible {
            return;
        }
        self.text.draw(pass);
    }

    fn push_text(
        &self,
        instances: &mut Vec<GlyphInstance>,
        text: &str,
        panel_x: f32,
        panel_y: f32,
        char_w: f32,
        char_h: f32,
        row: usize,
        color: [f32; 4],
    ) {
        let y = panel_y - (row as f32 + 1.0) * char_h;
        for (col, ch) in text.chars().enumerate() {
            if ch == ' ' {
                continue; // skip spaces (background handles them)
            }
            let x = panel_x + col as f32 * char_w;
            let (uv_offset, uv_size) = self.atlas.glyph_uv(ch);
            instances.push(GlyphInstance {
                pos: [x, y + char_h], // top-left in NDC (y points up)
                size: [char_w, -char_h],
                uv_offset,
                uv_size,
                color,
            });
        }
    }

    fn push_text_at_col(
        &self,
        instances: &mut Vec<GlyphInstance>,
        text: &str,
        panel_x: f32,
        panel_y: f32,
        char_w: f32,
        char_h: f32,
        row: usize,
        start_col: usize,
        color: [f32; 4],
    ) {
        let y = panel_y - (row as f32 + 1.0) * char_h;
        let base_x = panel_x + start_col as f32 * char_w;
        for (col, ch) in text.chars().enumerate() {
            if ch == ' ' {
                continue;
            }
            let x = base_x + col as f32 * char_w;
            let (uv_offset, uv_size) = self.atlas.glyph_uv(ch);
            instances.push(GlyphInstance {
                pos: [x, y + char_h],
                size: [char_w, -char_h],
                uv_offset,
                uv_size,
                color,
            });
        }
    }
}
