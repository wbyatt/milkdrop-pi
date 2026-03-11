use fontdue::Font;

const FONT_DATA: &[u8] = include_bytes!("../../fonts/VT323-Regular.ttf");
const FONT_SIZE: f32 = 24.0;
const ATLAS_COLUMNS: u32 = 16;
const FIRST_CHAR: u8 = 32;
const LAST_CHAR: u8 = 126;

pub struct GlyphAtlas {
    pub view: wgpu::TextureView,
    pub glyph_width: u32,
    pub glyph_height: u32,
    pub atlas_columns: u32,
    atlas_width: u32,
    atlas_height: u32,
}

impl GlyphAtlas {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let font = Font::from_bytes(FONT_DATA, fontdue::FontSettings::default())
            .expect("failed to load VT323 font");

        let (glyph_width, glyph_height) = measure_cell(&font);
        let char_count = (LAST_CHAR - FIRST_CHAR + 1) as u32;
        let rows = (char_count + ATLAS_COLUMNS - 1) / ATLAS_COLUMNS;
        let atlas_width = ATLAS_COLUMNS * glyph_width;
        let atlas_height = rows * glyph_height;

        let mut pixels = vec![0u8; (atlas_width * atlas_height) as usize];

        for c in FIRST_CHAR..=LAST_CHAR {
            let idx = (c - FIRST_CHAR) as u32;
            let col = idx % ATLAS_COLUMNS;
            let row = idx / ATLAS_COLUMNS;
            let origin_x = col * glyph_width;
            let origin_y = row * glyph_height;

            if c == FIRST_CHAR {
                // Fill space glyph cell with solid white for background quads
                for y in 0..glyph_height {
                    for x in 0..glyph_width {
                        pixels[((origin_y + y) * atlas_width + origin_x + x) as usize] = 255;
                    }
                }
            } else {
                let (metrics, bitmap) = font.rasterize(c as char, FONT_SIZE);
                let offset_y = glyph_height.saturating_sub(metrics.height as u32);
                for y in 0..metrics.height as u32 {
                    for x in 0..metrics.width as u32 {
                        let src = (y * metrics.width as u32 + x) as usize;
                        let dst_x = origin_x + x;
                        let dst_y = origin_y + offset_y + y;
                        if dst_x < atlas_width && dst_y < atlas_height {
                            pixels[(dst_y * atlas_width + dst_x) as usize] = bitmap[src];
                        }
                    }
                }
            }
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph_atlas"),
            size: wgpu::Extent3d {
                width: atlas_width,
                height: atlas_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(atlas_width),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: atlas_width,
                height: atlas_height,
                depth_or_array_layers: 1,
            },
        );

        let view = texture.create_view(&Default::default());

        Self {
            view,
            glyph_width,
            glyph_height,
            atlas_columns: ATLAS_COLUMNS,
            atlas_width,
            atlas_height,
        }
    }

    /// Returns (uv_offset, uv_size) for a character in the atlas.
    pub fn glyph_uv(&self, ch: char) -> ([f32; 2], [f32; 2]) {
        let c = (ch as u8).clamp(FIRST_CHAR, LAST_CHAR);
        let idx = (c - FIRST_CHAR) as u32;
        let col = idx % self.atlas_columns;
        let row = idx / self.atlas_columns;

        let u = (col * self.glyph_width) as f32 / self.atlas_width as f32;
        let v = (row * self.glyph_height) as f32 / self.atlas_height as f32;
        let uw = self.glyph_width as f32 / self.atlas_width as f32;
        let vh = self.glyph_height as f32 / self.atlas_height as f32;

        ([u, v], [uw, vh])
    }
}

fn measure_cell(font: &Font) -> (u32, u32) {
    let metrics = font.metrics('M', FONT_SIZE);
    let width = metrics.advance_width.ceil() as u32;
    let height = (FONT_SIZE).ceil() as u32;
    (width.max(1), height.max(1))
}
