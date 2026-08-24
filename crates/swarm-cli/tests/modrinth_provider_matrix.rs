use serde_json::json;
use sha1::Sha1;
use sha2::{Digest, Sha512};
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    path::Path,
    sync::{Arc, Mutex},
};
use swarm_cli::package_provider::{
    modrinth::{HttpResponse, ModrinthClient, ModrinthTransport},
    ArtifactRetrieval, DependencyKind, ModArtifactLocator, ModDownloadRequest, ModResolveRequest, PackageEnvironment,
    ProviderFailure, ProviderFailureKind, ProviderId, ReleaseType,
};

#[derive(Clone, Default)]
struct FixtureTransport {
    responses: Arc<Mutex<VecDeque<Result<HttpResponse, ProviderFailure>>>>,
    downloads: Arc<Mutex<VecDeque<Result<Vec<u8>, ProviderFailure>>>>,
    download_calls: Arc<Mutex<usize>>,
}

impl FixtureTransport {
    fn json(&self, value: serde_json::Value) {
        self.responses.lock().unwrap().push_back(Ok(HttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: serde_json::to_vec(&value).unwrap(),
        }));
    }

    fn download(&self, bytes: &[u8]) {
        self.downloads.lock().unwrap().push_back(Ok(bytes.to_vec()));
    }
}

impl ModrinthTransport for FixtureTransport {
    fn get(&self, _url: &str) -> Result<HttpResponse, ProviderFailure> {
        self.responses.lock().unwrap().pop_front().expect("fixture response queue exhausted")
    }

    fn download(&self, _url: &str, destination: &Path, _max_bytes: u64) -> Result<(), ProviderFailure> {
        *self.download_calls.lock().unwrap() += 1;
        match self.downloads.lock().unwrap().pop_front().expect("fixture download queue exhausted") {
            Ok(bytes) => {
                fs::write(destination, bytes).unwrap();
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
}

fn client(transport: FixtureTransport) -> ModrinthClient<FixtureTransport> {
    ModrinthClient::with_transport("https://fixtures.invalid/v2", transport).unwrap()
}

fn version(
    id: &str,
    project_id: &str,
    dependencies: serde_json::Value,
    filename: &str,
    bytes: &[u8],
    declared_size: usize,
) -> serde_json::Value {
    json!({
        "name": format!("Version {id}"),
        "version_number": id,
        "dependencies": dependencies,
        "game_versions": ["1.21.1"],
        "version_type": "release",
        "loaders": ["fabric"],
        "id": id,
        "project_id": project_id,
        "date_published": "2026-08-24T12:00:00Z",
        "environment": "server_only",
        "files": [{
            "hashes": {
                "sha1": sha1(bytes),
                "sha512": sha512(bytes)
            },
            "url": format!("https://cdn.modrinth.com/data/{project_id}/versions/{id}/{filename}"),
            "filename": filename,
            "primary": true,
            "size": declared_size,
            "file_type": null
        }]
    })
}

fn resolve_request(root: &str) -> ModResolveRequest {
    ModResolveRequest {
        root_version_id: root.into(),
        minecraft_version: "1.21.1".into(),
        loader: "fabric".into(),
        environment: PackageEnvironment::Server,
        allowed_release_types: vec![ReleaseType::Release],
    }
}

fn locator(project_id: &str, version_id: &str, bytes: &[u8]) -> ModArtifactLocator {
    ModArtifactLocator {
        provider: ProviderId::Modrinth,
        project_id: project_id.into(),
        version_id: version_id.into(),
        sha1: Some(sha1(bytes)),
        sha512: Some(sha512(bytes)),
    }
}

fn sha1(bytes: &[u8]) -> String {
    hex(&Sha1::digest(bytes))
}

fn sha512(bytes: &[u8]) -> String {
    hex(&Sha512::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[test]
fn resolver_includes_required_dependencies_and_preserves_optional_edges() {
    let transport = FixtureTransport::default();
    transport.json(version(
        "ROOT",
        "P_ROOT",
        json!([
            {
                "version_id": "DEP",
                "project_id": "P_DEP",
                "file_name": null,
                "dependency_type": "required"
            },
            {
                "version_id": null,
                "project_id": "P_OPTIONAL",
                "file_name": null,
                "dependency_type": "optional"
            }
        ]),
        "root.jar",
        b"root",
        4,
    ));
    transport.json(version("DEP", "P_DEP", json!([]), "dep.jar", b"dep", 3));

    let graph = client(transport).resolve(&resolve_request("ROOT")).unwrap();

    assert_eq!(graph.versions.len(), 2);
    assert!(graph.versions.iter().any(|item| item.version_id == "ROOT"));
    assert!(graph.versions.iter().any(|item| item.version_id == "DEP"));
    assert_eq!(graph.optional_dependencies.len(), 1);
    assert_eq!(graph.optional_dependencies[0].kind, DependencyKind::Optional);
    assert_eq!(graph.optional_dependencies[0].project_id.as_deref(), Some("P_OPTIONAL"));
}

#[test]
fn resolver_rejects_selected_versions_marked_incompatible() {
    let transport = FixtureTransport::default();
    transport.json(version(
        "ROOT",
        "P_ROOT",
        json!([
            {
                "version_id": "DEP",
                "project_id": "P_DEP",
                "file_name": null,
                "dependency_type": "required"
            },
            {
                "version_id": null,
                "project_id": "P_DEP",
                "file_name": null,
                "dependency_type": "incompatible"
            }
        ]),
        "root.jar",
        b"root",
        4,
    ));
    transport.json(version("DEP", "P_DEP", json!([]), "dep.jar", b"dep", 3));

    let error = client(transport).resolve(&resolve_request("ROOT")).unwrap_err();
    assert_eq!(error.kind, ProviderFailureKind::Incompatible);
}

#[test]
fn oversized_artifact_is_rejected_before_any_download_or_partial_file() {
    let transport = FixtureTransport::default();
    transport.json(version("V1", "P1", json!([]), "V1.jar", b"artifact", 4096));
    let directory = tempfile::tempdir().unwrap();
    let request = ModDownloadRequest {
        locator: locator("P1", "V1", b"artifact"),
        destination_dir: directory.path().to_path_buf(),
        max_bytes: Some(1024),
    };

    let error = client(transport.clone()).download(&request).unwrap_err();

    assert_eq!(error.kind, ProviderFailureKind::RetrievalRestricted);
    assert_eq!(*transport.download_calls.lock().unwrap(), 0);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn artifact_download_requires_exact_provider_project_version_and_hash_identity() {
    let transport = FixtureTransport::default();
    transport.json(version("V1", "P1", json!([]), "V1.jar", b"artifact", 8));
    let directory = tempfile::tempdir().unwrap();
    let request = ModDownloadRequest {
        locator: locator("P1", "V1", b"different"),
        destination_dir: directory.path().to_path_buf(),
        max_bytes: Some(1024),
    };

    let error = client(transport.clone()).download(&request).unwrap_err();

    assert_eq!(error.kind, ProviderFailureKind::NotFound);
    assert_eq!(*transport.download_calls.lock().unwrap(), 0);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn non_installable_provider_file_stays_manual_required_and_fails_closed() {
    let transport = FixtureTransport::default();
    let raw = version("V1", "P1", json!([]), "V1.zip", b"artifact", 8);
    transport.json(raw.clone());
    transport.json(raw);

    let mod_version = client(transport.clone()).version("V1").unwrap();
    assert!(matches!(&mod_version.files[0].retrieval, ArtifactRetrieval::ManualRequired { .. }));

    let directory = tempfile::tempdir().unwrap();
    let request = ModDownloadRequest {
        locator: locator("P1", "V1", b"artifact"),
        destination_dir: directory.path().to_path_buf(),
        max_bytes: Some(1024),
    };
    let error = client(transport.clone()).download(&request).unwrap_err();

    assert_eq!(error.kind, ProviderFailureKind::RetrievalRestricted);
    assert!(error.remediation.is_some());
    assert_eq!(*transport.download_calls.lock().unwrap(), 0);
}

#[test]
fn provider_failures_serialize_as_structured_tauri_errors() {
    let error = ProviderFailure::new(ProviderFailureKind::RateLimited, "rate limited")
        .with_retry_after(Some(42))
        .with_remediation("retry later")
        .with_detail("scope", "search");

    let value = serde_json::to_value(error).unwrap();

    assert_eq!(value["provider"], "modrinth");
    assert_eq!(value["kind"], "rate_limited");
    assert_eq!(value["message"], "rate limited");
    assert_eq!(value["retry_after_seconds"], 42);
    assert_eq!(value["remediation"], "retry later");
    assert_eq!(value["details"]["scope"], "search");
}
