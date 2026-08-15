from pathlib import Path

path = Path('crates/swarm-storage/src/state.rs')
text = path.read_text()
text = text.replace(
'''    fn ballot(world: WorldId, candidate: u8, round: u64) -> RecoveryBallotV1 {
        RecoveryBallotV1 {
''',
'''    fn ballot(world: WorldId, candidate: u8, round: u64) -> RecoveryBallotV1 {
        let candidate_public_key = [candidate; 32];
        RecoveryBallotV1 {
''',
1,
)
text = text.replace(
'''            candidate_peer_id: PeerId([candidate; 32]),
            candidate_public_key: [candidate; 32],
''',
'''            candidate_peer_id: peer_id_from_public_key(&candidate_public_key),
            candidate_public_key,
''',
1,
)
text = text.replace(
'''    fn vote(ballot: &RecoveryBallotV1, voter: u8) -> RecoveryVoteV1 {
        RecoveryVoteV1 {
''',
'''    fn vote(ballot: &RecoveryBallotV1, voter: u8) -> RecoveryVoteV1 {
        let voter_public_key = [voter; 32];
        RecoveryVoteV1 {
''',
1,
)
text = text.replace(
'''            voter_peer_id: PeerId([voter; 32]),
            voter_public_key: [voter; 32],
''',
'''            voter_peer_id: peer_id_from_public_key(&voter_public_key),
            voter_public_key,
''',
1,
)
path.write_text(text)
