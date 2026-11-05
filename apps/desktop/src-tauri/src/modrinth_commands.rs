use std::path::Path;
use swarm_provider::{
    modrinth::ModrinthClient, DownloadedArtifact, ModArtifactLocator, ModDownloadRequest, ModProjectDetails,
    ModResolveRequest, ModSearchQuery, ModSearchResult, ModVersionFilter, ModVersionList, ProviderFailure,
    ProviderFailureKind, ResolvedModGraph,
};

use super::launcher_commands::resolve_provider_staging_session;

fn client() -> Result<ModrinthClient, ProviderFailure> {
    ModrinthClient::production()
}

#[tauri::command]
pub fn modrinth_search(query: ModSearchQuery) -> Result<ModSearchResult, ProviderFailure> {
    client()?.search(&query)
}

#[tauri::command(rename_all = "camelCase")]
pub fn modrinth_project(project_id: String) -> Result<ModProjectDetails, ProviderFailure> {
    client()?.project(&project_id)
}

#[tauri::command(rename_all = "camelCase")]
pub fn modrinth_versions(project_id: String, filter: ModVersionFilter) -> Result<ModVersionList, ProviderFailure> {
    client()?.versions(&project_id, &filter)
}

#[tauri::command]
pub fn modrinth_resolve(request: ModResolveRequest) -> Result<ResolvedModGraph, ProviderFailure> {
    client()?.resolve(&request)
}

#[tauri::command(rename_all = "camelCase")]
pub fn modrinth_download(
    locator: ModArtifactLocator,
    staging_session: String,
    max_bytes: Option<u64>,
) -> Result<DownloadedArtifact, ProviderFailure> {
    safe_component(&locator.project_id, "Modrinth project ID")?;
    safe_component(&locator.version_id, "Modrinth version ID")?;
    let staging = resolve_provider_staging_session(&staging_session)
        .map_err(|message| ProviderFailure::new(ProviderFailureKind::InvalidRequest, message))?;
    let destination_dir = staging.join("modrinth").join(&locator.project_id).join(&locator.version_id);
    client()?.download(&ModDownloadRequest { locator, destination_dir, max_bytes })
}

fn safe_component(value: &str, label: &str) -> Result<(), ProviderFailure> {
    let trimmed = value.trim();
    let path = Path::new(trimmed);
    let portable = !trimmed.is_empty()
        && trimmed == value
        && trimmed.len() <= 255
        && !path.is_absolute()
        && path.components().count() == 1
        && !trimmed.contains(['/', '\\', ':', '\0'])
        && trimmed != "."
        && trimmed != ".."
        && !trimmed.ends_with(['.', ' ']);
    if portable {
        Ok(())
    } else {
        Err(ProviderFailure::new(
            ProviderFailureKind::MalformedResponse,
            format!("{label} is not a safe portable path component"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_identity_cannot_become_a_path() {
        for invalid in ["../x", "..\\x", "C:\\x", "\\\\server\\share", ".", "..", "x/../y", "x\\y"] {
            assert!(safe_component(invalid, "id").is_err(), "accepted {invalid}");
        }
        assert!(safe_component("A1-b_c.2", "id").is_ok());
    }
}
