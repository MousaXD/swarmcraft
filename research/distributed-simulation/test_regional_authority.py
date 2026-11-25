import copy
import json
from pathlib import Path
import tempfile
import unittest

from player_runtime import fixture
from region_runtime import Coordinator, Rejected, region_roots, serial_step, world_root
from regional_authority import (Authority, FencedRuntime, FencedWorker, bind_channels, candidate_receipts,
                                tag, wire)
from test_region_runtime import MemoryChannel
from verify_authority_evidence import verify

GROUPS = [list(range(4)), list(range(4, 8))]
SWAPPED = [list(range(4, 8)), list(range(4))]


def make_coordinator(current):
    channels = [MemoryChannel() for _ in current["grants"]]
    for channel in channels:
        channel.worker = FencedWorker()
    bind_channels(channels, current["grants"])
    placement = {r: slot for slot, g in enumerate(current["grants"]) for r in g["regions"]}
    return Coordinator(channels, current["state"], placement=placement, start_tick=current["tick"])


class AuthorityTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.path = Path(self.tmp.name) / "ledger.db"
        self.ledger = Authority.create(self.path, "a" * 64, fixture(), [0, 1, 2], GROUPS)
        self.addCleanup(self.ledger.close)

    def candidate(self):
        s = self.ledger.read()
        control = make_coordinator(self.ledger.claim(s["generation"], s["tick"], s["runtime_root"]))
        control.step()
        return control, candidate_receipts(control)

    def rotate(self):
        s = self.ledger.save()
        return self.ledger.rotate(s["generation"], s["tick"], s["runtime_root"], SWAPPED)

    def assert_refused(self, receipts):
        before = self.ledger.save()
        history = self.ledger.history()
        with self.assertRaises(Rejected):
            self.ledger.commit(receipts)
        self.assertEqual(self.ledger.save(), before)
        self.assertEqual(self.ledger.history(), history)

    def test_commit_reopen_and_global_consistent_cut(self):
        _, receipts = self.candidate()
        accepted = self.ledger.commit(receipts)
        expected = serial_step(fixture(), 1)
        self.assertEqual(accepted["runtime_root"], world_root(1, region_roots(expected)))
        reopened = Authority(self.path)
        try:
            self.assertEqual(reopened.save(), self.ledger.save())
            self.assertEqual(reopened.read()["state"], expected)
            self.assertEqual(reopened.save()["accepted_generation"], 1)
            self.assertNotIn("key", reopened.save()["grants"][0])
        finally:
            reopened.close()
        self.assert_refused(receipts)

    def test_reassign_while_old_candidate_exists(self):
        old, receipts = self.candidate()
        prior = self.ledger.save()
        self.rotate()
        self.assertEqual(self.ledger.save()["runtime_root"], prior["runtime_root"])
        self.assert_refused(receipts)
        old.step()  # old process is still able to compute tentatively
        self.assert_refused(candidate_receipts(old))
        _, current = self.candidate()
        self.ledger.commit(current)
        self.assertEqual(self.ledger.save()["accepted_generation"], 2)
        self.assertEqual(len(self.ledger.history()["commits"]), 2)

    def test_replayed_generation_and_overlap_refused(self):
        s = self.ledger.save()
        for groups in ([[0, 1, 2, 3], [3, 4, 5, 6, 7]], [[0], [2, 3, 4, 5, 6, 7]],
                       [[True, 0, 2, 3], [4, 5, 6, 7]]):
            with self.assertRaises(Rejected):
                self.ledger.rotate(s["generation"], s["tick"], s["runtime_root"], groups)
            self.assertEqual(self.ledger.save(), s)
        self.rotate()
        with self.assertRaises(Rejected):
            self.ledger.rotate(s["generation"], s["tick"], s["runtime_root"], GROUPS)

    def test_receipt_scope_and_authentication(self):
        _, receipts = self.candidate()
        grants = self.ledger.read()["grants"]
        self.assert_refused(receipts[:1])
        self.assert_refused([receipts[0], receipts[0]])
        for field, value in (("world", "b" * 64), ("generation", 2), ("tick", 2),
                             ("parent_root", "0" * 64), ("runtime_root", "0" * 64)):
            bad = copy.deepcopy(receipts)
            bad[0]["body"][field] = value
            bad[0]["mac"] = tag(grants[0]["key"], bad[0]["body"])
            self.assert_refused(bad)
        bad = copy.deepcopy(receipts)
        bad[0]["mac"] = "0" * 64
        self.assert_refused(bad)
        bad = copy.deepcopy(receipts)
        bad[0]["body"]["regions"] = bad[1]["body"]["regions"]
        bad[0]["mac"] = tag(grants[0]["key"], bad[0]["body"])
        self.assert_refused(bad)

    def test_wrong_candidate_full_state_even_with_valid_mac(self):
        _, receipts = self.candidate()
        grants = self.ledger.read()["grants"]
        bad = copy.deepcopy(receipts)
        r = bad[0]["body"]["regions"][0]
        r[2] = list(r[2])
        r[2][0] += 1
        bad[0]["mac"] = tag(grants[0]["key"], bad[0]["body"])
        self.assert_refused(bad)

    def test_transaction_rollback_before_commit(self):
        _, receipts = self.candidate()
        before = self.ledger.save()
        def fail():
            raise RuntimeError("interrupted transaction")
        with self.assertRaises(RuntimeError):
            self.ledger.commit(receipts, before_commit=fail)
        self.assertEqual(before, self.ledger.save())
        self.ledger.commit(receipts)
        self.assertEqual(self.ledger.save()["tick"], 1)

    def test_worker_cannot_rebind_or_sign_before_commit(self):
        grants = self.ledger.read()["grants"]
        worker = FencedWorker()
        with self.assertRaises(Rejected):
            worker.handle({"op": "fenced_snapshot"})
        worker.handle({"op": "bind_grant", "grant": grants[0]})
        with self.assertRaises(Rejected):
            worker.handle({"op": "bind_grant", "grant": grants[1]})
        with self.assertRaises(Rejected):
            worker.handle({"op": "fenced_snapshot"})
        with self.assertRaises(Rejected):
            worker.handle({"op": "init", "version": 1, "regions": wire(fixture())})

    def test_missing_authority_not_recreated(self):
        missing = Path(self.tmp.name) / "missing.db"
        with self.assertRaises(Exception):
            Authority(missing)
        self.assertFalse(missing.exists())

    def test_reader_never_sees_uncommitted_world(self):
        _, receipts = self.candidate()
        reader = Authority(self.path)
        try:
            before = reader.save()
            def observe():
                self.assertEqual(reader.save(), before)
            self.ledger.commit(receipts, before_commit=observe)
            self.assertEqual(reader.save(), self.ledger.save())
            self.assertEqual(reader.save()["tick"], 1)
        finally:
            reader.close()

    def test_unclaimed_generation_refuses_publication(self):
        control = make_coordinator(self.ledger.read())
        control.step()
        self.assert_refused(candidate_receipts(control))

    def test_snapshot_mismatch_freezes_adapter_before_publication(self):
        channels = [MemoryChannel(), MemoryChannel()]
        for channel in channels:
            channel.worker = FencedWorker()
        runtime = FencedRuntime(self.ledger, channels)
        before = self.ledger.save()
        def mutate(reply):
            if "body" in reply:
                reply["body"]["runtime_root"] = "0" * 64
            return reply
        channels[0].mutate = mutate
        with self.assertRaisesRegex(Rejected, "staged candidate"):
            runtime.advance()
        self.assertTrue(runtime.frozen and runtime.coordinator.poisoned)
        self.assertEqual(self.ledger.save(), before)
        with self.assertRaises(Rejected):
            runtime.advance()

    def test_one_time_claim_even_before_first_tick(self):
        s = self.ledger.save()
        self.ledger.claim(s["generation"], s["tick"], s["runtime_root"])
        claimed = self.ledger.save()
        other = Authority(self.path)
        try:
            with self.assertRaisesRegex(Rejected, "already claimed"):
                other.claim(s["generation"], s["tick"], s["runtime_root"])
            with self.assertRaisesRegex(Rejected, "already claimed"):
                FencedRuntime(other, [])
            self.assertEqual(other.save(), claimed)
        finally:
            other.close()
        self.rotate()
        self.assertFalse(self.ledger.save()["claimed"])
        self.ledger.claim(2, 0, s["runtime_root"])

    def test_restart_preserves_generation_and_fences_same_placement(self):
        _, receipts = self.candidate()
        before = self.ledger.save()
        reopened = Authority(self.path)
        try:
            reopened.rotate(before["generation"], before["tick"], before["runtime_root"], GROUPS)
        finally:
            reopened.close()
        self.assert_refused(receipts)
        self.assertEqual(self.ledger.save()["generation"], 2)


class AuthorityEvidenceTests(unittest.TestCase):
    def test_retained_evidence_and_mutations(self):
        evidence = Path(__file__).parent / "evidence"
        for suffix in (1, 2):
            data = json.loads((evidence / f"2026-11-26-authority-{suffix}.json").read_text())
            verify(data)
            for mutation in (
                lambda d: d["saves"][2]["save"].update(runtime_root="0" * 64),
                lambda d: d["saves"][5]["save"].update(claimed=True),
                lambda d: d["saves"][7]["save"]["regions"][0][2].__setitem__(0, -1),
                lambda d: d["saves"][5]["save"]["grants"][0].update(regions=[0, 1, 2, 3]),
                lambda d: d["history"]["commits"].append(d["history"]["commits"][-1]),
                lambda d: d["history"]["epochs"][1].__setitem__(4, False),
                lambda d: d["refusals"][2].update(generation=1),
                lambda d: d["races"]["kill_after"][0].update(exitcode=0),
                lambda d: d["checkpoints"][0].update(same_candidate_root=False),
            ):
                with self.subTest(receipt=suffix):
                    altered = copy.deepcopy(data)
                    mutation(altered)
                    with self.assertRaises(Rejected):
                        verify(altered)


if __name__ == "__main__":
    unittest.main()
