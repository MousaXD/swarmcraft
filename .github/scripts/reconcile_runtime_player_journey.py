from __future__ import annotations

from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[2]


def sh(*args: str) -> str:
    return subprocess.check_output(args, cwd=ROOT, text=True)


def read(path: str) -> str:
    return (ROOT / path).read_text()


def write(path: str, content: str) -> None:
    (ROOT / path).write_text(content)


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if old not in text:
        raise RuntimeError(f"missing reconciliation anchor for {label}")
    return text.replace(old, new, 1)


def host_file(path: str) -> str:
    return sh("git", "show", f"origin/agent/host-readiness:{path}")


def reconcile_cli_main() -> None:
    path = "crates/swarm-cli/src/main.rs"
    text = read(path)
    host = host_file(path)

    if "use swarm_cli::host_readiness;" not in text:
        text = replace_once(
            text,
            "use swarm_cli::server_mods;\n",
            "use swarm_cli::host_readiness;\nuse swarm_cli::server_mods;\n",
            "host readiness import",
        )

    if "HostReadiness {" not in text:
        start = host.index("    /// Show the authoritative backend answer to whether this device may safely shut down.")
        end = host.index("    /// Request a safe wake of a sleeping world.", start)
        text = replace_once(
            text,
            "    /// Request a safe wake of a sleeping world.",
            host[start:end] + "    /// Request a safe wake of a sleeping world.",
            "host readiness command",
        )

    if "WorldCommand::HostReadiness { world, json } =>" not in text:
        start = host.index("        WorldCommand::HostReadiness { world, json } => {")
        end = host.index("        WorldCommand::Wake { world } => {", start)
        text = replace_once(
            text,
            "        WorldCommand::Wake { world } => {",
            host[start:end] + "        WorldCommand::Wake { world } => {",
            "host readiness dispatch",
        )

    if "Stop { world: String }," not in text:
        text = replace_once(
            text,
            "    /// Request a safe wake of a sleeping world. Multi-member worlds remain blocked until a quorum transition exists.\n",
            "    /// Request a safe stop. Success is reported only after the Fabric save barrier, final checkpoint and durable sleep record complete.\n"
            "    Stop { world: String },\n"
            "    /// Request a safe wake of a sleeping world. Multi-member worlds remain blocked until a quorum transition exists.\n",
            "safe stop command",
        )

    if "WorldCommand::Stop { world } =>" not in text:
        block = """        WorldCommand::Stop { world } => {
            let world = parse_world(&world)?;
            migration::request_world_stop(&paths, &storage, world)?;
            println!("Safe stop requested for {world}; wait for migration status to become sleeping before treating shutdown as complete.");
        }
"""
        text = replace_once(text, "        WorldCommand::Wake { world } => {", block + "        WorldCommand::Wake { world } => {", "safe stop dispatch")

    # A newly created world needs a canonical empty base snapshot so the shared
    # migration/runtime path can let Minecraft generate the first playable world.
    if "initial empty canonical snapshot" not in text:
        create_anchor = "            storage.save_world_config(&config)?;"
        if create_anchor in text:
            block = """
            // Seed an initial empty canonical snapshot. The shared authority runtime restores this
            // empty directory and Minecraft creates the actual world on first launch; from then on
            // all state is checkpointed through the normal canonical snapshot path.
            let initial_source = paths.root.join("initial-world").join(world_id.to_hex());
            std::fs::create_dir_all(&initial_source)?;
            let mut initial_snapshot = storage.snapshot_directory(
                &initial_source,
                swarm_storage::SnapshotContext {
                    world: world_id,
                    snapshot_number: 1,
                    epoch: 0,
                    sequence: 1,
                    previous_snapshot_hash: None,
                    authority_peer_id: identity.peer_id(),
                    authority_public_key: identity.public_key(),
                },
            )?;
            identity.sign_snapshot(&mut initial_snapshot)?;
            storage.commit_snapshot(&initial_snapshot)?;
            let _ = std::fs::remove_dir_all(&initial_source);
            // initial empty canonical snapshot
"""
            text = text.replace(create_anchor, create_anchor + "\n" + block, 1)

    write(path, text)


def reconcile_migration() -> None:
    path = "crates/swarm-cli/src/migration.rs"
    text = read(path)
    host = host_file(path)

    if "host_readiness" not in text.split("\n", 3)[0:3].__str__():
        text = text.replace(
            "use crate::{authority_permit::PermitWatch, server_mods};",
            "use crate::{authority_permit::PermitWatch, host_readiness, server_mods};",
            1,
        )

    if "host_readiness::invalidate_runtime_verification(paths, world)?;" not in text:
        text = replace_once(
            text,
            "    let path = runtime_config_path(paths, world);",
            "    // Any launch-path change invalidates the prior machine-local runtime proof.\n"
            "    host_readiness::invalidate_runtime_verification(paths, world)?;\n"
            "    let path = runtime_config_path(paths, world);",
            "runtime proof invalidation",
        )

    if "host_readiness::record_runtime_verified(" not in text:
        comment = "    // A runtime becomes host-ready only after the actual Fabric process has"
        start = host.index(comment)
        end = host.index("    publish_status(", start)
        block = host[start:end]
        anchor = "        return Err(error);\n    }\n    publish_status("
        text = replace_once(
            text,
            anchor,
            "        return Err(error);\n    }\n" + block + "    publish_status(",
            "runtime proof after Fabric verification",
        )

    if "pub fn request_world_stop(" not in text:
        anchor = "pub fn request_world_wake("
        idx = text.index(anchor)
        block = """pub fn request_world_stop(paths: &DataPaths, storage: &Storage, world: WorldId) -> Result<()> {
    storage.load_world(world)?;
    if storage.load_sleep_record(world).is_ok() {
        return Ok(());
    }
    let identity = PeerIdentity::load_or_create(paths)?;
    let epoch = storage.load_epoch_record(world)?;
    if epoch.authority_peer_id != identity.peer_id() || epoch.authority_public_key != identity.public_key() {
        bail!("only the current authority may request a safe world stop");
    }
    let path = stop_intent_path(paths, world);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, b"stop\n").with_context(|| format!("cannot write safe-stop intent {}", path.display()))?;
    Ok(())
}

"""
        text = text[:idx] + block + text[idx:]

    if "fn stop_intent_path(" not in text:
        marker = "fn wake_intent_path("
        idx = text.index(marker)
        block = """fn stop_intent_path(paths: &DataPaths, world: WorldId) -> PathBuf {
    paths.root.join("runtime-control").join(world.to_hex()).join("stop.intent")
}

fn clear_stop_intent(paths: &DataPaths, world: WorldId) -> Result<()> {
    let path = stop_intent_path(paths, world);
    if path.exists() {
        fs::remove_file(&path).with_context(|| format!("cannot clear safe-stop intent {}", path.display()))?;
    }
    Ok(())
}

"""
        text = text[:idx] + block + text[idx:]

    if "safe stop save barrier failed; keeping Minecraft running" not in text:
        start = text.index("async fn wait_for_runtime_exit(")
        tail = text[start:]
        anchor = "        ensure_authority_generation(storage, identity, epoch)?;\n"
        pos = tail.index(anchor) + len(anchor)
        block = """        if stop_intent_path(paths, epoch.world_id).exists() {
            let stop_result = async {
                session.prepare_shutdown(1, FABRIC_SHUTDOWN_TIMEOUT).await?;
                timeout(FABRIC_SHUTDOWN_TIMEOUT, wait_for_child(child))
                    .await
                    .map_err(|_| anyhow!("Minecraft did not stop after the shutdown save barrier"))??;
                Ok::<(), anyhow::Error>(())
            }
            .await;
            clear_stop_intent(paths, epoch.world_id)?;
            match stop_result {
                Ok(()) => return Ok(RuntimeDisposition::Sleep),
                Err(error) => {
                    warn!(world = %epoch.world_id, %error, "safe stop save barrier failed; keeping Minecraft running");
                    continue;
                }
            }
        }
"""
        tail = tail[:pos] + block + tail[pos:]
        text = text[:start] + tail

    write(path, text)


def reconcile_runtime_installer() -> None:
    path = "crates/swarm-cli/src/runtime_installer.rs"
    text = read(path)

    if "server_mods," not in text[:800]:
        text = replace_once(text, "    },\n};\nuse anyhow", "    },\n    server_mods,\n};\nuse anyhow", "server mod installer import")
    if "use fs2::FileExt;" not in text:
        text = text.replace("use anyhow::{bail, Context, Result};\n", "use anyhow::{bail, Context, Result};\nuse fs2::FileExt;\nuse sha1::Sha1;\nuse sha2::{Digest, Sha256};\n", 1)
    text = text.replace("    io::Write,", "    io::{Read, Write},", 1)

    # Exact server-mod verification, including ID/version/side/hash, is the installer boundary.
    pattern = re.compile(r"    fn server_mods_status\(&self, world: WorldId\) -> RuntimeComponentStatus \{.*?\n    \}\n\}\n\nfn platform_components_ready", re.S)
    if pattern.search(text):
        replacement = """    fn server_mods_status(&self, world: WorldId) -> RuntimeComponentStatus {
        let config = match self.storage.load_world_config(world) {
            Ok(config) => config,
            Err(_) => {
                return RuntimeComponentStatus {
                    kind: RuntimeComponentKind::ServerMods,
                    state: RuntimeComponentState::Unavailable,
                    version: None,
                    path: Some(server_mods::mods_dir(self.paths, world)),
                    managed: false,
                    detail: Some("canonical runtime compatibility manifest is not synchronized yet".into()),
                };
            }
        };
        match server_mods::evaluate_world_mods(self.paths, world, &config.compatibility) {
            Ok(readiness) => RuntimeComponentStatus {
                kind: RuntimeComponentKind::ServerMods,
                state: if readiness.ready { RuntimeComponentState::Ready } else { RuntimeComponentState::Incompatible },
                version: None,
                path: Some(readiness.mods_dir),
                managed: false,
                detail: (!readiness.ready).then(|| {
                    readiness.issues.iter().map(|issue| issue.message.as_str()).collect::<Vec<_>>().join("; ")
                }),
            },
            Err(error) => RuntimeComponentStatus {
                kind: RuntimeComponentKind::ServerMods,
                state: RuntimeComponentState::Incompatible,
                version: None,
                path: Some(server_mods::mods_dir(self.paths, world)),
                managed: false,
                detail: Some(error.to_string()),
            },
        }
    }
}

fn platform_components_ready"""
        text = pattern.sub(replacement, text, count=1)

    # Never delete a known-good destination before publishing the verified replacement.
    text = text.replace(
        "    if destination.exists() {\n        fs::remove_file(destination).with_context(|| format!(\"cannot replace {}\", destination.display()))?;\n    }\n    fs::rename(&temporary, destination)\n        .with_context(|| format!(\"cannot publish downloaded artifact at {}\", destination.display()))?;",
        "    publish_replace(&temporary, destination)\n        .with_context(|| format!(\"cannot publish downloaded artifact at {}\", destination.display()))?;",
        1,
    )
    text = text.replace(
        "    if destination.exists() {\n        fs::remove_file(destination)?;\n    }\n    fs::rename(&temporary, destination)?;\n    Ok(())",
        "    publish_replace(&temporary, destination)?;\n    Ok(())",
        1,
    )
    text = text.replace(
        "    if path.exists() {\n        fs::remove_file(path)?;\n    }\n    fs::rename(&temporary, path)?;\n    Ok(())",
        "    publish_replace(&temporary, path)?;\n    Ok(())",
        1,
    )

    if "fn publish_replace(" not in text:
        anchor = "fn sync_file(path: &Path) -> Result<()> {"
        idx = text.index(anchor)
        block = """fn publish_replace(temporary: &Path, destination: &Path) -> Result<()> {
    if !destination.exists() {
        fs::rename(temporary, destination)?;
        return Ok(());
    }
    let backup = destination.with_extension(format!("backup-{}", unique_suffix()));
    fs::rename(destination, &backup).with_context(|| format!("cannot preserve {} before replacement", destination.display()))?;
    match fs::rename(temporary, destination) {
        Ok(()) => {
            let _ = fs::remove_file(&backup);
            Ok(())
        }
        Err(error) => {
            let restore = fs::rename(&backup, destination);
            let _ = fs::remove_file(temporary);
            if let Err(restore_error) = restore {
                bail!(
                    "replacement of {} failed ({error}) and rollback also failed ({restore_error}); preserved backup is {}",
                    destination.display(),
                    backup.display()
                );
            }
            Err(error.into())
        }
    }
}

"""
        text = text[:idx] + block + text[idx:]

    # Crash-safe ownership: the OS lock dies with the process; a stale lock file no longer wedges retries.
    pattern = re.compile(r"struct InstallGuard \{.*?impl Drop for InstallGuard \{.*?\n\}\n", re.S)
    if pattern.search(text):
        replacement = """struct InstallGuard {
    path: PathBuf,
    file: fs::File,
}

impl InstallGuard {
    fn acquire(path: &Path) -> Result<Self> {
        let parent = path.parent().context("runtime install lock has no parent")?;
        fs::create_dir_all(parent)?;
        let mut file = OpenOptions::new().create(true).read(true).write(true).open(path)?;
        file.try_lock_exclusive()
            .map_err(|_| anyhow::anyhow!("another runtime installation is already in progress for this world"))?;
        file.set_len(0)?;
        writeln!(file, "pid={}", std::process::id())?;
        file.sync_all()?;
        Ok(Self { path: path.to_path_buf(), file })
    }
}

impl Drop for InstallGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
        let _ = fs::remove_file(&self.path);
    }
}
"""
        text = pattern.sub(replacement, text, count=1)

    # Rust hashing on all platforms. Core integrity checks never shell out to PowerShell/coreutils.
    pattern = re.compile(r"fn hash_file\(path: &Path, kind: HashKind\) -> Result<String> \{.*?\n\}\n\n(?:#\[cfg\(windows\)\]\nfn parse_hash_output.*?\n\}\n\n)?fn eq_hash", re.S)
    if pattern.search(text):
        replacement = """fn hash_file(path: &Path, kind: HashKind) -> Result<String> {
    let mut file = fs::File::open(path).with_context(|| format!("cannot open {} for hashing", path.display()))?;
    let mut buffer = [0_u8; 64 * 1024];
    match kind {
        HashKind::Sha1 => {
            let mut hasher = Sha1::new();
            loop {
                let read = file.read(&mut buffer)?;
                if read == 0 { break; }
                hasher.update(&buffer[..read]);
            }
            Ok(format!("{:x}", hasher.finalize()))
        }
        HashKind::Sha256 => {
            let mut hasher = Sha256::new();
            loop {
                let read = file.read(&mut buffer)?;
                if read == 0 { break; }
                hasher.update(&buffer[..read]);
            }
            Ok(format!("{:x}", hasher.finalize()))
        }
    }
}

fn eq_hash"""
        text = pattern.sub(replacement, text, count=1)

    # Preserve an existing managed Java tree if publishing the replacement fails.
    old = """        if target_root.exists() {
            fs::remove_dir_all(&target_root).with_context(|| format!("cannot replace {}", target_root.display()))?;
        }
        let parent = target_root.parent().context("managed Java directory has no parent")?;
"""
    new = """        let parent = target_root.parent().context("managed Java directory has no parent")?;
"""
    if old in text:
        text = text.replace(old, new, 1)
    old = """        fs::rename(&staging, &target_root)
            .with_context(|| format!("cannot atomically publish managed Java at {}", target_root.display()))?;
        Ok(target_root.join(relative))
"""
    if old in text:
        new = """        let backup = parent.join(format!("java-backup-{}", unique_suffix()));
        let had_previous = target_root.exists();
        if had_previous {
            fs::rename(&target_root, &backup)
                .with_context(|| format!("cannot preserve existing managed Java at {}", target_root.display()))?;
        }
        if let Err(error) = fs::rename(&staging, &target_root) {
            if had_previous {
                let _ = fs::rename(&backup, &target_root);
            }
            let _ = fs::remove_dir_all(&staging);
            return Err(error).with_context(|| format!("cannot atomically publish managed Java at {}", target_root.display()));
        }
        if had_previous {
            let _ = fs::remove_dir_all(&backup);
        }
        Ok(target_root.join(relative))
"""
        text = text.replace(old, new, 1)

    write(path, text)


def reconcile_runtime_main() -> None:
    path = "crates/swarm-cli/src/runtime_main.rs"
    text = read(path)
    if "host_readiness" not in text:
        text = text.replace(
            "use swarm_cli::runtime_installer::{RuntimeInstallOptions, RuntimeInstaller, RuntimeProgress};",
            "use swarm_cli::{host_readiness, migration, runtime_installer::{RuntimeInstallOptions, RuntimeInstaller, RuntimeProgress}, server_mods};",
            1,
        )
        text = text.replace("use swarm_protocol::WorldId;", "use swarm_network::ServerModsReadinessV1;\nuse swarm_protocol::WorldId;", 1)

    old = """        RuntimeCommand::Verify { world } => {
            print_json(&installer.verify(parse_world(&world)?)?)?;
        }
"""
    if old in text:
        new = """        RuntimeCommand::Verify { world } => {
            let world = parse_world(&world)?;
            let status = installer.verify(world)?;
            if status.ready {
                let config = migration::load_runtime_config(&paths, world)?;
                let descriptor = storage.load_world_descriptor(world)?;
                host_readiness::record_runtime_verified(&paths, world, &config, descriptor.compatibility_fingerprint)?;
                let world_config = storage.load_world_config(world)?;
                let mods = server_mods::evaluate_world_mods(&paths, world, &world_config.compatibility)?;
                let state = if mods.ready {
                    ServerModsReadinessV1::Ready
                } else if mods.issues.iter().any(|issue| matches!(issue.kind, server_mods::ModIssueKind::MissingRequired)) {
                    ServerModsReadinessV1::Missing
                } else {
                    ServerModsReadinessV1::Incompatible
                };
                host_readiness::record_server_mod_readiness(
                    &paths,
                    world,
                    descriptor.compatibility_fingerprint,
                    state,
                )?;
            }
            print_json(&status)?;
        }
"""
        text = text.replace(old, new, 1)
    write(path, text)


def reconcile_desktop_bridge() -> None:
    cargo_path = "apps/desktop/src-tauri/Cargo.toml"
    cargo = read(cargo_path)
    if "serde_json" not in cargo:
        cargo += "serde_json = \"1.0\"\ntokio = { version = \"1.47\", features = [\"time\"] }\n"
    write(cargo_path, cargo)

    main_path = "apps/desktop/src-tauri/src/main.rs"
    text = read(main_path)
    if "async fn run_runtime_cli" not in text:
        marker = "fn require_value(value: String, label: &str) -> Result<String, String> {"
        idx = text.index(marker)
        block = """async fn run_runtime_cli(app: &AppHandle, arguments: Vec<String>) -> Result<String, String> {
    let output = app
        .shell()
        .sidecar("swarmcraft-runtime")
        .map_err(|error| error.to_string())?
        .args(arguments)
        .output()
        .await
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if error.is_empty() { "SwarmCraft runtime command failed".into() } else { error });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

"""
        text = text[:idx] + block + text[idx:]

    if "async fn host_readiness(" not in text:
        anchor = "#[tauri::command]\nasync fn world_compatibility"
        block = """#[tauri::command]
async fn host_readiness(app: AppHandle, world: String) -> Result<String, String> {
    let world = require_value(world, "World ID")?;
    run_cli(&app, vec!["world".into(), "host-readiness".into(), world, "--json".into()]).await
}

"""
        text = replace_once(text, anchor, block + anchor, "desktop host readiness command")

    if "async fn runtime_status(" not in text:
        anchor = "#[tauri::command]\nasync fn connectivity_diagnostics"
        block = """#[tauri::command]
async fn runtime_status(app: AppHandle, world: String) -> Result<String, String> {
    let world = require_value(world, "World ID")?;
    run_runtime_cli(&app, vec!["status".into(), world]).await
}

#[tauri::command]
async fn runtime_plan(app: AppHandle, world: String) -> Result<String, String> {
    let world = require_value(world, "World ID")?;
    run_runtime_cli(&app, vec!["plan".into(), world]).await
}

#[tauri::command(rename_all = "camelCase")]
async fn runtime_install(app: AppHandle, world: String, accept_eula: bool) -> Result<String, String> {
    let world = require_value(world, "World ID")?;
    let mut arguments = vec!["install".into(), world];
    if accept_eula { arguments.push("--accept-eula".into()); }
    run_runtime_cli(&app, arguments).await
}

#[tauri::command]
async fn runtime_repair(app: AppHandle, world: String) -> Result<String, String> {
    let world = require_value(world, "World ID")?;
    run_runtime_cli(&app, vec!["repair".into(), world]).await
}

#[tauri::command]
async fn runtime_verify(app: AppHandle, world: String) -> Result<String, String> {
    let world = require_value(world, "World ID")?;
    run_runtime_cli(&app, vec!["verify".into(), world]).await
}

#[tauri::command]
async fn runtime_launch(
    app: AppHandle,
    processes: State<'_, RuntimeProcesses>,
    world: String,
) -> Result<Option<u32>, String> {
    let world = require_value(world, "World ID")?;
    // Starting/ensuring the daemon is the launch trigger. The daemon's migration
    // supervisor consumes the persisted RuntimeLaunchConfig and owns the shared
    // authority -> restore -> Fabric launch orchestration path.
    processes.ensure_daemon_running(&app, "/ip4/0.0.0.0/udp/0/quic-v1".into())?;
    for _ in 0..160 {
        match run_cli(&app, vec!["world".into(), "migration-status".into(), world.clone(), "--json".into()]).await {
            Ok(raw) => {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) {
                    let phase = value.get("phase").and_then(|v| v.as_str()).unwrap_or_default();
                    let ready = value.get("runtime_ready").and_then(|v| v.as_bool()).unwrap_or(false);
                    if phase == "ready" && ready { return Ok(None); }
                    if matches!(phase, "failed" | "blocked") {
                        let detail = value.get("failure_reason").and_then(|v| v.as_str()).unwrap_or("shared runtime launch was blocked");
                        return Err(detail.to_owned());
                    }
                }
            }
            Err(error) => return Err(error),
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    Err("Minecraft did not reach the shared runtime ready state before the launch timeout".into())
}

"""
        text = replace_once(text, anchor, block + anchor, "desktop runtime bridge")

    handler_anchor = "            world_status,\n"
    if "            host_readiness,\n" not in text:
        text = text.replace(handler_anchor, handler_anchor + "            host_readiness,\n", 1)
    if "            runtime_status,\n" not in text:
        text = text.replace(
            "            configure_world_runtime,\n",
            "            configure_world_runtime,\n            runtime_status,\n            runtime_plan,\n            runtime_install,\n            runtime_repair,\n            runtime_verify,\n            runtime_launch,\n",
            1,
        )
    write(main_path, text)

    commands_path = "apps/desktop/src-tauri/src/runtime_commands.rs"
    commands = read(commands_path)
    old = """#[tauri::command]
pub fn stop_host(processes: State<'_, RuntimeProcesses>) -> Result<(), String> {
    processes.stop_host()
}
"""
    if old in commands:
        new = """async fn run_cli(app: &AppHandle, arguments: Vec<String>) -> Result<String, String> {
    use tauri_plugin_shell::ShellExt;
    let output = app.shell().sidecar("swarmcraft").map_err(|error| error.to_string())?
        .args(arguments).output().await.map_err(|error| error.to_string())?;
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
        let raw = run_cli(&app, vec!["world".into(), "migration-status".into(), world.clone(), "--json".into()]).await?;
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
"""
        commands = commands.replace(old, new, 1)
    write(commands_path, commands)


def reconcile_frontend() -> None:
    path = "apps/desktop/src/backend-adapter.js"
    text = read(path)

    if "const HOST_READINESS_STATES" not in text:
        anchor = "const RUNTIME_COMPONENT_ALIASES = Object.freeze({"
        block = """const HOST_READINESS_STATES = Object.freeze({
  safe: { label: 'Safe to shut down', kind: 'safe' },
  sleeping: { label: 'Safe to shut down', kind: 'safe' },
  world_will_stop: { label: 'World will go offline', kind: 'warning' },
  syncing: { label: 'Wait before shutting down', kind: 'syncing' },
  blocked_by_runtime: { label: 'Keep this PC on', kind: 'action' },
  blocked_by_mods: { label: 'Keep this PC on', kind: 'action' },
  blocked_by_quorum: { label: 'World will go offline', kind: 'action' },
  degraded_safety: { label: 'Keep this PC on', kind: 'warning' },
  conflict: { label: 'Host handoff unavailable', kind: 'danger' },
  not_current_host: { label: 'Shutdown safety not proven', kind: 'warning' },
  unknown: { label: 'Checking shutdown safety', kind: 'checking' },
});

"""
        text = replace_once(text, anchor, block + anchor, "host readiness frontend states")

    text = text.replace("swarmcraft_integration: ['swarmcraft_integration', 'swarmcraftIntegration', 'swarmcraft_mod', 'swarmcraftMod', 'mod'],", "swarmcraft_integration: ['swarmcraft_integration', 'swarmcraftIntegration', 'swarmcraft_mod', 'swarmcraftMod', 'swarmcraft_fabric', 'mod'],", 1)
    text = text.replace("world_directories: ['world_directories', 'worldDirectories', 'directories'],", "world_directories: ['world_directories', 'worldDirectories', 'server_directories', 'directories'],", 1)

    # Canonical Rust status uses a component array. Normalize that array without changing the backend contract.
    old = "const source = parseJsonContract(raw, 'Runtime status');\n  const componentSource = source.components ?? source.runtime_components ?? source.runtimeComponents ?? {};"
    if old in text:
        new = """const envelope = parseJsonContract(raw, 'Runtime status');
  const source = envelope.status && typeof envelope.status === 'object' ? envelope.status : envelope;
  const rawComponents = source.components ?? source.runtime_components ?? source.runtimeComponents ?? {};
  const componentSource = Array.isArray(rawComponents)
    ? Object.fromEntries(rawComponents.map((component) => [component.kind ?? component.id, component]))
    : rawComponents;"""
        text = text.replace(old, new, 1)

    old = """  const overall = connectivityKey(source.state ?? source.status ?? source.phase);
  const eulaAccepted = Boolean(source.eula_accepted ?? source.eulaAccepted ?? source.accept_eula ?? source.acceptEula);
  const eulaRequired = !eulaAccepted && Boolean(
    source.eula_required
      ?? source.eulaRequired
      ?? (overall === 'eula_required'),
  );
"""
    if old in text:
        new = """  const completedPhases = envelope.completed_phases ?? envelope.completedPhases ?? [];
  const inferredPhase = Array.isArray(completedPhases) && completedPhases.length ? completedPhases.at(-1) : null;
  const overall = connectivityKey(source.state ?? source.status ?? source.phase ?? inferredPhase);
  const eulaAccepted = Boolean(source.eula_accepted ?? source.eulaAccepted ?? source.accept_eula ?? source.acceptEula);
  const eulaComponent = Array.isArray(rawComponents)
    ? rawComponents.find((component) => connectivityKey(component.kind ?? component.id) === 'eula')
    : componentSource.eula;
  const eulaRequired = !eulaAccepted && Boolean(
    source.eula_required
      ?? source.eulaRequired
      ?? (overall === 'eula_required')
      ?? false
  ) || (!eulaAccepted && connectivityKey(eulaComponent?.state ?? eulaComponent?.status) === 'required');
"""
        text = text.replace(old, new, 1)

    if "export function normalizeHostReadiness" not in text:
        anchor = "export function connectivityFromStatus(status = {}) {"
        block = """export function normalizeHostReadiness(raw) {
  const source = raw && typeof raw === 'object' ? raw : {};
  const state = connectivityKey(source.state || 'unknown');
  const mapped = HOST_READINESS_STATES[state] || HOST_READINESS_STATES.unknown;
  return {
    available: state !== 'unknown',
    state: HOST_READINESS_STATES[state] ? state : 'unknown',
    kind: mapped.kind,
    label: mapped.label,
    detail: String(source.detail || '').trim() || 'SwarmCraft has not yet proven whether this computer may safely shut down.',
    safeToShutdown: Boolean(source.safe_to_shutdown ?? source.safeToShutdown),
    successorPeerId: source.successor_peer_id ?? source.successorPeerId ?? null,
    handoffCandidatePeerId: source.handoff_candidate_peer_id ?? source.handoffCandidatePeerId ?? null,
    worldDataReplicated: Boolean(source.world_data_replicated ?? source.worldDataReplicated),
    peers: Array.isArray(source.peers) ? source.peers : [],
    raw: source,
  };
}

"""
        text = replace_once(text, anchor, block + anchor, "host readiness normalizer")

    if "hostReadiness: async (world)" not in text:
        anchor = "    worldStatus: (world) => call('world_status', { world }),\n"
        block = """    hostReadiness: async (world) => {
      const raw = await call('host_readiness', { world });
      try {
        return normalizeHostReadiness(typeof raw === 'string' ? JSON.parse(raw) : raw);
      } catch (error) {
        throw new Error(`Host readiness was not valid JSON: ${error}`);
      }
    },
"""
        text = replace_once(text, anchor, anchor + block, "host readiness adapter")

    text = text.replace("    stopHost: () => call('stop_host'),", "    stopHost: (world) => call('stop_host', { world }),", 1)
    write(path, text)

    app_path = "apps/desktop/src/app.js"
    app = read(app_path)
    app = app.replace("() => backend.stopHost()", "() => backend.stopHost(worldId())", 1)
    app = app.replace("{ successMessage: 'Minecraft runtime stopped.' }", "{ successMessage: 'World stopped safely. Latest world state is checkpointed and sleeping.' }", 1)
    write(app_path, app)


def reconcile_workspace_dependencies() -> None:
    root = read("Cargo.toml")
    if "fs2 = \"0.4\"" not in root:
        root = root.replace("futures = \"0.3\"\n", "futures = \"0.3\"\nfs2 = \"0.4\"\n", 1)
    if "sha1 = \"0.10\"" not in root:
        root = root.replace("serde_json = \"1.0\"\n", "serde_json = \"1.0\"\nsha1 = \"0.10\"\nsha2 = \"0.10\"\n", 1)
    write("Cargo.toml", root)

    cargo = read("crates/swarm-cli/Cargo.toml")
    if "fs2.workspace = true" not in cargo:
        cargo = cargo.replace("clap.workspace = true\n", "clap.workspace = true\nfs2.workspace = true\n", 1)
    if "sha1.workspace = true" not in cargo:
        cargo = cargo.replace("serde_json.workspace = true\n", "serde_json.workspace = true\nsha1.workspace = true\nsha2.workspace = true\n", 1)
    write("crates/swarm-cli/Cargo.toml", cargo)


def reconcile_ci_packaging() -> None:
    path = ".github/workflows/ci.yml"
    text = read(path)
    if 'swarmcraft-runtime-$target' not in text:
        text = text.replace(
            '          cp target/release/swarmcraft-host "apps/desktop/src-tauri/binaries/swarmcraft-host-$target"\n',
            '          cp target/release/swarmcraft-host "apps/desktop/src-tauri/binaries/swarmcraft-host-$target"\n'
            '          cp target/release/swarmcraft-runtime "apps/desktop/src-tauri/binaries/swarmcraft-runtime-$target"\n',
            1,
        )
        text = text.replace(
            '          "apps/desktop/src-tauri/binaries/swarmcraft-host-$target" --version\n',
            '          "apps/desktop/src-tauri/binaries/swarmcraft-host-$target" --version\n'
            '          "apps/desktop/src-tauri/binaries/swarmcraft-runtime-$target" --version\n',
            1,
        )
    if 'swarmcraft-runtime-$target.exe' not in text:
        text = text.replace(
            '          Copy-Item target/release/swarmcraft-host.exe "apps/desktop/src-tauri/binaries/swarmcraft-host-$target.exe"\n',
            '          Copy-Item target/release/swarmcraft-host.exe "apps/desktop/src-tauri/binaries/swarmcraft-host-$target.exe"\n'
            '          Copy-Item target/release/swarmcraft-runtime.exe "apps/desktop/src-tauri/binaries/swarmcraft-runtime-$target.exe"\n',
            1,
        )
        text = text.replace(
            '          & "apps/desktop/src-tauri/binaries/swarmcraft-host-$target.exe" --version\n',
            '          & "apps/desktop/src-tauri/binaries/swarmcraft-host-$target.exe" --version\n'
            '          & "apps/desktop/src-tauri/binaries/swarmcraft-runtime-$target.exe" --version\n',
            1,
        )
    write(path, text)


def main() -> None:
    reconcile_workspace_dependencies()
    reconcile_cli_main()
    reconcile_migration()
    reconcile_runtime_installer()
    reconcile_runtime_main()
    reconcile_desktop_bridge()
    reconcile_frontend()


if __name__ == "__main__":
    main()
