//! Test-only reference model for RFC two-device-exclusive-turn.
//!
//! These are ideal authenticated records, not production wire formats. A
//! local state mutation represents one completed durable journal write.
//! Each device has its own journal: there is no shared authority oracle.
//! Crashes preserve the journal, while the installed save may be lost.

use std::collections::{HashSet, VecDeque};

type Peer = u8;
type Checkpoint = u8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Grant {
    world: u8,
    epoch: u64,
    fence: u64,
    holder: Peer,
    base: Checkpoint,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Proposal {
    parent: Grant,
    next: Grant,
    attempt: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Receipt {
    proposal: Proposal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Release {
    receipt: Receipt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Phase {
    Replica,
    Held,
    Prepared(Proposal),
    Released(Release),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Journal {
    peer: Peer,
    grant: Grant,
    head: Checkpoint,
    bytes: Option<Checkpoint>,
    phase: Phase,
    attempt: u64,
    receipt: Option<Receipt>,
    accepted_release: Option<Release>,
}

impl Proposal {
    fn extends(self, parent: Grant) -> bool {
        self.parent == parent
            && parent.holder < 2
            && self.next.holder == 1 - parent.holder
            && self.next.world == parent.world
            && parent.epoch.checked_add(1) == Some(self.next.epoch)
            && parent.fence.checked_add(1) == Some(self.next.fence)
            && self.attempt != 0
    }
}

impl Journal {
    fn new(peer: Peer, grant: Grant) -> Self {
        Self {
            peer,
            grant,
            head: grant.base,
            bytes: Some(grant.base),
            phase: if grant.holder == peer { Phase::Held } else { Phase::Replica },
            attempt: 0,
            receipt: None,
            accepted_release: None,
        }
    }

    fn can_write(&self) -> bool {
        self.phase == Phase::Held && self.grant.holder == self.peer && self.bytes == Some(self.head)
    }

    fn checkpoint(&mut self, head: Checkpoint) -> bool {
        if !self.can_write() {
            return false;
        }
        self.head = head;
        self.bytes = Some(head);
        true
    }

    fn prepare(&mut self, quiesced: bool) -> Option<Proposal> {
        if !quiesced || !self.can_write() {
            return None;
        }
        let attempt = self.attempt.checked_add(1)?;
        let proposal = Proposal {
            parent: self.grant,
            next: Grant {
                world: self.grant.world,
                epoch: self.grant.epoch.checked_add(1)?,
                fence: self.grant.fence.checked_add(1)?,
                holder: 1 - self.peer,
                base: self.head,
            },
            attempt,
        };
        self.attempt = attempt;
        self.phase = Phase::Prepared(proposal);
        Some(proposal)
    }

    fn cancel(&mut self) -> bool {
        if !matches!(self.phase, Phase::Prepared(_)) || self.grant.holder != self.peer {
            return false;
        }
        // A canceled attempt is never reused. If the save was restored
        // incorrectly, even this held turn cannot admit a writer.
        self.phase = Phase::Held;
        true
    }

    fn stage(&mut self, proposal: Proposal) -> Option<Receipt> {
        if !matches!(self.phase, Phase::Replica | Phase::Released(_))
            || !proposal.extends(self.grant)
            || proposal.next.holder != self.peer
            || self.bytes != Some(proposal.next.base)
        {
            return None;
        }
        if let Some(previous) = self.receipt {
            if previous.proposal == proposal {
                return Some(previous);
            }
            if proposal.attempt <= previous.proposal.attempt {
                return None;
            }
        }
        let receipt = Receipt { proposal };
        self.receipt = Some(receipt);
        Some(receipt)
    }

    fn commit(&mut self, receipt: Receipt) -> Option<Release> {
        let proposal = receipt.proposal;
        if let Phase::Released(release) = self.phase {
            // The durable outbox survives restart and resends only itself.
            return (release.receipt == receipt).then_some(release);
        }
        if self.phase != Phase::Prepared(proposal)
            || !proposal.extends(self.grant)
            || self.grant.holder != self.peer
            || self.head != proposal.next.base
            || self.bytes != Some(self.head)
        {
            return None;
        }
        let release = Release { receipt };
        // The source has permanently relinquished before this function
        // returns the message. Sending a release first is not a legal event.
        self.grant = proposal.next;
        self.phase = Phase::Released(release);
        self.receipt = None;
        Some(release)
    }

    fn accept(&mut self, release: Release, consented: bool) -> bool {
        let proposal = release.receipt.proposal;
        if self.accepted_release == Some(release) && self.grant == proposal.next {
            // Idempotence must never recreate a Held phase after prepare,
            // nor substitute for the exact checkpoint/consent check.
            return self.can_write() && consented;
        }
        if !consented
            || !matches!(self.phase, Phase::Replica | Phase::Released(_))
            || !proposal.extends(self.grant)
            || proposal.next.holder != self.peer
            || self.receipt != Some(release.receipt)
            || self.bytes != Some(proposal.next.base)
        {
            return false;
        }
        self.grant = proposal.next;
        self.head = proposal.next.base;
        self.phase = Phase::Held;
        self.receipt = None;
        self.accepted_release = Some(release);
        true
    }
}

fn pair() -> [Journal; 2] {
    let grant = Grant { world: 7, epoch: 1, fence: 1, holder: 0, base: 1 };
    [Journal::new(0, grant), Journal::new(1, grant)]
}

fn safe(journals: &[Journal; 2]) {
    assert!(
        !(journals[0].can_write() && journals[1].can_write()),
        "two writers after a partition or replay: {journals:?}"
    );
}

#[test]
fn handoff_requires_quiescence_exact_receipt_and_irreversible_release() {
    let [mut alice, mut bob] = pair();
    assert!(alice.checkpoint(2));
    assert!(alice.prepare(false).is_none());
    let first = alice.prepare(true).unwrap();
    assert!(!alice.can_write());
    assert!(bob.stage(first).is_none()); // old save
    bob.bytes = Some(2);
    let obsolete = bob.stage(first).unwrap();
    assert!(!bob.accept(Release { receipt: obsolete }, false));
    assert!(alice.cancel());
    let second = alice.prepare(true).unwrap();
    assert!(alice.commit(obsolete).is_none());
    let receipt = bob.stage(second).unwrap();
    assert!(bob.stage(first).is_none());
    assert!(!bob.can_write());
    let release = alice.commit(receipt).unwrap();
    // Restart after commit but before send. The exact outbox is durable.
    alice = alice.clone();
    bob = bob.clone();
    assert!(!alice.cancel());
    assert!(!alice.can_write());
    assert_eq!(alice.commit(receipt), Some(release));
    bob.bytes = Some(1);
    assert!(!bob.accept(release, true));
    bob.bytes = Some(2);
    assert!(!bob.accept(release, false));
    assert!(bob.accept(release, true));
    assert!(bob.accept(release, true));
    assert!(!alice.can_write());
    assert!(bob.can_write());
    assert!(!alice.checkpoint(3));
    bob.bytes = Some(1);
    assert!(!bob.can_write()); // restoring world data is not restoring a turn
}

#[test]
fn reverse_handoff_cannot_reactivate_an_old_committed_grant() {
    let [mut alice, mut bob] = pair();
    let proposal = alice.prepare(true).unwrap();
    let receipt = bob.stage(proposal).unwrap();
    let first = alice.commit(receipt).unwrap();
    assert!(bob.accept(first, true));
    assert!(bob.checkpoint(2));
    let reverse = bob.prepare(true).unwrap();
    assert!(!bob.accept(first, true)); // duplicate must not undo prepare
    alice.bytes = Some(2);
    let back_receipt = alice.stage(reverse).unwrap();
    let back = bob.commit(back_receipt).unwrap();
    assert!(!bob.accept(first, true));
    assert!(!bob.can_write());
    assert!(alice.accept(back, true));
    assert!(!bob.accept(first, true));
    assert!(alice.can_write());
    assert_eq!(alice.grant.epoch, 3);
    assert_eq!(alice.grant.fence, 3);
}

#[test]
fn lost_recipient_cannot_be_replaced_by_a_timeout_or_a_stale_save() {
    let [mut alice, mut bob] = pair();
    let plan = alice.prepare(true).unwrap();
    let receipt = bob.stage(plan).unwrap();
    let _undelivered = alice.commit(receipt).unwrap();
    // Neither peer can manufacture a third grant after the recipient's
    // device disappears. A restart/timeout is not a state transition.
    for _ in 0..100 {
        alice = alice.clone();
        assert!(!alice.can_write());
        assert!(alice.prepare(true).is_none());
        assert!(!alice.cancel());
    }
    assert!(!bob.can_write());
}

#[test]
fn invalid_generation_world_and_counter_exhaustion_are_denials() {
    let [mut alice, mut bob] = pair();
    let valid = alice.prepare(true).unwrap();
    for invalid in [
        Proposal { next: Grant { world: 8, ..valid.next }, ..valid },
        Proposal { next: Grant { holder: 0, ..valid.next }, ..valid },
        Proposal { next: Grant { fence: valid.parent.fence, ..valid.next }, ..valid },
        Proposal { next: Grant { epoch: valid.next.epoch + 1, ..valid.next }, ..valid },
        Proposal { attempt: 0, ..valid },
    ] {
        assert!(bob.stage(invalid).is_none());
        assert!(alice.commit(Receipt { proposal: invalid }).is_none());
    }
    assert!(alice.cancel());
    alice.grant.epoch = u64::MAX;
    assert!(alice.prepare(true).is_none());
    alice.grant.epoch = 1;
    alice.grant.fence = u64::MAX;
    assert!(alice.prepare(true).is_none());
    alice.grant.fence = 1;
    alice.attempt = u64::MAX;
    assert!(alice.prepare(true).is_none());
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Network {
    journals: [Journal; 2],
    online: [bool; 2],
    connected: bool,
    proposals: Vec<Proposal>,
    receipts: Vec<Receipt>,
    releases: Vec<Release>,
}

impl Network {
    fn successors(&self) -> Vec<Self> {
        let mut result = Vec::new();
        let mut partition = self.clone();
        partition.connected = !partition.connected;
        result.push(partition);
        for peer in 0..2 {
            let mut restarted = self.clone();
            restarted.online[peer] = !restarted.online[peer];
            result.push(restarted);
            if !self.online[peer] {
                continue;
            }
            // A local save rollback/reload does not change the durable journal.
            for bytes in [None, Some(1), Some(2)] {
                let mut next = self.clone();
                next.journals[peer].bytes = bytes;
                result.push(next);
            }
            let mut canceled = self.clone();
            if canceled.journals[peer].cancel() {
                result.push(canceled);
            }
            if !self.connected || !self.online[1 - peer] {
                continue;
            }
            // Bound the model to two attempts per peer, two generations of
            // handoff, and arbitrary delivery of every previously sent record.
            let mut next = self.clone();
            if next.journals[peer].attempt < 2 && next.journals[peer].grant.epoch < 3 {
                if let Some(proposal) = next.journals[peer].prepare(true) {
                    if !next.proposals.contains(&proposal) {
                        next.proposals.push(proposal);
                    }
                    result.push(next);
                }
            }
            for proposal in &self.proposals {
                let mut next = self.clone();
                if let Some(receipt) = next.journals[peer].stage(*proposal) {
                    if !next.receipts.contains(&receipt) {
                        next.receipts.push(receipt);
                    }
                    result.push(next);
                }
            }
            for receipt in &self.receipts {
                let mut next = self.clone();
                if let Some(release) = next.journals[peer].commit(*receipt) {
                    if !next.releases.contains(&release) {
                        next.releases.push(release);
                    }
                    result.push(next);
                }
            }
            for release in &self.releases {
                let mut next = self.clone();
                if next.journals[peer].accept(*release, true) {
                    result.push(next);
                }
            }
        }
        result
    }
}

#[test]
fn bounded_partition_crash_and_message_replay_search_has_at_most_one_writer() {
    let mut journals = pair();
    assert!(journals[0].checkpoint(2));
    let initial = Network {
        journals,
        online: [true, true],
        connected: true,
        proposals: Vec::new(),
        receipts: Vec::new(),
        releases: Vec::new(),
    };
    let mut visited = HashSet::from([initial.clone()]);
    let mut queue = VecDeque::from([(initial, 0)]);
    let mut reached_reverse = false;
    let mut reached_partition_after_commit = false;
    while let Some((state, depth)) = queue.pop_front() {
        // Check the independent durable journals even if a peer is offline.
        // Restarting both in a partition cannot create a second permission.
        safe(&state.journals);
        reached_reverse |= state.journals[0].grant.epoch == 3 && state.journals[0].can_write();
        reached_partition_after_commit |=
            !state.connected && state.journals.iter().any(|journal| matches!(journal.phase, Phase::Released(_)));
        if depth == 13 {
            continue;
        }
        for next in state.successors() {
            if visited.insert(next.clone()) {
                queue.push_back((next, depth + 1));
            }
        }
    }
    assert!(reached_reverse, "the search must include a completed reverse handoff");
    assert!(reached_partition_after_commit, "the search must include a committed partition");
    assert!(visited.len() > 100, "the search must explore nontrivial interleavings");
    eprintln!("checked {} unique two-journal states", visited.len());
}
