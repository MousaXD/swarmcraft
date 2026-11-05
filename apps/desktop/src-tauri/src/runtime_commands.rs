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
pub fn ensure_daemon_running(
    app: AppHandle,
    processes: State<'_, RuntimeProcesses>,
    listen: String,
) -> Result<u32, String> {
    processes.ensure_daemon_running(&app, required(listen, "Listen multiaddress")?)
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
pub fn runtime_diagnostics(
    app: AppHandle,
    processes: State<'_, RuntimeProcesses>,
    world: String,
) -> Result<String, String> {
    let world = required(world, "World ID")?;
    processes.runtime_diagnostics_reference(&app, &world)
}

async fn run_cli(app: &AppHandle, arguments: Vec<String>) -> Result<String, String> {
    use tauri_plugin_shell::ShellExt;
    let output = app
        .shell()
        .sidecar("swarmcraft")
        .map_err(|error| error.to_string())?
        .args(arguments)
        .output()
        .await
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if error.is_empty() { "SwarmCraft CLI command failed".into() } else { error });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[tauri::command]
pub async fn stop_host(app: AppHandle, world: String) -> Result<String, String> {
    let world = required(world, "World ID")?;
    run_cli(&app, vec!["world".into(), "stop".into(), world.clone()]).await?;
    for _ in 0..160 {
        let raw =
            run_cli(&app, vec!["world".into(), "migration-status".into(), world.clone(), "--json".into()]).await?;
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) {
            let phase = value.get("phase").and_then(|v| v.as_str()).unwrap_or_default();
            if phase == "sleeping" {
                return Ok("World stopped safely after save barrier and canonical checkpoint.".into());
            }
            if phase == "failed" {
                let detail = value.get("failure_reason").and_then(|v| v.as_str()).unwrap_or("safe stop failed");
                return Err(format!("World was not reported safely stopped: {detail}"));
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    Err("World was not reported safely stopped; Minecraft was not force-killed by Desktop".into())
}
