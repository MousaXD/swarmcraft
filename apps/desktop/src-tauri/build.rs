use image::{ExtendedColorType, ImageFormat};
use std::{fs, path::Path};

const ICON_SIZE: u32 = 32;

fn main() {
    let icon_dir = Path::new("icons");
    fs::create_dir_all(icon_dir).expect("create Tauri icon directory");

    let pixels = vec![45u8, 45, 45, 255].repeat((ICON_SIZE * ICON_SIZE) as usize);
    write_icon(&icon_dir.join("icon.png"), &pixels, ImageFormat::Png);
    write_icon(&icon_dir.join("icon.ico"), &pixels, ImageFormat::Ico);

    tauri_build::build()
}

fn write_icon(path: &Path, pixels: &[u8], format: ImageFormat) {
    if path.exists() {
        return;
    }
    image::save_buffer_with_format(path, pixels, ICON_SIZE, ICON_SIZE, ExtendedColorType::Rgba8, format)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
}
