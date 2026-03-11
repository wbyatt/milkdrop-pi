use std::io::BufWriter;
use std::path::Path;

use image::codecs::ico::{IcoEncoder, IcoFrame};
use image::imageops::FilterType;
use image::ColorType;

const ICO_SIZES: [u32; 4] = [16, 32, 48, 256];
const RUNTIME_ICON_SIZE: u32 = 48;

fn main() {
    println!("cargo:rerun-if-changed=icon.png");

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let icon_path = Path::new(&manifest_dir).join("icon.png");

    let img = image::open(&icon_path).expect("failed to open icon.png");

    write_ico(&img, Path::new(&out_dir));
    write_runtime_rgba(&img, Path::new(&out_dir));
}

fn write_ico(img: &image::DynamicImage, out_dir: &Path) {
    let frames: Vec<IcoFrame> = ICO_SIZES
        .iter()
        .map(|&size| {
            let resized = img.resize_exact(size, size, FilterType::Lanczos3).into_rgba8();
            IcoFrame::as_png(resized.as_raw(), size, size, ColorType::Rgba8.into())
                .expect("failed to create ICO frame")
        })
        .collect();

    let ico_path = out_dir.join("icon.ico");
    let file = std::fs::File::create(&ico_path).expect("failed to create icon.ico");
    let encoder = IcoEncoder::new(BufWriter::new(file));
    encoder.encode_images(&frames).expect("failed to encode ICO");

    let mut res = winresource::WindowsResource::new();
    res.set_icon(ico_path.to_str().unwrap());
    res.compile().expect("failed to compile Windows resources");
}

fn write_runtime_rgba(img: &image::DynamicImage, out_dir: &Path) {
    let resized = img
        .resize_exact(RUNTIME_ICON_SIZE, RUNTIME_ICON_SIZE, FilterType::Lanczos3)
        .into_rgba8();
    std::fs::write(out_dir.join("icon_rgba.bin"), resized.as_raw())
        .expect("failed to write icon RGBA");
}
