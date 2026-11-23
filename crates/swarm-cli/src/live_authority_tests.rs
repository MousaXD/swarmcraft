use super::*;
use crate::live_fabric_readiness::{verify_runtime_reply, ExpectedFabricRuntime};
use std::{collections::HashMap, path::Path, sync::RwLock};
use swarm_core::discovery::sign_discovery_freshness_vote;
use swarm_protocol::{MembershipRecordV1, WorldGenesisV1, WorldMemberV1};

fn identity(seed: u8) -> PeerIdentity {
    PeerIdentity::from_secret_bytes([seed; 32])
}

fn initial_membership(identities: &[PeerIdentity]) -> DiscoveryMembershipProofV1 {
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
        compatibility_fingerprint: Hash32([7; 32]),
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
    let canonical = RuntimeCanonicalBinding {
        membership_hash: Hash32([12; 32]),
        config_hash: Hash32([13; 32]),
        checkpoint_hash: Hash32([14; 32]),
        state_root: Hash32([15; 32]),
        epoch: 5,
        fencing_token: 7,
    };
    let attestation = sign_live_host_fabric_attestation(
        &host,
        &transport,
        identity(5).peer_id(),
        v1_realm_hash(world),
        canonical,
        &consent,
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
    let mut revoked = consent;
    revoked.enabled = false;
    assert!(sign_live_host_fabric_attestation(
        &host,
        &transport,
        identity(5).peer_id(),
        v1_realm_hash(world),
        canonical,
        &revoked,
        &runtime,
        1_000,
    )
    .is_err());
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
