use serde_json::{json, Value};
use std::{
    env, fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use swarm_core::DataPaths;
use swarm_provider::{ModResolveRequest, ModVersionFilter, PackageEnvironment, ResolvedModGraph};
use tauri::AppHandle;

use super::{curseforge, modrinth_commands, run_runtime_cli};

#[tauri::command]
pub(crate) fn provider_staging_dir() -> Result<String, String> {
    let paths = DataPaths::discover().map_err(|error| error.to_string())?;
    paths.ensure().map_err(|error| error.to_string())?;
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|error| error.to_string())?.as_nanos();
    let session = format!("desktop-{}-{nonce}", std::process::id());
    let path = paths.root.join("provider-staging").join(&session);
    fs::create_dir_all(&path).map_err(|error| format!("Could not create provider staging directory: {error}"))?;
    Ok(session)
}

pub(crate) fn resolve_provider_staging_session(session: &str) -> Result<PathBuf, String> {
    let session = session.trim();
    if !valid_provider_staging_session(session) {
        return Err("Provider staging session is invalid or expired".into());
    }
    let paths = DataPaths::discover().map_err(|error| error.to_string())?;
    paths.ensure().map_err(|error| error.to_string())?;
    let root = paths.root.join("provider-staging");
    let candidate = root.join(session);
    let metadata =
        fs::symlink_metadata(&candidate).map_err(|_| "Provider staging session is invalid or expired".to_owned())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Provider staging session is not a private directory".into());
    }
    Ok(candidate)
}

fn valid_provider_staging_session(session: &str) -> bool {
    session.starts_with("desktop-")
        && session.len() <= 128
        && session.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) fn modrinth_resolve_project(
    project_id: String,
    minecraft_version: String,
    loader: String,
    environment: PackageEnvironment,
) -> Result<ResolvedModGraph, String> {
    let versions = modrinth_commands::modrinth_versions(
        project_id.clone(),
        ModVersionFilter {
            minecraft_version: minecraft_version.clone(),
            loader: loader.clone(),
            environment,
            release_type: None,
        },
    )
    .map_err(|error| error.to_string())?;
    let root = versions.items.first().ok_or_else(|| {
        format!("Modrinth project {project_id} has no compatible {loader} build for Minecraft {minecraft_version}")
    })?;
    modrinth_commands::modrinth_resolve(ModResolveRequest {
        root_version_id: root.version_id.clone(),
        minecraft_version,
        loader,
        environment,
        allowed_release_types: Vec::new(),
    })
    .map_err(|error| error.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn curseforge_resolve_project(
    project_id: u64,
    minecraft: String,
    loader: String,
    environment: String,
) -> Value {
    let versions_envelope = curseforge::curseforge_versions(project_id, minecraft.clone(), loader.clone()).await;
    if versions_envelope.get("status").and_then(Value::as_str) != Some("ok") {
        return versions_envelope;
    }
    let Some(versions) =
        versions_envelope.get("data").and_then(|value| value.get("versions")).and_then(Value::as_array)
    else {
        return launcher_error("malformed_response", "CurseForge version response omitted its versions list");
    };
    let root = versions
        .iter()
        .filter(|version| version.get("is_available").and_then(Value::as_bool).unwrap_or(false))
        .max_by(|left, right| {
            let left_date = left.get("file_date").and_then(Value::as_str).unwrap_or_default();
            let right_date = right.get("file_date").and_then(Value::as_str).unwrap_or_default();
            left_date.cmp(right_date).then_with(|| {
                let left_id = left
                    .get("file_id")
                    .and_then(Value::as_str)
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or_default();
                let right_id = right
                    .get("file_id")
                    .and_then(Value::as_str)
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or_default();
                left_id.cmp(&right_id)
            })
        });
    let Some(file_id) =
        root.and_then(|value| value.get("file_id")).and_then(Value::as_str).and_then(|value| value.parse::<u64>().ok())
    else {
        return launcher_error(
            "incompatible",
            &format!(
                "CurseForge project {project_id} has no available compatible Fabric file for Minecraft {minecraft}"
            ),
        );
    };
    curseforge::curseforge_resolve(file_id, minecraft, loader, environment).await
}

#[tauri::command]
pub(crate) async fn inspect_mod_artifact(app: AppHandle, path: String) -> Result<Value, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("Mod artifact path is required".into());
    }
    let raw = run_runtime_cli(&app, vec!["inspect-mod".into(), path.into()]).await?;
    serde_json::from_str(&raw).map_err(|error| format!("Runtime returned malformed mod inspection JSON: {error}"))
}

#[tauri::command]
pub(crate) async fn discovery_search(app: AppHandle, query: Option<String>) -> Result<Value, String> {
    let mut arguments = vec!["discovery-search".into()];
    if let Some(query) = query.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty()) {
        arguments.push("--query".into());
        arguments.push(query);
    }
    add_discovery_bootstraps(&mut arguments);
    let raw = run_runtime_cli(&app, arguments).await?;
    serde_json::from_str(&raw).map_err(|error| format!("Runtime returned malformed discovery search JSON: {error}"))
}

#[tauri::command]
pub(crate) async fn discovery_resolve(app: AppHandle, world: String) -> Result<Value, String> {
    let world = world.trim();
    if world.is_empty() {
        return Err("World ID is required".into());
    }
    let mut arguments = vec!["discovery-resolve".into(), world.into()];
    add_discovery_bootstraps(&mut arguments);
    let raw = run_runtime_cli(&app, arguments).await?;
    serde_json::from_str(&raw).map_err(|error| format!("Runtime returned malformed discovery resolve JSON: {error}"))
}

fn add_discovery_bootstraps(arguments: &mut Vec<String>) {
    for address in configured_discovery_bootstraps() {
        arguments.push("--bootstrap".into());
        arguments.push(address);
    }
}

fn configured_discovery_bootstraps() -> Vec<String> {
    env::var("SWARMCRAFT_DISCOVERY_BOOTSTRAP")
        .ok()
        .into_iter()
        .flat_map(|value| value.split([',', ';', '\n']).map(str::to_owned).collect::<Vec<_>>())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect()
}

fn launcher_error(code: &str, message: &str) -> Value {
    json!({
        "status": "error",
        "provider": "curseforge",
        "error": { "code": code, "message": message }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curseforge_root_order_matches_provider_newest_then_highest_file_id() {
        let mut versions = [
            json!({"is_available": true, "file_date": "2026-08-01T00:00:00Z", "file_id": "10"}),
            json!({"is_available": true, "file_date": "2026-08-02T00:00:00Z", "file_id": "9"}),
            json!({"is_available": true, "file_date": "2026-08-02T00:00:00Z", "file_id": "11"}),
        ];
        versions.sort_by(|left, right| {
            let left_date = left.get("file_date").and_then(Value::as_str).unwrap_or_default();
            let right_date = right.get("file_date").and_then(Value::as_str).unwrap_or_default();
            right_date.cmp(left_date).then_with(|| {
                let left_id = left.get("file_id").and_then(Value::as_str).unwrap().parse::<u64>().unwrap();
                let right_id = right.get("file_id").and_then(Value::as_str).unwrap().parse::<u64>().unwrap();
                right_id.cmp(&left_id)
            })
        });
        assert_eq!(versions[0]["file_id"], "11");
    }

    #[test]
    fn staging_session_is_opaque_and_path_free() {
        assert!(valid_provider_staging_session("desktop-123-456"));
        for invalid in [
            "../desktop-123",
            "desktop/123",
            "desktop\\123",
            "C:\\desktop-123",
            "desktop-123/../../escape",
            "not-desktop-123",
        ] {
            assert!(!valid_provider_staging_session(invalid), "accepted {invalid}");
        }
    }
}
