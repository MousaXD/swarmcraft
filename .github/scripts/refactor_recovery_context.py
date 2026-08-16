from pathlib import Path

path = Path('crates/swarm-cli/src/daemon.rs')
text = path.read_text()

old_call = '''        drive_recovery_ballot(
            storage,
            identity,
            node,
            outbound,
            runtime,
            &descriptor,
            &epoch,
            &latest,
            &visible_peers,
            recovery_generation,
        )?;'''
new_call = '''        drive_recovery_ballot(
            RecoveryAttempt {
                storage,
                identity,
                descriptor: &descriptor,
                previous: &epoch,
                latest: &latest,
                visible_peers: &visible_peers,
                recovery_generation,
            },
            node,
            outbound,
            runtime,
        )?;'''
if old_call not in text:
    raise SystemExit('missing recovery driver call')
text = text.replace(old_call, new_call, 1)

old_signature = '''fn drive_recovery_ballot(
    storage: &Storage,
    identity: &PeerIdentity,
    node: &mut SwarmNode,
    outbound: &mut HashMap<String, OutboundContext>,
    runtime: &mut LeaseRuntime,
    descriptor: &WorldDescriptorV1,
    previous: &EpochRecordV1,
    latest: &SnapshotManifestV1,
    visible_peers: &[PeerId],
    recovery_generation: AuthorityGeneration,
) -> Result<()> {
    let world = descriptor.world_id;'''
new_signature = '''struct RecoveryAttempt<'a> {
    storage: &'a Storage,
    identity: &'a PeerIdentity,
    descriptor: &'a WorldDescriptorV1,
    previous: &'a EpochRecordV1,
    latest: &'a SnapshotManifestV1,
    visible_peers: &'a [PeerId],
    recovery_generation: AuthorityGeneration,
}

fn drive_recovery_ballot(
    attempt: RecoveryAttempt<'_>,
    node: &mut SwarmNode,
    outbound: &mut HashMap<String, OutboundContext>,
    runtime: &mut LeaseRuntime,
) -> Result<()> {
    let RecoveryAttempt {
        storage,
        identity,
        descriptor,
        previous,
        latest,
        visible_peers,
        recovery_generation,
    } = attempt;
    let world = descriptor.world_id;'''
if old_signature not in text:
    raise SystemExit('missing recovery driver signature')
path.write_text(text.replace(old_signature, new_signature, 1))

Path('.github/clippy-failure.txt').unlink(missing_ok=True)
Path(__file__).unlink()
