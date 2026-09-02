from pathlib import Path


def replace(path: str, old: str, new: str, count: int = 1) -> None:
    p = Path(path)
    text = p.read_text()
    found = text.count(old)
    if found != count:
        raise SystemExit(f"{path}: expected {count}, found {found}: {old[:120]!r}")
    p.write_text(text.replace(old, new, count))


daemon = "crates/swarm-cli/src/daemon.rs"
replace(
    daemon,
    "    RecoveryVoteV1, SnapshotManifestV1, SoloBranchV1, TransferPhase, WorldDescriptorV1, WorldId, WorldStatusV1,\n",
    "    RecoveryVoteV1, SnapshotManifestV1, TransferPhase, WorldDescriptorV1, WorldId, WorldStatusV1,\n",
)
replace(
    daemon,
    "    MembershipCommit { world: WorldId, peer: PeerId, sequence: u64 },\n",
    "    MembershipCommit { world: WorldId, sequence: u64 },\n",
)
replace(
    daemon,
    "                    outbound.insert(request_key(&id), OutboundContext::MembershipCommit {\n                        world: metadata.world_id,\n                        peer: application_peer,\n                        sequence: membership.sequence,\n                    });\n",
    "                    outbound.insert(request_key(&id), OutboundContext::MembershipCommit {\n                        world: metadata.world_id,\n                        sequence: membership.sequence,\n                    });\n",
)
replace(
    daemon,
    "            Some(OutboundContext::MembershipCommit { world, peer: _, sequence }),\n",
    "            Some(OutboundContext::MembershipCommit { world, sequence }),\n",
)
replace(
    daemon,
    "        outbound.insert(request_key(&id), OutboundContext::MembershipCommit {\n            world, peer: member.peer_id, sequence: certificate.proposal.proposed.sequence,\n        });\n",
    "        outbound.insert(request_key(&id), OutboundContext::MembershipCommit {\n            world, sequence: certificate.proposal.proposed.sequence,\n        });\n",
)

test = "crates/swarm-cli/tests/live_join_replication.rs"
replace(test, "        mode: EpochMode::Solo,\n", "        mode: EpochMode::Quorum,\n")
replace(
    test,
    "        reason: \"live join acceptance seed\".into(),\n",
    "        reason: \"live join quorum-of-one seed\".into(),\n",
)
