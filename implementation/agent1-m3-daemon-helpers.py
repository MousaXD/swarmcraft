from pathlib import Path


def replace(path, old, new, count=1):
    p=Path(path); s=p.read_text(); n=s.count(old)
    if n != count: raise SystemExit(f"{path}: expected {count}, found {n}: {old[:100]!r}")
    p.write_text(s.replace(old,new,count))

path="crates/swarm-cli/src/daemon.rs"

# Fence authority-generation messages while a membership prepare is durable.
fences = [
("        WireRequest::RecoveryBallot(ballot) => {\n            let ballot = *ballot;\n",
 "        WireRequest::RecoveryBallot(ballot) => {\n            let ballot = *ballot;\n            ensure_no_membership_prepare(storage, ballot.world_id)?;\n"),
("        WireRequest::RecoveryEpoch { record, certificate } => {\n            let certificate = *certificate;\n",
 "        WireRequest::RecoveryEpoch { record, certificate } => {\n            let certificate = *certificate;\n            ensure_no_membership_prepare(storage, record.world_id)?;\n"),
("        WireRequest::Epoch(record) => {\n",
 "        WireRequest::Epoch(record) => {\n            ensure_no_membership_prepare(storage, record.world_id)?;\n"),
("        WireRequest::AuthorityTransfer(transfer) => {\n",
 "        WireRequest::AuthorityTransfer(transfer) => {\n            ensure_no_membership_prepare(storage, transfer.world_id)?;\n"),
("        WireRequest::LeaseGrant(lease) => {\n",
 "        WireRequest::LeaseGrant(lease) => {\n            ensure_no_membership_prepare(storage, lease.world_id)?;\n"),
("        WireRequest::Sleep(record) => {\n",
 "        WireRequest::Sleep(record) => {\n            ensure_no_membership_prepare(storage, record.world_id)?;\n"),
]
for old,new in fences: replace(path,old,new)

replace(path,
"    response: WireResponse,\n    runtime: &mut LeaseRuntime,\n    now: Instant,\n",
"    response: WireResponse,\n    local_peer: PeerId,\n    outbound: &mut HashMap<String, OutboundContext>,\n    runtime: &mut LeaseRuntime,\n    now: Instant,\n")
replace(path,
"        (\n            Some(OutboundContext::Epoch { world, peer, generation }),\n            WireResponse::EpochAccepted { epoch, fencing_token },\n        ) => {\n",
"        (Some(OutboundContext::MembershipProposal { world, peer, proposal_hash }), WireResponse::MembershipVote(vote)) => {\n            let vote = *vote;\n            verify_membership_vote_signature(&vote)?;\n            let promise = storage.load_membership_promise(world)?;\n            if promise.proposal.proposal_hash()? != proposal_hash\n                || !vote.matches_proposal(&promise.proposal)?\n                || vote.voter_peer_id != peer\n            {\n                return Err(anyhow!(\"membership vote does not match the active proposal or authenticated peer\"));\n            }\n            runtime.membership_votes.insert((world, peer), vote);\n        }\n        (\n            Some(OutboundContext::MembershipCommit { world, peer: _, sequence }),\n            WireResponse::MembershipCommitAccepted { sequence: accepted },\n        ) => {\n            if accepted != sequence {\n                return Err(anyhow!(\"membership commit acknowledgement sequence mismatch\"));\n            }\n            let descriptor = storage.load_world_descriptor(world)?;\n            if descriptor.member(local_peer).is_some() {\n                push_committed_world_payload(storage, node, transport_peer, world, outbound, false)?;\n            }\n        }\n        (\n            Some(OutboundContext::Epoch { world, peer, generation }),\n            WireResponse::EpochAccepted { epoch, fencing_token },\n        ) => {\n")

helpers=r'''fn sign_membership_vote(identity: &PeerIdentity, proposal: &MembershipProposalV1) -> Result<MembershipVoteV1> {
    let mut vote = membership_vote_for(proposal, identity.peer_id(), identity.public_key())?;
    vote.signature = identity.sign(&vote.signing_bytes()?);
    Ok(vote)
}

fn verify_membership_vote_signature(vote: &MembershipVoteV1) -> Result<()> {
    verify_signature(vote.voter_peer_id, vote.voter_public_key, &vote.signing_bytes()?, &vote.signature)?;
    Ok(())
}

fn ensure_no_membership_prepare(storage: &Storage, world: WorldId) -> Result<()> {
    if storage.load_membership_promise(world).is_ok() {
        return Err(anyhow!("authority/recovery transition is fenced by a durable membership prepare"));
    }
    Ok(())
}

fn proposal_member<'a>(proposal: &'a MembershipProposalV1, peer: PeerId) -> Option<&'a swarm_protocol::WorldMemberV1> {
    proposal.previous.members.iter().chain(proposal.proposed.members.iter())
        .find(|member| member.peer_id == peer && !member.banned)
}

fn validate_membership_proposal_for_local(
    storage: &Storage,
    identity: &PeerIdentity,
    proposal: &MembershipProposalV1,
) -> Result<()> {
    let world = proposal.proposed.world_id;
    let local_member = proposal_member(proposal, identity.peer_id())
        .context("local peer is not an active voter in either membership configuration")?;
    if local_member.public_key != identity.public_key() {
        return Err(anyhow!("local peer key does not match the membership proposal"));
    }
    if let Ok(current) = storage.load_membership_record(world) {
        verify_membership_signature(&current)?;
        if current.record_hash()? != proposal.previous.record_hash()? {
            return Err(anyhow!("membership proposal does not extend the locally committed configuration"));
        }
        return Ok(());
    }
    let join = storage.load_pending_join(world)
        .context("new membership voter has neither committed membership nor a pending join")?;
    verify_join_request_signature(&join)?;
    verify_invite_signature(&join.invite)?;
    if join.invite.expires_unix_ms < unix_millis()? {
        return Err(anyhow!("pending join invite expired before membership commit"));
    }
    if join.joining_member.peer_id != identity.peer_id()
        || join.joining_member.public_key != identity.public_key()
        || proposal.proposed.members.iter().find(|m| m.peer_id == identity.peer_id()) != Some(&join.joining_member)
        || join.invite.inviter_peer_id != proposal.previous.authority_peer_id
        || join.invite.inviter_public_key != proposal.previous.authority_public_key
    {
        return Err(anyhow!("membership proposal does not match the locally staged join"));
    }
    Ok(())
}

fn validate_membership_certificate_signatures(certificate: &MembershipCertificateV1) -> Result<()> {
    verify_membership_signature(&certificate.proposal.previous)?;
    verify_membership_signature(&certificate.proposal.proposed)?;
    validate_membership_certificate_shape(certificate)?;
    for vote in &certificate.votes { verify_membership_vote_signature(vote)?; }
    Ok(())
}

fn validate_membership_certificate_for_local(
    storage: &Storage,
    identity: &PeerIdentity,
    certificate: &MembershipCertificateV1,
) -> Result<()> {
    validate_membership_certificate_signatures(certificate)?;
    let proposal=&certificate.proposal;
    let world=proposal.proposed.world_id;
    if let Ok(current)=storage.load_membership_record(world) {
        verify_membership_signature(&current)?;
        if current != proposal.proposed && current.record_hash()? != proposal.previous.record_hash()? {
            return Err(anyhow!("membership certificate does not extend the locally committed configuration"));
        }
    } else {
        validate_membership_proposal_for_local(storage, identity, proposal)?;
    }
    if let Ok(promise)=storage.load_membership_promise(world) {
        if promise.proposal.proposal_hash()? != proposal.proposal_hash()? {
            return Err(anyhow!("membership certificate conflicts with this peer's durable prepare"));
        }
    }
    Ok(())
}

fn clear_satisfied_pending_membership(storage: &Storage, descriptor: &WorldDescriptorV1) -> Result<()> {
    if let Ok(join)=storage.load_pending_join(descriptor.world_id) {
        if descriptor.member(join.joining_member.peer_id)
            .is_some_and(|m| m.public_key == join.joining_member.public_key && !m.banned)
        { storage.clear_pending_join(descriptor.world_id)?; }
    }
    if let Ok(leave)=storage.load_pending_leave(descriptor.world_id) {
        if descriptor.member(leave.leaving_peer_id).is_none() { storage.clear_pending_leave(descriptor.world_id)?; }
    }
    Ok(())
}

fn apply_membership_certificate(storage: &Storage, certificate: &MembershipCertificateV1) -> Result<()> {
    validate_membership_certificate_signatures(certificate)?;
    let proposal=&certificate.proposal;
    let world=proposal.proposed.world_id;
    if let Ok(current)=storage.load_membership_record(world) {
        if current != proposal.proposed && current.record_hash()? != proposal.previous.record_hash()? {
            return Err(anyhow!("cannot apply membership certificate over an unrelated committed configuration"));
        }
    } else {
        let join=storage.load_pending_join(world)
            .context("cannot bootstrap non-genesis membership without a pending join")?;
        if proposal.proposed.members.iter().find(|m| m.peer_id == join.joining_member.peer_id) != Some(&join.joining_member) {
            return Err(anyhow!("membership certificate does not contain the locally pending join"));
        }
    }
    let mut descriptor=storage.load_world_descriptor(world)?;
    descriptor.members=proposal.proposed.members.clone();
    descriptor.normalize();
    storage.save_membership_record(&proposal.proposed)?;
    storage.save_world_descriptor(&descriptor)?;
    let _=storage.clear_membership_promise_after_commit(world, proposal.proposed.record_hash()?)?;
    clear_satisfied_pending_membership(storage, &descriptor)?;
    Ok(())
}

fn recover_committed_membership(storage: &Storage, identity: &PeerIdentity, world: WorldId) -> Result<()> {
    let Ok(certificate)=storage.load_membership_certificate(world) else { return Ok(()); };
    validate_membership_certificate_for_local(storage, identity, &certificate)?;
    let current_matches=storage.load_membership_record(world).is_ok_and(|r| r == certificate.proposal.proposed);
    let has_matching_promise=storage.load_membership_promise(world).is_ok_and(|p| {
        p.proposal.proposal_hash().ok() == certificate.proposal.proposal_hash().ok()
    });
    if !current_matches || has_matching_promise { apply_membership_certificate(storage, &certificate)?; }
    Ok(())
}

fn membership_proposal_request_pending(
    outbound: &HashMap<String, OutboundContext>, world: WorldId, peer: PeerId, proposal_hash: Hash32,
) -> bool {
    outbound.values().any(|context| matches!(context,
        OutboundContext::MembershipProposal { world: w, peer: p, proposal_hash: h }
        if *w == world && *p == peer && *h == proposal_hash))
}

fn maintain_membership_transition(
    storage: &Storage,
    identity: &PeerIdentity,
    node: &mut SwarmNode,
    outbound: &mut HashMap<String, OutboundContext>,
    runtime: &mut LeaseRuntime,
    promise: &DurableMembershipPromiseV1,
) -> Result<()> {
    let proposal=&promise.proposal;
    let world=proposal.proposed.world_id;
    if proposal.proposed.authority_peer_id != identity.peer_id()
        || proposal.proposed.authority_public_key != identity.public_key() { return Ok(()); }
    let current=storage.load_membership_record(world)?;
    if current == proposal.proposed {
        let _=storage.clear_membership_promise_after_commit(world, current.record_hash()?)?;
        return Ok(());
    }
    if current.record_hash()? != proposal.previous.record_hash()? {
        return Err(anyhow!("durable membership proposal no longer extends the committed membership"));
    }
    verify_membership_vote_signature(&promise.vote)?;
    runtime.membership_votes.insert((world, identity.peer_id()), promise.vote.clone());
    let proposal_hash=proposal.proposal_hash()?;
    let mut sent=HashSet::new();
    for member in proposal.previous.members.iter().chain(proposal.proposed.members.iter()) {
        if member.banned || member.peer_id == identity.peer_id() || !sent.insert(member.peer_id) { continue; }
        let Some((transport_peer,_))=runtime.authenticated_peers.iter()
            .find(|(_,application_peer)| **application_peer == member.peer_id) else { continue; };
        if membership_proposal_request_pending(outbound, world, member.peer_id, proposal_hash) { continue; }
        let id=node.send_request(transport_peer, WireRequest::MembershipProposal(Box::new(proposal.clone())))?;
        outbound.insert(request_key(&id), OutboundContext::MembershipProposal {
            world, peer: member.peer_id, proposal_hash,
        });
    }
    let votes=runtime.membership_votes.iter().filter_map(|((w,_),vote)| {
        (*w == world && vote.matches_proposal(proposal).ok() == Some(true)).then_some(vote.clone())
    }).collect::<Vec<_>>();
    let certificate=MembershipCertificateV1 { proposal: proposal.clone(), votes };
    match validate_membership_certificate_shape(&certificate) {
        Ok(()) => {}
        Err(MembershipConsensusError::OldQuorumUnavailable { .. }
            | MembershipConsensusError::NewQuorumUnavailable { .. }) => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    for vote in &certificate.votes { verify_membership_vote_signature(vote)?; }
    storage.save_membership_certificate(&certificate)?;
    apply_membership_certificate(storage, &certificate)?;
    runtime.membership_votes.retain(|(w,_),_| *w != world);
    for member in certificate.proposal.proposed.members.iter()
        .filter(|m| !m.banned && m.peer_id != identity.peer_id())
    {
        let Some((transport_peer,_))=runtime.authenticated_peers.iter()
            .find(|(_,application_peer)| **application_peer == member.peer_id) else { continue; };
        let id=node.send_request(transport_peer, WireRequest::MembershipCommit(Box::new(certificate.clone())))?;
        outbound.insert(request_key(&id), OutboundContext::MembershipCommit {
            world, peer: member.peer_id, sequence: certificate.proposal.proposed.sequence,
        });
    }
    info!(world=%world, sequence=certificate.proposal.proposed.sequence, "joint membership configuration committed");
    Ok(())
}

'''
replace(path,"fn validate_generation_response(\n",helpers+"fn validate_generation_response(\n")

replace(path,
"    if let Ok(epoch) = storage.load_epoch_record(transfer.world_id) {\n        if transfer.next_epoch != epoch.epoch_number.saturating_add(1)\n            || transfer.next_fencing_token != epoch.fencing_token.saturating_add(1)\n        {\n            return Err(anyhow!(\"transfer generation does not advance the accepted epoch exactly once\"));\n        }\n    }\n",
"    if let Ok(epoch) = storage.load_epoch_record(transfer.world_id) {\n        let expected = AuthorityGeneration { epoch: epoch.epoch_number, fencing_token: epoch.fencing_token }\n            .checked_next()\n            .context(\"accepted authority generation is exhausted\")?;\n        if transfer.next_epoch != expected.epoch || transfer.next_fencing_token != expected.fencing_token {\n            return Err(anyhow!(\"transfer generation does not advance the accepted epoch exactly once\"));\n        }\n    }\n")
