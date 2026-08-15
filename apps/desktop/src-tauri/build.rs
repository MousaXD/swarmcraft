use image::{ExtendedColorType, ImageFormat};
use std::{fs, path::Path};

fn main() {
    let icon_dir = Path::new("icons");
    fs::create_dir_all(icon_dir).expect("create Tauri icon directory");

    write_icon(&icon_dir.join("32x32.png"), 32, ImageFormat::Png);
    write_icon(&icon_dir.join("128x128.png"), 128, ImageFormat::Png);
    write_icon(&icon_dir.join("128x128@2x.png"), 256, ImageFormat::Png);
    write_icon(&icon_dir.join("icon.png"), 512, ImageFormat::Png);
    write_icon(&icon_dir.join("icon.ico"), 32, ImageFormat::Ico);

    tauri_build::build()
}

fn write_icon(path: &Path, size: u32, format: ImageFormat) {
    if path.exists() {
        return;
    }
    let pixels = vec![45u8, 45, 45, 255].repeat((size * size) as usize);
    image::save_buffer_with_format(path, &pixels, size, size, ExtendedColorType::Rgba8, format)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
}

