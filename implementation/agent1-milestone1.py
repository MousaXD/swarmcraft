from pathlib import Path


def replace(path: str, old: str, new: str, count: int = 1) -> None:
    file = Path(path)
    text = file.read_text()
    actual = text.count(old)
    if actual != count:
        raise SystemExit(f"{path}: expected {count} copies, found {actual}: {old[:80]!r}")
    file.write_text(text.replace(old, new, count))


# Centralize strict authority-generation advancement in production-used consensus code.
replace(
    "crates/swarm-consensus/src/lib.rs",
    """#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]\npub struct AuthorityGeneration {\n    pub epoch: u64,\n    pub fencing_token: u64,\n}\n""",
    """#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]\npub struct AuthorityGeneration {\n    pub epoch: u64,\n    pub fencing_token: u64,\n}\n\n#[derive(Debug, Error, PartialEq, Eq)]\npub enum GenerationError {\n    #[error(\"authority epoch exhausted at u64::MAX\")]\n    EpochExhausted,\n    #[error(\"authority fencing token exhausted at u64::MAX\")]\n    FencingTokenExhausted,\n}\n\nimpl AuthorityGeneration {\n    /// Return the unique next authority generation, failing closed on counter exhaustion.\n    pub fn checked_next(self) -> Result<Self, GenerationError> {\n        let epoch = self.epoch.checked_add(1).ok_or(GenerationError::EpochExhausted)?;\n        let fencing_token = self\n            .fencing_token\n            .checked_add(1)\n            .ok_or(GenerationError::FencingTokenExhausted)?;\n        Ok(Self { epoch, fencing_token })\n    }\n}\n""",
)
replace(
    "crates/swarm-consensus/src/lib.rs",
    """    #[test]\n    fn lease_uses_monotonic_deadline_and_rejects_stale_generation() {\n""",
    """    #[test]\n    fn authority_generation_fails_closed_at_counter_exhaustion() {\n        let max = AuthorityGeneration { epoch: u64::MAX - 1, fencing_token: u64::MAX - 1 }\n            .checked_next()\n            .unwrap();\n        assert_eq!(max, AuthorityGeneration { epoch: u64::MAX, fencing_token: u64::MAX });\n        assert_eq!(max.checked_next(), Err(GenerationError::EpochExhausted));\n        assert_eq!(\n            AuthorityGeneration { epoch: 7, fencing_token: u64::MAX }.checked_next(),\n            Err(GenerationError::FencingTokenExhausted)\n        );\n    }\n\n    #[test]\n    fn lease_uses_monotonic_deadline_and_rejects_stale_generation() {\n""",
)
replace(
    "crates/swarm-consensus/src/lib.rs",
    """                        self.generation.epoch = self.generation.epoch.saturating_add(1);\n                        self.generation.fencing_token = self.generation.fencing_token.saturating_add(1);\n""",
    """                        self.generation = self\n                            .generation\n                            .checked_next()\n                            .expect(\"chaos simulation exhausted authority generation\");\n""",
)

# Recovery ballots must never treat MAX as its own successor.
replace(
    "crates/swarm-protocol/src/v2.rs",
    """        self.protocol_version == PROTOCOL_VERSION\n            && self.target_epoch == self.base_epoch.saturating_add(1)\n            && self.target_fencing_token == self.base_fencing_token.saturating_add(1)\n            && self.round > 0\n""",
    """        self.protocol_version == PROTOCOL_VERSION\n            && self.base_epoch.checked_add(1) == Some(self.target_epoch)\n            && self.base_fencing_token.checked_add(1) == Some(self.target_fencing_token)\n            && self.round > 0\n""",
)

# A voter that has accepted one recovery value may advance the round, but it may not
# switch candidates for the same target generation. A quorum certificate therefore
# locks a quorum of voters to one value, preventing a later conflicting certificate.
replace(
    "crates/swarm-storage/src/state.rs",
    """        && a.target_fencing_token == b.target_fencing_token\n        && a.base_snapshot_hash == b.base_snapshot_hash\n""",
    """        && a.target_fencing_token == b.target_fencing_token\n        && a.candidate_peer_id == b.candidate_peer_id\n        && a.candidate_public_key == b.candidate_public_key\n        && a.base_snapshot_hash == b.base_snapshot_hash\n""",
)
replace(
    "crates/swarm-storage/src/state.rs",
    """    fn recovery_promise_survives_restart_and_blocks_stale_candidate() {\n        let temp = tempfile::tempdir().unwrap();\n        let (_, world) = test_world();\n        let store = Storage::open(temp.path()).unwrap();\n        let bob = ballot(world, 2, 1);\n        assert_eq!(store.promise_recovery_ballot(&bob, &vote(&bob, 6)).unwrap(), RecoveryPromiseResult::Accepted);\n        drop(store);\n\n        let store = Storage::open(temp.path()).unwrap();\n        let charlie = ballot(world, 3, 2);\n        assert_eq!(\n            store.promise_recovery_ballot(&charlie, &vote(&charlie, 6)).unwrap(),\n            RecoveryPromiseResult::Accepted\n        );\n        assert_eq!(\n            store.promise_recovery_ballot(&bob, &vote(&bob, 6)).unwrap(),\n            RecoveryPromiseResult::Rejected { highest_round: 2 }\n        );\n    }\n""",
    """    fn recovery_promise_survives_restart_and_preserves_the_accepted_value() {\n        let temp = tempfile::tempdir().unwrap();\n        let (_, world) = test_world();\n        let store = Storage::open(temp.path()).unwrap();\n        let bob = ballot(world, 2, 1);\n        assert_eq!(store.promise_recovery_ballot(&bob, &vote(&bob, 6)).unwrap(), RecoveryPromiseResult::Accepted);\n        drop(store);\n\n        let store = Storage::open(temp.path()).unwrap();\n        let charlie = ballot(world, 3, 2);\n        assert_eq!(\n            store.promise_recovery_ballot(&charlie, &vote(&charlie, 6)).unwrap(),\n            RecoveryPromiseResult::Rejected { highest_round: 1 }\n        );\n        let bob_round_two = ballot(world, 2, 2);\n        assert_eq!(\n            store.promise_recovery_ballot(&bob_round_two, &vote(&bob_round_two, 6)).unwrap(),\n            RecoveryPromiseResult::Accepted\n        );\n    }\n""",
)

# Single-member worlds already constitute their own canonical quorum. Multi-member
# worlds must never manufacture a writable Solo authority merely because the old
# authority lost quorum.
replace(
    "crates/swarm-cli/src/daemon.rs",
    """            if member_count <= 1 {\n                clear_permit(paths, world)?;\n                continue;\n            }\n            request_world_statuses(storage, node, outbound, runtime, &descriptor, identity.peer_id())?;\n""",
    """            request_world_statuses(storage, node, outbound, runtime, &descriptor, identity.peer_id())?;\n""",
)
replace(
    "crates/swarm-cli/src/daemon.rs",
    """        let recovery_generation = AuthorityGeneration {\n            epoch: generation.epoch.saturating_add(1),\n            fencing_token: generation.fencing_token.saturating_add(1),\n        };\n""",
    """        let recovery_generation = generation\n            .checked_next()\n            .context(\"authority generation exhausted during crash recovery\")?;\n""",
)
replace(
    "crates/swarm-cli/src/daemon.rs",
    """        let round = floor.saturating_add(1).max(1);\n""",
    """        let round = floor.checked_add(1).context(\"recovery round counter exhausted\")?.max(1);\n""",
)
replace(
    "crates/swarm-cli/src/daemon.rs",
    """    } else if solo_mode_allowed(context.storage, world)? {\n        request_world_statuses(\n            context.storage,\n            node,\n            outbound,\n            runtime,\n            context.descriptor,\n            context.identity.peer_id(),\n        )?;\n        if context.epoch.mode != EpochMode::Solo {\n            promote_to_solo(context.storage, context.identity, context.epoch)?;\n            clear_permit(context.paths, world)?;\n            return Ok(());\n        }\n        refresh_solo_branch(context.storage, context.identity, context.epoch)?;\n        let heartbeat = runtime.permit_heartbeats.entry(world).or_default();\n        *heartbeat = heartbeat.saturating_add(1);\n        refresh_permit(context.paths, world, context.generation, *heartbeat)?;\n    } else {\n""",
    """    } else {\n""",
)
replace(
    "crates/swarm-cli/src/daemon.rs",
    """fn promote_to_solo(storage: &Storage, identity: &PeerIdentity, previous: &EpochRecordV1) -> Result<EpochRecordV1> {\n    if previous.authority_peer_id != identity.peer_id() || previous.authority_public_key != identity.public_key() {\n        return Err(anyhow!(\"only the accepted authority may enter solo mode\"));\n    }\n    if !solo_mode_allowed(storage, previous.world_id)? {\n        return Err(anyhow!(\"solo advancement is disabled by the signed world configuration\"));\n    }\n    let latest =\n        storage.latest_snapshot(previous.world_id)?.context(\"cannot enter solo mode without a canonical snapshot\")?;\n    verify_snapshot_signature(&latest)?;\n    let next_epoch = previous.epoch_number.saturating_add(1);\n    let mut branch = SoloBranchV1 {\n        protocol_version: PROTOCOL_VERSION,\n        world_id: previous.world_id,\n        base_snapshot_hash: latest.manifest_hash()?,\n        base_epoch: previous.epoch_number,\n        head_snapshot_hash: latest.manifest_hash()?,\n        head_epoch: next_epoch,\n        head_sequence: latest.sequence,\n        state_hash: latest.state_root,\n        authority_peer_id: identity.peer_id(),\n        authority_public_key: identity.public_key(),\n        signature: Vec::new(),\n    };\n    sign_solo_branch(identity, &mut branch)?;\n    // Preserve ancestry before making the solo epoch current. A crash can leave an\n    // inert future branch, but never an active solo epoch with forgotten ancestry.\n    storage.save_solo_branch(&branch)?;\n\n    let mut next = EpochRecordV1 {\n        protocol_version: PROTOCOL_VERSION,\n        world_id: previous.world_id,\n        epoch_number: next_epoch,\n        previous_epoch_hash: Some(epoch_record_hash(previous)?),\n        base_state_hash: latest.state_root,\n        authority_peer_id: identity.peer_id(),\n        authority_public_key: identity.public_key(),\n        mode: EpochMode::Solo,\n        fencing_token: previous.fencing_token.saturating_add(1),\n        reason: \"solo advancement permitted by signed world policy while quorum is unavailable\".into(),\n        signature: Vec::new(),\n    };\n    next.signature = identity.sign(&next.signing_bytes()?);\n    storage.save_epoch_record(&next)?;\n    info!(world = %previous.world_id, epoch = next.epoch_number, \"entered explicit solo mode\");\n    Ok(next)\n}\n\n""",
    """,
)
replace(
    "crates/swarm-cli/src/daemon.rs",
    """    let mut next = EpochRecordV1 {\n        protocol_version: PROTOCOL_VERSION,\n        world_id: context.descriptor.world_id,\n        epoch_number: context.epoch.epoch_number.saturating_add(1),\n""",
    """    let next_generation = context\n        .generation\n        .checked_next()\n        .context(\"authority generation exhausted while restoring quorum\")?;\n    let mut next = EpochRecordV1 {\n        protocol_version: PROTOCOL_VERSION,\n        world_id: context.descriptor.world_id,\n        epoch_number: next_generation.epoch,\n""",
)
replace(
    "crates/swarm-cli/src/daemon.rs",
    """        mode: EpochMode::Quorum,\n        fencing_token: context.epoch.fencing_token.saturating_add(1),\n""",
    """        mode: EpochMode::Quorum,\n        fencing_token: next_generation.fencing_token,\n""",
)
replace(
    "crates/swarm-cli/src/daemon.rs",
    """    let mut next = EpochRecordV1 {\n        protocol_version: PROTOCOL_VERSION,\n        world_id: previous.world_id,\n        epoch_number: previous.epoch_number.saturating_add(1),\n""",
    """    let next_generation = AuthorityGeneration {\n        epoch: previous.epoch_number,\n        fencing_token: previous.fencing_token,\n    }\n    .checked_next()\n    .context(\"authority generation exhausted during recovery promotion\")?;\n    let mut next = EpochRecordV1 {\n        protocol_version: PROTOCOL_VERSION,\n        world_id: previous.world_id,\n        epoch_number: next_generation.epoch,\n""",
)
replace(
    "crates/swarm-cli/src/daemon.rs",
    """        mode: EpochMode::Recovery,\n        fencing_token: previous.fencing_token.saturating_add(1),\n""",
    """        mode: EpochMode::Recovery,\n        fencing_token: next_generation.fencing_token,\n""",
)
replace(
    "crates/swarm-cli/src/daemon.rs",
    """        if latest.epoch.saturating_add(1) != epoch.epoch_number || latest.state_root != epoch.base_state_hash {\n""",
    """        if latest.epoch.checked_add(1) != Some(epoch.epoch_number) || latest.state_root != epoch.base_state_hash {\n""",
)
replace(
    "crates/swarm-cli/src/daemon.rs",
    """            sequence: latest.sequence.saturating_add(1),\n""",
    """            sequence: latest.sequence.checked_add(1).context(\"snapshot sequence counter exhausted\")?,\n""",
)
replace(
    "crates/swarm-cli/src/daemon.rs",
    """            sequence: membership.sequence.saturating_add(1),\n""",
    """            sequence: membership.sequence.checked_add(1).context(\"membership sequence counter exhausted\")?,\n""",
)
replace(
    "crates/swarm-cli/src/daemon.rs",
    """            if ballot.base_epoch != current.epoch_number\n                || ballot.base_fencing_token != current.fencing_token\n                || ballot.target_epoch != current.epoch_number.saturating_add(1)\n                || ballot.target_fencing_token != current.fencing_token.saturating_add(1)\n            {\n""",
    """            let expected_generation = AuthorityGeneration {\n                epoch: current.epoch_number,\n                fencing_token: current.fencing_token,\n            }\n            .checked_next()\n            .context(\"accepted authority generation is exhausted\")?;\n            if ballot.base_epoch != current.epoch_number\n                || ballot.base_fencing_token != current.fencing_token\n                || ballot.target_epoch != expected_generation.epoch\n                || ballot.target_fencing_token != expected_generation.fencing_token\n            {\n""",
)
replace(
    "crates/swarm-cli/src/daemon.rs",
    """                if record.epoch_number != current.epoch_number.saturating_add(1)\n                    || record.fencing_token != current.fencing_token.saturating_add(1)\n                    || record.previous_epoch_hash != Some(epoch_record_hash(&current)?)\n                {\n                    return Err(anyhow!(\"certified recovery epoch does not directly extend the accepted epoch\"));\n                }\n""",
    """                let expected_generation = AuthorityGeneration {\n                    epoch: current.epoch_number,\n                    fencing_token: current.fencing_token,\n                }\n                .checked_next()\n                .context(\"accepted authority generation is exhausted\")?;\n                if record.epoch_number != expected_generation.epoch\n                    || record.fencing_token != expected_generation.fencing_token\n                    || record.previous_epoch_hash != Some(epoch_record_hash(&current)?)\n                {\n                    return Err(anyhow!(\"certified recovery epoch does not directly extend the accepted epoch\"));\n                }\n""",
)
replace(
    "crates/swarm-cli/src/daemon.rs",
    """                if record.epoch_number != current.epoch_number.saturating_add(1)\n                    || record.fencing_token != current.fencing_token.saturating_add(1)\n                    || record.previous_epoch_hash != Some(epoch_record_hash(&current)?)\n                {\n                    return Err(anyhow!(\"epoch and fencing token must advance exactly once from the accepted epoch\"));\n                }\n""",
    """                let expected_generation = AuthorityGeneration {\n                    epoch: current.epoch_number,\n                    fencing_token: current.fencing_token,\n                }\n                .checked_next()\n                .context(\"accepted authority generation is exhausted\")?;\n                if record.epoch_number != expected_generation.epoch\n                    || record.fencing_token != expected_generation.fencing_token\n                    || record.previous_epoch_hash != Some(epoch_record_hash(&current)?)\n                {\n                    return Err(anyhow!(\"epoch and fencing token must advance exactly once from the accepted epoch\"));\n                }\n""",
)

# A signed Solo policy is not sufficient proof for a multi-member partition to create
# a canonical Solo generation. Single-member configurations remain safe because their
# ordinary quorum is one and never require this transition.
replace(
    "crates/swarm-cli/src/daemon.rs",
    """        if next.mode == EpochMode::Solo && !solo_mode_allowed(storage, next.world_id)? {\n            return Err(anyhow!(\"solo advancement is disabled by the signed world configuration\"));\n        }\n""",
    """        if next.mode == EpochMode::Solo {\n            if !solo_mode_allowed(storage, next.world_id)? {\n                return Err(anyhow!(\"solo advancement is disabled by the signed world configuration\"));\n            }\n            let descriptor = storage.load_world_descriptor(next.world_id)?;\n            let member_count = descriptor.members.iter().filter(|member| !member.banned).count();\n            if member_count > 1 {\n                return Err(anyhow!(\n                    \"multi-member worlds cannot enter writable solo mode without a committed clean relinquishment\"\n                ));\n            }\n        }\n""",
)
