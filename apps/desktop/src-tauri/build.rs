use std::{fs, path::Path};

const FALLBACK_ICON_PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0,
    1, 8, 4, 0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 252, 255,
    31, 0, 2, 235, 1, 245, 105, 118, 158, 76, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

fn main() {
    let icon_dir = Path::new("icons");
    fs::create_dir_all(icon_dir).expect("create Tauri icon directory");

    let png_path = icon_dir.join("icon.png");
    if !png_path.exists() {
        fs::write(&png_path, FALLBACK_ICON_PNG).expect("write fallback Tauri PNG icon");
    }

    let ico_path = icon_dir.join("icon.ico");
    if !ico_path.exists() {
        fs::write(&ico_path, fallback_ico()).expect("write fallback Tauri ICO icon");
    }

    tauri_build::build()
}

fn fallback_ico() -> Vec<u8> {
    let mut ico = Vec::with_capacity(22 + FALLBACK_ICON_PNG.len());
    ico.extend_from_slice(&0u16.to_le_bytes());
    ico.extend_from_slice(&1u16.to_le_bytes());
    ico.extend_from_slice(&1u16.to_le_bytes());
    ico.push(1);
    ico.push(1);
    ico.push(0);
    ico.push(0);
    ico.extend_from_slice(&1u16.to_le_bytes());
    ico.extend_from_slice(&32u16.to_le_bytes());
    ico.extend_from_slice(&(FALLBACK_ICON_PNG.len() as u32).to_le_bytes());
    ico.extend_from_slice(&22u32.to_le_bytes());
    ico.extend_from_slice(FALLBACK_ICON_PNG);
    ico
}
