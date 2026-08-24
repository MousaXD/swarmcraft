use crate::{require_value, run_cli};
use tauri::AppHandle;

const TRANSFER_WAIT_ATTEMPTS: usize = 160;
const TRANSFER_WAIT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);
const MAX_TRANSFER_TOKEN_BYTES: usize = 16 * 1024;
const TRANSFER_COMMANDS: [&str; 6] = [
    "transfer-prepare",
    "transfer-export",
    "transfer-accept",
    "transfer-commit",
    "transfer-activate",
    "transfer-observe",
];

fn looks_like_encoded_token(value: &str) -> bool {
    value.len() >= 64
        && value.len() <= MAX_TRANSFER_TOKEN_BYTES
        && (value.len() & 1) == 0
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn transfer_token(raw: &str) -> Option<String> {
    raw.lines()
        .map(str::trim)
        .find(|line| looks_like_encoded_token(line))
        .map(ToOwned::to_owned)
}

fn required_value(value: Option<String>, label: &str) -> Result<String, String> {
    require_value(value.unwrap_or_default(), label)
}

fn required_transfer_token(value: Option<String>, label: &str) -> Result<String, String> {
    let token = required_value(value, label)?;
    if !looks_like_encoded_token(&token) {
        return Err(format!("{label} must be a bounded even-length hex token from the signed backend transfer flow"));
    }
    Ok(token)
}

pub async fn transfer_supported(app: &AppHandle) -> bool {
    for command in TRANSFER_COMMANDS {
        if run_cli(
            app,
            vec!["world".into(), command.into(), "--help".into()],
        )
        .await
        .is_err()
        {
            return false;
        }
    }
    true
}

#[tauri::command(rename_all = "camelCase")]
pub async fn manual_transfer_step(
    app: AppHandle,
    world: String,
    action: String,
    value: Option<String>,
) -> Result<String, String> {
    let world = require_value(world, "World ID")?;
    match action.trim() {
        "prepare" => {
            let target = required_value(value, "Transfer target peer ID")?;
            let result = run_cli(
                &app,
                vec![
                    "world".into(),
                    "transfer-prepare".into(),
                    world.clone(),
                    target,
                ],
            )
            .await?;
            if let Some(token) = transfer_token(&result) {
                return Ok(token);
            }

            // A running authority must first complete the Fabric transfer save
            // barrier and publish the final canonical checkpoint. Do not expose a
            // prepared token to Desktop until the backend has durably reached that
            // state and transfer-export succeeds.
            for _ in 0..TRANSFER_WAIT_ATTEMPTS {
                if let Ok(raw) = run_cli(
                    &app,
                    vec!["world".into(), "transfer-export".into(), world.clone()],
                )
                .await
                {
                    if let Some(token) = transfer_token(&raw) {
                        return Ok(token);
                    }
                }
                tokio::time::sleep(TRANSFER_WAIT_INTERVAL).await;
            }
            Err("The host did not finish the transfer checkpoint before the Desktop handoff timeout. Minecraft was not force-killed; retry after migration status reaches the transfer acceptance stage.".into())
        }
        "accept" => {
            let token = required_transfer_token(value, "Prepared transfer token")?;
            let raw = run_cli(
                &app,
                vec!["world".into(), "transfer-accept".into(), world, token],
            )
            .await?;
            transfer_token(&raw).ok_or_else(|| "Backend did not return an accepted transfer token".into())
        }
        "commit" => {
            let token = required_transfer_token(value, "Accepted transfer token")?;
            let raw = run_cli(
                &app,
                vec!["world".into(), "transfer-commit".into(), world, token],
            )
            .await?;
            transfer_token(&raw).ok_or_else(|| "Backend did not return a committed transfer token".into())
        }
        "activate" => {
            let token = required_transfer_token(value, "Committed transfer token")?;
            let raw = run_cli(
                &app,
                vec!["world".into(), "transfer-activate".into(), world, token],
            )
            .await?;
            transfer_token(&raw).ok_or_else(|| "Backend did not return the signed successor epoch token".into())
        }
        "observe" => {
            let token = required_transfer_token(value, "Successor epoch token")?;
            run_cli(
                &app,
                vec!["world".into(), "transfer-observe".into(), world, token],
            )
            .await
        }
        _ => Err("Manual transfer action must be prepare, accept, commit, activate, or observe".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_only_bounded_hex_encoded_signed_tokens() {
        assert_eq!(transfer_token("Transfer checkpoint requested."), None);
        assert_eq!(transfer_token("abc"), None);
        let token = "ab".repeat(64);
        assert_eq!(transfer_token(&format!("{token}\ncopy this token")), Some(token));
        assert_eq!(transfer_token(&"ab".repeat((MAX_TRANSFER_TOKEN_BYTES / 2) + 1)), None);
    }

    #[test]
    fn pasted_tokens_are_bounded_before_cli_dispatch() {
        assert!(required_transfer_token(Some("ab".repeat(64)), "token").is_ok());
        assert!(required_transfer_token(Some("not-hex".into()), "token").is_err());
        assert!(required_transfer_token(Some("ab".repeat((MAX_TRANSFER_TOKEN_BYTES / 2) + 1)), "token").is_err());
    }
}
