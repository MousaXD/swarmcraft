use swarm_consensus::{elect_authority_with_quorum, has_quorum, AuthorityCandidate, AuthorityGeneration, LeaseTracker};
use swarm_protocol::PeerId;

const LEASE_MS: u64 = 3_000;
const MEMBER_COUNT: usize = 3;

#[derive(Clone)]
struct Peer {
    candidate: AuthorityCandidate,
    online: bool,
    partition: u8,
    lease: LeaseTracker,
    authority: Option<AuthorityGeneration>,
}

impl Peer {
    fn active_authority(&self, now_ms: u64) -> bool {
        let Some(generation) = self.authority else {
            return false;
        };
        self.online
            && !self.lease.is_expired(now_ms)
            && self.lease.observed().is_some_and(|lease| lease.generation == generation)
    }
}

struct Cluster {
    peers: Vec<Peer>,
    now_ms: u64,
    rng: u64,
}

impl Cluster {
    fn new(seed: u64) -> Self {
        let generation = AuthorityGeneration { epoch: 1, fencing_token: 1 };
        let mut peers = (1u8..=MEMBER_COUNT as u8)
            .enumerate()
            .map(|(index, id)| Peer {
                candidate: AuthorityCandidate {
                    peer_id: PeerId([id; 32]),
                    accepted_epoch: generation.epoch,
                    canonical_sequence: 1,
                    snapshot_complete: true,
                    compatible: true,
                    authority_eligible: true,
                    banned: false,
                },
                online: true,
                partition: 0,
                lease: LeaseTracker::default(),
                authority: (index == 0).then_some(generation),
            })
            .collect::<Vec<_>>();
        for peer in &mut peers {
            peer.lease.observe(generation, LEASE_MS, 0).unwrap();
        }
        Self { peers, now_ms: 0, rng: seed }
    }

    fn random(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    fn step(&mut self) {
        self.now_ms += 250;
        let event = self.random() % 12;
        let index = self.random() as usize % self.peers.len();
        let delay_lease_tick = event == 6;

        match event {
            0 => self.peers[index].online = false,
            1 => self.peers[index].online = true,
            2 => self.peers[index].partition = 1,
            3 => self.peers[index].partition = 0,
            4 => self.peers[index].candidate.snapshot_complete = false,
            5 => self.peers[index].candidate.snapshot_complete = true,
            _ => {}
        }

        self.renew_live_authorities(delay_lease_tick);
        self.elect_expired_partitions();
        self.demote_stale_authorities();

        let active = self.peers.iter().filter(|peer| peer.active_authority(self.now_ms)).count();
        assert!(active <= 1, "split brain: {active} peers simultaneously hold unexpired authority leases");
    }

    fn renew_live_authorities(&mut self, delay_tick: bool) {
        let authorities = self
            .peers
            .iter()
            .enumerate()
            .filter_map(|(index, peer)| peer.authority.map(|generation| (index, generation)))
            .collect::<Vec<_>>();

        for (authority_index, generation) in authorities {
            if !self.peers[authority_index].online {
                continue;
            }
            let partition = self.peers[authority_index].partition;
            let visible = self
                .peers
                .iter()
                .enumerate()
                .filter(|(_, peer)| peer.online && peer.partition == partition)
                .map(|(index, _)| index)
                .collect::<Vec<_>>();

            if !delay_tick && has_quorum(MEMBER_COUNT, visible.len()) {
                for index in visible {
                    self.peers[index].lease.observe(generation, LEASE_MS, self.now_ms).unwrap();
                }
            }
        }
    }

    fn elect_expired_partitions(&mut self) {
        for partition in [0, 1] {
            if self.peers.iter().any(|peer| peer.partition == partition && peer.active_authority(self.now_ms)) {
                continue;
            }

            let visible = self
                .peers
                .iter()
                .enumerate()
                .filter(|(_, peer)| peer.online && peer.partition == partition && peer.lease.is_expired(self.now_ms))
                .map(|(index, peer)| (index, peer.candidate.clone()))
                .collect::<Vec<_>>();
            let candidates = visible.iter().map(|(_, candidate)| candidate.clone()).collect::<Vec<_>>();
            let Ok(winner) = elect_authority_with_quorum(MEMBER_COUNT, &candidates) else {
                continue;
            };
            let winner_index = visible
                .iter()
                .find(|(_, candidate)| candidate.peer_id == winner)
                .map(|(index, _)| *index)
                .expect("elected peer must be visible");

            let next_epoch = visible
                .iter()
                .filter_map(|(index, _)| self.peers[*index].lease.observed())
                .map(|lease| lease.generation.epoch)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            let next_fencing = visible
                .iter()
                .filter_map(|(index, _)| self.peers[*index].lease.observed())
                .map(|lease| lease.generation.fencing_token)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            let generation = AuthorityGeneration { epoch: next_epoch, fencing_token: next_fencing };

            for (index, _) in &visible {
                self.peers[*index].candidate.accepted_epoch = generation.epoch;
                self.peers[*index].lease.observe(generation, LEASE_MS, self.now_ms).unwrap();
            }
            self.peers[winner_index].authority = Some(generation);
        }
    }

    fn demote_stale_authorities(&mut self) {
        for peer in &mut self.peers {
            let Some(authority_generation) = peer.authority else {
                continue;
            };
            if peer.lease.observed().is_some_and(|lease| lease.generation > authority_generation)
                || peer.lease.is_expired(self.now_ms)
            {
                peer.authority = None;
            }
        }
    }
}

#[test]
fn randomized_crash_partition_restart_and_delay_never_split_brains() {
    for seed in 1..=64 {
        let mut cluster = Cluster::new(0x7ac5_1d33_9e37_79b9 ^ seed);
        for _ in 0..2_000 {
            cluster.step();
        }
    }
}
