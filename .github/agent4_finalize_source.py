from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    p = Path(path)
    text = p.read_text()
    if old not in text:
        if new in text:
            return
        raise SystemExit(f"missing FINAL-028 source anchor in {path}: {old[:160]!r}")
    p.write_text(text.replace(old, new, 1))


def insert_before(path: str, marker: str, block: str) -> None:
    p = Path(path)
    text = p.read_text()
    if block.strip() in text:
        return
    if marker not in text:
        raise SystemExit(f"missing FINAL-028 insertion marker in {path}: {marker!r}")
    p.write_text(text.replace(marker, block + "\n" + marker, 1))


# 1. Bound and canonicalize genesis-anchored first-contact proof parsing.
replace_once(
    "crates/swarm-core/src/discovery.rs",
    "pub const MAX_DISCOVERY_MEMBERSHIP_CERTIFICATES: usize = 256;\n",
    "pub const MAX_DISCOVERY_MEMBERSHIP_CERTIFICATES: usize = 256;\n"
    "pub const MAX_DISCOVERY_MEMBERS: usize = 1_024;\n"
    "pub const MAX_DISCOVERY_FRESHNESS_VOTES: usize = 1_024;\n",
)
replace_once(
    "crates/swarm-core/src/discovery.rs",
    '''    if proof.membership_certificates.len() > MAX_DISCOVERY_MEMBERSHIP_CERTIFICATES\n        || proof.genesis.world_id().ok() != Some(proof.world_id)\n    {\n        return Err(DiscoveryRecordError::InvalidMembershipProof);\n    }\n    let initial = &proof.initial_membership;\n    if initial.protocol_version != PROTOCOL_VERSION\n''',
    '''    if proof.membership_certificates.len() > MAX_DISCOVERY_MEMBERSHIP_CERTIFICATES\n        || proof.genesis.world_id().ok() != Some(proof.world_id)\n        || proof.genesis.validate_semantics().is_err()\n        || proof.initial_membership.members.len() > MAX_DISCOVERY_MEMBERS\n        || proof.current_membership.members.len() > MAX_DISCOVERY_MEMBERS\n        || proof.pending_membership.as_ref().is_some_and(|proposal| {\n            proposal.previous.members.len() > MAX_DISCOVERY_MEMBERS\n                || proposal.proposed.members.len() > MAX_DISCOVERY_MEMBERS\n        })\n        || proof.membership_certificates.iter().any(|certificate| {\n            certificate.proposal.previous.members.len() > MAX_DISCOVERY_MEMBERS\n                || certificate.proposal.proposed.members.len() > MAX_DISCOVERY_MEMBERS\n                || certificate.votes.len() > MAX_DISCOVERY_FRESHNESS_VOTES\n        })\n    {\n        return Err(DiscoveryRecordError::InvalidMembershipProof);\n    }\n    let initial = &proof.initial_membership;\n    if initial.validate_semantics().is_err()\n        || proof.current_membership.validate_semantics().is_err()\n        || initial.protocol_version != PROTOCOL_VERSION\n''',
)
replace_once(
    "crates/swarm-core/src/discovery.rs",
    '''    for certificate in &proof.membership_certificates {\n        verify_membership_signature(&certificate.proposal.previous)\n            .map_err(|_| DiscoveryRecordError::InvalidMembershipProof)?;\n        verify_membership_signature(&certificate.proposal.proposed)\n            .map_err(|_| DiscoveryRecordError::InvalidMembershipProof)?;\n        let mut seen = HashSet::new();\n        for vote in &certificate.votes {\n            if !seen.insert(vote.voter_peer_id) {\n                return Err(DiscoveryRecordError::InvalidMembershipProof);\n            }\n            verify_signature(\n''',
    '''    let mut last_certificate_sequence = None;\n    for certificate in &proof.membership_certificates {\n        if !certificate.proposal.validate_shape().unwrap_or(false)\n            || certificate.proposal.previous.world_id != proof.world_id\n            || certificate.proposal.proposed.world_id != proof.world_id\n            || certificate.proposal.previous.validate_semantics().is_err()\n            || certificate.proposal.proposed.validate_semantics().is_err()\n            || last_certificate_sequence\n                .is_some_and(|sequence| certificate.proposal.proposed.sequence <= sequence)\n        {\n            return Err(DiscoveryRecordError::InvalidMembershipProof);\n        }\n        last_certificate_sequence = Some(certificate.proposal.proposed.sequence);\n        verify_membership_signature(&certificate.proposal.previous)\n            .map_err(|_| DiscoveryRecordError::InvalidMembershipProof)?;\n        verify_membership_signature(&certificate.proposal.proposed)\n            .map_err(|_| DiscoveryRecordError::InvalidMembershipProof)?;\n        let mut seen = HashSet::new();\n        let mut last_voter = None;\n        for vote in &certificate.votes {\n            if !vote.matches_proposal(&certificate.proposal).unwrap_or(false)\n                || !seen.insert(vote.voter_peer_id)\n                || last_voter.is_some_and(|peer| vote.voter_peer_id <= peer)\n            {\n                return Err(DiscoveryRecordError::InvalidMembershipProof);\n            }\n            last_voter = Some(vote.voter_peer_id);\n            verify_signature(\n''',
)
replace_once(
    "crates/swarm-core/src/discovery.rs",
    '''    if proof.current_membership.world_id != proof.world_id\n        || proof.current_membership.sequence != announcement.membership_sequence\n        || proof.current_membership.record_hash().ok() != Some(announcement.membership_hash)\n        || proof.current_membership.authority_peer_id != announcement.announcer_peer_id\n        || proof.current_membership.authority_public_key != announcement.announcer_public_key\n        || proof.current_membership.epoch != announcement.authority_epoch\n    {\n''',
    '''    if proof.current_membership.world_id != proof.world_id\n        || last_certificate_sequence.is_some_and(|sequence| proof.current_membership.sequence < sequence)\n        || proof.current_membership.sequence != announcement.membership_sequence\n        || proof.current_membership.record_hash().ok() != Some(announcement.membership_hash)\n        || proof.current_membership.authority_peer_id != announcement.announcer_peer_id\n        || proof.current_membership.authority_public_key != announcement.announcer_public_key\n        || proof.current_membership.epoch != announcement.authority_epoch\n        || !proof.current_membership.members.iter().any(|member| {\n            member.peer_id == proof.current_membership.authority_peer_id\n                && member.public_key == proof.current_membership.authority_public_key\n                && !member.banned\n        })\n    {\n''',
)

# 2. Discovery-only consensus proof shape is ordered/bounded and uses Agent 1 quorum rules.
replace_once(
    "crates/swarm-consensus/src/membership.rs",
    '''    let mut voters = active_voters(&proof.initial_membership.members)?;\n    for certificate in &proof.membership_certificates {\n        validate_membership_certificate_shape(certificate)?;\n        let previous = active_voters(&certificate.proposal.previous.members)?;\n        if previous != voters {\n            return Err(MembershipConsensusError::MalformedHistory);\n        }\n        voters = active_voters(&certificate.proposal.proposed.members)?;\n    }\n    if active_voters(&proof.current_membership.members)? != voters {\n        return Err(MembershipConsensusError::MalformedHistory);\n    }\n''',
    '''    let mut voters = active_voters(&proof.initial_membership.members)?;\n    let mut last_sequence = proof.initial_membership.sequence;\n    for certificate in &proof.membership_certificates {\n        validate_membership_certificate_shape(certificate)?;\n        if certificate.proposal.previous.world_id != proof.world_id\n            || certificate.proposal.proposed.world_id != proof.world_id\n            || certificate.proposal.proposed.sequence <= last_sequence\n        {\n            return Err(MembershipConsensusError::MalformedHistory);\n        }\n        last_sequence = certificate.proposal.proposed.sequence;\n        let previous = active_voters(&certificate.proposal.previous.members)?;\n        if previous != voters {\n            return Err(MembershipConsensusError::MalformedHistory);\n        }\n        voters = active_voters(&certificate.proposal.proposed.members)?;\n    }\n    if proof.current_membership.sequence < last_sequence\n        || active_voters(&proof.current_membership.members)? != voters\n    {\n        return Err(MembershipConsensusError::MalformedHistory);\n    }\n''',
)
replace_once(
    "crates/swarm-consensus/src/membership.rs",
    '''    validate_discovery_membership_proof_shape(proof)?;\n    let mut last = None;\n''',
    '''    validate_discovery_membership_proof_shape(proof)?;\n    if votes.len() > 1_024 {\n        return Err(MembershipConsensusError::NonCanonicalSignerSet);\n    }\n    let mut last = None;\n''',
)

# 3. Wire proof structures are bounded before JSON sizing/crypto.
replace_once(
    "crates/swarm-network/src/wire.rs",
    '''            Self::DiscoveryFreshnessContext(Some(proof)) => {\n                if proof.membership_certificates.len() > MAX_DISCOVERY_MEMBERSHIP_CERTIFICATES {\n                    return Err(WireLimitError::TooManyDiscoveryMembershipCertificates(\n                        proof.membership_certificates.len(),\n                    ));\n                }\n                let bytes = serde_json::to_vec(proof.as_ref())\n''',
    '''            Self::DiscoveryFreshnessContext(Some(proof)) => {\n                if proof.membership_certificates.len() > MAX_DISCOVERY_MEMBERSHIP_CERTIFICATES {\n                    return Err(WireLimitError::TooManyDiscoveryMembershipCertificates(\n                        proof.membership_certificates.len(),\n                    ));\n                }\n                let member_count = proof\n                    .membership_certificates\n                    .iter()\n                    .flat_map(|certificate| {\n                        [certificate.proposal.previous.members.len(), certificate.proposal.proposed.members.len()]\n                    })\n                    .chain([proof.initial_membership.members.len(), proof.current_membership.members.len()])\n                    .chain(proof.pending_membership.iter().flat_map(|proposal| {\n                        [proposal.previous.members.len(), proposal.proposed.members.len()]\n                    }))\n                    .max()\n                    .unwrap_or(0);\n                if member_count > MAX_WORLD_MEMBERS {\n                    return Err(WireLimitError::TooManyMembers(member_count));\n                }\n                let vote_count = proof\n                    .membership_certificates\n                    .iter()\n                    .map(|certificate| certificate.votes.len())\n                    .max()\n                    .unwrap_or(0);\n                if vote_count > MAX_MEMBERSHIP_VOTES {\n                    return Err(WireLimitError::TooManyMembershipVotes(vote_count));\n                }\n                let bytes = serde_json::to_vec(proof.as_ref())\n''',
)
replace_once(
    "crates/swarm-network/src/wire.rs",
    '''            config_sequence: 1,\n            config_hash: Hash32([3; 32]),\n            authority_epoch: 1,\n            fencing_token: 1,\n            announcement_sequence: 1,\n''',
    '''            config_sequence: 1,\n            config_hash: Hash32([3; 32]),\n            membership_sequence: 0,\n            membership_hash: Hash32([6; 32]),\n            authority_epoch: 1,\n            fencing_token: 1,\n            canonical_head: None,\n            announcement_sequence: 1,\n''',
)

# 4. Signers reload Agent 1 durable recovery promises before freshness-signing.
replace_once(
    "crates/swarm-cli/src/discovery.rs",
    '''    verify_membership_signature(&membership)?;\n    if membership.sequence != challenge.membership_sequence || membership.record_hash()? != challenge.membership_hash {\n        return Ok(false);\n    }\n    let epoch = match storage.load_epoch_record(world) {\n''',
    '''    verify_membership_signature(&membership)?;\n    if membership.sequence != challenge.membership_sequence || membership.record_hash()? != challenge.membership_hash {\n        return Ok(false);\n    }\n    if storage.load_recovery_promise(world).ok().is_some_and(|promise| {\n        (promise.ballot.target_epoch, promise.ballot.target_fencing_token)\n            > (challenge.authority_epoch, challenge.fencing_token)\n    }) {\n        return Ok(false);\n    }\n    let epoch = match storage.load_epoch_record(world) {\n''',
)

# Bound untrusted DHT locator and vote accumulation.
replace_once(
    "crates/swarm-cli/src/discovery.rs",
    '''                    for peer in found {\n                        if providers.insert(peer) {\n                            let _ = node.dial_peer(peer);\n                        }\n                    }\n''',
    '''                    for peer in found {\n                        if providers.len() >= swarm_core::MAX_DISCOVERY_FRESHNESS_VOTES {\n                            break;\n                        }\n                        if providers.insert(peer) {\n                            let _ = node.dial_peer(peer);\n                        }\n                    }\n''',
)
replace_once(
    "crates/swarm-cli/src/discovery.rs",
    '''                    if votes.iter().all(|existing| existing.voter_peer_id != vote.voter_peer_id) {\n                        votes.push(vote);\n                        votes.sort_by_key(|value| value.voter_peer_id);\n                    }\n''',
    '''                    if votes.len() < swarm_core::MAX_DISCOVERY_FRESHNESS_VOTES\n                        && votes.iter().all(|existing| existing.voter_peer_id != vote.voter_peer_id)\n                    {\n                        votes.push(vote);\n                        votes.sort_by_key(|value| value.voter_peer_id);\n                    }\n''',
)

# Make checked counter/expiry behavior directly testable instead of duplicating arithmetic.
replace_once(
    "crates/swarm-cli/src/discovery.rs",
    '''        let previous = state.sequences.get(&world).copied().unwrap_or(0);\n        let sequence = if now > previous {\n            now\n        } else {\n            previous.checked_add(1).context("discovery announcement sequence exhausted")?\n        };\n''',
    '''        let previous = state.sequences.get(&world).copied().unwrap_or(0);\n        let sequence = next_discovery_sequence(previous, now)?;\n''',
)
replace_once(
    "crates/swarm-cli/src/discovery.rs",
    '''            expires_unix_ms: now.checked_add(WORLD_ANNOUNCEMENT_TTL_MS).context("discovery expiry overflow")?,\n''',
    '''            expires_unix_ms: checked_discovery_expiry(now, WORLD_ANNOUNCEMENT_TTL_MS, "discovery")?,\n''',
)
replace_once(
    "crates/swarm-cli/src/discovery.rs",
    '''    let expires_unix_ms = issued_unix_ms\n        .checked_add(DISCOVERY_FRESHNESS_MAX_LIFETIME_MS)\n        .context("freshness challenge expiry overflow")?;\n''',
    '''    let expires_unix_ms = checked_discovery_expiry(\n        issued_unix_ms,\n        DISCOVERY_FRESHNESS_MAX_LIFETIME_MS,\n        "freshness challenge",\n    )?;\n''',
)
insert_before(
    "crates/swarm-cli/src/discovery.rs",
    "fn unix_millis() -> Result<u64> {",
    '''fn next_discovery_sequence(previous: u64, now: u64) -> Result<u64> {\n    if now > previous {\n        Ok(now)\n    } else {\n        previous.checked_add(1).context("discovery announcement sequence exhausted")\n    }\n}\n\nfn checked_discovery_expiry(issued: u64, ttl: u64, label: &str) -> Result<u64> {\n    issued.checked_add(ttl).with_context(|| format!("{label} expiry overflow"))\n}\n''',
)

# Repair the remaining in-module announcement fixture.
replace_once(
    "crates/swarm-cli/src/discovery.rs",
    '''            config_sequence: 1,\n            config_hash: Hash32([11; 32]),\n            authority_epoch: 1,\n            fencing_token: 1,\n            announcement_sequence: 1,\n''',
    '''            config_sequence: 1,\n            config_hash: Hash32([11; 32]),\n            membership_sequence: 0,\n            membership_hash: Hash32([13; 32]),\n            authority_epoch: 1,\n            fencing_token: 1,\n            canonical_head: None,\n            announcement_sequence: 1,\n''',
)

# 5. Expand permanent pure FINAL-028 regressions.
replace_once(
    "crates/swarm-cli/tests/discovery_freshness.rs",
    '''    for mutate in 0..7 {\n        let mut bad = challenge.clone();\n        match mutate {\n            0 => bad.world_id = swarm_protocol::WorldId([42; 32]),\n            1 => bad.membership_hash = Hash32([42; 32]),\n            2 => bad.membership_sequence += 1,\n            3 => bad.authority_epoch += 1,\n            4 => bad.fencing_token += 1,\n            5 => bad.canonical_head.as_mut().unwrap().manifest_hash = Hash32([42; 32]),\n            _ => bad.verifier_peer_id = PeerId([42; 32]),\n        }\n''',
    '''    for mutate in 0..10 {\n        let mut bad = challenge.clone();\n        match mutate {\n            0 => bad.world_id = swarm_protocol::WorldId([42; 32]),\n            1 => bad.membership_hash = Hash32([42; 32]),\n            2 => bad.membership_sequence += 1,\n            3 => bad.authority_epoch += 1,\n            4 => bad.fencing_token += 1,\n            5 => bad.canonical_head.as_mut().unwrap().manifest_hash = Hash32([42; 32]),\n            6 => bad.canonical_head.as_mut().unwrap().snapshot_number += 1,\n            7 => bad.canonical_head.as_mut().unwrap().sequence += 1,\n            8 => bad.canonical_head.as_mut().unwrap().epoch += 1,\n            _ => bad.verifier_peer_id = PeerId([42; 32]),\n        }\n''',
)
replace_once(
    "crates/swarm-cli/tests/discovery_freshness.rs",
    '''    let old_only = votes(ids[..2].iter(), &challenge);\n    assert!(validate_discovery_freshness_quorum(&proof, &old_only).is_err());\n    let joint = votes([&ids[0], &ids[1], &ids[3]], &challenge);\n''',
    '''    let old_only = votes(ids[..2].iter(), &challenge);\n    assert!(validate_discovery_freshness_quorum(&proof, &old_only).is_err());\n    let new_only = votes([&ids[0], &ids[3], &ids[4]], &challenge);\n    assert!(validate_discovery_freshness_quorum(&proof, &new_only).is_err());\n    let joint = votes([&ids[0], &ids[1], &ids[3]], &challenge);\n''',
)
extra_pure = r'''

#[test]
fn unsupported_malformed_removed_and_oversized_proofs_fail_closed() {
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

    let mut removed = proof.clone();
    let removed_peer = ids[1].peer_id();
    removed.initial_membership.members.retain(|member| member.peer_id != removed_peer);
    removed.current_membership.members.retain(|member| member.peer_id != removed_peer);
    assert!(validate_discovery_freshness_quorum(&removed, &valid_votes).is_err());

    let mut oversized = valid_votes.clone();
    oversized.resize(1_025, valid_votes[0].clone());
    assert!(validate_discovery_freshness_quorum(&proof, &oversized).is_err());
}

#[test]
fn verifier_bound_proof_and_pretransition_proof_cannot_cross_contexts() {
    let (ids, announcement, proof, challenge) = fixture(3);
    let valid_votes = votes(ids[..2].iter(), &challenge);
    let verifier_b = PeerIdentity::from_secret_bytes([100; 32]);
    let mut guard = DiscoveryFreshnessReplayGuard::default();
    assert!(validate_fresh_discovery_candidate(
        &announcement,
        &proof,
        &challenge,
        &valid_votes,
        verifier_b.peer_id(),
        challenge.nonce,
        3_000,
        &mut guard,
    )
    .is_err());

    let mut transitioned = proof.clone();
    transitioned.current_membership.sequence += 1;
    assert!(validate_fresh_discovery_candidate(
        &announcement,
        &transitioned,
        &challenge,
        &valid_votes,
        challenge.verifier_peer_id,
        challenge.nonce,
        3_000,
        &mut DiscoveryFreshnessReplayGuard::default(),
    )
    .is_err());
}
'''
p = Path("crates/swarm-cli/tests/discovery_freshness.rs")
text = p.read_text()
if "unsupported_malformed_removed_and_oversized_proofs_fail_closed" not in text:
    text += extra_pure
p.write_text(text)

# 6. Deterministic canonical signing bytes across a wire round-trip.
insert_before(
    "crates/swarm-protocol/src/discovery.rs",
    '''    #[test]\n    fn private_visibility_is_not_discoverable() {''',
    r'''    #[test]
    fn freshness_vote_signing_bytes_are_deterministic_across_round_trip() {
        let value = announcement();
        let challenge = DiscoveryFreshnessChallengeV1 {
            protocol_version: PROTOCOL_VERSION,
            verifier_peer_id: PeerId([21; 32]),
            nonce: [22; 32],
            world_id: value.world_id,
            announcement_hash: value.announcement_hash().unwrap(),
            membership_sequence: value.membership_sequence,
            membership_hash: value.membership_hash,
            pending_membership_proposal_hash: None,
            authority_peer_id: value.announcer_peer_id,
            authority_epoch: value.authority_epoch,
            fencing_token: value.fencing_token,
            config_sequence: value.config_sequence,
            config_hash: value.config_hash,
            canonical_head: value.canonical_head,
            issued_unix_ms: 100,
            expires_unix_ms: 200,
        };
        let vote = DiscoveryFreshnessVoteV1 {
            challenge,
            voter_peer_id: PeerId([23; 32]),
            voter_public_key: [24; 32],
            signature: vec![25; 64],
        };
        let encoded = postcard::to_allocvec(&vote).unwrap();
        let decoded: DiscoveryFreshnessVoteV1 = postcard::from_bytes(&encoded).unwrap();
        assert_eq!(vote, decoded);
        assert_eq!(vote.signing_bytes().unwrap(), decoded.signing_bytes().unwrap());
    }

''',
)

# 7. Durable 3-peer recovery transition: a promised newer generation fences stale freshness.
replace_once(
    "crates/swarm-cli/src/discovery.rs",
    '''    use swarm_core::PeerIdentity;\n''',
    '''    use swarm_core::{sign_recovery_ballot, sign_recovery_vote, sign_world_config, PeerIdentity};\n''',
)
replace_once(
    "crates/swarm-cli/src/discovery.rs",
    '''        AuthorityPolicyV1, EpochMode, EpochRecordV1, Hash32, MembershipRecordV1, RuntimeCompatibilityManifestV1,\n        WorldConfigV1, WorldMemberV1, WorldPresentationV1, STORAGE_SCHEMA_VERSION,\n''',
    '''        AuthorityPolicyV1, EpochMode, EpochRecordV1, Hash32, MembershipRecordV1, RecoveryBallotV1, RecoveryVoteV1,\n        RuntimeCompatibilityManifestV1, WorldConfigV1, WorldMemberV1, WorldPresentationV1, STORAGE_SCHEMA_VERSION,\n''',
)
durable_test = r'''
    #[test]
    fn durable_recovery_promise_fences_stale_freshness_and_current_majority_recovers() {
        let a = PeerIdentity::from_secret_bytes([31; 32]);
        let b = PeerIdentity::from_secret_bytes([32; 32]);
        let c = PeerIdentity::from_secret_bytes([33; 32]);
        let mut members = [&a, &b, &c].into_iter().map(|identity| WorldMemberV1 {
            peer_id: identity.peer_id(),
            public_key: identity.public_key(),
            authority_eligible: true,
            banned: false,
        }).collect::<Vec<_>>();
        members.sort_by_key(|member| member.peer_id);
        let genesis = swarm_protocol::WorldGenesisV1 {
            protocol_version: PROTOCOL_VERSION,
            minecraft_version: "1.21.8".into(),
            fabric_loader_version: "0.17.2".into(),
            compatibility_fingerprint: Hash32([34; 32]),
            creation_nonce: [35; 32],
            creator_public_key: a.public_key(),
            initial_membership: members.iter().map(|member| member.peer_id).collect(),
        };
        let world = genesis.world_id().unwrap();
        let metadata = WorldMetadataV1 {
            storage_schema_version: STORAGE_SCHEMA_VERSION,
            display_name: "durable-freshness".into(),
            world_id: world,
            genesis,
        };
        let roots = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
        let stores = roots.iter().map(|root| Storage::open(root.path()).unwrap()).collect::<Vec<_>>();

        let mut initial = MembershipRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            epoch: 0,
            sequence: 0,
            previous_membership_hash: None,
            members: members.clone(),
            authority_peer_id: a.peer_id(),
            authority_public_key: a.public_key(),
            signature: Vec::new(),
        };
        a.sign_membership(&mut initial).unwrap();
        let mut config1 = canonical_fixture(&a, world).0;
        sign_world_config(&a, &mut config1).unwrap();
        let mut epoch1 = canonical_fixture(&a, world).1;
        epoch1.signature = a.sign(&epoch1.signing_bytes().unwrap());
        let mut membership1 = initial.clone();
        membership1.epoch = epoch1.epoch_number;
        membership1.sequence = 1;
        membership1.previous_membership_hash = Some(initial.record_hash().unwrap());
        membership1.signature.clear();
        a.sign_membership(&mut membership1).unwrap();

        for store in &stores {
            store.create_world(&metadata).unwrap();
            store.save_membership_record(&initial).unwrap();
            store.save_world_config(&config1).unwrap();
            store.save_epoch_record(&epoch1).unwrap();
            store.save_membership_record(&membership1).unwrap();
        }

        let stale = DiscoveryFreshnessChallengeV1 {
            protocol_version: PROTOCOL_VERSION,
            verifier_peer_id: PeerIdentity::from_secret_bytes([36; 32]).peer_id(),
            nonce: [37; 32],
            world_id: world,
            announcement_hash: Hash32([38; 32]),
            membership_sequence: membership1.sequence,
            membership_hash: membership1.record_hash().unwrap(),
            pending_membership_proposal_hash: None,
            authority_peer_id: a.peer_id(),
            authority_epoch: epoch1.epoch_number,
            fencing_token: epoch1.fencing_token,
            config_sequence: config1.sequence,
            config_hash: config1.config_hash().unwrap(),
            canonical_head: None,
            issued_unix_ms: 1,
            expires_unix_ms: 2,
        };
        assert!(local_state_matches_freshness_challenge(&stores[0], &a, &stale).unwrap());

        let mut ballot = RecoveryBallotV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            base_epoch: epoch1.epoch_number,
            base_fencing_token: epoch1.fencing_token,
            target_epoch: epoch1.epoch_number.checked_add(1).unwrap(),
            target_fencing_token: epoch1.fencing_token.checked_add(1).unwrap(),
            round: 1,
            candidate_peer_id: b.peer_id(),
            candidate_public_key: b.public_key(),
            base_snapshot_hash: Hash32([39; 32]),
            base_state_hash: epoch1.base_state_hash,
            membership_hash: membership1.record_hash().unwrap(),
            signature: Vec::new(),
        };
        sign_recovery_ballot(&b, &mut ballot).unwrap();
        for (store, voter) in [(&stores[1], &b), (&stores[2], &c)] {
            let mut vote = RecoveryVoteV1 {
                protocol_version: PROTOCOL_VERSION,
                world_id: world,
                ballot_hash: ballot.ballot_hash().unwrap(),
                base_epoch: ballot.base_epoch,
                target_epoch: ballot.target_epoch,
                round: ballot.round,
                candidate_peer_id: ballot.candidate_peer_id,
                voter_peer_id: voter.peer_id(),
                voter_public_key: voter.public_key(),
                signature: Vec::new(),
            };
            sign_recovery_vote(voter, &mut vote).unwrap();
            assert_eq!(
                store.promise_recovery_ballot(&ballot, &vote).unwrap(),
                swarm_storage::RecoveryPromiseResult::Accepted
            );
        }
        assert!(!local_state_matches_freshness_challenge(&stores[1], &b, &stale).unwrap());
        assert!(!local_state_matches_freshness_challenge(&stores[2], &c, &stale).unwrap());

        let mut epoch2 = EpochRecordV1 {
            protocol_version: PROTOCOL_VERSION,
            world_id: world,
            epoch_number: ballot.target_epoch,
            previous_epoch_hash: None,
            base_state_hash: epoch1.base_state_hash,
            authority_peer_id: b.peer_id(),
            authority_public_key: b.public_key(),
            mode: EpochMode::Quorum,
            fencing_token: ballot.target_fencing_token,
            reason: "durable freshness recovery".into(),
            signature: Vec::new(),
        };
        epoch2.signature = b.sign(&epoch2.signing_bytes().unwrap());
        let mut membership2 = membership1.clone();
        membership2.epoch = epoch2.epoch_number;
        membership2.sequence = membership1.sequence.checked_add(1).unwrap();
        membership2.previous_membership_hash = Some(membership1.record_hash().unwrap());
        membership2.signature.clear();
        b.sign_membership(&mut membership2).unwrap();
        let mut config2 = config1.clone();
        config2.sequence = config1.sequence.checked_add(1).unwrap();
        config2.previous_config_hash = Some(config1.config_hash().unwrap());
        config2.signature.clear();
        sign_world_config(&b, &mut config2).unwrap();
        for store in [&stores[1], &stores[2]] {
            store.save_epoch_record(&epoch2).unwrap();
            store.save_membership_record(&membership2).unwrap();
            store.save_world_config(&config2).unwrap();
            assert!(store.clear_recovery_promise_after_epoch_advance(world, epoch2.epoch_number).unwrap());
        }
        let current = DiscoveryFreshnessChallengeV1 {
            protocol_version: PROTOCOL_VERSION,
            verifier_peer_id: stale.verifier_peer_id,
            nonce: [40; 32],
            world_id: world,
            announcement_hash: Hash32([41; 32]),
            membership_sequence: membership2.sequence,
            membership_hash: membership2.record_hash().unwrap(),
            pending_membership_proposal_hash: None,
            authority_peer_id: b.peer_id(),
            authority_epoch: epoch2.epoch_number,
            fencing_token: epoch2.fencing_token,
            config_sequence: config2.sequence,
            config_hash: config2.config_hash().unwrap(),
            canonical_head: None,
            issued_unix_ms: 3,
            expires_unix_ms: 4,
        };
        assert!(local_state_matches_freshness_challenge(&stores[1], &b, &current).unwrap());
        assert!(local_state_matches_freshness_challenge(&stores[2], &c, &current).unwrap());
        assert!(!local_state_matches_freshness_challenge(&stores[0], &a, &current).unwrap());
        assert!(!local_state_matches_freshness_challenge(&stores[1], &b, &stale).unwrap());
    }

    #[test]
    fn discovery_counter_exhaustion_fails_closed() {
        assert!(next_discovery_sequence(u64::MAX, u64::MAX).is_err());
        assert!(checked_discovery_expiry(u64::MAX, 1, "test").is_err());
    }

'''
insert_before("crates/swarm-cli/src/discovery.rs", "    fn sample_announcement(", durable_test)

# 8. Real network browse+resolve adversarial coverage. The network is real libp2p/Kademlia;
# responders are deterministic protocol peers so ordering and rejection can be asserted.
network_test = r'''use std::{
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use swarm_cli::discovery::{resolve_world, search_public_worlds, DiscoverySearchInputV1, DISCOVERY_CAPABILITY};
use swarm_core::{sign_discovery_freshness_vote, sign_world_announcement, DataPaths, PeerIdentity};
use swarm_network::{generate_transport_key, DiscoveryNetworkEvent, DiscoveryNode, WireRequest, WireResponse};
use swarm_protocol::{
    DiscoveryCanonicalHeadV1, DiscoveryCompatibilityV1, DiscoveryFreshnessChallengeV1, DiscoveryMembershipProofV1,
    Hash32, MembershipPolicyV1, MembershipRecordV1, WorldAnnouncementV1, WorldGenesisV1, WorldMemberV1,
    WorldPresentationV1, WorldVisibilityV1, PROTOCOL_VERSION,
};
use tokio::{task::JoinHandle, time::timeout};

const A: [u8; 32] = [51; 32];
const B: [u8; 32] = [52; 32];
const C: [u8; 32] = [53; 32];
const X: [u8; 32] = [54; 32];

fn identity(secret: [u8; 32]) -> PeerIdentity {
    PeerIdentity::from_secret_bytes(secret)
}

fn member(identity: &PeerIdentity) -> WorldMemberV1 {
    WorldMemberV1 {
        peer_id: identity.peer_id(),
        public_key: identity.public_key(),
        authority_eligible: true,
        banned: false,
    }
}

fn signed_membership(
    authority: &PeerIdentity,
    world: swarm_protocol::WorldId,
    epoch: u64,
    sequence: u64,
    previous: Option<Hash32>,
    members: &[WorldMemberV1],
) -> MembershipRecordV1 {
    let mut record = MembershipRecordV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        epoch,
        sequence,
        previous_membership_hash: previous,
        members: members.to_vec(),
        authority_peer_id: authority.peer_id(),
        authority_public_key: authority.public_key(),
        signature: Vec::new(),
    };
    authority.sign_membership(&mut record).unwrap();
    record
}

fn announcement(
    authority: &PeerIdentity,
    world: swarm_protocol::WorldId,
    membership: &MembershipRecordV1,
    epoch: u64,
    fence: u64,
    config_sequence: u64,
    config_hash: Hash32,
    canonical_head: DiscoveryCanonicalHeadV1,
    sequence: u64,
    name: &str,
) -> WorldAnnouncementV1 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    let mut value = WorldAnnouncementV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        presentation: WorldPresentationV1 {
            name: name.into(),
            description: "network freshness".into(),
            tags: vec!["survival".into()],
            icon_hash: None,
            approximate_region: Some("test".into()),
        },
        compatibility: DiscoveryCompatibilityV1 {
            minecraft_version: "1.21.8".into(),
            loader_id: "fabric".into(),
            loader_version: "0.17.2".into(),
            fabric_adapter_version: "0.5.0".into(),
            compatibility_fingerprint: Hash32([60; 32]),
        },
        visibility: WorldVisibilityV1::Public,
        membership_policy: MembershipPolicyV1::InviteOnly,
        config_sequence,
        config_hash,
        membership_sequence: membership.sequence,
        membership_hash: membership.record_hash().unwrap(),
        authority_epoch: epoch,
        fencing_token: fence,
        canonical_head: Some(canonical_head),
        announcement_sequence: sequence,
        issued_unix_ms: now.saturating_sub(1_000),
        expires_unix_ms: now.checked_add(60_000).unwrap(),
        announcer_peer_id: authority.peer_id(),
        announcer_public_key: authority.public_key(),
        signature: Vec::new(),
    };
    sign_world_announcement(authority, &mut value).unwrap();
    value
}

fn challenge_matches(
    announcement: &WorldAnnouncementV1,
    proof: &DiscoveryMembershipProofV1,
    challenge: &DiscoveryFreshnessChallengeV1,
) -> bool {
    let pending = proof
        .pending_membership
        .as_ref()
        .map(|proposal| proposal.proposal_hash().unwrap());
    challenge.protocol_version == PROTOCOL_VERSION
        && challenge.world_id == announcement.world_id
        && challenge.announcement_hash == announcement.announcement_hash().unwrap()
        && challenge.membership_sequence == announcement.membership_sequence
        && challenge.membership_hash == announcement.membership_hash
        && challenge.pending_membership_proposal_hash == pending
        && challenge.authority_peer_id == announcement.announcer_peer_id
        && challenge.authority_epoch == announcement.authority_epoch
        && challenge.fencing_token == announcement.fencing_token
        && challenge.config_sequence == announcement.config_sequence
        && challenge.config_hash == announcement.config_hash
        && challenge.canonical_head == announcement.canonical_head
}

struct PeerPlan {
    label: &'static str,
    identity: PeerIdentity,
    node: DiscoveryNode,
    announcement: Option<WorldAnnouncementV1>,
    context: Option<DiscoveryMembershipProofV1>,
    vote_state: Option<(WorldAnnouncementV1, DiscoveryMembershipProofV1)>,
    malformed_context: bool,
    public_provider: bool,
    world_provider: bool,
    delay_ms: u64,
}

async fn make_node(secret: [u8; 32]) -> (PeerIdentity, DiscoveryNode, String) {
    let identity = identity(secret);
    let hello = identity.signed_peer_hello(vec![DISCOVERY_CAPABILITY.into()]).unwrap();
    let mut node = DiscoveryNode::new(generate_transport_key(), hello, identity.network_signing_key()).unwrap();
    node.listen("/ip4/127.0.0.1/udp/0/quic-v1".parse().unwrap()).unwrap();
    let address = timeout(Duration::from_secs(10), async {
        loop {
            if let DiscoveryNetworkEvent::Listening { address } = node.next_event().await.unwrap() {
                break format!("{address}/p2p/{}", node.local_transport_peer_id());
            }
        }
    })
    .await
    .expect("discovery provider should listen");
    (identity, node, address)
}

fn spawn_peer(mut plan: PeerPlan, order: Arc<Mutex<Vec<&'static str>>>) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let Ok(event) = plan.node.next_event().await else { continue };
            let DiscoveryNetworkEvent::InboundRequest { request, channel, .. } = event else { continue };
            match request {
                WireRequest::DiscoveryPublic { .. } => {
                    if plan.delay_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(plan.delay_ms)).await;
                    }
                    order.lock().unwrap().push(plan.label);
                    let values = plan.announcement.clone().into_iter().collect();
                    let _ = plan.node.respond(channel, WireResponse::DiscoveryWorlds(values));
                }
                WireRequest::DiscoveryResolve { world_id } => {
                    if plan.delay_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(plan.delay_ms)).await;
                    }
                    order.lock().unwrap().push(plan.label);
                    let value = plan
                        .announcement
                        .clone()
                        .filter(|announcement| announcement.world_id == world_id)
                        .map(Box::new);
                    let _ = plan.node.respond(channel, WireResponse::DiscoveryResolved(value));
                }
                WireRequest::DiscoveryFreshnessContext { announcement_hash, .. } => {
                    let response = if plan.malformed_context {
                        plan.context.clone().map(|mut proof| {
                            proof.protocol_version = PROTOCOL_VERSION + 1;
                            Box::new(proof)
                        })
                    } else {
                        plan.announcement.as_ref().and_then(|announcement| {
                            (announcement.announcement_hash().ok() == Some(announcement_hash))
                                .then(|| plan.context.clone().map(Box::new))
                                .flatten()
                        })
                    };
                    let _ = plan.node.respond(channel, WireResponse::DiscoveryFreshnessContext(response));
                }
                WireRequest::DiscoveryFreshnessVote(challenge) => {
                    let response = plan.vote_state.as_ref().and_then(|(announcement, proof)| {
                        challenge_matches(announcement, proof, &challenge)
                            .then(|| sign_discovery_freshness_vote(&plan.identity, &challenge).ok())
                            .flatten()
                            .map(Box::new)
                    });
                    let _ = plan.node.respond(channel, WireResponse::DiscoveryFreshnessVote(response));
                }
                _ => {
                    let _ = plan.node.respond(
                        channel,
                        WireResponse::Error { code: "TEST_UNSUPPORTED".into(), message: "unsupported".into() },
                    );
                }
            }
        }
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn malicious_and_stale_providers_cannot_win_browse_or_exact_resolve() {
    let a_record_id = identity(A);
    let b_record_id = identity(B);
    let c_record_id = identity(C);
    let attacker_record_id = identity(X);
    let mut members = vec![member(&a_record_id), member(&b_record_id), member(&c_record_id)];
    members.sort_by_key(|member| member.peer_id);
    let genesis = WorldGenesisV1 {
        protocol_version: PROTOCOL_VERSION,
        minecraft_version: "1.21.8".into(),
        fabric_loader_version: "0.17.2".into(),
        compatibility_fingerprint: Hash32([60; 32]),
        creation_nonce: [61; 32],
        creator_public_key: a_record_id.public_key(),
        initial_membership: members.iter().map(|member| member.peer_id).collect(),
    };
    let world = genesis.world_id().unwrap();
    let initial = signed_membership(&a_record_id, world, 1, 0, None, &members);
    let current = signed_membership(
        &b_record_id,
        world,
        2,
        1,
        Some(initial.record_hash().unwrap()),
        &members,
    );
    let stale_proof = DiscoveryMembershipProofV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        genesis: genesis.clone(),
        initial_membership: initial.clone(),
        membership_certificates: Vec::new(),
        current_membership: initial.clone(),
        pending_membership: None,
    };
    let current_proof = DiscoveryMembershipProofV1 {
        protocol_version: PROTOCOL_VERSION,
        world_id: world,
        genesis,
        initial_membership: initial.clone(),
        membership_certificates: Vec::new(),
        current_membership: current.clone(),
        pending_membership: None,
    };
    let stale_announcement = announcement(
        &a_record_id,
        world,
        &initial,
        1,
        1,
        1,
        Hash32([62; 32]),
        DiscoveryCanonicalHeadV1 { snapshot_number: 1, manifest_hash: Hash32([63; 32]), epoch: 1, sequence: 1 },
        1,
        "stale-first",
    );
    let current_announcement = announcement(
        &b_record_id,
        world,
        &current,
        2,
        2,
        2,
        Hash32([64; 32]),
        DiscoveryCanonicalHeadV1 { snapshot_number: 2, manifest_hash: Hash32([65; 32]), epoch: 2, sequence: 2 },
        2,
        "current",
    );
    let mut attacker_announcement = current_announcement.clone();
    attacker_announcement.presentation.name = "attacker-first".into();
    attacker_announcement.announcement_sequence = 3;
    attacker_announcement.signature.clear();
    sign_world_announcement(&attacker_record_id, &mut attacker_announcement).unwrap();

    let (b_identity, mut b_node, b_address) = make_node(B).await;
    let (c_identity, mut c_node, c_address) = make_node(C).await;
    let (a_identity, mut a_node, a_address) = make_node(A).await;
    let (x_identity, mut x_node, x_address) = make_node(X).await;

    b_node.add_bootstrap_address(c_address.parse().unwrap()).unwrap();
    c_node.add_bootstrap_address(b_address.parse().unwrap()).unwrap();
    a_node.add_bootstrap_address(b_address.parse().unwrap()).unwrap();
    x_node.add_bootstrap_address(b_address.parse().unwrap()).unwrap();
    let _ = b_node.bootstrap();
    let _ = c_node.bootstrap();
    let _ = a_node.bootstrap();
    let _ = x_node.bootstrap();
    for node in [&mut b_node, &mut a_node, &mut x_node] {
        node.start_providing_public_directory().unwrap();
    }
    for node in [&mut b_node, &mut c_node, &mut a_node, &mut x_node] {
        node.start_providing_world(world).unwrap();
    }

    let order = Arc::new(Mutex::new(Vec::new()));
    let tasks = vec![
        spawn_peer(PeerPlan {
            label: "current",
            identity: b_identity,
            node: b_node,
            announcement: Some(current_announcement.clone()),
            context: Some(current_proof.clone()),
            vote_state: Some((current_announcement.clone(), current_proof.clone())),
            malformed_context: false,
            public_provider: true,
            world_provider: true,
            delay_ms: 250,
        }, order.clone()),
        spawn_peer(PeerPlan {
            label: "voter",
            identity: c_identity,
            node: c_node,
            announcement: None,
            context: None,
            vote_state: Some((current_announcement.clone(), current_proof.clone())),
            malformed_context: false,
            public_provider: false,
            world_provider: true,
            delay_ms: 0,
        }, order.clone()),
        spawn_peer(PeerPlan {
            label: "stale",
            identity: a_identity,
            node: a_node,
            announcement: Some(stale_announcement.clone()),
            context: Some(stale_proof.clone()),
            vote_state: Some((stale_announcement, stale_proof)),
            malformed_context: false,
            public_provider: true,
            world_provider: true,
            delay_ms: 0,
        }, order.clone()),
        spawn_peer(PeerPlan {
            label: "attacker",
            identity: x_identity,
            node: x_node,
            announcement: Some(attacker_announcement),
            context: Some(current_proof.clone()),
            vote_state: None,
            malformed_context: true,
            public_provider: true,
            world_provider: true,
            delay_ms: 10,
        }, order.clone()),
    ];

    tokio::time::sleep(Duration::from_secs(3)).await;
    let temp = tempfile::tempdir().unwrap();
    let paths = DataPaths::from_root(temp.path());
    let bootstraps = vec![b_address.clone(), c_address.clone(), a_address.clone(), x_address.clone()];
    let report = search_public_worlds(&paths, DiscoverySearchInputV1::default(), &bootstraps).await.unwrap();
    assert_eq!(report.results.len(), 1, "only the live canonical proof may survive browse: {report:?}");
    assert_eq!(report.results[0].announcer_peer_id, b_record_id.peer_id().to_string());
    let browse_order = order.lock().unwrap().clone();
    assert!(browse_order.contains(&"stale"));
    assert!(browse_order.contains(&"attacker"));
    assert!(browse_order.iter().position(|label| *label == "stale") < browse_order.iter().position(|label| *label == "current"));

    order.lock().unwrap().clear();
    let resolved = resolve_world(&paths, world, &bootstraps).await.unwrap();
    assert_eq!(resolved.state, "found", "current authority should resolve after stale/attacker candidates: {resolved:?}");
    let card = resolved.world.expect("current world card");
    assert_eq!(card.announcer_peer_id, b_record_id.peer_id().to_string());
    let resolve_order = order.lock().unwrap().clone();
    assert!(resolve_order.contains(&"stale"));
    assert!(resolve_order.contains(&"attacker"));
    assert_ne!(resolve_order.first().copied(), Some("current"), "resolver must tolerate a noncanonical first response");

    for task in tasks {
        task.abort();
    }
}
'''
network_path = Path("crates/swarm-cli/tests/discovery_network_freshness.rs")
if network_path.exists():
    existing = network_path.read_text()
    if existing != network_test:
        raise SystemExit("unexpected pre-existing discovery_network_freshness.rs")
else:
    network_path.write_text(network_test)

# Remove fields that are intentionally compile-time dead in the plan definition if rustc flags them.
replace_once(
    "crates/swarm-cli/tests/discovery_network_freshness.rs",
    '''    malformed_context: bool,\n    public_provider: bool,\n    world_provider: bool,\n    delay_ms: u64,\n''',
    '''    malformed_context: bool,\n    delay_ms: u64,\n''',
)
p = Path("crates/swarm-cli/tests/discovery_network_freshness.rs")
text = p.read_text().replace("            public_provider: true,\n            world_provider: true,\n", "")
text = text.replace("            public_provider: false,\n            world_provider: true,\n", "")
p.write_text(text)

# Final semantic assertions: the one-shot materializer never silently omits a required repair.
required = {
    "crates/swarm-cli/src/discovery.rs": [
        "load_recovery_promise(world)",
        "MAX_DISCOVERY_FRESHNESS_VOTES",
        "durable_recovery_promise_fences_stale_freshness_and_current_majority_recovers",
        "membership_sequence: 0",
        "canonical_head: None",
    ],
    "crates/swarm-core/src/discovery.rs": [
        "MAX_DISCOVERY_MEMBERS: usize = 1_024",
        "MAX_DISCOVERY_FRESHNESS_VOTES: usize = 1_024",
        "last_certificate_sequence",
    ],
    "crates/swarm-consensus/src/membership.rs": ["votes.len() > 1_024", "last_sequence"],
    "crates/swarm-network/src/wire.rs": ["membership_hash: Hash32([6; 32])", "member_count"],
    "crates/swarm-cli/tests/discovery_freshness.rs": [
        "unsupported_malformed_removed_and_oversized_proofs_fail_closed",
        "verifier_bound_proof_and_pretransition_proof_cannot_cross_contexts",
    ],
    "crates/swarm-cli/tests/discovery_network_freshness.rs": [
        "malicious_and_stale_providers_cannot_win_browse_or_exact_resolve",
    ],
}
for path, needles in required.items():
    text = Path(path).read_text()
    for needle in needles:
        if needle not in text:
            raise SystemExit(f"missing required FINAL-028 semantic state in {path}: {needle}")

print("FINAL-028 direct production/test source materialized")
