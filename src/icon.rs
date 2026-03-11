use winit::window::Icon;

const ICON_RGBA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_rgba.bin"));
const ICON_SIZE: u32 = 48;

pub fn window_icon() -> Icon {
    Icon::from_rgba(ICON_RGBA.to_vec(), ICON_SIZE, ICON_SIZE).expect("invalid icon data")
}
