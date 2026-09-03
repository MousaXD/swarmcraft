#![cfg(unix)]

use std::{
    fs,
    net::UdpSocket,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use swarm_cli::authority_permit::permit_path;
use swarm_consensus::membership_vote_for;
use swarm_core::{create_world_genesis_with_fingerprint, random_nonce, sign_world_config, DataPaths, PeerIdentity};
use swarm_network::load_or_create_transport_key;
use swarm_protocol::{
    AuthorityPolicyV1, EpochMode, EpochRecordV1, InviteV1, JoinRequestV1, MembershipPolicyV1, MembershipProposalV1,
    MembershipRecordV1, RuntimeCompatibilityManifestV1, SnapshotManifestV1, WorldConfigV1, WorldDescriptorV1, WorldId,
    WorldMemberV1, WorldPresentationV1, WorldVisibilityV1, PROTOCOL_VERSION, STORAGE_SCHEMA_VERSION,
};
use swarm_storage::{MembershipPromiseResult, SnapshotContext, Storage, WorldMetadataV1};
use tempfile::TempDir;

const WAIT_STEP: Duration = Duration::from_millis(200);

struct PeerFixture {
    _temp: TempDir,
    paths: DataPaths,
    storage: Storage,
    identity: PeerIdentity,
    port: u16,
    transport_peer: String,
}

struct Seed<'a> {
    metadata: &'a WorldMetadataV1,
    config: &'a WorldConfigV1,
    descriptor: &'a WorldDescriptorV1,
    membership: &'a MembershipRecordV1,
    epoch: &'a EpochRecordV1,
    manifest: &'a SnapshotManifestV1,
    source: &'a std::path::Path,
    authority: &'a PeerIdentity,
}

struct ManagedChild(Child);

impl ManagedChild {
    fn stop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }

    fn alive(&mut self) -> bool {
        self.0.try_wait().unwrap().is_none()
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        self.stop();
    }
}

fn free_udp_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn peer_fixture() -> PeerFixture {
    let temp = tempfile::tempdir().unwrap();
    let paths = DataPaths::from_root(temp.path().join("data"));
    let storage = Storage::open(paths.root.clone()).unwrap();
    let identity = PeerIdentity::load_or_create(&paths).unwrap();
    let transport_key = load_or_create_transport_key(&paths.transport_key()).unwrap();
    let transport_peer = transport_key.public().to_peer_id().to_string();
    PeerFixture { _temp: temp, paths, storage, identity, port: free_udp_port(), transport_peer }
}

fn address(peer: &PeerFixture) -> String {
    format!("/ip4/127.0.0.1/udp/{}/quic-v1/p2p/{}", peer.port, peer.transport_peer)
}

fn spawn_daemon(peer: &PeerFixture, bootstraps: &[String]) -> ManagedChild {
    let mut command = Command::new(env!("CARGO_BIN_EXE_swarmcraft"));
    command
        .arg("--data-dir")
        .arg(&peer.paths.root)
        .arg("daemon")
        .arg("--listen")
        .arg(format!("/ip4/127.0.0.1/udp/{}/quic-v1", peer.port))
        .env("RUST_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    if !bootstraps.is_empty() {
        command.env("SWARMCRAFT_BOOTSTRAP", bootstraps.join(","));
    }
    ManagedChild(command.spawn().unwrap())
}

fn wait_until(label: &str, timeout: Duration, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        thread::sleep(WAIT_STEP);
    }
    panic!("timed out waiting for {label}");
}

fn member(peer: &PeerFixture) -> WorldMemberV1 {
    WorldMemberV1 {
        peer_id: peer.identity.peer_id(),
        public_key: peer.identity.public_key(),
        authority_eligible: true,
        banned: false,
    }
}

fn genesis_membership(metadata: &WorldMetadataV1, authority: &PeerIdentity) -> MembershipRecordV1 {
    let mut membership = MembershipRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: metadata.world_id,
        epoch: 0,
        sequence: 0,
        previous_membership_hash: None,
        members: vec![WorldMemberV1 {
            peer_id: authority.peer_id(),
            public_key: authority.public_key(),
            authority_eligible: true,
            banned: false,
        }],
        authority_peer_id: authority.peer_id(),
        authority_public_key: authority.public_key(),
        signature: Vec::new(),
    };
    authority.sign_membership(&mut membership).unwrap();
    membership
}

fn permit(peer: &PeerFixture, world: WorldId) -> Option<(u64, u64, u64)> {
    let value = fs::read_to_string(permit_path(&peer.paths, world)).ok()?;
    let mut fields = value.split_whitespace();
    Some((fields.next()?.parse().ok()?, fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
}

fn build_seed<'a>(
    authority: &'a PeerFixture,
    members: &[&PeerFixture],
    label: &str,
    source_temp: &'a TempDir,
) -> (WorldMetadataV1, WorldConfigV1, WorldDescriptorV1, MembershipRecordV1, EpochRecordV1, SnapshotManifestV1) {
    let compatibility = RuntimeCompatibilityManifestV1 {
        minecraft_version: "26.1.2".into(),
        loader_id: "fabric".into(),
        loader_version: "0.19.3".into(),
        swarmcraft_protocol_version: PROTOCOL_VERSION,
        fabric_adapter_version: env!("CARGO_PKG_VERSION").into(),
        required_server_mods: Vec::new(),
        required_client_mods: Vec::new(),
        datapacks: Vec::new(),
    };
    let fingerprint = compatibility.fingerprint().unwrap();
    let (world, genesis) = create_world_genesis_with_fingerprint(
        &authority.identity,
        compatibility.minecraft_version.clone(),
        compatibility.loader_version.clone(),
        fingerprint,
    )
    .unwrap();
    let metadata = WorldMetadataV1 {
        storage_schema_version: STORAGE_SCHEMA_VERSION,
        display_name: label.into(),
        world_id: world,
        genesis,
    };
    authority.storage.create_world(&metadata).unwrap();
    let genesis_membership = genesis_membership(&metadata, &authority.identity);
    authority.storage.save_membership_record(&genesis_membership).unwrap();
    let mut config = WorldConfigV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        sequence: 1,
        previous_config_hash: None,
        compatibility,
        visibility: WorldVisibilityV1::Private,
        authority_policy: AuthorityPolicyV1 {
            allow_solo_advancement: true,
            preferred_replication_factor: members.len() as u16,
        },
        membership_policy: MembershipPolicyV1::InviteOnly,
        presentation: WorldPresentationV1 {
            name: label.into(),
            description: String::new(),
            tags: Vec::new(),
            icon_hash: None,
            approximate_region: None,
        },
        authority_peer_id: authority.identity.peer_id(),
        authority_public_key: authority.identity.public_key(),
        signature: Vec::new(),
    };
    sign_world_config(&authority.identity, &mut config).unwrap();
    authority.storage.save_world_config(&config).unwrap();
    let mut descriptor = WorldDescriptorV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        compatibility_fingerprint: metadata.genesis.compatibility_fingerprint,
        members: members.iter().map(|peer| member(peer)).collect(),
        preferred_replication_factor: members.len() as u16,
    };
    descriptor.normalize();
    authority.storage.save_world_descriptor(&descriptor).unwrap();

    let source = source_temp.path().join("world");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("level.dat"), format!("{label}\n")).unwrap();
    let mut manifest = authority
        .storage
        .snapshot_directory(
            &source,
            SnapshotContext {
                world,
                snapshot_number: 1,
                epoch: 1,
                sequence: 1,
                previous_snapshot_hash: None,
                authority_peer_id: authority.identity.peer_id(),
                authority_public_key: authority.identity.public_key(),
            },
        )
        .unwrap();
    authority.identity.sign_snapshot(&mut manifest).unwrap();
    let mut epoch = EpochRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        epoch_number: 1,
        previous_epoch_hash: None,
        base_state_hash: manifest.manifest().state_root,
        authority_peer_id: authority.identity.peer_id(),
        authority_public_key: authority.identity.public_key(),
        mode: EpochMode::Quorum,
        fencing_token: 1,
        reason: "partition safety seed".into(),
        signature: Vec::new(),
    };
    epoch.signature = authority.identity.sign(&epoch.signing_bytes().unwrap());
    authority.storage.save_epoch_record(&epoch).unwrap();

    let mut membership = MembershipRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        epoch: 1,
        sequence: 1,
        previous_membership_hash: Some(genesis_membership.record_hash().unwrap()),
        members: descriptor.members.clone(),
        authority_peer_id: authority.identity.peer_id(),
        authority_public_key: authority.identity.public_key(),
        signature: Vec::new(),
    };
    authority.identity.sign_membership(&mut membership).unwrap();
    authority.storage.save_membership_record(&membership).unwrap();
    authority.storage.commit_snapshot(&manifest).unwrap();
    let manifest = manifest.manifest().clone();
    (metadata, config, descriptor, membership, epoch, manifest)
}

fn install_seed(peer: &PeerFixture, seed: &Seed<'_>) {
    if peer.storage.load_world(seed.metadata.world_id).is_err() {
        peer.storage.create_world(seed.metadata).unwrap();
    }
    if peer.storage.load_membership_record(seed.metadata.world_id).is_err() {
        let genesis_membership = genesis_membership(seed.metadata, seed.authority);
        peer.storage.save_membership_record(&genesis_membership).unwrap();
    }
    peer.storage.save_world_config(seed.config).unwrap();
    peer.storage.save_world_descriptor(seed.descriptor).unwrap();
    peer.storage.save_epoch_record(seed.epoch).unwrap();
    peer.storage.save_membership_record(seed.membership).unwrap();
    if peer.storage.latest_snapshot(seed.metadata.world_id).unwrap().is_none() {
        let mut manifest = peer
            .storage
            .snapshot_directory(
                seed.source,
                SnapshotContext {
                    world: seed.metadata.world_id,
                    snapshot_number: seed.manifest.snapshot_number,
                    epoch: seed.manifest.epoch,
                    sequence: seed.manifest.sequence,
                    previous_snapshot_hash: seed.manifest.previous_snapshot_hash,
                    authority_peer_id: seed.authority.peer_id(),
                    authority_public_key: seed.authority.public_key(),
                },
            )
            .unwrap();
        seed.authority.sign_snapshot(&mut manifest).unwrap();
        assert_eq!(manifest.manifest_hash().unwrap(), seed.manifest.manifest_hash().unwrap());
        peer.storage.commit_snapshot(&manifest).unwrap();
    }
}

fn pending_join(new_peer: &PeerFixture, authority: &PeerFixture, metadata: &WorldMetadataV1) {
    if new_peer.storage.load_world(metadata.world_id).is_err() {
        new_peer.storage.create_world(metadata).unwrap();
    }
    let mut invite = InviteV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: metadata.world_id,
        display_name: metadata.display_name.clone(),
        genesis: metadata.genesis.clone(),
        inviter_peer_id: authority.identity.peer_id(),
        inviter_public_key: authority.identity.public_key(),
        bootstrap_addrs: vec![address(authority)],
        expires_unix_ms: u64::MAX,
        nonce: random_nonce(),
        signature: Vec::new(),
    };
    authority.identity.sign_invite(&mut invite).unwrap();
    let mut join = JoinRequestV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: metadata.world_id,
        invite,
        joining_member: member(new_peer),
        nonce: random_nonce(),
        signature: Vec::new(),
    };
    new_peer.identity.sign_join_request(&mut join).unwrap();
    new_peer.storage.save_pending_join(&join).unwrap();
}

fn prepare_proposal(authority: &PeerFixture, current: &MembershipRecordV1, proposed_members: Vec<WorldMemberV1>) {
    let mut proposed = MembershipRecordV1 {
        protocol_version: current.protocol_version,
        world_id: current.world_id,
        epoch: current.epoch,
        sequence: current.sequence + 1,
        previous_membership_hash: Some(current.record_hash().unwrap()),
        members: proposed_members,
        authority_peer_id: authority.identity.peer_id(),
        authority_public_key: authority.identity.public_key(),
        signature: Vec::new(),
    };
    proposed.members.sort_by_key(|entry| entry.peer_id);
    authority.identity.sign_membership(&mut proposed).unwrap();
    let proposal = MembershipProposalV1 { previous: current.clone(), proposed };
    let mut vote =
        membership_vote_for(&proposal, authority.identity.peer_id(), authority.identity.public_key()).unwrap();
    vote.signature = authority.identity.sign(&vote.signing_bytes().unwrap());
    assert_eq!(
        authority.storage.promise_membership_proposal(&proposal, &vote).unwrap(),
        MembershipPromiseResult::Accepted
    );
}

#[test]
fn five_peer_membership_churn_partition_cannot_activate_new_voter_universe() {
    let a = peer_fixture();
    let b = peer_fixture();
    let c = peer_fixture();
    let d = peer_fixture();
    let e = peer_fixture();
    let source_temp = tempfile::tempdir().unwrap();
    let (metadata, config, descriptor, membership, epoch, manifest) =
        build_seed(&a, &[&a, &b, &c], "membership-3-to-5-partition", &source_temp);
    let source = source_temp.path().join("world");
    let seed = Seed {
        metadata: &metadata,
        config: &config,
        descriptor: &descriptor,
        membership: &membership,
        epoch: &epoch,
        manifest: &manifest,
        source: &source,
        authority: &a.identity,
    };
    install_seed(&b, &seed);
    install_seed(&c, &seed);
    pending_join(&d, &a, &metadata);
    pending_join(&e, &a, &metadata);
    let mut proposed = descriptor.members.clone();
    proposed.push(member(&d));
    proposed.push(member(&e));
    prepare_proposal(&a, &membership, proposed);

    let mut daemon_a = spawn_daemon(&a, &[]);
    thread::sleep(Duration::from_millis(500));
    let mut daemon_d = spawn_daemon(&d, &[]);
    let mut daemon_e = spawn_daemon(&e, &[]);
    wait_until("future-member durable prepares", Duration::from_secs(20), || {
        d.storage.load_membership_promise(metadata.world_id).is_ok()
            && e.storage.load_membership_promise(metadata.world_id).is_ok()
    });
    thread::sleep(Duration::from_secs(2));
    assert!(a.storage.load_membership_certificate(metadata.world_id).is_err());
    assert_eq!(a.storage.load_membership_record(metadata.world_id).unwrap(), membership);
    assert!(permit(&a, metadata.world_id).is_none());
    assert!(daemon_a.alive() && daemon_d.alive() && daemon_e.alive());
    daemon_a.stop();
    daemon_d.stop();
    daemon_e.stop();

    let bootstrap_b = vec![address(&b)];
    let mut daemon_b = spawn_daemon(&b, &[]);
    thread::sleep(Duration::from_millis(500));
    let mut daemon_c = spawn_daemon(&c, &bootstrap_b);
    wait_until("old committed 2-of-3 recovery quorum", Duration::from_secs(35), || {
        [permit(&b, metadata.world_id), permit(&c, metadata.world_id)]
            .into_iter()
            .flatten()
            .any(|(ep, fence, heartbeat)| ep == 2 && fence == 2 && heartbeat >= 1)
    });
    assert!(daemon_b.alive() && daemon_c.alive());
}

#[test]
fn three_peer_divergent_membership_partition_cannot_form_two_quorums() {
    let a = peer_fixture();
    let b = peer_fixture();
    let c = peer_fixture();
    let source_temp = tempfile::tempdir().unwrap();
    let (metadata, config, descriptor, membership, epoch, manifest) =
        build_seed(&a, &[&a, &b], "membership-2-to-2-partition", &source_temp);
    let source = source_temp.path().join("world");
    let seed = Seed {
        metadata: &metadata,
        config: &config,
        descriptor: &descriptor,
        membership: &membership,
        epoch: &epoch,
        manifest: &manifest,
        source: &source,
        authority: &a.identity,
    };
    install_seed(&b, &seed);
    pending_join(&c, &a, &metadata);
    prepare_proposal(&a, &membership, vec![member(&a), member(&c)]);

    let mut daemon_a = spawn_daemon(&a, &[]);
    thread::sleep(Duration::from_millis(500));
    let mut daemon_c = spawn_daemon(&c, &[]);
    wait_until("new-side prepare", Duration::from_secs(20), || {
        c.storage.load_membership_promise(metadata.world_id).is_ok()
    });
    thread::sleep(Duration::from_secs(2));
    assert!(a.storage.load_membership_certificate(metadata.world_id).is_err());
    assert!(permit(&a, metadata.world_id).is_none());
    daemon_a.stop();
    daemon_c.stop();

    let mut daemon_b = spawn_daemon(&b, &[]);
    thread::sleep(Duration::from_secs(10));
    assert!(daemon_b.alive());
    assert!(permit(&b, metadata.world_id).is_none());
    assert_eq!(b.storage.load_epoch_record(metadata.world_id).unwrap().epoch_number, 1);
}

fn assert_unclean_quorum_loss_does_not_enter_solo(member_count: usize) {
    assert!(member_count == 3 || member_count == 5);
    let peers = (0..member_count).map(|_| peer_fixture()).collect::<Vec<_>>();
    let refs = peers.iter().collect::<Vec<_>>();
    let source_temp = tempfile::tempdir().unwrap();
    let (metadata, config, descriptor, membership, epoch, manifest) =
        build_seed(refs[0], &refs, &format!("solo-race-{member_count}"), &source_temp);
    let source = source_temp.path().join("world");
    let seed = Seed {
        metadata: &metadata,
        config: &config,
        descriptor: &descriptor,
        membership: &membership,
        epoch: &epoch,
        manifest: &manifest,
        source: &source,
        authority: &refs[0].identity,
    };
    for peer in refs.iter().skip(1) {
        install_seed(peer, &seed);
    }

    let first_addr = address(refs[0]);
    let mut daemons = Vec::new();
    daemons.push(spawn_daemon(refs[0], &[]));
    thread::sleep(Duration::from_millis(400));
    for peer in refs.iter().skip(1) {
        daemons.push(spawn_daemon(peer, std::slice::from_ref(&first_addr)));
        thread::sleep(Duration::from_millis(250));
    }
    wait_until("initial authority permit", Duration::from_secs(25), || {
        permit(refs[0], metadata.world_id).is_some_and(|(ep, fence, heartbeat)| ep == 1 && fence == 1 && heartbeat >= 1)
    });

    let majority_start = if member_count == 3 { 1 } else { 2 };
    for daemon in daemons.iter_mut().skip(majority_start) {
        daemon.stop();
    }
    wait_until("old authority permit removal", Duration::from_secs(12), || {
        permit(refs[0], metadata.world_id).is_none()
    });
    let old_epoch = refs[0].storage.load_epoch_record(metadata.world_id).unwrap();
    assert_eq!(old_epoch.epoch_number, 1);
    assert_eq!(old_epoch.mode, EpochMode::Quorum);

    for daemon in daemons.iter_mut().take(majority_start) {
        daemon.stop();
    }
    let majority = &refs[majority_start..];
    let majority_addrs = majority.iter().map(|peer| address(peer)).collect::<Vec<_>>();
    let mut survivors = Vec::new();
    for (index, peer) in majority.iter().enumerate() {
        let bootstraps = majority_addrs.iter().take(index).cloned().collect::<Vec<_>>();
        survivors.push(spawn_daemon(peer, &bootstraps));
        thread::sleep(Duration::from_millis(300));
    }
    wait_until("majority recovery permit", Duration::from_secs(35), || {
        majority.iter().any(|peer| permit(peer, metadata.world_id).is_some_and(|(ep, fence, _)| ep == 2 && fence == 2))
    });
    assert!(survivors.iter_mut().all(ManagedChild::alive));
}

#[test]
fn three_daemon_unclean_quorum_loss_never_falls_back_to_solo() {
    assert_unclean_quorum_loss_does_not_enter_solo(3);
}

#[test]
fn five_daemon_unclean_quorum_loss_never_falls_back_to_solo() {
    assert_unclean_quorum_loss_does_not_enter_solo(5);
}

#[test]
fn stale_removed_voter_prepare_blocks_old_quorum_and_receives_revocation() {
    let a = peer_fixture();
    let b = peer_fixture();
    let c = peer_fixture();
    let d = peer_fixture();
    let e = peer_fixture();
    let source_temp = tempfile::tempdir().unwrap();
    let (metadata, config, descriptor, membership, epoch, manifest) =
        build_seed(&a, &[&a, &b, &c, &d, &e], "membership-5-to-3-removal", &source_temp);
    let source = source_temp.path().join("world");
    let seed = Seed {
        metadata: &metadata,
        config: &config,
        descriptor: &descriptor,
        membership: &membership,
        epoch: &epoch,
        manifest: &manifest,
        source: &source,
        authority: &a.identity,
    };
    for peer in [&b, &c, &d, &e] {
        install_seed(peer, &seed);
    }

    let mut proposed = MembershipRecordV1 {
        protocol_version: membership.protocol_version,
        world_id: metadata.world_id,
        epoch: membership.epoch,
        sequence: membership.sequence + 1,
        previous_membership_hash: Some(membership.record_hash().unwrap()),
        members: vec![member(&a), member(&b), member(&c)],
        authority_peer_id: a.identity.peer_id(),
        authority_public_key: a.identity.public_key(),
        signature: Vec::new(),
    };
    proposed.members.sort_by_key(|entry| entry.peer_id);
    a.identity.sign_membership(&mut proposed).unwrap();
    let proposal = MembershipProposalV1 { previous: membership.clone(), proposed: proposed.clone() };
    let signed_vote = |peer: &PeerFixture| {
        let mut vote = membership_vote_for(&proposal, peer.identity.peer_id(), peer.identity.public_key()).unwrap();
        vote.signature = peer.identity.sign(&vote.signing_bytes().unwrap());
        vote
    };
    let a_vote = signed_vote(&a);
    let c_vote = signed_vote(&c);
    let d_vote = signed_vote(&d);
    let certificate = swarm_protocol::MembershipCertificateV1 {
        proposal: proposal.clone(),
        votes: vec![a_vote, c_vote, d_vote.clone()],
    };
    swarm_consensus::validate_membership_certificate_shape(&certificate).unwrap();

    // D participated in the intersecting old quorum but missed the final commit.
    // Its durable prepare must therefore fence the stale {B,D,E} old majority.
    assert_eq!(d.storage.promise_membership_proposal(&proposal, &d_vote).unwrap(), MembershipPromiseResult::Accepted);
    a.storage.save_membership_certificate(&certificate).unwrap();
    a.storage.save_membership_record(&proposed).unwrap();
    let mut current_descriptor = descriptor.clone();
    current_descriptor.members = proposed.members.clone();
    current_descriptor.normalize();
    a.storage.save_world_descriptor(&current_descriptor).unwrap();

    let d_addr = address(&d);
    let mut daemon_d = spawn_daemon(&d, &[]);
    thread::sleep(Duration::from_millis(400));
    let mut daemon_b = spawn_daemon(&b, std::slice::from_ref(&d_addr));
    let mut daemon_e = spawn_daemon(&e, std::slice::from_ref(&d_addr));
    thread::sleep(Duration::from_secs(10));
    for peer in [&b, &d, &e] {
        assert!(permit(peer, metadata.world_id).is_none());
        assert_eq!(peer.storage.load_epoch_record(metadata.world_id).unwrap().epoch_number, 1);
    }
    assert!(d.storage.load_membership_promise(metadata.world_id).is_ok());
    assert!(daemon_b.alive() && daemon_d.alive() && daemon_e.alive());
    daemon_b.stop();
    daemon_d.stop();
    daemon_e.stop();

    // On reconnect the current authority must deliver the exact revocation
    // certificate even though D is no longer in the current descriptor.
    let a_addr = address(&a);
    let mut daemon_a = spawn_daemon(&a, &[]);
    thread::sleep(Duration::from_millis(500));
    let mut daemon_d = spawn_daemon(&d, std::slice::from_ref(&a_addr));
    wait_until("removed peer revocation certificate", Duration::from_secs(20), || {
        d.storage.load_membership_record(metadata.world_id).is_ok_and(|record| record == proposed)
            && d.storage
                .load_world_descriptor(metadata.world_id)
                .is_ok_and(|descriptor| descriptor.member(d.identity.peer_id()).is_none())
    });
    assert!(d.storage.load_membership_promise(metadata.world_id).is_err());
    assert!(permit(&d, metadata.world_id).is_none());
    assert!(daemon_a.alive() && daemon_d.alive());
}
