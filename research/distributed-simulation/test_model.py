"""Deterministic, intentionally limited safety-model regression checks."""
import unittest
from model import Blocked, Oracle, Simulation

A = ("overworld", 0, 0)
B = ("overworld", 1, 0)
VOTERS = ["alice", "bob", "charlie"]


class ResearchModelTests(unittest.TestCase):
    def setUp(self):
        self.oracle = Oracle(VOTERS)
        self.a = self.oracle.grant(A, "alice", ["alice", "bob"])
        self.b = self.oracle.grant(B, "bob", ["bob", "charlie"])
        self.sim = Simulation(self.oracle)
        self.sim.spawn("sheep-1", A, "alice", self.a.epoch)

    def test_minorities_cannot_grant(self):
        with self.assertRaisesRegex(Blocked, "majority"):
            self.oracle.grant(A, "charlie", ["charlie"])
        self.oracle.verify(A, "alice", self.a.epoch)

    def test_duplicate_votes_do_not_form_quorum(self):
        with self.assertRaisesRegex(Blocked, "majority"):
            self.oracle.grant(A, "bob", ["bob", "bob"])

    def test_unknown_member_denied(self):
        with self.assertRaisesRegex(Blocked, "membership"):
            self.oracle.grant(A, "mallory", ["alice", "bob"])

    def test_stale_fence_denied(self):
        new = self.oracle.grant(A, "bob", ["bob", "charlie"])
        with self.assertRaisesRegex(Blocked, "fenced"):
            self.oracle.verify(A, "alice", self.a.epoch)
        self.oracle.verify(A, "bob", new.epoch)

    def test_partition_no_majority_freezes_grant(self):
        with self.assertRaises(Blocked):
            self.oracle.grant(A, "charlie", ["charlie"])
        self.assertEqual(self.oracle._grants[A].epoch, self.a.epoch)

    def test_transactional_transfer_exactly_one_location(self):
        self.sim.prepare("t1", "sheep-1", B, "alice", self.a.epoch, "bob", self.b.epoch)
        self.assertEqual(self.sim.entities["sheep-1"], A)
        self.sim.commit("t1", "alice", "bob")
        self.sim.commit("t1", "alice", "bob")
        self.assertEqual(self.sim.entities, {"sheep-1": B})

    def test_abort_after_destination_unavailable(self):
        self.sim.prepare("t1", "sheep-1", B, "alice", self.a.epoch, "bob", self.b.epoch)
        self.oracle.grant(B, "charlie", ["alice", "charlie"])
        with self.assertRaisesRegex(Blocked, "fenced"):
            self.sim.commit("t1", "alice", "bob")
        self.sim.abort("t1")
        self.assertEqual(self.sim.entities["sheep-1"], A)

    def test_concurrent_entity_transfer_rejected(self):
        self.sim.prepare("t1", "sheep-1", B, "alice", self.a.epoch, "bob", self.b.epoch)
        with self.assertRaisesRegex(Blocked, "already in transit"):
            self.sim.prepare("t2", "sheep-1", B, "alice", self.a.epoch, "bob", self.b.epoch)

    def test_cross_dimension_transfer_uses_same_fence(self):
        nether = ("the_nether", 0, 0)
        ng = self.oracle.grant(nether, "charlie", ["alice", "charlie"])
        self.sim.prepare("portal", "sheep-1", nether, "alice", self.a.epoch, "charlie", ng.epoch)
        self.sim.commit("portal", "alice", "charlie")
        self.assertEqual(self.sim.entities["sheep-1"], nether)

    def test_uncommitted_transfer_never_visible_at_destination(self):
        self.sim.prepare("t1", "sheep-1", B, "alice", self.a.epoch, "bob", self.b.epoch)
        self.assertNotIn("t1", self.sim.completed)
        self.assertEqual(self.sim.entities["sheep-1"], A)


if __name__ == "__main__":
    unittest.main()
