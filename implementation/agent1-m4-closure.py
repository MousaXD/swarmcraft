from pathlib import Path


def replace(path: str, old: str, new: str, count: int = 1) -> None:
    p = Path(path)
    text = p.read_text()
    found = text.count(old)
    if found != count:
        raise SystemExit(f"{path}: expected {count}, found {found}: {old[:140]!r}")
    p.write_text(text.replace(old, new, count))


# Clippy closure after the main daemon hardening has removed the obsolete local_peer argument.
daemon = "crates/swarm-cli/src/daemon.rs"
replace(
    daemon,
    "    response: WireResponse,\n    outbound: &mut HashMap<String, OutboundContext>,\n    runtime: &mut LeaseRuntime,\n    now: Instant,\n) -> Result<()> {\n    match (context, response) {\n",
    "    response: WireResponse,\n    outbound: &mut HashMap<String, OutboundContext>,\n    runtime: &mut LeaseRuntime,\n) -> Result<()> {\n    let now = Instant::now();\n    match (context, response) {\n",
)
replace(
    daemon,
    "                            response,\n                            &mut outbound,\n                            &mut leases,\n                            Instant::now(),\n",
    "                            response,\n                            &mut outbound,\n                            &mut leases,\n",
)
replace(
    daemon,
    "fn proposal_member<'a>(proposal: &'a MembershipProposalV1, peer: PeerId) -> Option<&'a swarm_protocol::WorldMemberV1> {\n",
    "fn proposal_member(proposal: &MembershipProposalV1, peer: PeerId) -> Option<&swarm_protocol::WorldMemberV1> {\n",
)

# Finish FINAL-039 in production migration/wake/artifact paths. The earlier hardening
# script already converts checkpoint sequences and manual-transfer generation checks.
migration = "crates/swarm-cli/src/migration.rs"
replace(
    migration,
    "            epoch_number: previous.epoch_number.saturating_add(1),\n",
    "            epoch_number: previous.epoch_number.checked_add(1).context(\"authority epoch exhausted during wake\")?,\n",
)
replace(
    migration,
    "            fencing_token: previous.fencing_token.saturating_add(1),\n",
    "            fencing_token: previous.fencing_token.checked_add(1).context(\"authority fencing token exhausted during wake\")?,\n",
)
replace(
    migration,
    "        if latest.epoch.saturating_add(1) != epoch.epoch_number || latest.state_root != epoch.base_state_hash {\n",
    "        if latest.epoch.checked_add(1).context(\"snapshot epoch counter exhausted during authority promotion\")? != epoch.epoch_number\n            || latest.state_root != epoch.base_state_hash\n        {\n",
)
replace(
    migration,
    "                sequence: membership.sequence.saturating_add(1),\n",
    "                sequence: membership.sequence.checked_add(1).context(\"membership sequence counter exhausted\")?,\n",
)

# FINAL-045: make the legacy migration model fail closed at the same counter boundary
# as production instead of wrapping in release builds.
legacy = "crates/swarm-consensus/src/migration.rs"
replace(
    legacy,
    '''    #[error("automatic crash takeover requires the configured quorum")]
    NoQuorum,
''',
    '''    #[error("automatic crash takeover requires the configured quorum")]
    NoQuorum,
    #[error("authority generation counter exhausted")]
    GenerationExhausted,
''',
)
replace(
    legacy,
    '''    Ok(AuthorityGeneration {
        authority_peer_id: candidate.candidate.peer_id,
        epoch: current_lease.epoch() + 1,
        fencing_token: current_lease.fencing_token() + 1,
        base_snapshot_hash: required_snapshot_hash,
        mode: TakeoverMode::Quorum,
    })
''',
    '''    let epoch = current_lease.epoch().checked_add(1).ok_or(LeaseError::GenerationExhausted)?;
    let fencing_token = current_lease
        .fencing_token()
        .checked_add(1)
        .ok_or(LeaseError::GenerationExhausted)?;
    Ok(AuthorityGeneration {
        authority_peer_id: candidate.candidate.peer_id,
        epoch,
        fencing_token,
        base_snapshot_hash: required_snapshot_hash,
        mode: TakeoverMode::Quorum,
    })
''',
)
replace(
    legacy,
    '''    #[error("manual transfer signer is not the expected participant")]
    WrongPeer,
''',
    '''    #[error("manual transfer signer is not the expected participant")]
    WrongPeer,
    #[error("authority generation counter exhausted")]
    GenerationExhausted,
''',
)
replace(
    legacy,
    '''    pub fn prepare(
        from_peer: PeerId,
        to_peer: PeerId,
        snapshot_hash: Hash32,
        current_epoch: u64,
        current_fencing_token: u64,
    ) -> Self {
        Self {
            from_peer,
            to_peer,
            snapshot_hash,
            next_epoch: current_epoch + 1,
            next_fencing_token: current_fencing_token + 1,
            phase: TransferPhase::Prepared,
        }
    }
''',
    '''    pub fn prepare(
        from_peer: PeerId,
        to_peer: PeerId,
        snapshot_hash: Hash32,
        current_epoch: u64,
        current_fencing_token: u64,
    ) -> Result<Self, TransferError> {
        let next_epoch = current_epoch.checked_add(1).ok_or(TransferError::GenerationExhausted)?;
        let next_fencing_token = current_fencing_token.checked_add(1).ok_or(TransferError::GenerationExhausted)?;
        Ok(Self {
            from_peer,
            to_peer,
            snapshot_hash,
            next_epoch,
            next_fencing_token,
            phase: TransferPhase::Prepared,
        })
    }
''',
)
replace(
    legacy,
    '''        let generation = AuthorityGeneration {
            authority_peer_id: candidate,
            epoch: *epoch + 1,
            fencing_token: *fencing_token + 1,
            base_snapshot_hash: *latest_snapshot_hash,
            mode: TakeoverMode::Solo,
        };
''',
    '''        let next_epoch = epoch.checked_add(1).ok_or(LeaseError::GenerationExhausted)?;
        let next_fencing_token = fencing_token.checked_add(1).ok_or(LeaseError::GenerationExhausted)?;
        let generation = AuthorityGeneration {
            authority_peer_id: candidate,
            epoch: next_epoch,
            fencing_token: next_fencing_token,
            base_snapshot_hash: *latest_snapshot_hash,
            mode: TakeoverMode::Solo,
        };
''',
)

legacy_tests = "crates/swarm-consensus/tests/migration.rs"
replace(
    legacy_tests,
    "    let mut transfer = ManualTransferState::prepare(from, to, Hash32([9; 32]), 7, 11);\n",
    "    let mut transfer = ManualTransferState::prepare(from, to, Hash32([9; 32]), 7, 11).unwrap();\n",
)
p = Path(legacy_tests)
text = p.read_text()
if "legacy_generation_counter_exhaustion_fails_closed" not in text:
    text += r'''

#[test]
fn legacy_generation_counter_exhaustion_fails_closed() {
    let start = Instant::now();
    let lease = LeaseTracker::new(u64::MAX, u64::MAX, Duration::from_secs(1), start);
    let candidate = TakeoverCandidate {
        candidate: eligible(2, u64::MAX, 1),
        snapshot_hash: Hash32([7; 32]),
        peer_votes: 2,
    };
    assert_eq!(
        evaluate_crash_takeover(
            &lease,
            &candidate,
            Hash32([7; 32]),
            start + Duration::from_secs(2),
            TakeoverPolicy::default(),
        ),
        Err(LeaseError::GenerationExhausted)
    );

    assert_eq!(
        ManualTransferState::prepare(PeerId([1; 32]), PeerId([2; 32]), Hash32([9; 32]), u64::MAX, 11),
        Err(TransferError::GenerationExhausted)
    );

    let mut sleeping = WorldRuntimeState::Active(AuthorityGeneration {
        authority_peer_id: PeerId([1; 32]),
        epoch: u64::MAX,
        fencing_token: u64::MAX,
        base_snapshot_hash: Hash32([4; 32]),
        mode: TakeoverMode::Quorum,
    });
    sleeping.sleep(Hash32([7; 32]));
    assert_eq!(
        sleeping.wake(PeerId([2; 32]), Hash32([7; 32])),
        Err(LeaseError::GenerationExhausted)
    );
}
'''
    p.write_text(text)

# The five-member recovery fixture must form a real three-voter majority mesh.
# A star lets only the hub see all three voters; if the deterministic winner is a
# leaf, that leaf correctly refuses recovery because it sees only two of five.
partition = "crates/swarm-cli/tests/consensus_partition_safety.rs"
replace(
    partition,
    '''    let majority = &refs[majority_start..];
    let bootstrap = address(majority[0]);
    let mut survivors = Vec::new();
    survivors.push(spawn_daemon(majority[0], &[]));
    thread::sleep(Duration::from_millis(400));
    for peer in majority.iter().skip(1) {
        survivors.push(spawn_daemon(peer, std::slice::from_ref(&bootstrap)));
        thread::sleep(Duration::from_millis(250));
    }
''',
    '''    let majority = &refs[majority_start..];
    let majority_addrs = majority.iter().map(|peer| address(peer)).collect::<Vec<_>>();
    let mut survivors = Vec::new();
    for (index, peer) in majority.iter().enumerate() {
        let bootstraps = majority_addrs.iter().take(index).cloned().collect::<Vec<_>>();
        survivors.push(spawn_daemon(peer, &bootstraps));
        thread::sleep(Duration::from_millis(300));
    }
''',
)
