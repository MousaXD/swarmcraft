#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;

async fn run_cli(app: &AppHandle, arguments: Vec<String>) -> Result<String, String> {
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
async fn initialize_node(app: AppHandle) -> Result<String, String> {
    run_cli(&app, vec!["init".into()]).await
}

#[tauri::command]
async fn list_worlds(app: AppHandle) -> Result<String, String> {
    run_cli(&app, vec!["world".into(), "list".into()]).await
}

#[tauri::command(rename_all = "camelCase")]
fn host_world(
    app: AppHandle,
    world: String,
    java: String,
    server_jar: String,
    mod_jar: String,
    accept_eula: bool,
) -> Result<u32, String> {
    if world.trim().is_empty() || server_jar.trim().is_empty() || mod_jar.trim().is_empty() {
        return Err("world ID, Fabric server jar, and SwarmCraft mod jar are required".into());
    }
    if !accept_eula {
        return Err("Minecraft server EULA acceptance is required before hosting".into());
    }
    let (_events, child) = app
        .shell()
        .sidecar("swarmcraft-host")
        .map_err(|error| error.to_string())?
        .args([
            "--world",
            world.as_str(),
            "--java",
            java.as_str(),
            "--server-jar",
            server_jar.as_str(),
            "--mod-jar",
            mod_jar.as_str(),
            "--accept-eula",
        ])
        .spawn()
        .map_err(|error| error.to_string())?;
    Ok(child.pid())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![initialize_node, list_worlds, host_world])
        .run(tauri::generate_context!())
        .expect("failed to run SwarmCraft desktop application");
}
