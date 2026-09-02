from pathlib import Path


def replace(path: str, old: str, new: str, count: int = 1) -> None:
    p = Path(path)
    text = p.read_text()
    found = text.count(old)
    if found != count:
        raise SystemExit(f"{path}: expected {count}, found {found}: {old[:140]!r}")
    p.write_text(text.replace(old, new, count))


# FINAL-039: world-config history must have an explicit checked successor rather
# than relying on saturating arithmetic at u64::MAX.
state = "crates/swarm-storage/src/state.rs"
replace(
    state,
    '''            if config.sequence != existing.sequence.saturating_add(1)
                || config.previous_config_hash != Some(existing.config_hash()?)
            {
''',
    '''            let expected_sequence = next_world_config_sequence(existing.sequence)?;
            if config.sequence != expected_sequence || config.previous_config_hash != Some(existing.config_hash()?) {
''',
)
replace(
    state,
    '''fn same_recovery_base(a: &RecoveryBallotV1, b: &RecoveryBallotV1) -> bool {
''',
    '''fn next_world_config_sequence(sequence: u64) -> Result<u64, StorageError> {
    sequence.checked_add(1).ok_or(StorageError::WorldMetadataMismatch)
}

fn same_recovery_base(a: &RecoveryBallotV1, b: &RecoveryBallotV1) -> bool {
''',
)

p = Path(state)
text = p.read_text()
marker = '''    #[test]
    fn world_config_and_background_seeding_round_trip() {
'''
if marker not in text:
    raise SystemExit("world config test marker missing")
insert = '''    #[test]
    fn world_config_sequence_exhaustion_fails_closed() {
        assert_eq!(next_world_config_sequence(u64::MAX - 1).unwrap(), u64::MAX);
        assert!(next_world_config_sequence(u64::MAX).is_err());
    }

'''
if "fn world_config_sequence_exhaustion_fails_closed()" not in text:
    text = text.replace(marker, insert + marker, 1)
p.write_text(text)
