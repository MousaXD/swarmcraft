//! Local contact and privacy controls. No contact action changes membership.
use crate::{launcher_commands::configured_discovery_bootstraps, run_cli, RuntimeProcesses};
use serde_json::{json, Value};
use std::str::FromStr;
use swarm_protocol::PeerId;
use tauri::{AppHandle, State};

fn peer_id(peer: &str) -> Result<String, String> {
    let peer = peer.trim();
    if !peer.starts_with("scpeer:") || peer.len() != 71 {
        return Err("A complete peer ID is required".into());
    }
    PeerId::from_str(peer).map(|value| value.to_string()).map_err(|_| "Invalid peer ID".into())
}

async fn contact_command(app: &AppHandle, args: Vec<String>) -> Result<Value, String> {
    let mut command = vec!["contact".into()];
    command.extend(args);
    let raw = run_cli(app, command).await.map_err(|_| "Contact operation could not be confirmed".to_owned())?;
    serde_json::from_str(&raw).map_err(|_| "Malformed native contact result".into())
}

#[tauri::command]
pub async fn friend_settings(app: AppHandle) -> Result<Value, String> {
    contact_command(&app, vec!["settings".into()]).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn friend_add(app: AppHandle, peer: String, public_key: String, label: String) -> Result<Value, String> {
    let peer = peer_id(&peer)?;
    let key: [u8; 32] = hex::decode(public_key.trim())
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or("A complete public key is required")?;
    if swarm_protocol::peer_id_from_public_key(&key).to_string() != peer {
        return Err("Peer ID and public key do not match".into());
    }
    let label = label.trim();
    if label.is_empty() || label.len() > 96 || label.chars().any(char::is_control) {
        return Err("Use a label of 1 to 96 UTF-8 bytes without control characters".into());
    }
    contact_command(
        &app,
        vec!["add".into(), peer, format!("--public-key={}", hex::encode(key)), format!("--label={label}")],
    )
    .await
}

#[tauri::command]
pub async fn friend_remove(app: AppHandle, peer: String) -> Result<Value, String> {
    contact_command(&app, vec!["remove".into(), peer_id(&peer)?]).await
}

#[tauri::command]
pub async fn friend_share_presence(app: AppHandle, peer: String, enabled: bool) -> Result<Value, String> {
    contact_command(&app, vec!["share-presence".into(), peer_id(&peer)?, format!("--enabled={enabled}")]).await
}

#[tauri::command]
pub async fn friend_presence_mode(
    app: AppHandle,
    processes: State<'_, RuntimeProcesses>,
    enabled: bool,
) -> Result<Value, String> {
    // Persist denial before attempting to stop the Desktop-owned process.
    // A failed kill or stale directory hint cannot authorize new proofs.
    let result = contact_command(&app, vec!["presence-mode".into(), format!("--enabled={enabled}")]).await?;
    if !enabled && processes.presence_service_pid()?.is_some() {
        let _ = processes.stop_presence_service();
    }
    Ok(result)
}

#[tauri::command]
pub async fn friend_probe(app: AppHandle, peer: String) -> Result<Value, String> {
    let mut args = vec!["presence".into(), peer_id(&peer)?];
    for locator in configured_discovery_bootstraps() {
        args.push(format!("--bootstrap={locator}"));
    }
    contact_command(&app, args).await
}

#[tauri::command]
pub fn friend_service_status(processes: State<'_, RuntimeProcesses>) -> Result<Value, String> {
    Ok(json!({"process_started": processes.presence_service_pid()?.is_some()}))
}

#[tauri::command]
pub async fn friend_service_start(app: AppHandle, processes: State<'_, RuntimeProcesses>) -> Result<Value, String> {
    let settings = friend_settings(app.clone()).await?;
    if settings["presence_enabled"].as_bool() != Some(true)
        || !settings["contacts"]
            .as_array()
            .is_some_and(|contacts| contacts.iter().any(|c| c["share_presence"].as_bool() == Some(true)))
    {
        return Err("Enable presence and explicitly allow a contact first".into());
    }
    let listen = std::env::var("SWARMCRAFT_PRESENCE_LISTEN").unwrap_or_else(|_| "/ip4/127.0.0.1/tcp/0".into());
    processes.start_presence_service(&app, listen, configured_discovery_bootstraps())?;
    friend_service_status(processes)
}

#[tauri::command]
pub fn friend_service_stop(processes: State<'_, RuntimeProcesses>) -> Result<Value, String> {
    if processes.presence_service_pid()?.is_some() {
        processes.stop_presence_service()?;
    }
    friend_service_status(processes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_peer_id_rejects_invites_flags_and_short_ids() {
        assert!(peer_id("scinvite:00").is_err());
        assert!(peer_id("--help").is_err());
        assert!(peer_id("scpeer:01").is_err());
        assert_eq!(peer_id(&format!("scpeer:{}", "AB".repeat(32))).unwrap(), format!("scpeer:{}", "ab".repeat(32)));
    }
}
