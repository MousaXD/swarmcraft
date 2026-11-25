use image::{ExtendedColorType, ImageFormat};
use std::{fs, path::Path, process::Command};

fn main() {
    capture_build_provenance();
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
    let pixels = [45u8, 45, 45, 255].repeat((size * size) as usize);
    image::save_buffer_with_format(path, &pixels, size, size, ExtendedColorType::Rgba8, format)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
}

// The commit is build-time provenance, not the current runtime directory.
// A dirty worktree is explicitly identified rather than claiming exact SHA.
fn git_output(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok().map(|value| value.trim().to_owned())
}

fn capture_build_provenance() {
    for path in ["src", "build.rs", "Cargo.toml", "Cargo.lock", "../src", "../frontend/src", "../../../crates"] {
        println!("cargo:rerun-if-changed={path}");
    }
    for path in ["HEAD", "index", "packed-refs"] {
        if let Some(path) = git_output(&["rev-parse", "--git-path", path]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(reference) = git_output(&["symbolic-ref", "-q", "HEAD"]) {
        if let Some(path) = git_output(&["rev-parse", "--git-path", &reference]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    let in_repo = git_output(&["rev-parse", "--show-prefix"]).as_deref() == Some("apps/desktop/src-tauri/");
    let commit = in_repo
        .then(|| git_output(&["rev-parse", "--verify", "HEAD"]))
        .flatten()
        .filter(|value| value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let tree = match in_repo.then(|| git_output(&["status", "--porcelain", "--untracked-files=normal"])).flatten() {
        Some(status) if status.is_empty() => "clean",
        Some(_) => "modified",
        None => "unknown",
    };
    println!("cargo:rustc-env=SWARMCRAFT_SUPPORT_COMMIT={}", commit.unwrap_or_default());
    println!("cargo:rustc-env=SWARMCRAFT_SUPPORT_TREE={tree}");
}
