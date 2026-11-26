import copy
import hashlib
import hmac
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import patch

from player_runtime import fixture
from region_runtime import Coordinator, Rejected, canonical, region_roots, serial_step, world_root
from regional_authority import (Authority, FencedRuntime, FencedWorker, bind_channels, candidate_receipts,
                                encoded_receipt, encoded_receipt_hash, tag, tag_encoded, validated_state, wire)
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

    def test_state_cache_reuses_only_exact_validated_blob(self):
        baseline = self.ledger.save()
        with patch("regional_authority.validated_state", wraps=validated_state) as validate:
            # The initial validated state is already cached. Mutable copies
            # returned to callers cannot alter cached state or metadata.
            exposed = self.ledger.read()
            exposed["state"].clear()
            exposed["ids"].clear()
            exposed["grants"].clear()
            self.assertEqual(self.ledger.save(), baseline)
            self.assertEqual(validate.call_count, 0)
            _, receipts = self.candidate()
            self.ledger.commit(receipts)
            self.assertEqual(self.ledger.read()["state"], serial_step(fixture(), 1))
            # The candidate is already parsed and globally verified.
            # A successful commit seeds the exact-BLOB cache directly.
            self.assertEqual(validate.call_count, 0)
            self.ledger.save()
            self.ledger.history()
            self.assertEqual(validate.call_count, 0)

    def test_candidate_parsed_once_and_exact_receipt_hash_preserved(self):
        from region_runtime import REGIONS, read_region
        _, receipts = self.candidate()
        expected_hash = hashlib.sha256(canonical(sorted(
            receipts, key=lambda r:r["body"]["session"]))).hexdigest()
        with patch("regional_authority.read_region", wraps=read_region) as parse:
            committed = self.ledger.commit(list(reversed(receipts)))
            self.assertEqual(committed["receipt_hash"], expected_hash)
            self.assertEqual(parse.call_count, REGIONS)
            self.assertEqual(self.ledger.read()["state"], serial_step(fixture(), 1))
            self.assertEqual(parse.call_count, REGIONS)
        # Receipt containers are caller-owned. Mutating them after commit
        # cannot alter any cached Region or persisted BLOB.
        receipts[0]["body"]["regions"].clear()
        self.assertEqual(self.ledger.read()["state"], serial_step(fixture(), 1))
        reopened = Authority(self.path)
        try:
            self.assertEqual(reopened.save(), self.ledger.save())
            self.assertEqual(reopened.history(), self.ledger.history())
        finally:
            reopened.close()

    def test_receipt_encoding_reuse_matches_original_mac_and_canonical_hash(self):
        _, receipts = self.candidate()
        grants = {g["session"]:g for g in self.ledger.read()["grants"]}
        for receipt in receipts:
            body = receipt["body"]
            raw = canonical(body)
            key = grants[body["session"]]["key"]
            old = hmac.digest(bytes.fromhex(key),b"sc60-region-receipt-v1\0"+raw,"sha256").hex()
            self.assertEqual(tag_encoded(key,raw),old)
            self.assertEqual(tag(key,body),old)
            for outer in (receipt,{**receipt,"binding":"sc65v1:test","policy_mac":"a"*64}):
                self.assertEqual(encoded_receipt(outer,body,raw),canonical(outer))
                # A policy adapter that supplies a different body object
                # must not accidentally substitute its authenticated bytes.
                self.assertEqual(encoded_receipt(outer,copy.deepcopy(body),b"not-the-body"),canonical(outer))
        pairs = [(r["body"]["session"],encoded_receipt(r,r["body"],canonical(r["body"])))
                 for r in reversed(receipts)]
        expected = hashlib.sha256(canonical(sorted(receipts,key=lambda r:r["body"]["session"]))).hexdigest()
        self.assertEqual(encoded_receipt_hash(pairs),expected)


    def test_reparse_control_and_candidate_cache_invalidation(self):
        from region_runtime import REGIONS, read_region
        _, receipts = self.candidate()
        self.ledger.reuse_validated_candidate = False
        with patch("regional_authority.read_region", wraps=read_region) as parse:
            self.ledger.commit(receipts)
            self.assertEqual(parse.call_count, REGIONS * 2)
            self.ledger.read()
            self.assertEqual(parse.call_count, REGIONS * 3)
        self.ledger.reuse_validated_candidate = True
        current = self.ledger.read()
        self.ledger.rotate(current["generation"], current["tick"], current["runtime_root"], GROUPS)
        _, receipts = self.candidate()
        self.ledger.commit(receipts)
        expected = self.ledger.read()["state"]
        with sqlite3.connect(self.path) as other:
            raw, = other.execute("SELECT state FROM world WHERE singleton=1").fetchone()
            changed = json.loads(raw)
            changed[0][1][0] = (changed[0][1][0] + 1) % 256
            other.execute("UPDATE world SET state=?", (canonical(changed),))
            other.commit()
            with self.assertRaisesRegex(Rejected, "invalid persisted cut"):
                self.ledger.read()
            other.execute("UPDATE world SET state=?", (raw,))
            other.commit()
        self.assertEqual(self.ledger.read()["state"], expected)

    def test_candidate_reuse_still_refuses_malformed_and_duplicate_global_ids(self):
        _, receipts = self.candidate()
        grants = {g["session"]:g for g in self.ledger.read()["grants"]}
        for mutation in ("bad_position", "bad_integer", "duplicate_global"):
            bad = json.loads(canonical(receipts))
            populated = [region for receipt in bad for region in receipt["body"]["regions"] if region[3]]
            self.assertGreaterEqual(len(populated), 2)
            region = populated[0]
            if mutation == "bad_position":
                region[3][0][1] = -1
            elif mutation == "bad_integer":
                region[3][0][2] = True
            else:
                other = populated[1]
                region[3][0][0] = other[3][0][0]
                region[3].sort(key=lambda a:a[0])
            for receipt in bad:
                receipt["mac"] = tag(grants[receipt["body"]["session"]]["key"], receipt["body"])
            self.assert_refused(bad)
        self.ledger.commit(receipts)
        self.assertEqual(self.ledger.read()["state"], serial_step(fixture(), 1))

    def test_boundary_observer_matches_sql_and_reports_rollback(self):
        from scale_profile import PhaseClock
        trace, boundary = PhaseClock(), PhaseClock()
        self.ledger.db.set_trace_callback(trace.trace)
        self.ledger.transaction_event = boundary.boundary
        try:
            trace.begin()
            boundary.begin()
            self.ledger.read()
            with self.assertRaises(RuntimeError):
                with self.ledger.transaction(write=True):
                    raise RuntimeError("injected failure")
            self.assertEqual(trace.transactions, boundary.transactions)
            self.assertEqual(boundary.transactions, {"write":1,"read":1,"commit":1,"rollback":1})
        finally:
            self.ledger.db.set_trace_callback(None)
            self.ledger.transaction_event = None

    def test_state_cache_rechecks_other_connection_mutations(self):
        baseline = self.ledger.save()
        with sqlite3.connect(self.path) as other:
            original, = other.execute("SELECT state FROM world WHERE singleton=1").fetchone()
            bad = json.loads(original)
            bad[0][1][0] = (bad[0][1][0] + 1) % 256
            # Same tick, metadata and committed root; the exact state
            # changed. A cache keyed only by head would miss this.
            other.execute("UPDATE world SET state=?", (canonical(bad),))
            other.commit()
            with self.assertRaisesRegex(Rejected, "invalid persisted cut"):
                self.ledger.read()
            other.execute("UPDATE world SET state=?", (original,))
            other.commit()
            self.assertEqual(self.ledger.save(), baseline)
            # A cache hit on the unchanged state must still reject altered
            # fencing/grant and commit metadata.
            other.execute("UPDATE epochs SET claimed=1")
            other.commit()
            with self.assertRaisesRegex(Rejected, "grant history mismatch"):
                self.ledger.read()
            other.execute("UPDATE epochs SET claimed=0")
            other.execute("UPDATE commits SET cut_root=?", ("0"*64,))
            other.commit()
            with self.assertRaisesRegex(Rejected, "commit history mismatch"):
                self.ledger.read()
            other.execute("UPDATE commits SET cut_root=?", (baseline["cut_root"],))
            other.commit()
        self.assertEqual(self.ledger.save(), baseline)

    def test_state_cache_cannot_publish_uncommitted_candidate(self):
        _, receipts = self.candidate()
        before = self.ledger.save()

        def read_then_fail():
            # Exercise a cache miss inside the uncommitted write
            # transaction, then roll it back. The next read MUST use
            # the original persisted bytes, not that tentative state.
            self.assertEqual(self.ledger._read()["tick"], 1)
            raise RuntimeError("interrupted after tentative read")

        with self.assertRaisesRegex(RuntimeError, "interrupted"):
            self.ledger.commit(receipts, before_commit=read_then_fail)
        self.assertEqual(self.ledger.save(), before)
        self.ledger.commit(receipts)
        self.assertEqual(self.ledger.read()["state"], serial_step(fixture(), 1))

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
            if reply.get("op") == "committed_snapshot":
                reply["snapshot"]["body"]["runtime_root"] = "0" * 64
            return reply
        channels[0].mutate = mutate
        with self.assertRaisesRegex(Rejected, "staged candidate"):
            runtime.advance()
        self.assertTrue(runtime.frozen and runtime.coordinator.poisoned)
        self.assertEqual(self.ledger.save(), before)
        with self.assertRaises(Rejected):
            runtime.advance()

    def test_fused_receipt_matches_separate_snapshot_and_reduces_rounds(self):
        class RecordingChannel(MemoryChannel):
            def __init__(self):
                super().__init__()
                self.worker = FencedWorker()
                self.requests = []

            def send(self, request, deadline):
                self.requests.append(request["op"])
                super().send(request, deadline)

        for mode in ("separate", "fused"):
            current = self.ledger.read()
            if current["claimed"]:
                self.ledger.rotate(current["generation"], current["tick"], current["runtime_root"], GROUPS)
                current = self.ledger.read()
            channels = [RecordingChannel(), RecordingChannel()]
            runtime = FencedRuntime(self.ledger, channels, worker_exchange=mode)
            for c in channels:
                c.requests.clear()
            with patch.object(self.ledger, "commit", wraps=self.ledger.commit) as commit:
                result = runtime.advance()
                receipts = commit.call_args.args[0]
            expected = ["prepare", "seal"] + (["commit_snapshot"] if mode == "fused" else ["commit", "fenced_snapshot"])
            self.assertTrue(all(c.requests == expected for c in channels))
            # The new wire envelope does not change the exact signed
            # receipt or canonical durable receipt hash.
            self.assertEqual(receipts, candidate_receipts(runtime.coordinator))
            self.assertEqual(result["receipt_hash"], hashlib.sha256(canonical(sorted(
                receipts, key=lambda r:r["body"]["session"]))).hexdigest())
            self.assertEqual(self.ledger.read()["state"], serial_step(current["state"], current["tick"]+1))

    def test_fused_invalid_ack_or_receipt_fences_without_publication(self):
        for mutate in (
            lambda r:r.update(root="0"*64),
            lambda r:r.update(tick=True),
            lambda r:r.pop("snapshot"),
            lambda r:r["snapshot"]["body"].update(session="0"*64),
            lambda r:r["snapshot"].update(mac="0"*64),
            lambda r:r["snapshot"]["body"].update(parent_root="0"*64),
        ):
            with self.subTest(mutate=mutate):
                current = self.ledger.read()
                if current["claimed"]:
                    self.ledger.rotate(current["generation"], current["tick"], current["runtime_root"], GROUPS)
                channels = [MemoryChannel(), MemoryChannel()]
                for c in channels:
                    c.worker = FencedWorker()
                runtime = FencedRuntime(self.ledger, channels)
                before = self.ledger.save()
                history = self.ledger.history()
                def corrupt(reply):
                    if reply.get("op") == "committed_snapshot":
                        mutate(reply)
                    return reply
                channels[0].mutate = corrupt
                with self.assertRaises(Rejected):
                    runtime.advance()
                self.assertTrue(runtime.frozen and runtime.coordinator.poisoned)
                self.assertEqual(self.ledger.save(), before)
                self.assertEqual(self.ledger.history(), history)
                self.assertEqual([c.worker.worker.tick for c in channels], [1,1])

    def test_lost_fused_reply_never_retries_or_publishes_and_requires_new_grants(self):
        class LostReply(MemoryChannel):
            def recv(self, deadline):
                reply = super().recv(deadline)
                if reply.get("op") == "committed_snapshot":
                    raise EOFError("lost committed snapshot")
                return reply

        channels = [MemoryChannel(), LostReply()]
        for c in channels:
            c.worker = FencedWorker()
        runtime = FencedRuntime(self.ledger, channels)
        before = self.ledger.save()
        history = self.ledger.history()
        with self.assertRaises(EOFError):
            runtime.advance()
        self.assertTrue(runtime.frozen and runtime.coordinator.poisoned)
        self.assertEqual(self.ledger.save(), before)
        self.assertEqual(self.ledger.history(), history)
        self.assertEqual([c.worker.worker.tick for c in channels], [1,1])
        with self.assertRaises(Rejected):
            runtime.advance()
        with self.assertRaises(Rejected):
            FencedRuntime(self.ledger, [])
        self.rotate()
        fresh = [MemoryChannel(), MemoryChannel()]
        for c in fresh:
            c.worker = FencedWorker()
        restarted = FencedRuntime(self.ledger, fresh)
        self.assertEqual(restarted.advance()["tick"], 1)
        self.assertEqual(self.ledger.read()["state"], serial_step(fixture(), 1))

    def test_fused_worker_cannot_commit_without_exact_seal(self):
        worker = FencedWorker()
        current = self.ledger.read()
        grant = current["grants"][0]
        worker.handle({"op":"bind_grant","grant":grant})
        worker.handle({"op":"init","version":1,
                       "regions":[current["state"][r].wire() for r in grant["regions"]]})
        request = {"op":"commit_snapshot","tick":1,"root":"a"*64,"receipt":"b"*64}
        with self.assertRaises(Rejected):
            worker.handle(request)
        self.assertEqual(worker.worker.tick, 0)
        self.assertEqual(worker.worker.phase, "idle")
        self.assertIsNone(worker.parent_root)

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
