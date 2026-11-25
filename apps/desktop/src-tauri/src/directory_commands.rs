//! Non-authoritative local directory policy and local listing publication controls.
use crate::{launcher_commands::configured_discovery_bootstraps, run_cli, run_runtime_cli};
use serde_json::Value;
use tauri::AppHandle;

fn world_id(value: &str) -> Result<String, String> {
    let value = value.trim();
    if !value.starts_with("scworld:") || value.len() != 72 {
        return Err("A full World ID is required".into());
    }
    value.parse::<swarm_protocol::WorldId>().map(|id| id.to_string()).map_err(|_| "Invalid World ID".into())
}

fn hash(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.len() != 64 {
        return Err("A complete listing hash is required".into());
    }
    value.parse::<swarm_protocol::Hash32>().map(|hash| hash.to_string()).map_err(|_| "Invalid listing hash".into())
}

async fn moderation(app: &AppHandle, args: Vec<String>) -> Result<Value, String> {
    let mut command = vec!["moderation".into()];
    command.extend(args);
    let raw = run_runtime_cli(app, command)
        .await
        .map_err(|_| "Local directory operation could not be confirmed".to_owned())?;
    serde_json::from_str(&raw).map_err(|_| "Malformed directory policy result".into())
}

#[tauri::command]
pub async fn directory_policy(app: AppHandle) -> Result<Value, String> {
    moderation(&app, vec!["state".into()]).await
}

#[tauri::command]
pub async fn directory_hide(app: AppHandle, world: String, enabled: bool) -> Result<Value, String> {
    moderation(&app, vec!["hide".into(), world_id(&world)?, format!("--enabled={enabled}")]).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn directory_report(
    app: AppHandle,
    world: String,
    config_hash: String,
    reason: String,
) -> Result<Value, String> {
    if !matches!(
        reason.as_str(),
        "spam" | "impersonation" | "malicious_content" | "harassment" | "misleading_metadata" | "other"
    ) {
        return Err("Choose a report category".into());
    }
    let mut args = vec![
        "report".into(),
        world_id(&world)?,
        format!("--config={}", hash(&config_hash)?),
        format!("--reason={reason}"),
    ];
    for address in configured_discovery_bootstraps() {
        args.push(format!("--bootstrap={address}"));
    }
    moderation(&app, args).await
}

#[tauri::command]
pub async fn directory_dismiss_report(app: AppHandle, report: String) -> Result<Value, String> {
    moderation(&app, vec!["dismiss".into(), hash(&report)?]).await
}

#[tauri::command]
pub async fn world_listing(app: AppHandle, world: String, paused: Option<bool>) -> Result<Value, String> {
    let mut args = vec!["world".into(), "discovery-listing".into(), world_id(&world)?];
    if let Some(paused) = paused {
        args.extend(["--paused".into(), paused.to_string()]);
    }
    let raw = run_cli(&app, args).await.map_err(|_| "Local listing preference could not be confirmed".to_owned())?;
    serde_json::from_str(&raw).map_err(|_| "Malformed local listing receipt".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_ids_reject_invite_tokens_or_cli_flags() {
        assert!(world_id("--help").is_err());
        assert!(world_id("scinvite:aa").is_err());
        assert!(world_id("scworld:ff").is_err());
        assert_eq!(world_id(&format!("scworld:{}", "AB".repeat(32))).unwrap(), format!("scworld:{}", "ab".repeat(32)));
        assert!(hash("not a hash").is_err());
    }
}
