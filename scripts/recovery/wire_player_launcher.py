from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    target = Path(path)
    text = target.read_text()
    if new in text:
        return
    if old not in text:
        raise SystemExit(f"expected patch seam not found in {path}: {old[:80]!r}")
    target.write_text(text.replace(old, new, 1))


# Activate the authoritative catalog selectors and the player journey controller.
replace_once(
    "apps/desktop/src/app.js",
    "import { createImportRequest, parseImportResult } from './import-flow.js';\n",
    "import { createImportRequest, parseImportResult } from './import-flow.js';\nimport './catalog-selectors.js';\nimport './launcher-controller.js';\n",
)

# Desktop needs shared CLI discovery/mod-inspection APIs, not a duplicate frontend authority layer.
replace_once(
    "apps/desktop/src-tauri/Cargo.toml",
    'swarm-catalog = { path = "../../../crates/swarm-catalog" }\n',
    'swarm-catalog = { path = "../../../crates/swarm-catalog" }\nswarm-cli = { path = "../../../crates/swarm-cli" }\n',
)

# Resolve a Modrinth project to the deterministic newest compatible exact root, then the existing resolver closes dependencies.
modrinth_path = Path("apps/desktop/src-tauri/src/modrinth_commands.rs")
modrinth = modrinth_path.read_text()
modrinth = modrinth.replace(
    "    ModSearchQuery, ModSearchResult, ModVersionFilter, ModVersionList, ProviderFailure, ResolvedModGraph,\n",
    "    ModSearchQuery, ModSearchResult, ModVersionFilter, ModVersionList, PackageEnvironment, ProviderFailure,\n    ProviderFailureKind, ResolvedModGraph,\n",
    1,
)
if "pub fn modrinth_resolve_project" not in modrinth:
    marker = "\n#[tauri::command]\npub fn modrinth_download"
    if marker not in modrinth:
        raise SystemExit("Modrinth command seam changed")
    addition = r'''
#[tauri::command(rename_all = "camelCase")]
pub fn modrinth_resolve_project(
    project_id: String,
    minecraft_version: String,
    loader: String,
    environment: PackageEnvironment,
) -> Result<ResolvedModGraph, ProviderFailure> {
    let client = client()?;
    let filter = ModVersionFilter {
        minecraft_version: minecraft_version.clone(),
        loader: loader.clone(),
        environment,
        release_type: None,
    };
    let versions = client.versions(&project_id, &filter)?;
    let root = versions.items.first().ok_or_else(|| {
        ProviderFailure::new(
            ProviderFailureKind::Incompatible,
            format!("Modrinth project {project_id} has no compatible {loader} build for Minecraft {minecraft_version}"),
        )
    })?;
    client.resolve(&ModResolveRequest {
        root_version_id: root.version_id.clone(),
        minecraft_version,
        loader,
        environment,
        allowed_release_types: Vec::new(),
    })
}
'''
    modrinth = modrinth.replace(marker, addition + marker, 1)
modrinth_path.write_text(modrinth)

# CurseForge gets the same project-level deterministic root selection without weakening its existing download restrictions.
curse_path = Path("apps/desktop/src-tauri/src/curseforge.rs")
curse = curse_path.read_text()
if "pub async fn curseforge_resolve_project" not in curse:
    marker = '\n#[tauri::command(rename_all = "camelCase")]\npub async fn curseforge_resolve('
    if marker not in curse:
        raise SystemExit("CurseForge command seam changed")
    addition = r'''
#[tauri::command(rename_all = "camelCase")]
pub async fn curseforge_resolve_project(
    project_id: u64,
    minecraft: String,
    loader: String,
    environment: String,
) -> Value {
    match async {
        let target = Target::parse(minecraft, loader, environment)?;
        let client = CurseForgeClient::from_environment()?;
        let root = select_best_file(client.compatible_files(project_id, &target).await?)?;
        resolve_dependency_graph(&client, root, &target).await
    }
    .await
    {
        Ok(value) => ok(value),
        Err(error) => error.into_response(),
    }
}
'''
    curse = curse.replace(marker, addition + marker, 1)
curse_path.write_text(curse)

main_path = Path("apps/desktop/src-tauri/src/main.rs")
main = main_path.read_text()
main = main.replace(
    "use curseforge::{\n    curseforge_download, curseforge_project, curseforge_provider_status, curseforge_resolve, curseforge_search,\n    curseforge_versions,\n};",
    "use curseforge::{\n    curseforge_download, curseforge_project, curseforge_provider_status, curseforge_resolve, curseforge_resolve_project,\n    curseforge_search, curseforge_versions,\n};",
    1,
)
main = main.replace(
    "use modrinth_commands::{modrinth_download, modrinth_project, modrinth_resolve, modrinth_search, modrinth_versions};",
    "use modrinth_commands::{\n    modrinth_download, modrinth_project, modrinth_resolve, modrinth_resolve_project, modrinth_search, modrinth_versions,\n};",
    1,
)
if "use swarm_cli::discovery::" not in main:
    main = main.replace(
        "use runtime_commands::{ensure_daemon_running, start_daemon, stop_daemon, stop_host};\n",
        "use runtime_commands::{ensure_daemon_running, start_daemon, stop_daemon, stop_host};\nuse swarm_cli::discovery::{self, DiscoverySearchInputV1};\n",
        1,
    )

if "fn provider_staging_dir()" not in main:
    marker = "\n#[tauri::command]\nasync fn initialize_node"
    if marker not in main:
        raise SystemExit("Desktop helper insertion seam changed")
    helpers = r'''
fn configured_discovery_bootstraps() -> Vec<String> {
    std::env::var("SWARMCRAFT_DISCOVERY_BOOTSTRAP")
        .ok()
        .into_iter()
        .flat_map(|value| value.split([',', ';', '\n']).map(str::to_owned).collect::<Vec<_>>())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect()
}

#[tauri::command]
fn provider_staging_dir() -> Result<String, String> {
    let paths = swarm_core::DataPaths::discover().map_err(|error| error.to_string())?;
    paths.ensure().map_err(|error| error.to_string())?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let path = paths.root.join("provider-staging").join(format!("{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
fn inspect_mod_artifact(path: String) -> Result<swarm_cli::server_mods::InstalledServerMod, String> {
    let path = require_value(path, "Mod artifact path")?;
    swarm_cli::server_mods::inspect_fabric_mod(std::path::Path::new(&path)).map_err(|error| error.to_string())
}

#[tauri::command]
async fn discovery_search(query: Option<String>) -> Result<discovery::PublicWorldSearchReportV1, String> {
    let paths = swarm_core::DataPaths::discover().map_err(|error| error.to_string())?;
    discovery::search_public_worlds(
        &paths,
        DiscoverySearchInputV1 { query, ..Default::default() },
        &configured_discovery_bootstraps(),
    )
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
async fn discovery_resolve(world: String) -> Result<discovery::ResolveWorldReportV1, String> {
    let paths = swarm_core::DataPaths::discover().map_err(|error| error.to_string())?;
    let world: swarm_protocol::WorldId = require_value(world, "World ID")?
        .parse()
        .map_err(|error| format!("invalid world ID: {error}"))?;
    discovery::resolve_world(&paths, world, &configured_discovery_bootstraps())
        .await
        .map_err(|error| error.to_string())
}
'''
    main = main.replace(marker, helpers + marker, 1)

registrations = [
    ("            fabric_loader_versions,\n", "            fabric_loader_versions,\n            validate_fabric_selection,\n            provider_staging_dir,\n            inspect_mod_artifact,\n            discovery_search,\n            discovery_resolve,\n"),
    ("            modrinth_resolve,\n", "            modrinth_resolve,\n            modrinth_resolve_project,\n"),
    ("            curseforge_resolve,\n", "            curseforge_resolve,\n            curseforge_resolve_project,\n"),
]
for old, new in registrations:
    if new not in main:
        if old not in main:
            raise SystemExit(f"Desktop command registration seam changed: {old!r}")
        main = main.replace(old, new, 1)
main_path.write_text(main)
