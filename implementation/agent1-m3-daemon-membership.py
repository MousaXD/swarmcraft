from pathlib import Path


def between(path, start, end, new):
    p=Path(path); s=p.read_text(); a=s.find(start); b=s.find(end, a+len(start)) if a>=0 else -1
    if a<0 or b<0: raise SystemExit(f"markers missing: {start!r} / {end!r}")
    p.write_text(s[:a]+new+s[b:])


path="crates/swarm-cli/src/daemon.rs"
between(path,"        WireRequest::JoinRequest(request) => {\n","        WireRequest::LeaveRequest(request) => {\n",'''        WireRequest::JoinRequest(request) => {
            let request = *request;
            if application_peer != request.joining_member.peer_id {
                return Err(anyhow!("join request transport identity does not match joining peer"));
            }
            verify_join_request_signature(&request)?;
            verify_invite_signature(&request.invite)?;
            if request.invite.expires_unix_ms < unix_millis()? { return Err(anyhow!("invite has expired")); }
            let world = request.world_id;
            let metadata = storage.load_world(world)?;
            if request.invite.genesis.world_id()? != world
                || request.invite.genesis.compatibility_fingerprint != metadata.genesis.compatibility_fingerprint
            { return Err(anyhow!("invite does not match local world genesis")); }
            let current = storage.load_membership_record(world)?;
            verify_membership_signature(&current)?;
            if current.authority_peer_id != identity.peer_id() || current.authority_public_key != identity.public_key() {
                return Err(anyhow!("only the current local authority may accept a join request"));
            }
            if request.invite.inviter_peer_id != current.authority_peer_id
                || request.invite.inviter_public_key != current.authority_public_key
            { return Err(anyhow!("join invite was not issued by the current authority")); }
            let descriptor = storage.load_world_descriptor(world)?;
            let inviter = descriptor.member(request.invite.inviter_peer_id).context("invite signer is not a current world member")?;
            if inviter.banned || inviter.public_key != request.invite.inviter_public_key {
                return Err(anyhow!("invite signer is banned or key does not match current membership"));
            }
            if let Some(member) = descriptor.member(request.joining_member.peer_id) {
                if member.public_key != request.joining_member.public_key || member.banned {
                    return Err(anyhow!("joining peer conflicts with existing membership"));
                }
                node.respond(channel, WireResponse::JoinAccepted { membership_sequence: current.sequence })?;
                push_known_worlds(storage, node, &transport_peer, application_peer, identity.peer_id(), state.outbound)?;
                return Ok(());
            }
            if let Ok(promise) = storage.load_membership_promise(world) {
                let proposed_member = promise.proposal.proposed.members.iter()
                    .find(|member| member.peer_id == request.joining_member.peer_id);
                if promise.proposal.previous.record_hash()? == current.record_hash()?
                    && proposed_member == Some(&request.joining_member)
                {
                    let id = node.send_request(&transport_peer, WireRequest::MembershipProposal(Box::new(promise.proposal.clone())))?;
                    state.outbound.insert(request_key(&id), OutboundContext::MembershipProposal {
                        world, peer: application_peer, proposal_hash: promise.proposal.proposal_hash()?,
                    });
                    node.respond(channel, WireResponse::JoinAccepted { membership_sequence: promise.proposal.proposed.sequence })?;
                    return Ok(());
                }
                return Err(anyhow!("another membership transition is already durably prepared"));
            }
            let mut proposed_descriptor = descriptor.clone();
            proposed_descriptor.members.push(request.joining_member.clone());
            proposed_descriptor.normalize();
            let mut next = MembershipRecordV1 {
                protocol_version: current.protocol_version,
                world_id: world,
                epoch: current.epoch,
                sequence: current.sequence.checked_add(1).context("membership sequence counter exhausted")?,
                previous_membership_hash: Some(current.record_hash()?),
                members: proposed_descriptor.members,
                authority_peer_id: identity.peer_id(),
                authority_public_key: identity.public_key(),
                signature: Vec::new(),
            };
            identity.sign_membership(&mut next)?;
            let proposal = MembershipProposalV1 { previous: current, proposed: next };
            validate_membership_proposal_shape(&proposal)?;
            let vote = sign_membership_vote(identity, &proposal)?;
            match storage.promise_membership_proposal(&proposal, &vote)? {
                MembershipPromiseResult::Accepted | MembershipPromiseResult::Idempotent => {}
                MembershipPromiseResult::Rejected => return Err(anyhow!("membership proposal conflicts with a durable prepare")),
            }
            state.leases.membership_votes.insert((world, identity.peer_id()), vote);
            clear_permit(context.paths, world)?;
            state.leases.permit_heartbeats.remove(&world);
            let id = node.send_request(&transport_peer, WireRequest::MembershipProposal(Box::new(proposal.clone())))?;
            state.outbound.insert(request_key(&id), OutboundContext::MembershipProposal {
                world, peer: application_peer, proposal_hash: proposal.proposal_hash()?,
            });
            node.respond(channel, WireResponse::JoinAccepted { membership_sequence: proposal.proposed.sequence })?;
        }
''')

between(path,"        WireRequest::LeaveRequest(request) => {\n","        WireRequest::SnapshotManifest(manifest) => {\n",'''        WireRequest::LeaveRequest(request) => {
            let request = *request;
            if application_peer != request.leaving_peer_id {
                return Err(anyhow!("leave request transport identity does not match leaving peer"));
            }
            verify_leave_request_signature(&request)?;
            let current = storage.load_membership_record(request.world_id)?;
            verify_membership_signature(&current)?;
            if current.authority_peer_id != identity.peer_id() || current.authority_public_key != identity.public_key() {
                return Err(anyhow!("only the current local authority may accept a leave request"));
            }
            if request.leaving_peer_id == current.authority_peer_id {
                return Err(anyhow!("authority must transfer authority before leaving"));
            }
            if current.record_hash()? != request.membership_hash { return Err(anyhow!("leave request references stale membership")); }
            let descriptor = storage.load_world_descriptor(request.world_id)?;
            let leaving = descriptor.member(request.leaving_peer_id).context("leaving peer is not a current world member")?;
            if leaving.banned || leaving.public_key != request.leaving_public_key {
                return Err(anyhow!("leaving peer key does not match current membership"));
            }
            if let Ok(promise) = storage.load_membership_promise(request.world_id) {
                let absent = promise.proposal.proposed.members.iter().all(|member| member.peer_id != request.leaving_peer_id);
                if promise.proposal.previous.record_hash()? == current.record_hash()? && absent {
                    let id = node.send_request(&transport_peer, WireRequest::MembershipProposal(Box::new(promise.proposal.clone())))?;
                    state.outbound.insert(request_key(&id), OutboundContext::MembershipProposal {
                        world: request.world_id, peer: application_peer, proposal_hash: promise.proposal.proposal_hash()?,
                    });
                    node.respond(channel, WireResponse::LeaveAccepted { membership_sequence: promise.proposal.proposed.sequence })?;
                    return Ok(());
                }
                return Err(anyhow!("another membership transition is already durably prepared"));
            }
            let mut proposed_descriptor = descriptor.clone();
            proposed_descriptor.members.retain(|member| member.peer_id != request.leaving_peer_id);
            proposed_descriptor.normalize();
            let mut next = MembershipRecordV1 {
                protocol_version: current.protocol_version,
                world_id: current.world_id,
                epoch: current.epoch,
                sequence: current.sequence.checked_add(1).context("membership sequence counter exhausted")?,
                previous_membership_hash: Some(current.record_hash()?),
                members: proposed_descriptor.members,
                authority_peer_id: identity.peer_id(),
                authority_public_key: identity.public_key(),
                signature: Vec::new(),
            };
            identity.sign_membership(&mut next)?;
            let proposal = MembershipProposalV1 { previous: current, proposed: next };
            validate_membership_proposal_shape(&proposal)?;
            let vote = sign_membership_vote(identity, &proposal)?;
            match storage.promise_membership_proposal(&proposal, &vote)? {
                MembershipPromiseResult::Accepted | MembershipPromiseResult::Idempotent => {}
                MembershipPromiseResult::Rejected => return Err(anyhow!("membership proposal conflicts with a durable prepare")),
            }
            state.leases.membership_votes.insert((request.world_id, identity.peer_id()), vote);
            clear_permit(context.paths, request.world_id)?;
            state.leases.permit_heartbeats.remove(&request.world_id);
            let id = node.send_request(&transport_peer, WireRequest::MembershipProposal(Box::new(proposal.clone())))?;
            state.outbound.insert(request_key(&id), OutboundContext::MembershipProposal {
                world: request.world_id, peer: application_peer, proposal_hash: proposal.proposal_hash()?,
            });
            node.respond(channel, WireResponse::LeaveAccepted { membership_sequence: proposal.proposed.sequence })?;
        }
''')

between(path,"        WireRequest::Membership(record) => {\n","        WireRequest::WorldConfig(config) => {\n",'''        WireRequest::MembershipProposal(proposal) => {
            let proposal = *proposal;
            if application_peer != proposal.proposed.authority_peer_id {
                return Err(anyhow!("membership proposal sender is not its signed authority"));
            }
            verify_membership_signature(&proposal.previous)?;
            verify_membership_signature(&proposal.proposed)?;
            validate_membership_proposal_shape(&proposal)?;
            validate_membership_proposal_for_local(storage, identity, &proposal)?;
            let vote = sign_membership_vote(identity, &proposal)?;
            let durable_vote = match storage.promise_membership_proposal(&proposal, &vote)? {
                MembershipPromiseResult::Accepted => vote,
                MembershipPromiseResult::Idempotent => storage.load_membership_promise(proposal.proposed.world_id)?.vote,
                MembershipPromiseResult::Rejected => return Err(anyhow!("membership proposal conflicts with this peer's durable prepare")),
            };
            clear_permit(context.paths, proposal.proposed.world_id)?;
            state.leases.permit_heartbeats.remove(&proposal.proposed.world_id);
            node.respond(channel, WireResponse::MembershipVote(Box::new(durable_vote)))?;
        }
        WireRequest::MembershipCommit(certificate) => {
            let certificate = *certificate;
            let world = certificate.proposal.proposed.world_id;
            if application_peer != certificate.proposal.proposed.authority_peer_id {
                return Err(anyhow!("membership commit sender is not its signed authority"));
            }
            validate_membership_certificate_for_local(storage, identity, &certificate)?;
            storage.save_membership_certificate(&certificate)?;
            apply_membership_certificate(storage, &certificate)?;
            clear_permit(context.paths, world)?;
            state.leases.permit_heartbeats.remove(&world);
            state.leases.membership_votes.retain(|(vote_world, _), _| *vote_world != world);
            node.respond(channel, WireResponse::MembershipCommitAccepted { sequence: certificate.proposal.proposed.sequence })?;
        }
        WireRequest::Membership(record) => {
            verify_membership_signature(&record)?;
            authorize_member(storage, record.world_id, record.authority_peer_id)?;
            if let Ok(epoch) = storage.load_epoch_record(record.world_id) {
                if record.epoch > epoch.epoch_number {
                    return Err(anyhow!("membership cannot advance before its authority epoch is accepted"));
                }
                if record.epoch == epoch.epoch_number
                    && (record.authority_peer_id != epoch.authority_peer_id
                        || record.authority_public_key != epoch.authority_public_key)
                { return Err(anyhow!("membership authority does not match the accepted epoch")); }
            }
            if let Ok(current) = storage.load_membership_record(record.world_id) {
                if record == current {
                    node.respond(channel, WireResponse::MembershipAccepted { sequence: record.sequence })?;
                    return Ok(());
                }
                if record.members != current.members {
                    return Err(anyhow!("membership voter-set changes require a joint membership certificate"));
                }
                if record.previous_membership_hash != Some(current.record_hash()?)
                    || record.sequence != current.sequence.checked_add(1).context("membership sequence counter exhausted")?
                { return Err(anyhow!("same-voter membership record must directly extend the committed membership")); }
                if record.epoch < current.epoch { return Err(anyhow!("stale membership record rejected")); }
            } else if record.sequence != 1 || record.previous_membership_hash.is_some() {
                return Err(anyhow!("non-genesis membership requires a joint membership certificate"));
            }
            let mut descriptor = storage.load_world_descriptor(record.world_id)?;
            descriptor.members = record.members.clone();
            descriptor.normalize();
            storage.save_membership_record(&record)?;
            storage.save_world_descriptor(&descriptor)?;
            clear_satisfied_pending_membership(storage, &descriptor)?;
            node.respond(channel, WireResponse::MembershipAccepted { sequence: record.sequence })?;
        }
''')
