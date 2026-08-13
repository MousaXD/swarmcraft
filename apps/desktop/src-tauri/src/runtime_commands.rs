use crate::runtime::RuntimeProcesses;
use tauri::{AppHandle, State};

fn required(value: String, label: &str) -> Result<String, String> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err(format!("{label} is required"));
    }
    Ok(value)
}

#[tauri::command]
pub fn start_daemon(app: AppHandle, processes: State<'_, RuntimeProcesses>, listen: String) -> Result<u32, String> {
    processes.start_daemon(&app, required(listen, "Listen multiaddress")?)
}

#[tauri::command]
pub fn stop_daemon(processes: State<'_, RuntimeProcesses>) -> Result<(), String> {
    processes.stop_daemon()
}

#[tauri::command]
pub fn stop_host(processes: State<'_, RuntimeProcesses>) -> Result<(), String> {
    processes.stop_host()
}
