use super::*;
use crate::live_fabric_readiness::{verify_runtime_reply, ExpectedFabricRuntime};
use std::{collections::HashMap, path::Path, sync::RwLock};
use swarm_core::discovery::sign_discovery_freshness_vote;
use swarm_protocol::{MembershipRecordV1, WorldGenesisV1, WorldMemberV1};

fn identity(seed: u8) -> PeerIdentity {
    PeerIdentity::from_secret_bytes([seed; 32])
}

fn initial_membership(identities: &[PeerIdentity]) -> DiscoveryMembershipProofV1 {
    initial_membership_with_fingerprint(identities, Hash32([7; 32]))
}

fn initial_membership_with_fingerprint(
    identities: &[PeerIdentity],
    compatibility_fingerprint: Hash32,
) -> DiscoveryMembershipProofV1 {
    let creator = &identities[0];
    let mut members = identities
        .iter()
        .map(|id| WorldMemberV1 {
            peer_id: id.peer_id(),
            public_key: id.public_key(),
            authority_eligible: true,
            banned: false,
        })
        .collect::<Vec<_>>();
    members.sort_by_key(|member| member.peer_id);
    let genesis = WorldGenesisV1 {
        protocol_version: PROTOCOL_VERSION,
        minecraft_version: "26.1.2".into(),
        fabric_loader_version: "0.19.3".into(),
        compatibility_fingerprint,
        creation_nonce: [8; 32],
        creator_public_key: creator.public_key(),
        initial_membership: members.iter().map(|member| member.peer_id).collect(),
    };
    let world_id = genesis.world_id().unwrap();
    let mut record = MembershipRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id,
        epoch: 1,
        sequence: 0,
        previous_membership_hash: None,
        members,
        authority_peer_id: creator.peer_id(),
        authority_public_key: creator.public_key(),
        signature: Vec::new(),
    };
    creator.sign_membership(&mut record).unwrap();
    DiscoveryMembershipProofV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id,
        genesis,
        initial_membership: record.clone(),
        membership_certificates: Vec::new(),
        current_membership: record,
        pending_membership: None,
    }
}

fn freshness_challenge(proof: &DiscoveryMembershipProofV1, verifier: PeerId) -> DiscoveryFreshnessChallengeV1 {
    DiscoveryFreshnessChallengeV1 {
        protocol_version: PROTOCOL_VERSION,
        verifier_peer_id: verifier,
        nonce: [19; 32],
        world_id: proof.world_id,
        announcement_hash: Hash32([21; 32]),
        membership_sequence: proof.current_membership.sequence,
        membership_hash: proof.current_membership.record_hash().unwrap(),
        pending_membership_proposal_hash: None,
        authority_peer_id: proof.current_membership.authority_peer_id,
        authority_epoch: 1,
        fencing_token: 1,
        config_sequence: 1,
        config_hash: Hash32([22; 32]),
        canonical_head: None,
        issued_unix_ms: 1_000,
        expires_unix_ms: 2_000,
    }
}

#[test]
fn genesis_membership_requires_real_signatures_and_bound_keys() {
    let alice = identity(1);
    let bob = identity(2);
    let proof = initial_membership(&[alice, bob]);
    verify_genesis_anchored_membership(&proof).unwrap();

    let mut forged = proof.clone();
    forged.current_membership.members[0].public_key = [55; 32];
    assert!(verify_genesis_anchored_membership(&forged).is_err());
    let mut unsigned = proof.clone();
    unsigned.current_membership.signature.clear();
    assert!(verify_genesis_anchored_membership(&unsigned).is_err());
    let mut wrong_world = proof.clone();
    wrong_world.genesis.creation_nonce = [0; 32];
    assert_ne!(wrong_world.genesis.world_id().unwrap(), wrong_world.world_id);
}

#[test]
fn private_world_signed_votes_require_intersecting_live_majority() {
    let alice = identity(1);
    let bob = identity(2);
    let proof = initial_membership(&[identity(1), identity(2), identity(3)]);
    verify_genesis_anchored_membership(&proof).unwrap();
    let challenge = freshness_challenge(&proof, alice.peer_id());
    let mut votes = vec![
        sign_discovery_freshness_vote(&alice, &challenge).unwrap(),
        sign_discovery_freshness_vote(&bob, &challenge).unwrap(),
    ];
    votes.sort_by_key(|vote| vote.voter_peer_id);
    for vote in &votes {
        verify_discovery_freshness_vote(vote, &challenge).unwrap();
    }
    validate_discovery_freshness_quorum(&proof, &votes).unwrap();
    assert!(validate_discovery_freshness_quorum(&proof, &votes[..1]).is_err());
    let mut modified = challenge.clone();
    modified.nonce = [20; 32];
    assert!(verify_discovery_freshness_vote(&votes[0], &modified).is_err());
    votes.push(votes[0].clone());
    assert!(validate_discovery_freshness_quorum(&proof, &votes).is_err());
}

#[test]
fn signed_fabric_attestation_binds_consent_world_nonce_and_port() {
    let host = identity(4);
    let transport = swarm_network::generate_transport_key().public().to_peer_id();
    let world = WorldId([7; 32]);
    let temp = tempfile::tempdir().unwrap();
    let paths = DataPaths::from_root(temp.path().join("data"));
    paths.ensure().unwrap();
    std::fs::write(paths.identity_key(), host.network_signing_key().to_bytes()).unwrap();
    let expected = ExpectedFabricRuntime {
        world_id: world,
        world_directory: Path::new("/private/world"),
        compatibility_fingerprint: Hash32([8; 32]),
        managed_loopback_port: 25567,
        request_id: 42,
        challenge_nonce: [9; 32],
    };
    let response = format!(
        "RUNTIME_READY\t42\t{}\t{}\t25567\t{}\t{}",
        hex::encode([9; 32]),
        hex::encode([10; 32]),
        hex::encode("/private/world"),
        Hash32([8; 32]),
    );
    let runtime = verify_runtime_reply(&response, &expected).unwrap();
    let consent = hosting_budget::HostingConsentV1 {
        schema_version: 1,
        world_id: world,
        peer_id: host.peer_id(),
        enabled: true,
        heap_mib: 2_048,
        revision: 3,
    };
    let consent_path = paths.root.join("control").join(world.to_hex()).join("hosting-consent.json");
    std::fs::create_dir_all(consent_path.parent().unwrap()).unwrap();
    std::fs::write(&consent_path, serde_json::to_vec(&consent).unwrap()).unwrap();
    let canonical = RuntimeCanonicalBinding {
        membership_hash: Hash32([12; 32]),
        config_hash: Hash32([13; 32]),
        checkpoint_hash: Hash32([14; 32]),
        state_root: Hash32([15; 32]),
        epoch: 5,
        fencing_token: 7,
    };
    let attestation = sign_live_host_fabric_attestation(
        &paths,
        &host,
        &transport,
        identity(5).peer_id(),
        v1_realm_hash(world),
        canonical,
        &runtime,
        1_000,
    )
    .unwrap();
    attestation.verify(1_001).unwrap();
    assert!(attestation.verify(4_000).is_err());
    let mut tampered = attestation.clone();
    tampered.game_server_port = 25565;
    assert!(tampered.verify(1_001).is_err());
    tampered = attestation.clone();
    tampered.challenge_nonce = [11; 32];
    assert!(tampered.verify(1_001).is_err());
    tampered = attestation.clone();
    tampered.consent_revision += 1;
    assert!(tampered.verify(1_001).is_err());
    tampered = attestation.clone();
    tampered.canonical.fencing_token += 1;
    assert!(tampered.verify(1_001).is_err());
    tampered = attestation.clone();
    tampered.canonical.checkpoint_hash = Hash32([99; 32]);
    assert!(tampered.verify(1_001).is_err());
    let mut revoked = consent.clone();
    revoked.enabled = false;
    revoked.heap_mib = 0;
    revoked.revision += 1;
    std::fs::write(&consent_path, serde_json::to_vec(&revoked).unwrap()).unwrap();
    assert!(sign_live_host_fabric_attestation(
        &paths,
        &host,
        &transport,
        identity(5).peer_id(),
        v1_realm_hash(world),
        canonical,
        &runtime,
        1_000,
    )
    .is_err());
    // Re-enabling creates a different revision; the signer must obtain it
    // from the CURRENT record, never from the previous caller's object.
    let reenabled = hosting_budget::HostingConsentV1 {
        enabled: true,
        heap_mib: consent.heap_mib,
        revision: revoked.revision + 1,
        ..consent.clone()
    };
    std::fs::write(&consent_path, serde_json::to_vec(&reenabled).unwrap()).unwrap();
    let refreshed = sign_live_host_fabric_attestation(
        &paths,
        &host,
        &transport,
        identity(5).peer_id(),
        v1_realm_hash(world),
        canonical,
        &runtime,
        1_000,
    )
    .unwrap();
    assert_eq!(refreshed.consent_revision, reenabled.revision);
    assert_ne!(refreshed.signature, attestation.signature);
    let lock =
        OpenOptions::new().read(true).write(true).open(paths.root.join("control/hosting-consent-write.lock")).unwrap();
    lock.try_lock_exclusive().unwrap();
    assert!(sign_live_host_fabric_attestation(
        &paths,
        &host,
        &transport,
        identity(5).peer_id(),
        v1_realm_hash(world),
        canonical,
        &runtime,
        1_000,
    )
    .is_err());
    drop(lock);
    assert!(sign_live_host_fabric_attestation(
        &paths,
        &host,
        &transport,
        identity(5).peer_id(),
        v1_realm_hash(world),
        canonical,
        &runtime,
        1_000,
    )
    .is_ok());
}

struct NoLiveProof;

impl LiveAuthorityProofProvider for NoLiveProof {
    fn acquire(&self, _: WorldId, _: PeerId, _: [u8; 32]) -> Result<LiveAuthorityObservation> {
        bail!("no current authenticated quorum or Fabric IPC session")
    }
}

#[test]
fn missing_live_provider_can_never_authorize_game_streams() {
    let temp = tempfile::tempdir().unwrap();
    let paths = DataPaths::from_root(temp.path());
    let local = identity(6);
    let transport = swarm_network::generate_transport_key().public().to_peer_id();
    let registry: GameAuthenticatedPeers = Arc::new(RwLock::new(HashMap::new()));
    let source = SignedLiveAuthoritySource::new(paths, local.peer_id(), transport, registry, Arc::new(NoLiveProof));
    assert!(source.verify_fresh(WorldId([1; 32]), identity(7).peer_id(), [8; 32]).is_err());
    assert!(source.verify_fresh(WorldId([1; 32]), identity(7).peer_id(), [0; 32]).is_err());
}

struct FixedSignedMaterial(Arc<RwLock<LiveAuthorityObservation>>);

impl LiveAuthorityProofProvider for FixedSignedMaterial {
    fn acquire(&self, _: WorldId, _: PeerId, _: [u8; 32]) -> Result<LiveAuthorityObservation> {
        Ok(self.0.read().unwrap().clone())
    }
}

/// This is deliberately synthetic Java/quorum input. It proves the complete
/// verifier consumes REAL signatures and accepted disk records rather than a
/// provider-supplied readiness flag; it is not a three-machine field test.
#[test]
fn canonical_route_requires_signed_majority_and_current_host_consent() {
    use std::fs;
    use swarm_protocol::{
        AuthorityPolicyV1, MembershipPolicyV1, RuntimeCompatibilityManifestV1, SoloBranchV1, WorldConfigV1,
        WorldDescriptorV1, WorldPresentationV1, WorldVisibilityV1, STORAGE_SCHEMA_VERSION,
    };
    use swarm_storage::{SnapshotContext, WorldMetadataV1};

    let temp = tempfile::tempdir().unwrap();
    let paths = DataPaths::from_root(temp.path().join("data"));
    paths.ensure().unwrap();
    let alice = identity(21);
    let bob = identity(22);
    let carol = identity(23);
    fs::write(paths.identity_key(), alice.network_signing_key().to_bytes()).unwrap();
    let storage = Storage::open(paths.root.clone()).unwrap();
    let compatibility = RuntimeCompatibilityManifestV1 {
        minecraft_version: "26.1.2".into(),
        loader_id: "fabric".into(),
        loader_version: "0.19.3".into(),
        swarmcraft_protocol_version: PROTOCOL_VERSION,
        fabric_adapter_version: "0.5.0".into(),
        required_server_mods: Vec::new(),
        required_client_mods: Vec::new(),
        datapacks: Vec::new(),
    };
    let fingerprint = compatibility.fingerprint().unwrap();
    let membership = initial_membership_with_fingerprint(&[identity(21), identity(22), identity(23)], fingerprint);
    let world = membership.world_id;
    storage
        .create_world(&WorldMetadataV1 {
            storage_schema_version: STORAGE_SCHEMA_VERSION,
            display_name: "live authority verification fixture".into(),
            world_id: world,
            genesis: membership.genesis.clone(),
        })
        .unwrap();
    storage
        .save_world_descriptor(&WorldDescriptorV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            compatibility_fingerprint: fingerprint,
            members: membership.current_membership.members.clone(),
            preferred_replication_factor: 3,
        })
        .unwrap();

    let source_dir = temp.path().join("snapshot");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(source_dir.join("level.dat"), b"signed snapshot").unwrap();
    let mut snapshot = storage
        .snapshot_directory(
            &source_dir,
            SnapshotContext {
                world,
                snapshot_number: 1,
                epoch: 1,
                sequence: 1,
                previous_snapshot_hash: None,
                authority_peer_id: alice.peer_id(),
                authority_public_key: alice.public_key(),
            },
        )
        .unwrap();
    alice.sign_snapshot(&mut snapshot).unwrap();
    storage.commit_snapshot(&snapshot).unwrap();
    storage.verify_snapshot(&snapshot).unwrap();

    let mut epoch = EpochRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        epoch_number: 1,
        previous_epoch_hash: None,
        base_state_hash: snapshot.state_root,
        authority_peer_id: alice.peer_id(),
        authority_public_key: alice.public_key(),
        mode: EpochMode::Quorum,
        fencing_token: 1,
        reason: "test accepted quorum generation".into(),
        signature: Vec::new(),
    };
    epoch.signature = alice.sign(&epoch.signing_bytes().unwrap());
    storage.save_epoch_record(&epoch).unwrap();
    storage.save_membership_record(&membership.current_membership).unwrap();
    let mut config = WorldConfigV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        sequence: 1,
        previous_config_hash: None,
        compatibility,
        visibility: WorldVisibilityV1::Private,
        authority_policy: AuthorityPolicyV1 { allow_solo_advancement: false, preferred_replication_factor: 3 },
        membership_policy: MembershipPolicyV1::InviteOnly,
        presentation: WorldPresentationV1 {
            name: "live authority verification fixture".into(),
            description: String::new(),
            tags: Vec::new(),
            icon_hash: None,
            approximate_region: None,
        },
        authority_peer_id: alice.peer_id(),
        authority_public_key: alice.public_key(),
        signature: Vec::new(),
    };
    swarm_core::sign_world_config(&alice, &mut config).unwrap();
    storage.save_world_config(&config).unwrap();

    let consent = hosting_budget::HostingConsentV1 {
        schema_version: 1,
        world_id: world,
        peer_id: alice.peer_id(),
        enabled: true,
        heap_mib: 2_048,
        revision: 123,
    };
    let consent_path = paths.root.join("control").join(world.to_hex()).join("hosting-consent.json");
    fs::create_dir_all(consent_path.parent().unwrap()).unwrap();
    fs::write(&consent_path, serde_json::to_vec(&consent).unwrap()).unwrap();

    let nonce = [49; 32];
    let world_dir = paths.root.join("runtime").join(world.to_hex()).join("world");
    let ready_line = format!(
        "RUNTIME_READY\t42\t{}\t{}\t25567\t{}\t{}",
        hex::encode(nonce),
        hex::encode([25; 32]),
        hex::encode(world_dir.to_string_lossy().as_bytes()),
        fingerprint,
    );
    let runtime = verify_runtime_reply(
        &ready_line,
        &ExpectedFabricRuntime {
            world_id: world,
            world_directory: &world_dir,
            compatibility_fingerprint: fingerprint,
            managed_loopback_port: 25567,
            request_id: 42,
            challenge_nonce: nonce,
        },
    )
    .unwrap();

    let head = DiscoveryCanonicalHeadV1 {
        snapshot_number: 1,
        manifest_hash: snapshot.manifest_hash().unwrap(),
        epoch: 1,
        sequence: 1,
    };
    let membership_hash = membership.current_membership.record_hash().unwrap();
    let config_hash = config.config_hash().unwrap();
    let now = unix_ms().unwrap();
    let mut challenge = freshness_challenge(&membership, alice.peer_id());
    challenge.nonce = nonce;
    challenge.issued_unix_ms = now;
    challenge.expires_unix_ms = now + 2_500;
    challenge.announcement_hash = canonical_quorum_hash(world, membership_hash, &epoch, config_hash, &head).unwrap();
    challenge.config_hash = config_hash;
    challenge.canonical_head = Some(head);
    let mut votes = vec![
        sign_discovery_freshness_vote(&alice, &challenge).unwrap(),
        sign_discovery_freshness_vote(&carol, &challenge).unwrap(),
    ];
    votes.sort_by_key(|vote| vote.voter_peer_id);
    let transport = swarm_network::generate_transport_key().public().to_peer_id();
    let canonical = RuntimeCanonicalBinding {
        membership_hash,
        config_hash,
        checkpoint_hash: head.manifest_hash,
        state_root: snapshot.state_root,
        epoch: epoch.epoch_number,
        fencing_token: epoch.fencing_token,
    };
    let host_fabric = sign_live_host_fabric_attestation(
        &paths,
        &alice,
        &transport,
        bob.peer_id(),
        v1_realm_hash(world),
        canonical,
        &runtime,
        now,
    )
    .unwrap();
    let proof = LiveAuthorityObservation {
        membership,
        epoch,
        config,
        snapshot: storage.latest_snapshot(world).unwrap().unwrap(),
        quorum_challenge: challenge,
        quorum_votes: votes,
        host_fabric,
    };
    let signed = Arc::new(RwLock::new(proof));
    let registry: GameAuthenticatedPeers = Arc::new(RwLock::new(HashMap::new()));
    let source = SignedLiveAuthoritySource::new(
        paths,
        alice.peer_id(),
        transport,
        registry,
        Arc::new(FixedSignedMaterial(Arc::clone(&signed))),
    );
    let verified = source.verify_fresh(world, bob.peer_id(), nonce).unwrap();
    assert_eq!(verified.authority_peer_id, alice.peer_id());
    assert_eq!(verified.game_server_port, 25567);
    assert_eq!(verified.challenge_nonce, nonce);
    assert!(verified.runtime_ready_verified && verified.hosting_consent_valid && verified.conflict_free);

    // Even previously valid signed material fails after consent withdrawal,
    // before a new route can be minted or an existing connection refreshed.
    let mut withdrawn = consent.clone();
    withdrawn.enabled = false;
    withdrawn.heap_mib = 0;
    withdrawn.revision += 1;
    fs::write(&consent_path, serde_json::to_vec(&withdrawn).unwrap()).unwrap();
    assert!(source.verify_fresh(world, bob.peer_id(), nonce).is_err());
    fs::write(&consent_path, serde_json::to_vec(&consent).unwrap()).unwrap();

    // Durable canonical metadata is not sufficient if a snapshot blob has
    // disappeared or changed after acceptance. Refuse without falling back
    // to a cached runtime status or a signed but unrestorable manifest.
    let blob = storage
        .world_dir(world)
        .join("blobs")
        .join(format!("{}.zst", signed.read().unwrap().snapshot.entries[0].blob.hash,));
    let original_blob = fs::read(&blob).unwrap();
    fs::write(&blob, b"tampered compressed snapshot").unwrap();
    assert!(source.verify_fresh(world, bob.peer_id(), nonce).is_err());
    fs::write(&blob, original_blob).unwrap();
    assert!(source.verify_fresh(world, bob.peer_id(), nonce).is_ok());

    let mut solo_branch = SoloBranchV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        base_snapshot_hash: head.manifest_hash,
        base_epoch: 1,
        head_snapshot_hash: head.manifest_hash,
        head_epoch: 1,
        head_sequence: 1,
        state_hash: canonical.state_root,
        authority_peer_id: alice.peer_id(),
        authority_public_key: alice.public_key(),
        signature: Vec::new(),
    };
    swarm_core::sign_solo_branch(&alice, &mut solo_branch).unwrap();
    storage.save_solo_branch(&solo_branch).unwrap();
    assert!(source.verify_fresh(world, bob.peer_id(), nonce).is_err());
    fs::remove_file(storage.world_dir(world).join("metadata/solo-branch.postcard")).unwrap();
    assert!(source.verify_fresh(world, bob.peer_id(), nonce).is_ok());

    // A preserved conflict is not resolved merely because the current host
    // can gather fresh majority signatures. Corrupt conflict records must
    // fail closed instead of being silently treated as an empty ledger.
    let preserved_conflict = storage.preserve_solo_conflict(&solo_branch).unwrap();
    assert!(source.verify_fresh(world, bob.peer_id(), nonce).is_err());
    fs::remove_file(preserved_conflict).unwrap();
    assert!(source.verify_fresh(world, bob.peer_id(), nonce).is_ok());
    let conflict_dir = storage.world_dir(world).join("recovery").join("solo-conflicts");
    fs::create_dir_all(&conflict_dir).unwrap();
    let unreadable_conflict = conflict_dir.join("unreadable.postcard");
    fs::write(&unreadable_conflict, b"corrupted solo conflict").unwrap();
    assert!(source.verify_fresh(world, bob.peer_id(), nonce).is_err());
    fs::remove_file(&unreadable_conflict).unwrap();
    assert!(source.verify_fresh(world, bob.peer_id(), nonce).is_ok());

    // An attacker cannot obtain a route with one vote in this 3-member world.
    signed.write().unwrap().quorum_votes.pop();
    assert!(source.verify_fresh(world, bob.peer_id(), nonce).is_err());

    // A newer accepted epoch/fence invalidates the old evidence even when
    // it was correctly signed and its snapshot bytes remain intact.
    let original_proof = signed.read().unwrap().clone();
    assert!(require_current_canonical_generation(&storage, world, &original_proof).is_ok());
    let mut next_epoch = original_proof.epoch.clone();
    next_epoch.epoch_number += 1;
    next_epoch.fencing_token += 1;
    next_epoch.reason = "superseded while verifying route authority".into();
    next_epoch.signature = alice.sign(&next_epoch.signing_bytes().unwrap());
    storage.save_epoch_record(&next_epoch).unwrap();
    assert!(require_current_canonical_generation(&storage, world, &original_proof).is_err());
    assert!(source.verify_fresh(world, bob.peer_id(), nonce).is_err());
}
