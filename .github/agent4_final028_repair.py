from pathlib import Path


def replace(path, old, new):
    p = Path(path)
    text = p.read_text()
    if old not in text:
        if new in text:
            return
        raise SystemExit(f"missing repair anchor in {path}: {old[:120]!r}")
    p.write_text(text.replace(old, new, 1))

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

print("Agent 4 FINAL-028 repair patch applied")
