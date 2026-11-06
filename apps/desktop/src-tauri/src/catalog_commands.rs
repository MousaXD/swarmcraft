use swarm_catalog::{
    CatalogErrorPayload, CatalogOrigin, CatalogProvider, CatalogResponse, CatalogService, FabricLoaderVersion,
    MinecraftVersion,
};
use swarm_protocol::{minecraft_supported_by_shipped_adapter, validate_runtime_selection};

// Official-provider availability and shipped Fabric-adapter compatibility are
// distinct. Keep the original `supported` flag and annotate each version
// using the authoritative protocol adapter contract. Newer releases remain
// visible in the GUI but cannot be selected for canonical create/import.
#[derive(serde::Serialize)]
pub struct DesktopCatalogVersion<T> {
    #[serde(flatten)]
    catalog: T,
    adapter_supported: bool,
}

#[derive(serde::Serialize)]
pub struct DesktopCatalogResponse<T> {
    provider: CatalogProvider,
    source_url: String,
    fetched_at_unix_seconds: u64,
    cache_expires_at_unix_seconds: u64,
    origin: CatalogOrigin,
    warning: Option<String>,
    versions: Vec<DesktopCatalogVersion<T>>,
}

fn mark_adapter_support<T>(response: CatalogResponse<T>, compatible: impl Fn(&T) -> bool) -> DesktopCatalogResponse<T> {
    DesktopCatalogResponse {
        provider: response.provider,
        source_url: response.source_url,
        fetched_at_unix_seconds: response.fetched_at_unix_seconds,
        cache_expires_at_unix_seconds: response.cache_expires_at_unix_seconds,
        origin: response.origin,
        warning: response.warning,
        versions: response
            .versions
            .into_iter()
            .map(|catalog| {
                let adapter_supported = compatible(&catalog);
                DesktopCatalogVersion { catalog, adapter_supported }
            })
            .collect(),
    }
}

fn error_payload(provider: CatalogProvider, error: swarm_catalog::CatalogError) -> CatalogErrorPayload {
    CatalogErrorPayload::from_error(provider, &error)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn minecraft_versions(
    include_snapshots: bool,
    refresh: bool,
) -> Result<DesktopCatalogResponse<MinecraftVersion>, CatalogErrorPayload> {
    tauri::async_runtime::spawn_blocking(move || {
        let service = CatalogService::discover().map_err(|error| error_payload(CatalogProvider::Mojang, error))?;
        service
            .minecraft_versions(include_snapshots, refresh)
            .map(|response| {
                mark_adapter_support(response, |version: &MinecraftVersion| {
                    minecraft_supported_by_shipped_adapter(&version.id)
                })
            })
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
) -> Result<DesktopCatalogResponse<FabricLoaderVersion>, CatalogErrorPayload> {
    tauri::async_runtime::spawn_blocking(move || {
        let service = CatalogService::discover().map_err(|error| error_payload(CatalogProvider::Fabric, error))?;
        service
            .fabric_loader_versions(&minecraft_version, refresh)
            .map(|response| {
                mark_adapter_support(response, |loader: &FabricLoaderVersion| {
                    validate_runtime_selection(&loader.minecraft_version, &loader.version, None).is_ok()
                })
            })
            .map_err(|error| error_payload(CatalogProvider::Fabric, error))
    })
    .await
    .map_err(|error| CatalogErrorPayload {
        code: "catalog_task_failed".into(),
        provider: CatalogProvider::Fabric.as_str().into(),
        message: error.to_string(),
    })?
}

#[tauri::command(rename_all = "camelCase")]
pub async fn validate_fabric_selection(
    minecraft_version: String,
    fabric_loader_version: String,
) -> Result<(), CatalogErrorPayload> {
    tauri::async_runtime::spawn_blocking(move || {
        validate_runtime_selection(&minecraft_version, &fabric_loader_version, None).map_err(|error| {
            CatalogErrorPayload {
                code: "unsupported_runtime_adapter".into(),
                provider: CatalogProvider::Fabric.as_str().into(),
                message: error.to_string(),
            }
        })?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annotate_runtime_compatibility_without_erasing_official_provider_versions() {
        let catalog = CatalogResponse {
            provider: CatalogProvider::Mojang,
            source_url: "https://piston-meta.mojang.com/".into(),
            fetched_at_unix_seconds: 1,
            cache_expires_at_unix_seconds: 2,
            origin: CatalogOrigin::Network,
            warning: None,
            versions: vec![
                MinecraftVersion {
                    id: "26.3".into(),
                    version_type: "release".into(),
                    release_time: "2026-11-01".into(),
                    supported: true,
                },
                MinecraftVersion {
                    id: "26.1.2".into(),
                    version_type: "release".into(),
                    release_time: "2026-01-01".into(),
                    supported: true,
                },
            ],
        };
        let result = mark_adapter_support(catalog, |version: &MinecraftVersion| {
            minecraft_supported_by_shipped_adapter(&version.id)
        });
        let encoded = serde_json::to_value(result).expect("serialize native catalogue");
        assert_eq!(encoded["versions"][0]["supported"], true, "newer official releases must remain visible");
        assert_eq!(encoded["versions"][0]["adapter_supported"], false);
        assert_eq!(encoded["versions"][1]["adapter_supported"], true);
    }

    #[test]
    fn annotate_loader_requirements_from_the_shipped_adapter() {
        let catalog = CatalogResponse {
            provider: CatalogProvider::Fabric,
            source_url: "https://meta.fabricmc.net/".into(),
            fetched_at_unix_seconds: 1,
            cache_expires_at_unix_seconds: 2,
            origin: CatalogOrigin::Network,
            warning: None,
            versions: vec![
                FabricLoaderVersion { version: "0.19.2".into(), stable: true, minecraft_version: "26.1.2".into() },
                FabricLoaderVersion { version: "0.20.1".into(), stable: true, minecraft_version: "26.1.2".into() },
            ],
        };
        let result = mark_adapter_support(catalog, |loader: &FabricLoaderVersion| {
            validate_runtime_selection(&loader.minecraft_version, &loader.version, None).is_ok()
        });
        let encoded = serde_json::to_value(result).expect("serialize native loader catalogue");
        assert_eq!(encoded["versions"][0]["adapter_supported"], false);
        assert_eq!(encoded["versions"][1]["adapter_supported"], true);
    }
}
