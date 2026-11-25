//! Native directory selection only. No import or EULA write.
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

static PICKER_ACTIVE: AtomicBool = AtomicBool::new(false);

struct PickerGuard;

impl Drop for PickerGuard {
    fn drop(&mut self) {
        PICKER_ACTIVE.store(false, Ordering::Release);
    }
}

#[tauri::command]
pub async fn pick_import_folder(window: tauri::WebviewWindow) -> Result<Option<String>, String> {
    choose_folder(window, "Select Minecraft world folder")
        .await?
        .map(|value| {
            value
                .into_os_string()
                .into_string()
                .map_err(|_| "The selected folder path cannot be represented as text".to_string())
        })
        .transpose()
}

pub async fn choose_folder(window: tauri::WebviewWindow, title: &'static str) -> Result<Option<PathBuf>, String> {
    if PICKER_ACTIVE.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return Err("A folder picker is already open".into());
    }
    let guard = PickerGuard;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let parent = window.clone();
    window
        .run_on_main_thread(move || {
            let _guard = guard;
            let _ = sender.send(pick_folder(&parent, title));
        })
        .map_err(|_| "Native folder picker unavailable".to_string())?;
    receiver.await.map_err(|_| "Native folder picker did not complete".to_string())?
}

#[cfg(target_os = "linux")]
fn pick_folder(window: &tauri::WebviewWindow, title: &'static str) -> Result<Option<PathBuf>, String> {
    use gtk::prelude::*;
    let parent = window.gtk_window().map_err(|_| "Native folder picker unavailable".to_string())?;
    let chooser = gtk::FileChooserNative::new(
        Some(title),
        Some(&parent),
        gtk::FileChooserAction::SelectFolder,
        Some("Select folder"),
        Some("Cancel"),
    );
    chooser.set_modal(true);
    chooser.set_local_only(true);
    let result = if chooser.run() == gtk::ResponseType::Accept { chooser.filename() } else { None };
    chooser.destroy();
    Ok(result)
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn pick_folder(window: &tauri::WebviewWindow, title: &'static str) -> Result<Option<PathBuf>, String> {
    Ok(rfd::FileDialog::new().set_parent(window).set_title(title).pick_folder())
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn pick_folder(_window: &tauri::WebviewWindow, _title: &'static str) -> Result<Option<PathBuf>, String> {
    Err("Native folder picker unavailable; enter the path manually".into())
}
