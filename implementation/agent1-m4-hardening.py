from pathlib import Path


def replace(path: str, old: str, new: str, count: int = 1) -> None:
    p = Path(path)
    text = p.read_text()
    found = text.count(old)
    if found != count:
        raise SystemExit(f"{path}: expected {count}, found {found}: {old[:140]!r}")
    p.write_text(text.replace(old, new, count))


daemon = "crates/swarm-cli/src/daemon.rs"
replace(
    daemon,
    "    RecoveryVoteV1, SnapshotManifestV1, SoloBranchV1, TransferPhase, WorldDescriptorV1, WorldId, WorldStatusV1,\n",
    "    RecoveryVoteV1, SnapshotManifestV1, TransferPhase, WorldDescriptorV1, WorldId, WorldStatusV1,\n",
)

old_push = '''    for metadata in storage.list_worlds()? {
        let Ok(descriptor) = storage.load_world_descriptor(metadata.world_id) else { continue };
        if descriptor.member(application_peer).is_none() {
            continue;
        }
        let local_is_authority =
            storage.load_epoch_record(metadata.world_id).is_ok_and(|epoch| epoch.authority_peer_id == local_peer);
        if !local_is_authority && !storage.background_seeding_enabled(metadata.world_id)? {
            continue;
        }

        if let Ok(membership) = storage.load_membership_record(metadata.world_id) {
            if let Ok(certificate) = storage.load_membership_certificate(metadata.world_id) {
                if certificate.proposal.proposed.record_hash()? == membership.record_hash()? {
                    let id = node.send_request(transport_peer, WireRequest::MembershipCommit(Box::new(certificate)))?;
                    outbound.insert(
                        request_key(&id),
                        OutboundContext::MembershipCommit {
                            world: metadata.world_id,
                            peer: application_peer,
                            sequence: membership.sequence,
                        },
                    );
                    continue;
                }
            }
        }
        push_committed_world_payload(storage, node, transport_peer, metadata.world_id, outbound, true)?;
    }
'''
new_push = '''    for metadata in storage.list_worlds()? {
        let world = metadata.world_id;
        let Ok(descriptor) = storage.load_world_descriptor(world) else { continue };
        let local_is_authority = storage.load_epoch_record(world).is_ok_and(|epoch| epoch.authority_peer_id == local_peer);
        if !local_is_authority && !storage.background_seeding_enabled(world)? {
            continue;
        }

        if let Ok(membership) = storage.load_membership_record(world) {
            if let Ok(certificate) = storage.load_membership_certificate(world) {
                if certificate.proposal.proposed.record_hash()? == membership.record_hash()?
                    && proposal_member(&certificate.proposal, application_peer).is_some()
                {
                    let id = node.send_request(transport_peer, WireRequest::MembershipCommit(Box::new(certificate)))?;
                    outbound.insert(
                        request_key(&id),
                        OutboundContext::MembershipCommit { world, peer: application_peer, sequence: membership.sequence },
                    );
                    continue;
                }
            }
        }
        if descriptor.member(application_peer).map_or(true, |member| member.banned) {
            continue;
        }
        push_committed_world_payload(storage, node, transport_peer, world, outbound, true)?;
    }
'''
replace(daemon, old_push, new_push)

replace(
    daemon,
    '''        (
            Some(OutboundContext::MembershipCommit { world, peer: _, sequence }),
            WireResponse::MembershipCommitAccepted { sequence: accepted },
        ) => {
            if accepted != sequence {
                return Err(anyhow!("membership commit acknowledgement sequence mismatch"));
            }
            let descriptor = storage.load_world_descriptor(world)?;
            if descriptor.member(local_peer).is_some() {
                push_committed_world_payload(storage, node, transport_peer, world, outbound, false)?;
            }
        }
''',
    '''        (
            Some(OutboundContext::MembershipCommit { world, peer, sequence }),
            WireResponse::MembershipCommitAccepted { sequence: accepted },
        ) => {
            if accepted != sequence {
                return Err(anyhow!("membership commit acknowledgement sequence mismatch"));
            }
            let descriptor = storage.load_world_descriptor(world)?;
            if descriptor.member(peer).is_some_and(|member| !member.banned) {
                push_committed_world_payload(storage, node, transport_peer, world, outbound, false)?;
            }
            info!(world = %world, peer = %peer, sequence, "peer acknowledged committed membership configuration");
        }
''',
)
# local_peer became response-handler dead state after checking the actual remote peer above.
replace(
    daemon,
    "    response: WireResponse,\n    local_peer: PeerId,\n    outbound: &mut HashMap<String, OutboundContext>,\n",
    "    response: WireResponse,\n    outbound: &mut HashMap<String, OutboundContext>,\n",
)
replace(
    daemon,
    "                            response,\n                            identity.peer_id(),\n                            &mut outbound,\n                            &mut leases,\n",
    "                            response,\n                            &mut outbound,\n                            &mut leases,\n",
)

replace(
    daemon,
    '''fn recover_committed_membership(storage: &Storage, identity: &PeerIdentity, world: WorldId) -> Result<()> {
    let Ok(certificate) = storage.load_membership_certificate(world) else {
        return Ok(());
    };
    validate_membership_certificate_for_local(storage, identity, &certificate)?;
    let current_matches = storage.load_membership_record(world).is_ok_and(|r| r == certificate.proposal.proposed);
    let has_matching_promise = storage
        .load_membership_promise(world)
        .is_ok_and(|p| p.proposal.proposal_hash().ok() == certificate.proposal.proposal_hash().ok());
    if !current_matches || has_matching_promise {
        apply_membership_certificate(storage, &certificate)?;
    }
    Ok(())
}
''',
    '''fn recover_committed_membership(storage: &Storage, identity: &PeerIdentity, world: WorldId) -> Result<()> {
    let Ok(certificate) = storage.load_membership_certificate(world) else {
        return Ok(());
    };
    if let Ok(current) = storage.load_membership_record(world) {
        verify_membership_signature(&current)?;
        if current == certificate.proposal.proposed {
            let _ = storage.clear_membership_promise_after_commit(world, current.record_hash()?)?;
            return Ok(());
        }
        if (current.epoch, current.sequence)
            > (certificate.proposal.proposed.epoch, certificate.proposal.proposed.sequence)
        {
            // Membership certificates are durable historical evidence. A later same-world
            // membership generation (for example recovery authority promotion) supersedes
            // this certificate and must not be rolled back on every daemon tick.
            return Ok(());
        }
    }
    validate_membership_certificate_for_local(storage, identity, &certificate)?;
    apply_membership_certificate(storage, &certificate)
}
''',
)

replace(
    daemon,
    '''    runtime.membership_votes.retain(|(w, _), _| *w != world);
    for member in certificate.proposal.proposed.members.iter().filter(|m| !m.banned && m.peer_id != identity.peer_id())
    {
        let Some((transport_peer, _)) =
            runtime.authenticated_peers.iter().find(|(_, application_peer)| **application_peer == member.peer_id)
        else {
            continue;
        };
        let id = node.send_request(transport_peer, WireRequest::MembershipCommit(Box::new(certificate.clone())))?;
        outbound.insert(
            request_key(&id),
            OutboundContext::MembershipCommit {
                world,
                peer: member.peer_id,
                sequence: certificate.proposal.proposed.sequence,
            },
        );
    }
''',
    '''    runtime.membership_votes.retain(|(w, _), _| *w != world);
    let mut notified = HashSet::new();
    for member in certificate
        .proposal
        .previous
        .members
        .iter()
        .chain(certificate.proposal.proposed.members.iter())
    {
        if member.peer_id == identity.peer_id() || !notified.insert(member.peer_id) {
            continue;
        }
        let Some((transport_peer, _)) =
            runtime.authenticated_peers.iter().find(|(_, application_peer)| **application_peer == member.peer_id)
        else {
            continue;
        };
        let id = node.send_request(transport_peer, WireRequest::MembershipCommit(Box::new(certificate.clone())))?;
        outbound.insert(
            request_key(&id),
            OutboundContext::MembershipCommit {
                world,
                peer: member.peer_id,
                sequence: certificate.proposal.proposed.sequence,
            },
        );
    }
''',
)

migration = "crates/swarm-cli/src/migration.rs"
replace(
    migration,
    "            sequence: latest.sequence.saturating_add(1),\n",
    "            sequence: latest.sequence.checked_add(1).context(\"snapshot sequence counter exhausted\")?,\n",
    count=2,
)
replace(
    migration,
    '''    if current.authority_peer_id != committed.from_peer_id
        || current.epoch_number.saturating_add(1) != committed.next_epoch
        || current.fencing_token.saturating_add(1) != committed.next_fencing_token
    {
        bail!("committed transfer no longer advances the current authority generation");
    }
''',
    '''    let expected_generation = AuthorityGeneration {
        epoch: current.epoch_number,
        fencing_token: current.fencing_token,
    }
    .checked_next()
    .context("accepted authority generation is exhausted")?;
    if current.authority_peer_id != committed.from_peer_id
        || expected_generation.epoch != committed.next_epoch
        || expected_generation.fencing_token != committed.next_fencing_token
    {
        bail!("committed transfer no longer advances the current authority generation");
    }
''',
)
replace(
    migration,
    '''    if next.epoch_number != current.epoch_number.saturating_add(1)
        || next.fencing_token != current.fencing_token.saturating_add(1)
        || next.previous_epoch_hash != Some(epoch_record_hash(&current)?)
    {
        bail!("observed epoch does not directly advance the local accepted generation");
    }
''',
    '''    let expected_generation = AuthorityGeneration {
        epoch: current.epoch_number,
        fencing_token: current.fencing_token,
    }
    .checked_next()
    .context("accepted authority generation is exhausted")?;
    if next.epoch_number != expected_generation.epoch
        || next.fencing_token != expected_generation.fencing_token
        || next.previous_epoch_hash != Some(epoch_record_hash(&current)?)
    {
        bail!("observed epoch does not directly advance the local accepted generation");
    }
''',
)

sim = "crates/swarm-consensus/src/simulator.rs"
replace(sim, "use crate::FencingState;\n", "use crate::{AuthorityGeneration, FencingState};\n")
replace(
    sim,
    '''    #[error("stale authority generation")]
    StaleGeneration,
''',
    '''    #[error("stale authority generation")]
    StaleGeneration,
    #[error("authority generation counter exhausted")]
    GenerationExhausted,
''',
)
replace(
    sim,
    '''        if !recovery.epoch_committed {
            self.epoch = self.epoch.saturating_add(1);
            self.fencing
                .advance(self.fencing.accepted_token().saturating_add(1))
                .expect("simulator only increments fencing tokens");
            recovery.epoch_committed = true;
''',
    '''        if !recovery.epoch_committed {
            let next = AuthorityGeneration { epoch: self.epoch, fencing_token: self.fencing.accepted_token() }
                .checked_next()
                .map_err(|_| SimError::GenerationExhausted)?;
            self.epoch = next.epoch;
            self.fencing.advance(next.fencing_token).map_err(|_| SimError::GenerationExhausted)?;
            recovery.epoch_committed = true;
''',
)
replace(
    sim,
    '''        self.epoch = self.epoch.saturating_add(1);
        self.fencing
            .advance(self.fencing.accepted_token().saturating_add(1))
            .expect("simulator only increments fencing tokens");
        self.state = SimWorldState::Active { authority: candidate };
''',
    '''        let next = AuthorityGeneration { epoch: self.epoch, fencing_token: self.fencing.accepted_token() }
            .checked_next()
            .map_err(|_| SimError::GenerationExhausted)?;
        self.epoch = next.epoch;
        self.fencing.advance(next.fencing_token).map_err(|_| SimError::GenerationExhausted)?;
        self.state = SimWorldState::Active { authority: candidate };
''',
)
replace(
    sim,
    '''    fn peer(id: u8, hash: Hash32) -> SimPeer {
''',
    '''    fn force_generation(sim: &mut FailureSimulator, generation: AuthorityGeneration) {
        sim.epoch = generation.epoch;
        sim.fencing = FencingState::new(generation.fencing_token);
    }

    fn peer(id: u8, hash: Hash32) -> SimPeer {
''',
)
p = Path(sim)
text = p.read_text()
idx = text.rfind("\n}\n")
if idx < 0:
    raise SystemExit("simulator tests closing brace not found")
extra = r'''

    #[test]
    fn recovery_generation_exhaustion_fails_closed_in_legacy_simulator() {
        let hash = Hash32([0x44; 32]);
        let mut sim = FailureSimulator::new(peer(1, hash), [peer(2, hash), peer(3, hash)], 1_000);
        force_generation(&mut sim, AuthorityGeneration { epoch: u64::MAX, fencing_token: u64::MAX });
        sim.set_online(PeerId([1; 32]), false).unwrap();
        sim.begin_recovery(PeerId([2; 32]), 1_000, 2).unwrap();
        sim.acknowledge_reservation(PeerId([3; 32])).unwrap();
        assert_eq!(sim.commit_recovery_epoch(), Err(SimError::GenerationExhausted));
        assert_eq!(sim.epoch(), u64::MAX);
        assert_eq!(sim.fencing_token(), u64::MAX);
    }
'''
p.write_text(text[:idx] + extra + text[idx:])
