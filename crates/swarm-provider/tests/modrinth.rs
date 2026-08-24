use serde_json::json;
use sha1::Sha1;
use sha2::{Digest, Sha512};
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    path::Path,
    sync::{Arc, Mutex},
};
use swarm_provider::{
    modrinth::{HttpResponse, ModrinthClient, ModrinthTransport},
    DependencyKind, ModDownloadRequest, ModEnvironment, ModResolveRequest, ModSearchQuery, ModVersionFilter,
    PackageEnvironment, ProviderFailure, ProviderFailureKind, ProviderId, ReleaseType,
};

#[derive(Clone, Default)]
struct FixtureTransport {
    responses: Arc<Mutex<VecDeque<Result<HttpResponse, ProviderFailure>>>>,
    downloads: Arc<Mutex<VecDeque<Result<Vec<u8>, ProviderFailure>>>>,
    requested_urls: Arc<Mutex<Vec<String>>>,
}

impl FixtureTransport {
    fn push_json(&self, status: u16, value: serde_json::Value) {
        self.responses.lock().unwrap().push_back(Ok(HttpResponse {
            status,
            headers: BTreeMap::new(),
            body: serde_json::to_vec(&value).unwrap(),
        }));
    }

    fn push_raw(&self, status: u16, headers: BTreeMap<String, String>, body: &[u8]) {
        self.responses.lock().unwrap().push_back(Ok(HttpResponse { status, headers, body: body.to_vec() }));
    }

    fn push_error(&self, error: ProviderFailure) {
        self.responses.lock().unwrap().push_back(Err(error));
    }

    fn push_download(&self, bytes: &[u8]) {
        self.downloads.lock().unwrap().push_back(Ok(bytes.to_vec()));
    }

    fn push_download_error(&self, error: ProviderFailure) {
        self.downloads.lock().unwrap().push_back(Err(error));
    }
}

impl ModrinthTransport for FixtureTransport {
    fn get(&self, url: &str) -> Result<HttpResponse, ProviderFailure> {
        self.requested_urls.lock().unwrap().push(url.to_owned());
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("fixture response queue exhausted")
    }

    fn download(&self, _url: &str, destination: &Path, _max_bytes: u64) -> Result<(), ProviderFailure> {
        match self.downloads.lock().unwrap().pop_front().expect("fixture download queue exhausted") {
            Ok(bytes) => {
                fs::write(destination, bytes).unwrap();
                Ok(())
            }
            Err(error) => {
                fs::write(destination, b"partial").unwrap();
                Err(error)
            }
        }
    }
}

fn client(transport: FixtureTransport) -> ModrinthClient<FixtureTransport> {
    ModrinthClient::with_transport("https://fixtures.invalid/v2", transport).unwrap()
}

fn response_headers(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values.iter().map(|(key, value)| ((*key).to_owned(), (*value).to_owned())).collect()
}

fn version_json(
    id: &str,
    project_id: &str,
    environment: &str,
    game_versions: &[&str],
    loaders: &[&str],
    dependencies: serde_json::Value,
    bytes: &[u8],
) -> serde_json::Value {
    let sha1 = hash_sha1(bytes);
    let sha512 = hash_sha512(bytes);
    json!({
        "name": format!("Version {id}"),
        "version_number": id,
        "dependencies": dependencies,
        "game_versions": game_versions,
        "version_type": "release",
        "loaders": loaders,
        "id": id,
        "project_id": project_id,
        "date_published": "2026-08-24T12:00:00Z",
        "environment": environment,
        "files": [{
            "hashes": { "sha1": sha1, "sha512": sha512 },
            "url": format!("https://cdn.modrinth.com/data/{project_id}/versions/{id}/{id}.jar"),
            "filename": format!("{id}.jar"),
            "primary": true,
            "size": bytes.len(),
            "file_type": null
        }]
    })
}

fn hash_sha1(bytes: &[u8]) -> String {
    hex_lower(&Sha1::digest(bytes))
}

fn hash_sha512(bytes: &[u8]) -> String {
    hex_lower(&Sha512::digest(bytes))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[test]
fn search_parses_results_pagination_rate_limit_and_server_facets() {
    let transport = FixtureTransport::default();
    transport.push_raw(
        200,
        response_headers(&[
            ("x-ratelimit-limit", "300"),
            ("x-ratelimit-remaining", "299"),
            ("x-ratelimit-reset", "42"),
        ]),
        serde_json::to_vec(&json!({
            "hits": [{
                "project_id": "P1",
                "project_type": "mod",
                "slug": "example-mod",
                "title": "Example Mod",
                "description": "Example",
                "categories": ["fabric", "utility"],
                "icon_url": "https://cdn.modrinth.com/data/P1/icon.png"
            }],
            "offset": 20,
            "limit": 10,
            "total_hits": 101
        }))
        .unwrap()
        .as_slice(),
    );
    let result = client(transport.clone())
        .search(&ModSearchQuery {
            query: "example".into(),
            minecraft_version: "1.21.1".into(),
            loader: "fabric".into(),
            environment: PackageEnvironment::Server,
            release_type: Some(ReleaseType::Release),
            offset: 20,
            limit: 10,
        })
        .unwrap();
    assert_eq!(result.offset, 20);
    assert_eq!(result.limit, 10);
    assert_eq!(result.total_hits, 101);
    assert_eq!(result.items[0].provider, ProviderId::Modrinth);
    assert_eq!(result.rate_limit.unwrap().remaining, Some(299));
    let url = &transport.requested_urls.lock().unwrap()[0];
    assert!(url.contains("offset=20"));
    assert!(url.contains("limit=10"));
    assert!(url.contains("environment%3Aserver_only"));
    assert!(url.contains("categories%3Afabric"));
    assert!(url.contains("versions%3A1.21.1"));
}

#[test]
fn versions_filter_exact_minecraft_fabric_environment_and_release_type() {
    let transport = FixtureTransport::default();
    transport.push_json(
        200,
        json!([
            version_json("server", "P1", "server_only", &["1.21.1"], &["fabric"], json!([]), b"server"),
            version_json("client", "P1", "client_only", &["1.21.1"], &["fabric"], json!([]), b"client"),
            version_json("wrong-mc", "P1", "server_only", &["1.20.1"], &["fabric"], json!([]), b"wrong"),
            version_json("wrong-loader", "P1", "server_only", &["1.21.1"], &["forge"], json!([]), b"wrong")
        ]),
    );
    let result = client(transport)
        .versions(
            "P1",
            &ModVersionFilter {
                minecraft_version: "1.21.1".into(),
                loader: "fabric".into(),
                environment: PackageEnvironment::Server,
                release_type: Some(ReleaseType::Release),
            },
        )
        .unwrap();
    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].version_id, "server");
    assert_eq!(result.items[0].environment, ModEnvironment::ServerOnly);
}

#[test]
fn version_preserves_required_optional_incompatible_and_embedded_dependencies() {
    let transport = FixtureTransport::default();
    transport.push_json(
        200,
        version_json(
            "root",
            "P1",
            "client_and_server",
            &["1.21.1"],
            &["fabric"],
            json!([
                {"version_id":"V2","project_id":"P2","file_name":null,"dependency_type":"required"},
                {"version_id":null,"project_id":"P3","file_name":null,"dependency_type":"optional"},
                {"version_id":null,"project_id":"P4","file_name":null,"dependency_type":"incompatible"},
                {"version_id":null,"project_id":"P5","file_name":"embedded.jar","dependency_type":"embedded"}
            ]),
            b"root",
        ),
    );
    let version = client(transport).version("root").unwrap();
    assert_eq!(version.dependencies.len(), 4);
    assert_eq!(version.dependencies[0].kind, DependencyKind::Required);
    assert_eq!(version.dependencies[1].kind, DependencyKind::Optional);
    assert_eq!(version.dependencies[2].kind, DependencyKind::Incompatible);
    assert_eq!(version.dependencies[3].kind, DependencyKind::Embedded);
}

#[test]
fn malformed_json_is_not_converted_to_empty_success() {
    let transport = FixtureTransport::default();
    transport.push_raw(200, BTreeMap::new(), b"{not-json");
    let error = client(transport).project("P1").unwrap_err();
    assert_eq!(error.kind, ProviderFailureKind::MalformedResponse);
}

#[test]
fn not_found_and_rate_limit_are_structured() {
    let transport = FixtureTransport::default();
    transport.push_raw(404, BTreeMap::new(), b"{}");
    let error = client(transport.clone()).project("removed").unwrap_err();
    assert_eq!(error.kind, ProviderFailureKind::NotFound);

    transport.push_raw(
        429,
        response_headers(&[("x-ratelimit-reset", "17")]),
        b"{}",
    );
    let error = client(transport).project("limited").unwrap_err();
    assert_eq!(error.kind, ProviderFailureKind::RateLimited);
    assert_eq!(error.retry_after_seconds, Some(17));
}

#[test]
fn provider_unavailable_is_preserved() {
    let transport = FixtureTransport::default();
    transport.push_error(ProviderFailure::new(ProviderFailureKind::Unavailable, "offline"));
    let error = client(transport).project("P1").unwrap_err();
    assert_eq!(error.kind, ProviderFailureKind::Unavailable);
}

#[test]
fn resolver_detects_dependency_cycles() {
    let transport = FixtureTransport::default();
    transport.push_json(
        200,
        version_json(
            "A",
            "PA",
            "server_only",
            &["1.21.1"],
            &["fabric"],
            json!([{"version_id":"B","project_id":"PB","file_name":null,"dependency_type":"required"}]),
            b"A",
        ),
    );
    transport.push_json(
        200,
        version_json(
            "B",
            "PB",
            "server_only",
            &["1.21.1"],
            &["fabric"],
            json!([{"version_id":"A","project_id":"PA","file_name":null,"dependency_type":"required"}]),
            b"B",
        ),
    );
    transport.push_json(
        200,
        version_json("A", "PA", "server_only", &["1.21.1"], &["fabric"], json!([]), b"A"),
    );
    let error = client(transport)
        .resolve(&ModResolveRequest {
            root_version_id: "A".into(),
            minecraft_version: "1.21.1".into(),
            loader: "fabric".into(),
            environment: PackageEnvironment::Server,
            allowed_release_types: vec![ReleaseType::Release],
        })
        .unwrap_err();
    assert_eq!(error.kind, ProviderFailureKind::DependencyCycle);
    assert!(error.details.get("cycle").unwrap().contains("A -> B -> A"));
}

#[test]
fn resolver_reports_unresolved_required_dependency() {
    let transport = FixtureTransport::default();
    transport.push_json(
        200,
        version_json(
            "root",
            "P1",
            "server_only",
            &["1.21.1"],
            &["fabric"],
            json!([{"version_id":null,"project_id":"MISSING","file_name":null,"dependency_type":"required"}]),
            b"root",
        ),
    );
    transport.push_json(200, json!([]));
    let error = client(transport)
        .resolve(&ModResolveRequest {
            root_version_id: "root".into(),
            minecraft_version: "1.21.1".into(),
            loader: "fabric".into(),
            environment: PackageEnvironment::Server,
            allowed_release_types: vec![],
        })
        .unwrap_err();
    assert_eq!(error.kind, ProviderFailureKind::UnresolvedDependency);
}

#[test]
fn resolver_rejects_two_required_versions_of_same_project() {
    let transport = FixtureTransport::default();
    transport.push_json(
        200,
        version_json(
            "root",
            "P1",
            "server_only",
            &["1.21.1"],
            &["fabric"],
            json!([
                {"version_id":"D1","project_id":"PD","file_name":null,"dependency_type":"required"},
                {"version_id":"D2","project_id":"PD","file_name":null,"dependency_type":"required"}
            ]),
            b"root",
        ),
    );
    transport.push_json(200, version_json("D1", "PD", "server_only", &["1.21.1"], &["fabric"], json!([]), b"D1"));
    transport.push_json(200, version_json("D2", "PD", "server_only", &["1.21.1"], &["fabric"], json!([]), b"D2"));
    let error = client(transport)
        .resolve(&ModResolveRequest {
            root_version_id: "root".into(),
            minecraft_version: "1.21.1".into(),
            loader: "fabric".into(),
            environment: PackageEnvironment::Server,
            allowed_release_types: vec![],
        })
        .unwrap_err();
    assert_eq!(error.kind, ProviderFailureKind::Incompatible);
}

#[test]
fn hash_mismatch_removes_temporary_file_and_preserves_existing_destination() {
    let transport = FixtureTransport::default();
    transport.push_json(200, version_json("V1", "P1", "server_only", &["1.21.1"], &["fabric"], json!([]), b"good"));
    transport.push_download(b"evil");
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("V1.jar");
    fs::write(&destination, b"old").unwrap();
    let request = ModDownloadRequest {
        locator: swarm_provider::ModArtifactLocator {
            provider: ProviderId::Modrinth,
            project_id: "P1".into(),
            version_id: "V1".into(),
            sha1: Some(hash_sha1(b"good")),
            sha512: Some(hash_sha512(b"good")),
        },
        destination_dir: directory.path().to_path_buf(),
        max_bytes: Some(1024),
    };
    let error = client(transport).download(&request).unwrap_err();
    assert!(matches!(error.kind, ProviderFailureKind::DownloadInterrupted | ProviderFailureKind::HashMismatch));
    assert_eq!(fs::read(&destination).unwrap(), b"old");
    assert!(fs::read_dir(directory.path())
        .unwrap()
        .all(|entry| !entry.unwrap().file_name().to_string_lossy().contains(".part-")));
}

#[test]
fn interrupted_download_never_publishes_partial_artifact() {
    let transport = FixtureTransport::default();
    transport.push_json(200, version_json("V1", "P1", "server_only", &["1.21.1"], &["fabric"], json!([]), b"good"));
    transport.push_download_error(ProviderFailure::new(ProviderFailureKind::DownloadInterrupted, "connection reset"));
    let directory = tempfile::tempdir().unwrap();
    let request = ModDownloadRequest {
        locator: swarm_provider::ModArtifactLocator {
            provider: ProviderId::Modrinth,
            project_id: "P1".into(),
            version_id: "V1".into(),
            sha1: Some(hash_sha1(b"good")),
            sha512: Some(hash_sha512(b"good")),
        },
        destination_dir: directory.path().to_path_buf(),
        max_bytes: Some(1024),
    };
    let error = client(transport).download(&request).unwrap_err();
    assert_eq!(error.kind, ProviderFailureKind::DownloadInterrupted);
    assert!(!directory.path().join("V1.jar").exists());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn safe_publication_replaces_only_after_hash_verification_and_records_local_sha256() {
    let transport = FixtureTransport::default();
    let bytes = b"verified mod bytes";
    transport.push_json(200, version_json("V1", "P1", "server_only", &["1.21.1"], &["fabric"], json!([]), bytes));
    transport.push_download(bytes);
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("V1.jar");
    fs::write(&destination, b"old").unwrap();
    let request = ModDownloadRequest {
        locator: swarm_provider::ModArtifactLocator {
            provider: ProviderId::Modrinth,
            project_id: "P1".into(),
            version_id: "V1".into(),
            sha1: Some(hash_sha1(bytes)),
            sha512: Some(hash_sha512(bytes)),
        },
        destination_dir: directory.path().to_path_buf(),
        max_bytes: Some(1024),
    };
    let result = client(transport).download(&request).unwrap();
    assert_eq!(fs::read(&destination).unwrap(), bytes);
    assert_eq!(result.path, destination);
    assert!(result.hashes.sha256.is_some());
    let names: Vec<_> = fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["V1.jar"]);
}

#[test]
fn exact_version_rejects_client_only_for_server_resolution() {
    let transport = FixtureTransport::default();
    transport.push_json(200, version_json("V1", "P1", "client_only", &["1.21.1"], &["fabric"], json!([]), b"client"));
    let error = client(transport)
        .resolve(&ModResolveRequest {
            root_version_id: "V1".into(),
            minecraft_version: "1.21.1".into(),
            loader: "fabric".into(),
            environment: PackageEnvironment::Server,
            allowed_release_types: vec![],
        })
        .unwrap_err();
    assert_eq!(error.kind, ProviderFailureKind::Incompatible);
}

#[test]
#[ignore = "requires SWARMCRAFT_LIVE_MODRINTH=1 and live Modrinth availability"]
fn optional_live_provider_validation() {
    if std::env::var("SWARMCRAFT_LIVE_MODRINTH").as_deref() != Ok("1") {
        return;
    }
    let result = ModrinthClient::production()
        .unwrap()
        .search(&ModSearchQuery {
            query: "fabric api".into(),
            minecraft_version: "1.21.1".into(),
            loader: "fabric".into(),
            environment: PackageEnvironment::Server,
            release_type: None,
            offset: 0,
            limit: 5,
        })
        .unwrap();
    assert!(result.limit > 0);
}
