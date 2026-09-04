from pathlib import Path


def replace(path, old, new):
    p = Path(path)
    text = p.read_text()
    if old not in text:
        if new in text:
            return
        raise SystemExit(f"missing repair anchor in {path}: {old[:120]!r}")
    p.write_text(text.replace(old, new, 1))


def append_before(path, marker, block):
    p = Path(path)
    text = p.read_text()
    if block.strip() in text:
        return
    if marker not in text:
        raise SystemExit(f"missing insertion marker in {path}: {marker!r}")
    p.write_text(text.replace(marker, block + "\n" + marker, 1))

# Serde tuple implementations stop below the expanded announcement field count.
# Keep a stable canonical postcard representation by nesting the signed state in
# two ordered tuples rather than dropping any binding.
replace(
    "crates/swarm-protocol/src/discovery.rs",
    '''        let unsigned = (\n            self.protocol_version,\n            self.world_id,\n            presentation,\n            &self.compatibility,\n            self.visibility,\n            self.membership_policy,\n            self.config_sequence,\n            self.config_hash,\n            self.membership_sequence,\n            self.membership_hash,\n            self.authority_epoch,\n            self.fencing_token,\n            self.canonical_head,\n            self.announcement_sequence,\n            self.issued_unix_ms,\n            self.expires_unix_ms,\n            self.announcer_peer_id,\n            self.announcer_public_key,\n        );\n''',
    '''        let unsigned = (\n            (\n                self.protocol_version,\n                self.world_id,\n                presentation,\n                &self.compatibility,\n                self.visibility,\n                self.membership_policy,\n                self.config_sequence,\n                self.config_hash,\n                self.membership_sequence,\n                self.membership_hash,\n                self.authority_epoch,\n                self.fencing_token,\n            ),\n            (\n                self.canonical_head,\n                self.announcement_sequence,\n                self.issued_unix_ms,\n                self.expires_unix_ms,\n                self.announcer_peer_id,\n                self.announcer_public_key,\n            ),\n        );\n''',
)

# A pending new voter must be reachable as an untrusted exact-world locator so
# the verifier can satisfy Agent 1 joint old+new quorum. Only a *current* member
# that is also the durable current authority may publish an announcement.
replace(
    "crates/swarm-cli/src/discovery.rs",
    '''        let Ok(membership) = storage.load_membership_record(world) else { continue };\n        verify_membership_signature(&membership)?;\n        let Some(local_member) = membership.members.iter().find(|member| member.peer_id == identity.peer_id()) else {\n            continue;\n        };\n        if local_member.banned || local_member.public_key != identity.public_key() {\n            continue;\n        }\n        match config.visibility {\n''',
    '''        let Ok(membership) = storage.load_membership_record(world) else { continue };\n        verify_membership_signature(&membership)?;\n        let local_is_current = membership.members.iter().any(|member| {\n            member.peer_id == identity.peer_id()\n                && member.public_key == identity.public_key()\n                && !member.banned\n        });\n        let local_is_pending = storage.load_membership_promise(world).ok().is_some_and(|promise| {\n            promise.proposal.proposed.members.iter().any(|member| {\n                member.peer_id == identity.peer_id()\n                    && member.public_key == identity.public_key()\n                    && !member.banned\n            })\n        });\n        if !local_is_current && !local_is_pending {\n            continue;\n        }\n        match config.visibility {\n''',
)
replace(
    "crates/swarm-cli/src/discovery.rs",
    '''        let Ok(epoch) = storage.load_epoch_record(world) else { continue };\n        if epoch.authority_peer_id != identity.peer_id() || epoch.authority_public_key != identity.public_key() {\n            continue;\n        }\n''',
    '''        let Ok(epoch) = storage.load_epoch_record(world) else { continue };\n        if !local_is_current\n            || epoch.authority_peer_id != identity.peer_id()\n            || epoch.authority_public_key != identity.public_key()\n        {\n            continue;\n        }\n''',
)

# A recovery voter persists its promise before its vote matters. A signer that
# has promised a newer authority generation must not help the old authority
# answer a later first-contact challenge while the recovery commit propagates.
replace(
    "crates/swarm-cli/src/discovery.rs",
    '''    verify_membership_signature(&membership)?;\n    if membership.sequence != challenge.membership_sequence || membership.record_hash()? != challenge.membership_hash {\n        return Ok(false);\n    }\n    let epoch = match storage.load_epoch_record(world) {\n''',
    '''    verify_membership_signature(&membership)?;\n    if membership.sequence != challenge.membership_sequence || membership.record_hash()? != challenge.membership_hash {\n        return Ok(false);\n    }\n    if storage.load_recovery_promise(world).ok().is_some_and(|promise| {\n        (promise.ballot.target_epoch, promise.ballot.target_fencing_token)\n            > (challenge.authority_epoch, challenge.fencing_token)\n    }) {\n        return Ok(false);\n    }\n    let epoch = match storage.load_epoch_record(world) {\n''',
)

# Core proof parsing rejects oversized collections and non-canonical membership
# material before expensive signature work.
replace(
    "crates/swarm-core/src/discovery.rs",
    "pub const MAX_DISCOVERY_MEMBERSHIP_CERTIFICATES: usize = 256;\n",
    "pub const MAX_DISCOVERY_MEMBERSHIP_CERTIFICATES: usize = 256;\npub const MAX_DISCOVERY_MEMBERS: usize = 1_024;\npub const MAX_DISCOVERY_FRESHNESS_VOTES: usize = 1_024;\n",
)
replace(
    "crates/swarm-core/src/discovery.rs",
    '''    if proof.membership_certificates.len() > MAX_DISCOVERY_MEMBERSHIP_CERTIFICATES\n        || proof.genesis.world_id().ok() != Some(proof.world_id)\n    {\n        return Err(DiscoveryRecordError::InvalidMembershipProof);\n    }\n''',
    '''    if proof.membership_certificates.len() > MAX_DISCOVERY_MEMBERSHIP_CERTIFICATES\n        || proof.genesis.world_id().ok() != Some(proof.world_id)\n        || proof.genesis.validate_semantics().is_err()\n        || proof.initial_membership.members.len() > MAX_DISCOVERY_MEMBERS\n        || proof.current_membership.members.len() > MAX_DISCOVERY_MEMBERS\n        || proof.pending_membership.as_ref().is_some_and(|proposal| {\n            proposal.previous.members.len() > MAX_DISCOVERY_MEMBERS\n                || proposal.proposed.members.len() > MAX_DISCOVERY_MEMBERS\n        })\n        || proof.membership_certificates.iter().any(|certificate| {\n            certificate.proposal.previous.members.len() > MAX_DISCOVERY_MEMBERS\n                || certificate.proposal.proposed.members.len() > MAX_DISCOVERY_MEMBERS\n                || certificate.votes.len() > MAX_DISCOVERY_FRESHNESS_VOTES\n        })\n    {\n        return Err(DiscoveryRecordError::InvalidMembershipProof);\n    }\n''',
)
replace(
    "crates/swarm-core/src/discovery.rs",
    '''    let initial = &proof.initial_membership;\n    if initial.protocol_version != PROTOCOL_VERSION\n''',
    '''    let initial = &proof.initial_membership;\n    if initial.validate_semantics().is_err()\n        || proof.current_membership.validate_semantics().is_err()\n        || initial.protocol_version != PROTOCOL_VERSION\n''',
)
replace(
    "crates/swarm-core/src/discovery.rs",
    '''    for certificate in &proof.membership_certificates {\n        verify_membership_signature(&certificate.proposal.previous)\n            .map_err(|_| DiscoveryRecordError::InvalidMembershipProof)?;\n''',
    '''    let mut last_certificate_sequence = None;\n    for certificate in &proof.membership_certificates {\n        if !certificate.proposal.validate_shape().unwrap_or(false)\n            || certificate.proposal.previous.world_id != proof.world_id\n            || certificate.proposal.proposed.world_id != proof.world_id\n            || certificate.proposal.previous.validate_semantics().is_err()\n            || certificate.proposal.proposed.validate_semantics().is_err()\n            || last_certificate_sequence.is_some_and(|sequence| certificate.proposal.proposed.sequence <= sequence)\n        {\n            return Err(DiscoveryRecordError::InvalidMembershipProof);\n        }\n        last_certificate_sequence = Some(certificate.proposal.proposed.sequence);\n        verify_membership_signature(&certificate.proposal.previous)\n            .map_err(|_| DiscoveryRecordError::InvalidMembershipProof)?;\n''',
)
replace(
    "crates/swarm-core/src/discovery.rs",
    '''        for vote in &certificate.votes {\n            if !seen.insert(vote.voter_peer_id) {\n                return Err(DiscoveryRecordError::InvalidMembershipProof);\n            }\n''',
    '''        for vote in &certificate.votes {\n            if !vote.matches_proposal(&certificate.proposal).unwrap_or(false)\n                || !seen.insert(vote.voter_peer_id)\n            {\n                return Err(DiscoveryRecordError::InvalidMembershipProof);\n            }\n''',
)
replace(
    "crates/swarm-core/src/discovery.rs",
    '''    if proof.current_membership.world_id != proof.world_id\n        || proof.current_membership.sequence != announcement.membership_sequence\n''',
    '''    if proof.current_membership.world_id != proof.world_id\n        || last_certificate_sequence.is_some_and(|sequence| proof.current_membership.sequence < sequence)\n        || proof.current_membership.sequence != announcement.membership_sequence\n''',
)
replace(
    "crates/swarm-core/src/discovery.rs",
    '''        || proof.current_membership.authority_public_key != announcement.announcer_public_key\n        || proof.current_membership.epoch != announcement.authority_epoch\n    {\n''',
    '''        || proof.current_membership.authority_public_key != announcement.announcer_public_key\n        || proof.current_membership.epoch != announcement.authority_epoch\n        || !proof.current_membership.members.iter().any(|member| {\n            member.peer_id == proof.current_membership.authority_peer_id\n                && member.public_key == proof.current_membership.authority_public_key\n                && !member.banned\n        })\n    {\n''',
)

# Consensus proof shape additionally rejects reordered certificate chains and
# oversized signer collections before quorum counting.
replace(
    "crates/swarm-consensus/src/membership.rs",
    '''    let mut voters = active_voters(&proof.initial_membership.members)?;\n    for certificate in &proof.membership_certificates {\n        validate_membership_certificate_shape(certificate)?;\n''',
    '''    let mut voters = active_voters(&proof.initial_membership.members)?;\n    let mut last_sequence = proof.initial_membership.sequence;\n    for certificate in &proof.membership_certificates {\n        validate_membership_certificate_shape(certificate)?;\n        if certificate.proposal.previous.world_id != proof.world_id\n            || certificate.proposal.proposed.world_id != proof.world_id\n            || certificate.proposal.proposed.sequence <= last_sequence\n        {\n            return Err(MembershipConsensusError::MalformedHistory);\n        }\n        last_sequence = certificate.proposal.proposed.sequence;\n''',
)
replace(
    "crates/swarm-consensus/src/membership.rs",
    '''    if active_voters(&proof.current_membership.members)? != voters {\n        return Err(MembershipConsensusError::MalformedHistory);\n    }\n''',
    '''    if proof.current_membership.sequence < last_sequence\n        || active_voters(&proof.current_membership.members)? != voters\n    {\n        return Err(MembershipConsensusError::MalformedHistory);\n    }\n''',
)
replace(
    "crates/swarm-consensus/src/membership.rs",
    '''    validate_discovery_membership_proof_shape(proof)?;\n    let mut last = None;\n''',
    '''    validate_discovery_membership_proof_shape(proof)?;\n    if votes.len() > 1_024 {\n        return Err(MembershipConsensusError::NonCanonicalSignerSet);\n    }\n    let mut last = None;\n''',
)

# Wire-level structural bounds are checked before JSON sizing or cryptographic
# validation, so an attacker cannot hide huge member/vote collections inside a
# proof that happens to have few certificates.
replace(
    "crates/swarm-network/src/wire.rs",
    '''            Self::DiscoveryFreshnessContext(Some(proof)) => {\n                if proof.membership_certificates.len() > MAX_DISCOVERY_MEMBERSHIP_CERTIFICATES {\n                    return Err(WireLimitError::TooManyDiscoveryMembershipCertificates(\n                        proof.membership_certificates.len(),\n                    ));\n                }\n                let bytes = serde_json::to_vec(proof.as_ref())\n''',
    '''            Self::DiscoveryFreshnessContext(Some(proof)) => {\n                if proof.membership_certificates.len() > MAX_DISCOVERY_MEMBERSHIP_CERTIFICATES {\n                    return Err(WireLimitError::TooManyDiscoveryMembershipCertificates(\n                        proof.membership_certificates.len(),\n                    ));\n                }\n                let member_count = proof\n                    .membership_certificates\n                    .iter()\n                    .flat_map(|certificate| {\n                        [certificate.proposal.previous.members.len(), certificate.proposal.proposed.members.len()]\n                    })\n                    .chain([proof.initial_membership.members.len(), proof.current_membership.members.len()])\n                    .chain(proof.pending_membership.iter().flat_map(|proposal| {\n                        [proposal.previous.members.len(), proposal.proposed.members.len()]\n                    }))\n                    .max()\n                    .unwrap_or(0);\n                if member_count > MAX_WORLD_MEMBERS {\n                    return Err(WireLimitError::TooManyMembers(member_count));\n                }\n                let vote_count = proof\n                    .membership_certificates\n                    .iter()\n                    .map(|certificate| certificate.votes.len())\n                    .max()\n                    .unwrap_or(0);\n                if vote_count > MAX_MEMBERSHIP_VOTES {\n                    return Err(WireLimitError::TooManyMembershipVotes(vote_count));\n                }\n                let bytes = serde_json::to_vec(proof.as_ref())\n''',
)

# Repair the existing oversized-announcement unit fixture for the expanded
# announcement schema without weakening the fixture's original size assertion.
replace(
    "crates/swarm-network/src/wire.rs",
    '''            config_sequence: 1,\n            config_hash: Hash32([3; 32]),\n            authority_epoch: 1,\n            fencing_token: 1,\n            announcement_sequence: 1,\n''',
    '''            config_sequence: 1,\n            config_hash: Hash32([3; 32]),\n            membership_sequence: 0,\n            membership_hash: Hash32([6; 32]),\n            authority_epoch: 1,\n            fencing_token: 1,\n            canonical_head: None,\n            announcement_sequence: 1,\n''',
)

# Bound live provider/vote accumulation. Quorum requires at most 1,024 voters,
# matching the protocol member cap; extra untrusted DHT locators are ignored.
replace(
    "crates/swarm-cli/src/discovery.rs",
    '''                    for peer in found {\n                        if providers.insert(peer) {\n                            let _ = node.dial_peer(peer);\n                        }\n                    }\n''',
    '''                    for peer in found {\n                        if providers.len() >= swarm_core::MAX_DISCOVERY_FRESHNESS_VOTES {\n                            break;\n                        }\n                        if providers.insert(peer) {\n                            let _ = node.dial_peer(peer);\n                        }\n                    }\n''',
)
replace(
    "crates/swarm-cli/src/discovery.rs",
    '''                    if votes.iter().all(|existing| existing.voter_peer_id != vote.voter_peer_id) {\n                        votes.push(vote);\n                        votes.sort_by_key(|value| value.voter_peer_id);\n                    }\n''',
    '''                    if votes.len() < swarm_core::MAX_DISCOVERY_FRESHNESS_VOTES\n                        && votes.iter().all(|existing| existing.voter_peer_id != vote.voter_peer_id)\n                    {\n                        votes.push(vote);\n                        votes.sort_by_key(|value| value.voter_peer_id);\n                    }\n''',
)

# PeerIdentity intentionally is not Clone. Keep key material borrowed in tests.
replace(
    "crates/swarm-cli/tests/discovery_freshness.rs",
    '''fn votes(\n    ids: &[PeerIdentity],\n    challenge: &DiscoveryFreshnessChallengeV1,\n) -> Vec<swarm_protocol::DiscoveryFreshnessVoteV1> {\n    let mut result = ids.iter().map(|id| sign_discovery_freshness_vote(id, challenge).unwrap()).collect::<Vec<_>>();\n''',
    '''fn votes<'a>(\n    ids: impl IntoIterator<Item = &'a PeerIdentity>,\n    challenge: &DiscoveryFreshnessChallengeV1,\n) -> Vec<swarm_protocol::DiscoveryFreshnessVoteV1> {\n    let mut result = ids.into_iter().map(|id| sign_discovery_freshness_vote(id, challenge).unwrap()).collect::<Vec<_>>();\n''',
)
p = Path("crates/swarm-cli/tests/discovery_freshness.rs")
text = p.read_text()
text = text.replace("votes(&ids[..2],", "votes(ids[..2].iter(),")
text = text.replace("votes(&ids[..1],", "votes(ids[..1].iter(),")
text = text.replace("votes(&[ids[0].clone(), ids[1].clone()],", "votes([&ids[0], &ids[1]],")
text = text.replace(
    "votes(&[ids[0].clone(), ids[1].clone(), ids[3].clone()],",
    "votes([&ids[0], &ids[1], &ids[3]],",
)
text = text.replace(
    "let (mut ids, mut announcement, mut proof, _) = fixture(3);",
    "let (mut ids, announcement, mut proof, _) = fixture(3);",
)
p.write_text(text)

# Expand permanent FINAL-028 negative coverage for exact head coordinates,
# unsupported proof versions, malformed member ordering, and oversized signer
# collections. These all hit the same final acceptance gate used by browse and
# exact resolve.
replace(
    "crates/swarm-cli/tests/discovery_freshness.rs",
    '''    for mutate in 0..7 {\n        let mut bad = challenge.clone();\n        match mutate {\n            0 => bad.world_id = swarm_protocol::WorldId([42; 32]),\n            1 => bad.membership_hash = Hash32([42; 32]),\n            2 => bad.membership_sequence += 1,\n            3 => bad.authority_epoch += 1,\n            4 => bad.fencing_token += 1,\n            5 => bad.canonical_head.as_mut().unwrap().manifest_hash = Hash32([42; 32]),\n            _ => bad.verifier_peer_id = PeerId([42; 32]),\n        }\n''',
    '''    for mutate in 0..10 {\n        let mut bad = challenge.clone();\n        match mutate {\n            0 => bad.world_id = swarm_protocol::WorldId([42; 32]),\n            1 => bad.membership_hash = Hash32([42; 32]),\n            2 => bad.membership_sequence += 1,\n            3 => bad.authority_epoch += 1,\n            4 => bad.fencing_token += 1,\n            5 => bad.canonical_head.as_mut().unwrap().manifest_hash = Hash32([42; 32]),\n            6 => bad.canonical_head.as_mut().unwrap().snapshot_number += 1,\n            7 => bad.canonical_head.as_mut().unwrap().sequence += 1,\n            8 => bad.canonical_head.as_mut().unwrap().epoch += 1,\n            _ => bad.verifier_peer_id = PeerId([42; 32]),\n        }\n''',
)
extra_tests = r'''

#[test]
fn unsupported_malformed_and_oversized_proofs_fail_closed() {
    let (ids, announcement, proof, challenge) = fixture(3);
    let valid_votes = votes(ids[..2].iter(), &challenge);

    let mut unsupported = proof.clone();
    unsupported.protocol_version = PROTOCOL_VERSION + 1;
    let mut guard = DiscoveryFreshnessReplayGuard::default();
    assert!(validate_fresh_discovery_candidate(
        &announcement,
        &unsupported,
        &challenge,
        &valid_votes,
        challenge.verifier_peer_id,
        challenge.nonce,
        3_000,
        &mut guard,
    )
    .is_err());

    let mut malformed = proof.clone();
    malformed.current_membership.members.reverse();
    assert!(swarm_consensus::validate_discovery_membership_proof_shape(&malformed).is_err());

    let mut oversized = valid_votes.clone();
    oversized.resize(1_025, valid_votes[0].clone());
    assert!(validate_discovery_freshness_quorum(&proof, &oversized).is_err());
}
'''
if extra_tests.strip() not in text:
    text += extra_tests
p.write_text(text)

print("Agent 4 FINAL-028 repair patch applied")
