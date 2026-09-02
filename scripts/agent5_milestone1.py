from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def edit(path, old, new):
    target = ROOT / path
    text = target.read_text()
    if old not in text:
        raise SystemExit(f"expected text not found in {path}: {old[:120]!r}")
    target.write_text(text.replace(old, new, 1))


def append(path, text):
    target = ROOT / path
    current = target.read_text()
    if text.strip() in current:
        return
    target.write_text(current + text)


# Desktop staging sessions are opaque tokens. The webview never receives or chooses a filesystem path.
edit(
    "apps/desktop/src-tauri/src/launcher_commands.rs",
    """use std::{\n    env, fs,\n    time::{SystemTime, UNIX_EPOCH},\n};""",
    """use std::{\n    env, fs,\n    path::PathBuf,\n    time::{SystemTime, UNIX_EPOCH},\n};""",
)
edit(
    "apps/desktop/src-tauri/src/launcher_commands.rs",
    """#[tauri::command]\npub(crate) fn provider_staging_dir() -> Result<String, String> {\n    let paths = DataPaths::discover().map_err(|error| error.to_string())?;\n    paths.ensure().map_err(|error| error.to_string())?;\n    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|error| error.to_string())?.as_nanos();\n    let path = paths.root.join(\"provider-staging\").join(format!(\"desktop-{}-{nonce}\", std::process::id()));\n    fs::create_dir_all(&path).map_err(|error| format!(\"Could not create provider staging directory: {error}\"))?;\n    Ok(path.to_string_lossy().into_owned())\n}\n""",
    """#[tauri::command]\npub(crate) fn provider_staging_dir() -> Result<String, String> {\n    let paths = DataPaths::discover().map_err(|error| error.to_string())?;\n    paths.ensure().map_err(|error| error.to_string())?;\n    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|error| error.to_string())?.as_nanos();\n    let session = format!(\"desktop-{}-{nonce}\", std::process::id());\n    let path = paths.root.join(\"provider-staging\").join(&session);\n    fs::create_dir_all(&path).map_err(|error| format!(\"Could not create provider staging directory: {error}\"))?;\n    Ok(session)\n}\n\npub(crate) fn resolve_provider_staging_session(session: &str) -> Result<PathBuf, String> {\n    let session = session.trim();\n    if !valid_provider_staging_session(session) {\n        return Err(\"Provider staging session is invalid or expired\".into());\n    }\n    let paths = DataPaths::discover().map_err(|error| error.to_string())?;\n    paths.ensure().map_err(|error| error.to_string())?;\n    let root = paths.root.join(\"provider-staging\");\n    let candidate = root.join(session);\n    let metadata = fs::symlink_metadata(&candidate)\n        .map_err(|_| \"Provider staging session is invalid or expired\".to_owned())?;\n    if !metadata.is_dir() || metadata.file_type().is_symlink() {\n        return Err(\"Provider staging session is not a private directory\".into());\n    }\n    Ok(candidate)\n}\n\nfn valid_provider_staging_session(session: &str) -> bool {\n    session.starts_with(\"desktop-\")\n        && session.len() <= 128\n        && session.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')\n}\n""",
)
edit(
    "apps/desktop/src-tauri/src/launcher_commands.rs",
    """    fn staging_path_is_not_player_supplied() {\n        let path = std::path::PathBuf::from(\"provider-staging\").join(\"desktop-test\");\n        assert!(path.ends_with(\"desktop-test\"));\n    }""",
    """    fn staging_session_is_opaque_and_path_free() {\n        assert!(valid_provider_staging_session(\"desktop-123-456\"));\n        for invalid in [\n            \"../desktop-123\",\n            \"desktop/123\",\n            \"desktop\\\\123\",\n            \"C:\\\\desktop-123\",\n            \"desktop-123/../../escape\",\n            \"not-desktop-123\",\n        ] {\n            assert!(!valid_provider_staging_session(invalid), \"accepted {invalid}\");\n        }\n    }""",
)

# Replace the Desktop Modrinth command wrapper so the webview can supply identities, never a destination directory.
(ROOT / "apps/desktop/src-tauri/src/modrinth_commands.rs").write_text(r'''use std::path::Path;
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
    let staging = resolve_provider_staging_session(&staging_session).map_err(|message| {
        ProviderFailure::new(ProviderFailureKind::InvalidRequest, message)
    })?;
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
''')

# CurseForge download publication is now derived from the opaque staging session and provider identity.
edit(
    "apps/desktop/src-tauri/src/curseforge.rs",
    "use tokio::io::AsyncWriteExt;\n",
    "use tokio::io::AsyncWriteExt;\n\nuse super::launcher_commands::resolve_provider_staging_session;\n",
)
edit(
    "apps/desktop/src-tauri/src/curseforge.rs",
    """    let file_name = required_string(file, \"fileName\")?;\n    let release = release_type(file.get(\"releaseType\").and_then(Value::as_u64).unwrap_or_default());""",
    """    let file_name = required_string(file, \"fileName\")?;\n    safe_jar_filename(file_name)?;\n    let release = release_type(file.get(\"releaseType\").and_then(Value::as_u64).unwrap_or_default());""",
)
edit(
    "apps/desktop/src-tauri/src/curseforge.rs",
    """    let file_name = required_string(file, \"fileName\")?;\n    let display_name = required_string(file, \"displayName\")?;\n    let website_url = project.get(\"links\").and_then(|links| links.get(\"websiteUrl\")).cloned().unwrap_or(Value::Null);""",
    """    let file_name = required_string(file, \"fileName\")?;\n    safe_jar_filename(file_name)?;\n    let display_name = required_string(file, \"displayName\")?;\n    let website_url = project.get(\"links\").and_then(|links| links.get(\"websiteUrl\")).cloned().unwrap_or(Value::Null);""",
)
edit(
    "apps/desktop/src-tauri/src/curseforge.rs",
    """fn validate_download_url(url: &str) -> Result<String, ProviderError> {""",
    r'''fn safe_jar_filename(value: &str) -> Result<(), ProviderError> {
    let path = Path::new(value);
    let stem = value.split('.').next().unwrap_or_default().to_ascii_uppercase();
    let windows_reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'));
    let portable = !value.is_empty()
        && value == value.trim()
        && value.len() <= 255
        && !path.is_absolute()
        && path.components().count() == 1
        && !value.contains(['/', '\\', ':', '\0'])
        && value != "."
        && value != ".."
        && !value.ends_with(['.', ' '])
        && !windows_reserved
        && value.to_ascii_lowercase().ends_with(".jar");
    if portable {
        Ok(())
    } else {
        Err(ProviderError::new(
            "download_failed",
            "unsafe_artifact_filename",
            format!("CurseForge returned an unsafe JAR filename: {value}"),
        ))
    }
}

fn validate_download_url(url: &str) -> Result<String, ProviderError> {''',
)
edit(
    "apps/desktop/src-tauri/src/curseforge.rs",
    """    let file_id = required_u64(file, \"id\")?;\n    let file_name = required_string(file, \"fileName\")?;\n    if !file_name.to_ascii_lowercase().ends_with(\".jar\") {\n        return Err(ProviderError::new(\n            \"download_failed\",\n            \"unsupported_artifact_type\",\n            \"SwarmCraft will not automatically execute or install non-JAR CurseForge artifacts\",\n        ));\n    }\n    if destination\n        .extension()\n        .and_then(|extension| extension.to_str())\n        .map(|extension| !extension.eq_ignore_ascii_case(\"jar\"))\n        .unwrap_or(true)\n    {\n        return Err(ProviderError::new(\n            \"download_failed\",\n            \"invalid_destination\",\n            \"CurseForge mod artifacts must be published to a .jar destination\",\n        ));\n    }""",
    """    let file_id = required_u64(file, \"id\")?;\n    let file_name = required_string(file, \"fileName\")?;\n    safe_jar_filename(file_name)?;""",
)
edit(
    "apps/desktop/src-tauri/src/curseforge.rs",
    """#[tauri::command(rename_all = \"camelCase\")]\npub async fn curseforge_download(file_id: u64, destination: String) -> Value {\n    match async {\n        let destination = destination.trim();\n        if destination.is_empty() {\n            return Err(ProviderError::new(\n                \"invalid_request\",\n                \"destination_required\",\n                \"Artifact destination is required\",\n            ));\n        }\n        let client = CurseForgeClient::from_environment()?;\n        let file = client.fetch_file(file_id).await?;\n        let project_id = required_u64(&file, \"modId\")?;\n        let project = client.fetch_project(project_id).await?;\n        download_artifact(&client, &file, &project, PathBuf::from(destination)).await\n    }\n    .await\n    {\n        Ok(value) => value,\n        Err(error) => error.into_response(),\n    }\n}""",
    """#[tauri::command(rename_all = \"camelCase\")]\npub async fn curseforge_download(file_id: u64, staging_session: String) -> Value {\n    match async {\n        let staging = resolve_provider_staging_session(&staging_session).map_err(|message| {\n            ProviderError::new(\"invalid_request\", \"invalid_staging_session\", message)\n        })?;\n        let client = CurseForgeClient::from_environment()?;\n        let file = client.fetch_file(file_id).await?;\n        let project_id = required_u64(&file, \"modId\")?;\n        let file_name = required_string(&file, \"fileName\")?;\n        safe_jar_filename(file_name)?;\n        let project = client.fetch_project(project_id).await?;\n        let destination = staging\n            .join(\"curseforge\")\n            .join(project_id.to_string())\n            .join(file_id.to_string())\n            .join(file_name);\n        download_artifact(&client, &file, &project, destination).await\n    }\n    .await\n    {\n        Ok(value) => value,\n        Err(error) => error.into_response(),\n    }\n}""",
)
append(
    "apps/desktop/src-tauri/src/curseforge.rs",
    r'''

#[cfg(test)]
mod agent5_security_tests {
    use super::*;

    #[test]
    fn provider_filename_traversal_matrix_fails_closed() {
        for invalid in [
            "../evil.jar",
            "..\\evil.jar",
            "/tmp/evil.jar",
            "C:\\tmp\\evil.jar",
            "\\\\server\\share\\evil.jar",
            ".jar",
            "CON.jar",
            "NUL.jar",
            "evil.jar.",
            "evil.jar ",
            "dir/evil.jar",
            "dir\\evil.jar",
        ] {
            assert!(safe_jar_filename(invalid).is_err(), "accepted {invalid}");
        }
        assert!(safe_jar_filename("safe-mod_1.2.3.jar").is_ok());
    }
}
''',
)

# Frontend sends the opaque session token to native code. Canonical retrieval only promises automatic reacquisition with a strong provider hash.
edit(
    "apps/desktop/src/launcher-controller.js",
    """  return {\n    artifactId: clean(inspection.mod_id ?? inspection.modId),""",
    """  const hashes = providerHashes(downloaded.hashes ?? file.hashes);\n  const hasStrongProviderHash = hashes.some((hash) => ['sha512', 'sha256', 'sha1'].includes(hash.algorithm.toLowerCase()));\n  return {\n    artifactId: clean(inspection.mod_id ?? inspection.modId),""",
)
edit(
    "apps/desktop/src/launcher-controller.js",
    """    providerHashes: providerHashes(downloaded.hashes ?? file.hashes),\n    retrieval: 'provider_download',""",
    """    providerHashes: hashes,\n    retrieval: hasStrongProviderHash ? 'provider_download' : 'manual_required',""",
)
edit(
    "apps/desktop/src/launcher-controller.js",
    """  async function prepareModrinth(root, staging) {""",
    """  async function prepareModrinth(root, stagingSession) {""",
)
edit(
    "apps/desktop/src/launcher-controller.js",
    """      const destinationDir = `${staging}/modrinth/${version.project_id}/${version.version_id}`;\n      const downloaded = await call('modrinth_download', {\n        request: { locator: file.locator, destination_dir: destinationDir, max_bytes: null },\n      });""",
    """      const downloaded = await call('modrinth_download', {\n        locator: file.locator,\n        stagingSession,\n        maxBytes: null,\n      });""",
)
edit(
    "apps/desktop/src/launcher-controller.js",
    """  async function prepareCurseForge(root, staging) {""",
    """  async function prepareCurseForge(root, stagingSession) {""",
)
edit(
    "apps/desktop/src/launcher-controller.js",
    """      const fileName = clean(version.file_name);\n      const destination = `${staging}/curseforge/${version.project_id}/${version.version_id}/${fileName}`;\n      const downloadedEnvelope = await call('curseforge_download', {\n        fileId: Number(version.file_id),\n        destination,\n      });""",
    """      const downloadedEnvelope = await call('curseforge_download', {\n        fileId: Number(version.file_id),\n        stagingSession,\n      });""",
)
edit(
    "apps/desktop/src/launcher-controller.js",
    """        const staging = await call('provider_staging_dir');\n        const packages = [];\n        for (const root of [...roots].sort((a, b) => `${a.provider}:${a.projectId}`.localeCompare(`${b.provider}:${b.projectId}`))) {\n          packages.push(...(await (root.provider === 'modrinth' ? prepareModrinth(root, staging) : prepareCurseForge(root, staging))));""",
    """        const stagingSession = await call('provider_staging_dir');\n        const packages = [];\n        for (const root of [...roots].sort((a, b) => `${a.provider}:${a.projectId}`.localeCompare(`${b.provider}:${b.projectId}`))) {\n          packages.push(...(await (root.provider === 'modrinth' ? prepareModrinth(root, stagingSession) : prepareCurseForge(root, stagingSession))));""",
)
append(
    "apps/desktop/tests/launcher-controller.test.mjs",
    r'''

test('MD5-only provider provenance is canonicalized as manual-required', () => {
  const result = canonicalPackageFromDownloaded({
    provider: 'curseforge',
    version: { project_id: '1', version_id: '2', dependencies: [] },
    file: { file_name: 'x.jar', hashes: [{ algorithm: 'md5', value: 'ab'.repeat(16) }] },
    downloaded: { destination: '/tmp/x.jar', bytes: 1 },
    inspection: { mod_id: 'x', version: '1', environment: 'server' },
    selectedByProject: new Map(),
  });
  assert.equal(result.retrieval, 'manual_required');
});
''',
)

# Canonical provider-download is a reproducibility promise, so MD5-only provenance cannot use it.
edit(
    "apps/desktop/src-tauri/src/canonical_commands.rs",
    """            let retrieval =\n                parse_retrieval(request.retrieval.as_deref().unwrap_or(\"provider_download\")).map_err(|message| {\n                    CanonicalizationFailure::for_artifact(\"invalid_retrieval_state\", &request.artifact_id, message)\n                })?;""",
    """            let retrieval =\n                parse_retrieval(request.retrieval.as_deref().unwrap_or(\"provider_download\")).map_err(|message| {\n                    CanonicalizationFailure::for_artifact(\"invalid_retrieval_state\", &request.artifact_id, message)\n                })?;\n            if retrieval == CanonicalRetrievalV1::ProviderDownload\n                && !hashes.iter().any(|hash| {\n                    matches!(\n                        hash.algorithm,\n                        CanonicalHashAlgorithmV1::Sha512\n                            | CanonicalHashAlgorithmV1::Sha256\n                            | CanonicalHashAlgorithmV1::Sha1\n                    )\n                })\n            {\n                return Err(CanonicalizationFailure::for_artifact(\n                    \"provider_download_requires_strong_hash\",\n                    &request.artifact_id,\n                    \"automatic provider reacquisition requires SHA-1, SHA-256, or SHA-512 provenance; MD5-only artifacts must be recorded as manual_required\",\n                ));\n            }""",
)

edit(
    "crates/swarm-protocol/src/canonical_modpack.rs",
    """    for dependency in &artifact.dependencies {""",
    """    if artifact.retrieval == CanonicalRetrievalV1::ProviderDownload\n        && !algorithms.keys().any(|algorithm| {\n            matches!(\n                algorithm,\n                CanonicalHashAlgorithmV1::Sha512\n                    | CanonicalHashAlgorithmV1::Sha256\n                    | CanonicalHashAlgorithmV1::Sha1\n            )\n        })\n    {\n        return Err(CanonicalModpackError::InvalidProviderHash(format!(\n            \"{} is marked provider_download but has only MD5/unsupported reacquisition proof\",\n            artifact.identity.display_key()\n        )));\n    }\n    for dependency in &artifact.dependencies {""",
)
append(
    "crates/swarm-protocol/src/canonical_modpack.rs",
    r'''

#[cfg(test)]
mod agent5_supply_chain_tests {
    use super::*;

    #[test]
    fn md5_only_provider_download_is_not_a_valid_reacquisition_contract() {
        let artifact = CanonicalProviderArtifactV1 {
            identity: CanonicalPackageIdentityV1 {
                provider: CanonicalProviderV1::CurseForge,
                project_id: "1".into(),
                version_id: "2".into(),
            },
            file_name: "safe.jar".into(),
            file_size: Some(1),
            hashes: vec![CanonicalProviderHashV1 {
                algorithm: CanonicalHashAlgorithmV1::Md5,
                digest_hex: "ab".repeat(16),
            }],
            retrieval: CanonicalRetrievalV1::ProviderDownload,
            dependencies: vec![],
        };
        assert!(matches!(
            validate_provider_artifact(&artifact),
            Err(CanonicalModpackError::InvalidProviderHash(_))
        ));
        let mut manual = artifact;
        manual.retrieval = CanonicalRetrievalV1::ManualRequired;
        assert!(validate_provider_artifact(&manual).is_ok());
    }
}
''',
)

print("Agent 5 milestone 1 patch applied")
