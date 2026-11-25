//! Executable specification of the proposed cutover transaction. NOT a
//! production storage implementation or evidence of filesystem crash safety.
//! Model transitions are atomic; real persistence, fencing, and live V2
//! admission remain separate implementation and field acceptance gates.
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Cutover {
    source_head: u64,
    target_realm: u64,
    proposal: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
struct DurableModel {
    // Single irrevocable promise per voter and V1 source world. This
    // simplified model has one source world and a fixed reviewed roster.
    promises: BTreeMap<u8, Cutover>,
    staged: BTreeSet<Cutover>,
    accepted: Option<Cutover>,
}

impl DurableModel {
    fn stage(&mut self, migration: Cutover, actual_head: u64) -> bool {
        if self.accepted.is_some_and(|old| old != migration) || migration.source_head != actual_head {
            return false;
        }
        self.staged.insert(migration);
        true
    }

    fn freeze_vote(&mut self, voter: u8, migration: Cutover, actual_head: u64) -> bool {
        if self.accepted.is_some_and(|old| old != migration)
            || migration.source_head != actual_head
            || !self.staged.contains(&migration)
        {
            return false;
        }
        // A restarted voter must never switch to a different proposal,
        // even if another peer advertises a newer source checkpoint.
        match self.promises.get(&voter) {
            Some(old) => *old == migration,
            None => {
                self.promises.insert(voter, migration);
                true
            }
        }
    }

    fn cancel_unfrozen(&mut self, migration: Cutover) -> bool {
        if self.promises.values().any(|value| *value == migration) || self.accepted == Some(migration) {
            return false;
        }
        self.staged.remove(&migration)
    }

    fn commit(&mut self, migration: Cutover, actual_head: u64, old: &[u8], new: &[u8]) -> bool {
        if let Some(accepted) = self.accepted {
            return accepted == migration; // exact idempotent resume only
        }
        if actual_head != migration.source_head || !self.staged.contains(&migration) {
            return false;
        }
        let majority = |roster: &[u8]| {
            let distinct: BTreeSet<_> = roster.iter().copied().collect();
            !distinct.is_empty()
                && distinct.iter().filter(|voter| self.promises.get(voter) == Some(&migration)).count()
                    > distinct.len() / 2
        };
        if !majority(old) || !majority(new) {
            return false;
        }
        self.accepted = Some(migration);
        true
    }

    fn permits_v1_write_vote(&self, voter: u8) -> bool {
        !self.promises.contains_key(&voter)
    }
}

#[test]
fn restart_at_each_planned_boundary_never_publishes_unaccepted_target() {
    let migration = Cutover { source_head: 10, target_realm: 20, proposal: 30 };
    let mut model = DurableModel::default();
    let voters = [1, 2, 3];
    assert!(model.stage(migration, 10));
    // Clone only the modeled durable state to represent reopening after
    // process loss. Nothing volatile can establish an accepted head.
    let mut reopened = model.clone();
    assert_eq!(reopened.accepted, None);
    assert!(reopened.cancel_unfrozen(migration));
    assert!(reopened.stage(migration, 10));
    assert!(reopened.freeze_vote(1, migration, 10));
    let mut reopened = reopened.clone();
    assert!(!reopened.cancel_unfrozen(migration));
    assert!(!reopened.permits_v1_write_vote(1));
    assert!(!reopened.commit(migration, 10, &voters, &voters));
    assert!(reopened.freeze_vote(2, migration, 10));
    assert!(!reopened.commit(migration, 11, &voters, &voters));
    assert_eq!(reopened.accepted, None);
    let mut reopened = reopened.clone();
    assert!(reopened.commit(migration, 10, &voters, &voters));
    let mut reopened = reopened.clone();
    assert!(reopened.commit(migration, 10, &voters, &voters));
    assert_eq!(reopened.accepted, Some(migration));
    assert!(!reopened.cancel_unfrozen(migration));
    assert_eq!(voters.iter().filter(|voter| reopened.permits_v1_write_vote(**voter)).count(), 1);
}

#[test]
fn joint_majorities_intersect_and_reject_competing_cutover() {
    let first = Cutover { source_head: 10, target_realm: 20, proposal: 30 };
    let second = Cutover { source_head: 10, target_realm: 21, proposal: 31 };
    let mut model = DurableModel::default();
    let old = [1, 2, 3];
    let new = [2, 3, 4];
    assert!(model.stage(first, 10));
    assert!(model.stage(second, 10));
    assert!(model.freeze_vote(1, first, 10));
    assert!(model.freeze_vote(2, first, 10));
    assert!(!model.commit(first, 10, &old, &new)); // old majority only
    assert!(model.freeze_vote(3, second, 10));
    assert!(model.freeze_vote(4, second, 10));
    assert!(!model.commit(second, 10, &old, &new)); // new majority only
    assert!(!model.freeze_vote(2, second, 10));
    assert!(!model.freeze_vote(3, first, 10));
    // Conflicting freezes can strand a migration; the reviewed plan
    // deliberately prefers loss of availability to unsafe rollback.
    assert_eq!(model.accepted, None);
    assert!(!model.cancel_unfrozen(first));
    assert!(!model.cancel_unfrozen(second));
}

#[test]
fn accepted_cutover_cannot_be_retargeted_or_rolled_back() {
    let first = Cutover { source_head: 10, target_realm: 20, proposal: 30 };
    let second = Cutover { source_head: 10, target_realm: 21, proposal: 31 };
    let mut model = DurableModel::default();
    let old = [1, 2, 3];
    let new = [2, 3, 4];
    assert!(model.stage(first, 10));
    assert!(model.freeze_vote(2, first, 10));
    assert!(model.freeze_vote(3, first, 10));
    assert!(model.commit(first, 10, &old, &new));
    assert!(!model.stage(second, 10));
    assert!(!model.freeze_vote(1, second, 10));
    assert!(!model.commit(second, 10, &old, &new));
    assert!(!model.cancel_unfrozen(first));
    assert_eq!(model.accepted, Some(first));
}
