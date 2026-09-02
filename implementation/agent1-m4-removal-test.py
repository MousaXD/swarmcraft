from pathlib import Path

path = Path("crates/swarm-cli/tests/consensus_partition_safety.rs")
text = path.read_text()
if "stale_removed_voter_prepare_blocks_old_quorum_and_receives_revocation" in text:
    raise SystemExit("removal regression already present")
append = r'''

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
    assert_eq!(
        d.storage.promise_membership_proposal(&proposal, &d_vote).unwrap(),
        MembershipPromiseResult::Accepted
    );
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
'''
path.write_text(text + append)
