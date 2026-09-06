use std::{collections::BTreeMap, path::Path};
use swarm_cli::package_provider::{
    modrinth::{HttpResponse, ModrinthClient, ModrinthTransport, MAX_PROVIDER_METADATA_BYTES},
    ModSearchQuery, PackageEnvironment, ProviderFailure, ProviderFailureKind, ReleaseType,
};
use swarm_protocol::{
    ArtifactSideV1, CanonicalArtifactSourceV1, CanonicalHashAlgorithmV1, CanonicalLoaderV1,
    CanonicalModpackV1, CanonicalPackageIdentityV1, CanonicalPackageV1,
    CanonicalProviderArtifactV1, CanonicalProviderHashV1, CanonicalProviderV1,
    CanonicalRetrievalV1, Hash32, CANONICAL_MODPACK_SCHEMA_VERSION,
};

#[derive(Clone)]
struct OversizedTransport;

impl ModrinthTransport for OversizedTransport {
    fn get(&self, _url: &str) -> Result<HttpResponse, ProviderFailure> {
        Ok(HttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: vec![b'{'; MAX_PROVIDER_METADATA_BYTES + 1],
        })
    }

    fn download(
        &self,
        _url: &str,
        _destination: &Path,
        _max_bytes: u64,
    ) -> Result<(), ProviderFailure> {
        unreachable!("metadata-bound regression never downloads an artifact")
    }
}

fn strong_curseforge_pack(retrieval: CanonicalRetrievalV1) -> CanonicalModpackV1 {
    CanonicalModpackV1 {
        schema_version: CANONICAL_MODPACK_SCHEMA_VERSION,
        minecraft_version: "1.21.1".into(),
        loader: CanonicalLoaderV1 {
            id: "fabric".into(),
            version: "0.16.14".into(),
        },
        packages: vec![CanonicalPackageV1 {
            artifact_id: "example".into(),
            version: "1.0.0".into(),
            artifact_hash: Hash32([7; 32]),
            side: ArtifactSideV1::Server,
            source: CanonicalArtifactSourceV1::Provider {
                artifact: CanonicalProviderArtifactV1 {
                    identity: CanonicalPackageIdentityV1 {
                        provider: CanonicalProviderV1::CurseForge,
                        project_id: "123".into(),
                        version_id: "456".into(),
                    },
                    file_name: "example.jar".into(),
                    file_size: Some(3),
                    hashes: vec![CanonicalProviderHashV1 {
                        algorithm: CanonicalHashAlgorithmV1::Sha1,
                        digest_hex: "a9993e364706816aba3e25717850c26c9cd0d89d".into(),
                    }],
                    retrieval,
                    dependencies: vec![],
                },
            },
        }],
        datapacks: vec![],
    }
}

#[test]
fn oversized_modrinth_metadata_is_rejected_before_json_parsing() {
    let client =
        ModrinthClient::with_transport("https://fixtures.invalid/v2", OversizedTransport).unwrap();
    let error = client
        .search(&ModSearchQuery {
            query: "example".into(),
            minecraft_version: "1.21.1".into(),
            loader: "fabric".into(),
            environment: PackageEnvironment::Server,
            release_type: Some(ReleaseType::Release),
            offset: 0,
            limit: 10,
        })
        .unwrap_err();
    assert_eq!(error.kind, ProviderFailureKind::MalformedResponse);
    assert!(error.message.contains("response_too_large"));
}

#[test]
fn strong_hash_provider_contract_survives_clean_peer_runtime_round_trip() {
    let canonical = strong_curseforge_pack(CanonicalRetrievalV1::ProviderDownload);
    canonical.validate().unwrap();
    let runtime = canonical.to_runtime_compatibility("0.5.0").unwrap();
    let rebuilt = CanonicalModpackV1::from_runtime_compatibility(&runtime).unwrap();
    rebuilt.validate().unwrap();
    assert_eq!(rebuilt, canonical);
    let CanonicalArtifactSourceV1::Provider { artifact } = &rebuilt.packages[0].source else {
        panic!("provider provenance collapsed during clean-peer round trip");
    };
    assert_eq!(artifact.retrieval, CanonicalRetrievalV1::ProviderDownload);
    assert!(artifact
        .hashes
        .iter()
        .any(|hash| hash.algorithm == CanonicalHashAlgorithmV1::Sha1));
}

#[test]
fn md5_only_provider_download_is_rejected_but_manual_contract_remains_valid() {
    let mut canonical = strong_curseforge_pack(CanonicalRetrievalV1::ProviderDownload);
    let CanonicalArtifactSourceV1::Provider { artifact } = &mut canonical.packages[0].source else {
        unreachable!();
    };
    artifact.hashes = vec![CanonicalProviderHashV1 {
        algorithm: CanonicalHashAlgorithmV1::Md5,
        digest_hex: "900150983cd24fb0d6963f7d28e17f72".into(),
    }];
    assert!(canonical.validate().is_err());
    let CanonicalArtifactSourceV1::Provider { artifact } = &mut canonical.packages[0].source else {
        unreachable!();
    };
    artifact.retrieval = CanonicalRetrievalV1::ManualRequired;
    canonical.validate().unwrap();
}
