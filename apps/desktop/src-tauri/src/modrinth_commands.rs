use swarm_provider::{
    modrinth::ModrinthClient, DownloadedArtifact, ModDownloadRequest, ModProjectDetails, ModResolveRequest,
    ModSearchQuery, ModSearchResult, ModVersionFilter, ModVersionList, ProviderFailure, ResolvedModGraph,
};

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

#[tauri::command]
pub fn modrinth_download(request: ModDownloadRequest) -> Result<DownloadedArtifact, ProviderFailure> {
    client()?.download(&request)
}
