#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod runtime;
mod runtime_commands;

use runtime::RuntimeProcesses;
use runtime_commands::{ensure_daemon_running, start_daemon, stop_daemon, stop_host};
use tauri::{AppHandle, State};
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
        return Err(if error.is_empty() {
            "SwarmCraft CLI command failed".into()
        } else {
            error
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn require_value(value: String, label: &str) -> Result<String, String> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        Err(format!("{label} is required"))
    } else {
        Ok(value)
    }
}

#[tauri::command]
async fn initialize_node(app: AppHandle) -> Result<String, String> {
    run_cli(&app, vec!["init".into()]).await
}

#[tauri::command]
async fn node_identity(app: AppHandle) -> Result<String, String> {
    run_cli(&app, vec!["identity".into()]).await
}

#[tauri::command]
async fn list_worlds(app: AppHandle) -> Result<String, String> {
    run_cli(&app, vec!["world".into(), "list".into()]).await
}

#[tauri::command(rename_all = "camelCase")]
async fn create_world(
    app: AppHandle,
    name: String,
    minecraft: String,
    fabric_loader: String,
    compatibility: String,
    visibility: String,
) -> Result<String, String> {
    let name = require_value(name, "World name")?;
    let minecraft = require_value(minecraft, "Minecraft version")?;
    let fabric_loader = require_value(fabric_loader, "Fabric loader version")?;
    let compatibility = require_value(compatibility, "Compatibility profile")?;
    let visibility = require_value(visibility, "Visibility")?;
    run_cli(
        &app,
        vec![
            "world".into(),
            "create".into(),
            "--name".into(),
            name,
            "--minecraft".into(),
            minecraft,
            "--fabric-loader".into(),
            fabric_loader,
            "--compatibility".into(),
            compatibility,
            "--visibility".into(),
            visibility,
        ],
    )
    .await
}

#[tauri::command]
async fn join_world(app: AppHandle, invite: String) -> Result<String, String> {
    let invite = require_value(invite, "Invite")?;
    run_cli(&app, vec!["world".into(), "join".into(), invite]).await
}

#[tauri::command]
async fn leave_world(app: AppHandle, world: String) -> Result<String, String> {
    let world = require_value(world, "World ID")?;
    run_cli(&app, vec!["world".into(), "leave".into(), world]).await
}

#[tauri::command(rename_all = "camelCase")]
async fn create_invite(
    app: AppHandle,
    world: String,
    expires_minutes: u64,
    bootstrap_addrs: Vec<String>,
) -> Result<String, String> {
    let world = require_value(world, "World ID")?;
    let mut arguments = vec![
        "invite".into(),
        "create".into(),
        world,
        "--expires-minutes".into(),
        expires_minutes.max(1).to_string(),
    ];
    for address in bootstrap_addrs
        .into_iter()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    {
        arguments.push("--bootstrap".into());
        arguments.push(address);
    }
    run_cli(&app, arguments).await
}

#[tauri::command]
async fn world_status(app: AppHandle, world: String) -> Result<String, String> {
    let world = require_value(world, "World ID")?;
    run_cli(&app, vec!["world".into(), "status".into(), world]).await
}

#[tauri::command]
async fn world_compatibility(app: AppHandle, world: String) -> Result<String, String> {
    let world = require_value(world, "World ID")?;
    run_cli(&app, vec!["world".into(), "compatibility".into(), world]).await
}

#[tauri::command]
async fn world_conflicts(app: AppHandle, world: String) -> Result<String, String> {
    let world = require_value(world, "World ID")?;
    run_cli(&app, vec!["world".into(), "conflicts".into(), world]).await
}

#[tauri::command]
async fn set_background_seeding(
    app: AppHandle,
    world: String,
    enabled: bool,
) -> Result<String, String> {
    let world = require_value(world, "World ID")?;
    run_cli(
        &app,
        vec!["world".into(), "seed".into(), world, enabled.to_string()],
    )
    .await
}

#[tauri::command]
async fn world_peers(app: AppHandle, world: String) -> Result<String, String> {
    let world = require_value(world, "World ID")?;
    run_cli(&app, vec!["peers".into(), world]).await
}

#[tauri::command]
async fn verify_world(app: AppHandle, world: String) -> Result<String, String> {
    let world = require_value(world, "World ID")?;
    run_cli(&app, vec!["world".into(), "verify".into(), world]).await
}

#[tauri::command(rename_all = "camelCase")]
async fn export_world(
    app: AppHandle,
    world: String,
    destination: String,
) -> Result<String, String> {
    let world = require_value(world, "World ID")?;
    let destination = require_value(destination, "Export destination")?;
    run_cli(
        &app,
        vec!["world".into(), "export".into(), world, destination],
    )
    .await
}

#[tauri::command(rename_all = "camelCase")]
async fn recover_world(
    app: AppHandle,
    world: String,
    snapshot: u64,
    destination: String,
) -> Result<String, String> {
    let world = require_value(world, "World ID")?;
    let destination = require_value(destination, "Recovery destination")?;
    run_cli(
        &app,
        vec![
            "world".into(),
            "recover".into(),
            world,
            snapshot.to_string(),
            destination,
        ],
    )
    .await
}

#[tauri::command]
async fn migration_capabilities(app: AppHandle) -> String {
    let mut supported = Vec::new();
    if run_cli(
        &app,
        vec![
            "world".into(),
            "migration-status".into(),
            "--help".into(),
        ],
    )
    .await
    .is_ok()
    {
        supported.push("status");
    }
    if run_cli(
        &app,
        vec!["world".into(), "wake".into(), "--help".into()],
    )
    .await
    .is_ok()
    {
        supported.push("wake");
    }
    supported.join(",")
}

#[tauri::command]
async fn migration_status(app: AppHandle, world: String) -> Result<String, String> {
    let world = require_value(world, "World ID")?;
    run_cli(
        &app,
        vec![
            "world".into(),
            "migration-status".into(),
            world,
            "--json".into(),
        ],
    )
    .await
}

#[tauri::command]
async fn wake_world(app: AppHandle, world: String) -> Result<String, String> {
    let world = require_value(world, "World ID")?;
    run_cli(&app, vec!["world".into(), "wake".into(), world]).await
}

#[tauri::command(rename_all = "camelCase")]
async fn configure_world_runtime(
    app: AppHandle,
    world: String,
    java: String,
    server_jar: String,
    mod_jar: String,
    accept_eula: bool,
    game_endpoint: Option<String>,
) -> Result<String, String> {
    let world = require_value(world, "World ID")?;
    let java = require_value(java, "Java executable")?;
    let server_jar = require_value(server_jar, "Fabric server jar")?;
    let mod_jar = require_value(mod_jar, "SwarmCraft mod jar")?;
    if !accept_eula {
        return Err("Minecraft server EULA acceptance is required before runtime setup".into());
    }

    let mut arguments = vec![
        "world".into(),
        "runtime-configure".into(),
        world,
        "--java".into(),
        java,
        "--server-jar".into(),
        server_jar,
        "--mod-jar".into(),
        mod_jar,
        "--accept-eula".into(),
    ];
    if let Some(endpoint) = game_endpoint
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    {
        arguments.push("--game-endpoint".into());
        arguments.push(endpoint);
    }
    run_cli(&app, arguments).await
}

#[tauri::command]
async fn connectivity_diagnostics(app: AppHandle) -> Result<String, String> {
    run_cli(
        &app,
        vec![
            "diagnostics".into(),
            "connectivity".into(),
            "--json".into(),
        ],
    )
    .await
}

#[tauri::command(rename_all = "camelCase")]
fn host_world(
    app: AppHandle,
    processes: State<'_, RuntimeProcesses>,
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
    processes.start_host(
        &app,
        vec![
            "--world".into(),
            world,
            "--java".into(),
            java,
            "--server-jar".into(),
            server_jar,
            "--mod-jar".into(),
            mod_jar,
            "--accept-eula".into(),
        ],
    )
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(RuntimeProcesses::default())
        .invoke_handler(tauri::generate_handler![
            initialize_node,
            node_identity,
            list_worlds,
            create_world,
            join_world,
            leave_world,
            create_invite,
            world_status,
            world_compatibility,
            world_conflicts,
            set_background_seeding,
            world_peers,
            verify_world,
            export_world,
            recover_world,
            migration_capabilities,
            migration_status,
            wake_world,
            configure_world_runtime,
            connectivity_diagnostics,
            ensure_daemon_running,
            start_daemon,
            stop_daemon,
            stop_host,
            host_world
        ])
        .run(tauri::generate_context!())
        .expect("failed to run SwarmCraft desktop application");
}
