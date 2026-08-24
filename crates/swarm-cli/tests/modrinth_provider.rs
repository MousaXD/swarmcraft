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
    DependencyKind, ModArtifactLocator, ModDownloadRequest, ModEnvironment, ModResolveRequest, ModSearchQuery,
    ModVersionFilter, PackageEnvironment, ProviderFailure, ProviderFailureKind, ProviderId, ReleaseType,
};

#[derive(Clone, Default)]
struct FixtureTransport {
    responses: Arc<Mutex<VecDeque<Result<HttpResponse, ProviderFailure>>>>,
    downloads: Arc<Mutex<VecDeque<Result<Vec<u8>, ProviderFailure>>>>,
    requested_urls: Arc<Mutex<Vec<String>>>,
}

impl FixtureTransport {
    fn json(&self, status: u16, value: serde_json::Value) {
        self.raw(status, BTreeMap::new(), serde_json::to_vec(&value).unwrap());
    }

    fn raw(&self, status: u16, headers: BTreeMap<String, String>, body: impl Into<Vec<u8>>) {
        self.responses.lock().unwrap().push_back(Ok(HttpResponse {
            status,
            headers,
            body: body.into(),
        }));
    }

    fn get_error(&self, kind: ProviderFailureKind) {
        self.responses
            .lock()
            .unwrap()
            .push_back(Err(ProviderFailure::new(kind, "fixture failure")));
    }

    fn download(&self, bytes: &[u8]) {
        self.downloads.lock().unwrap().push_back(Ok(bytes.to_vec()));
    }

    fn download_error(&self) {
        self.downloads.lock().unwrap().push_back(Err(ProviderFailure::new(
            ProviderFailureKind::DownloadInterrupted,
            "fixture connection reset",
        )));
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
        match self
            .downloads
            .lock()
            .unwrap()
            .pop_front()
            .expect("fixture download queue exhausted")
        {
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

fn headers(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn version(
    id: &str,
    project_id: &str,
    environment: &str,
    game_versions: &[&str],
    loaders: &[&str],
    dependencies: serde_json::Value,
    bytes: &[u8],
) -> serde_json::Value {
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
            "hashes": { "sha1": sha1(bytes), "sha512": sha512(bytes) },
            "url": format!("https://cdn.modrinth.com/data/{project_id}/versions/{id}/{id}.jar"),
            "filename": format!("{id}.jar"),
            "primary": true,
            "size": bytes.len(),
            "file_type": null
        }]
    })
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

fn server_filter() -> ModVersionFilter {
    ModVersionFilter {
        minecraft_version: "1.21.1".into(),
        loader: "fabric".into(),
        environment: PackageEnvironment::Server,
        release_type: Some(ReleaseType::Release),
    }
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

#[test]
fn search_parses_pagination_rate_limits_and_backend_compatibility_facets() {
    let transport = FixtureTransport::default();
    transport.raw(
        200,
        headers(&[
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
                "icon_url": null
            }],
            "offset": 20,
            "limit": 10,
            "total_hits": 101
        }))
        .unwrap(),
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
    assert!(url.contains("categories%3Afabric"));
    assert!(url.contains("versions%3A1.21.1"));
    assert!(url.contains("environment%3Aserver_only"));
}

#[test]
fn versions_keep_only_exact_minecraft_fabric_server_compatible_releases() {
    let transport = FixtureTransport::default();
    transport.json(
        200,
        json!([
            version("server", "P1", "server_only", &["1.21.1"], &["fabric"], json!([]), b"server"),
            version("client", "P1", "client_only", &["1.21.1"], &["fabric"], json!([]), b"client"),
            version("wrong-mc", "P1", "server_only", &["1.20.1"], &["fabric"], json!([]), b"wrong"),
            version("wrong-loader", "P1", "server_only", &["1.21.1"], &["forge"], json!([]), b"wrong")
        ]),
    );

    let result = client(transport).versions("P1", &server_filter()).unwrap();
    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].version_id, "server");
    assert_eq!(result.items[0].environment, ModEnvironment::ServerOnly);
}

#[test]
fn dependency_metadata_keeps_required_optional_incompatible_and_embedded_distinct() {
    let transport = FixtureTransport::default();
    transport.json(
        200,
        version(
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

    let dependencies = client(transport).version("root").unwrap().dependencies;
    assert_eq!(dependencies[0].kind, DependencyKind::Required);
    assert_eq!(dependencies[1].kind, DependencyKind::Optional);
    assert_eq!(dependencies[2].kind, DependencyKind::Incompatible);
    assert_eq!(dependencies[3].kind, DependencyKind::Embedded);
}

#[test]
fn malformed_404_rate_limit_and_unavailable_fail_structurally() {
    let malformed = FixtureTransport::default();
    malformed.raw(200, BTreeMap::new(), b"{not-json".to_vec());
    assert_eq!(
        client(malformed).project("P1").unwrap_err().kind,
        ProviderFailureKind::MalformedResponse
    );

    let missing = FixtureTransport::default();
    missing.raw(404, BTreeMap::new(), b"{}".to_vec());
    assert_eq!(client(missing).project("P1").unwrap_err().kind, ProviderFailureKind::NotFound);

    let limited = FixtureTransport::default();
    limited.raw(429, headers(&[("x-ratelimit-reset", "17")]), b"{}".to_vec());
    let error = client(limited).project("P1").unwrap_err();
    assert_eq!(error.kind, ProviderFailureKind::RateLimited);
    assert_eq!(error.retry_after_seconds, Some(17));

    let unavailable = FixtureTransport::default();
    unavailable.get_error(ProviderFailureKind::Unavailable);
    assert_eq!(
        client(unavailable).project("P1").unwrap_err().kind,
        ProviderFailureKind::Unavailable
    );
}

#[test]
fn resolver_detects_cycles_and_does_not_promote_optional_dependencies() {
    let transport = FixtureTransport::default();
    transport.json(
        200,
        version(
            "A",
            "PA",
            "server_only",
            &["1.21.1"],
            &["fabric"],
            json!([
                {"version_id":"B","project_id":"PB","file_name":null,"dependency_type":"required"},
                {"version_id":null,"project_id":"OPTIONAL","file_name":null,"dependency_type":"optional"}
            ]),
            b"A",
        ),
    );
    transport.json(
        200,
        version(
            "B",
            "PB",
            "server_only",
            &["1.21.1"],
            &["fabric"],
            json!([{"version_id":"A","project_id":"PA","file_name":null,"dependency_type":"required"}]),
            b"B",
        ),
    );
    transport.json(200, version("A", "PA", "server_only", &["1.21.1"], &["fabric"], json!([]), b"A"));

    let error = client(transport).resolve(&resolve_request("A")).unwrap_err();
    assert_eq!(error.kind, ProviderFailureKind::DependencyCycle);
    assert!(error.details.get("cycle").unwrap().contains("A -> B -> A"));
}

#[test]
fn resolver_reports_unresolved_or_conflicting_required_versions() {
    let missing = FixtureTransport::default();
    missing.json(
        200,
        version(
            "root",
            "P1",
            "server_only",
            &["1.21.1"],
            &["fabric"],
            json!([{"version_id":null,"project_id":"MISSING","file_name":null,"dependency_type":"required"}]),
            b"root",
        ),
    );
    missing.json(200, json!([]));
    assert_eq!(
        client(missing).resolve(&resolve_request("root")).unwrap_err().kind,
        ProviderFailureKind::UnresolvedDependency
    );

    let conflicting = FixtureTransport::default();
    conflicting.json(
        200,
        version(
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
    conflicting.json(200, version("D1", "PD", "server_only", &["1.21.1"], &["fabric"], json!([]), b"D1"));
    conflicting.json(200, version("D2", "PD", "server_only", &["1.21.1"], &["fabric"], json!([]), b"D2"));
    assert_eq!(
        client(conflicting).resolve(&resolve_request("root")).unwrap_err().kind,
        ProviderFailureKind::Incompatible
    );
}

#[test]
fn client_only_exact_version_is_invalid_for_required_server_resolution() {
    let transport = FixtureTransport::default();
    transport.json(
        200,
        version("V1", "P1", "client_only", &["1.21.1"], &["fabric"], json!([]), b"client"),
    );
    assert_eq!(
        client(transport).resolve(&resolve_request("V1")).unwrap_err().kind,
        ProviderFailureKind::Incompatible
    );
}

#[test]
fn hash_mismatch_and_interrupted_download_never_publish_partial_artifacts() {
    let mismatch = FixtureTransport::default();
    mismatch.json(
        200,
        version("V1", "P1", "server_only", &["1.21.1"], &["fabric"], json!([]), b"good"),
    );
    mismatch.download(b"evil");
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("V1.jar");
    fs::write(&destination, b"old").unwrap();
    let request = ModDownloadRequest {
        locator: locator("P1", "V1", b"good"),
        destination_dir: directory.path().to_path_buf(),
        max_bytes: Some(1024),
    };
    let error = client(mismatch).download(&request).unwrap_err();
    assert!(matches!(
        error.kind,
        ProviderFailureKind::DownloadInterrupted | ProviderFailureKind::HashMismatch
    ));
    assert_eq!(fs::read(&destination).unwrap(), b"old");
    assert!(fs::read_dir(directory.path())
        .unwrap()
        .all(|entry| !entry.unwrap().file_name().to_string_lossy().contains(".part-")));

    let interrupted = FixtureTransport::default();
    interrupted.json(
        200,
        version("V2", "P2", "server_only", &["1.21.1"], &["fabric"], json!([]), b"good"),
    );
    interrupted.download_error();
    let directory = tempfile::tempdir().unwrap();
    let request = ModDownloadRequest {
        locator: locator("P2", "V2", b"good"),
        destination_dir: directory.path().to_path_buf(),
        max_bytes: Some(1024),
    };
    assert_eq!(
        client(interrupted).download(&request).unwrap_err().kind,
        ProviderFailureKind::DownloadInterrupted
    );
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn verified_download_is_fsynced_published_and_records_local_sha256() {
    let transport = FixtureTransport::default();
    let bytes = b"verified mod bytes";
    transport.json(
        200,
        version("V1", "P1", "server_only", &["1.21.1"], &["fabric"], json!([]), bytes),
    );
    transport.download(bytes);
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("V1.jar"), b"old").unwrap();
    let request = ModDownloadRequest {
        locator: locator("P1", "V1", bytes),
        destination_dir: directory.path().to_path_buf(),
        max_bytes: Some(1024),
    };

    let result = client(transport).download(&request).unwrap();
    assert_eq!(fs::read(&result.path).unwrap(), bytes);
    assert!(result.hashes.sha256.is_some());
    let names: Vec<_> = fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["V1.jar"]);
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
