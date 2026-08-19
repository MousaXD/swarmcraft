use crate::{require_value, run_cli};
use tauri::AppHandle;

const TRANSFER_WAIT_ATTEMPTS: usize = 160;
const TRANSFER_WAIT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);
const TRANSFER_COMMANDS: [&str; 6] = [
    "transfer-prepare",
    "transfer-export",
    "transfer-accept",
    "transfer-commit",
    "transfer-activate",
    "transfer-observe",
];

fn transfer_token(raw: &str) -> Option<String> {
    raw.lines()
        .map(str::trim)
        .find(|line| line.len() >= 64 && line.len() % 2 == 0 && line.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(ToOwned::to_owned)
}

fn required_value(value: Option<String>, label: &str) -> Result<String, String> {
    require_value(value.unwrap_or_default(), label)
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
                match run_cli(
                    &app,
                    vec!["world".into(), "transfer-export".into(), world.clone()],
                )
                .await
                {
                    Ok(raw) => {
                        if let Some(token) = transfer_token(&raw) {
                            return Ok(token);
                        }
                    }
                    Err(_) => {}
                }
                tokio::time::sleep(TRANSFER_WAIT_INTERVAL).await;
            }
            Err("The host did not finish the transfer checkpoint before the Desktop handoff timeout. Minecraft was not force-killed; retry after migration status reaches the transfer acceptance stage.".into())
        }
        "accept" => {
            let token = required_value(value, "Prepared transfer token")?;
            let raw = run_cli(
                &app,
                vec!["world".into(), "transfer-accept".into(), world, token],
            )
            .await?;
            transfer_token(&raw).ok_or_else(|| "Backend did not return an accepted transfer token".into())
        }
        "commit" => {
            let token = required_value(value, "Accepted transfer token")?;
            let raw = run_cli(
                &app,
                vec!["world".into(), "transfer-commit".into(), world, token],
            )
            .await?;
            transfer_token(&raw).ok_or_else(|| "Backend did not return a committed transfer token".into())
        }
        "activate" => {
            let token = required_value(value, "Committed transfer token")?;
            let raw = run_cli(
                &app,
                vec!["world".into(), "transfer-activate".into(), world, token],
            )
            .await?;
            transfer_token(&raw).ok_or_else(|| "Backend did not return the signed successor epoch token".into())
        }
        "observe" => {
            let token = required_value(value, "Successor epoch token")?;
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
    fn extracts_only_hex_encoded_signed_tokens() {
        assert_eq!(transfer_token("Transfer checkpoint requested."), None);
        assert_eq!(transfer_token("abc"), None);
        let token = "ab".repeat(64);
        assert_eq!(transfer_token(&format!("{token}\ncopy this token")), Some(token));
    }
}
