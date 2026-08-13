use std::{fs, path::Path};

const FALLBACK_ICON_PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0,
    1, 8, 4, 0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 252, 255,
    31, 0, 2, 235, 1, 245, 105, 118, 158, 76, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

fn main() {
    let icon_path = Path::new("icons/icon.png");
    if !icon_path.exists() {
        if let Some(parent) = icon_path.parent() {
            fs::create_dir_all(parent).expect("create Tauri icon directory");
        }
        fs::write(icon_path, FALLBACK_ICON_PNG).expect("write fallback Tauri icon");
    }

    tauri_build::build()
}
