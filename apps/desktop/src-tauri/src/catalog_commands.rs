use swarm_catalog::{
    CatalogErrorPayload, CatalogProvider, CatalogResponse, CatalogService, FabricLoaderVersion, MinecraftVersion,
};

fn error_payload(provider: CatalogProvider, error: swarm_catalog::CatalogError) -> CatalogErrorPayload {
    CatalogErrorPayload::from_error(provider, &error)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn minecraft_versions(
    include_snapshots: bool,
    refresh: bool,
) -> Result<CatalogResponse<MinecraftVersion>, CatalogErrorPayload> {
    tauri::async_runtime::spawn_blocking(move || {
        let service = CatalogService::discover().map_err(|error| error_payload(CatalogProvider::Mojang, error))?;
        service
            .minecraft_versions(include_snapshots, refresh)
            .map_err(|error| error_payload(CatalogProvider::Mojang, error))
    })
    .await
    .map_err(|error| CatalogErrorPayload {
        code: "catalog_task_failed".into(),
        provider: CatalogProvider::Mojang.as_str().into(),
        message: error.to_string(),
    })?
}

#[tauri::command(rename_all = "camelCase")]
pub async fn fabric_loader_versions(
    minecraft_version: String,
    refresh: bool,
) -> Result<CatalogResponse<FabricLoaderVersion>, CatalogErrorPayload> {
    tauri::async_runtime::spawn_blocking(move || {
        let service = CatalogService::discover().map_err(|error| error_payload(CatalogProvider::Fabric, error))?;
        service
            .fabric_loader_versions(&minecraft_version, refresh)
            .map_err(|error| error_payload(CatalogProvider::Fabric, error))
    })
    .await
    .map_err(|error| CatalogErrorPayload {
        code: "catalog_task_failed".into(),
        provider: CatalogProvider::Fabric.as_str().into(),
        message: error.to_string(),
    })?
}

pub async fn validate_fabric_selection(
    minecraft_version: String,
    fabric_loader_version: String,
) -> Result<(), CatalogErrorPayload> {
    tauri::async_runtime::spawn_blocking(move || {
        let service = CatalogService::discover().map_err(|error| error_payload(CatalogProvider::Fabric, error))?;
        service
            .validate_fabric_selection(&minecraft_version, &fabric_loader_version, false)
            .map(|_| ())
            .map_err(|error| error_payload(CatalogProvider::Fabric, error))
    })
    .await
    .map_err(|error| CatalogErrorPayload {
        code: "catalog_task_failed".into(),
        provider: CatalogProvider::Fabric.as_str().into(),
        message: error.to_string(),
    })?
}
