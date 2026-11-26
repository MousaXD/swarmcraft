"""Durable player input, full-state publication and reference action tests."""
import copy
import json
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import unittest

from authority_lab import GROUPS, SWAPPED
from player_runtime import fixture
from region_runtime import (MAX_BANK, REGIONS, WIDTH, Region, Rejected,
                            checked_controls, region_roots, totals)
from regional_authority import Authority, FencedWorker, candidate_receipts
from test_region_runtime import MemoryChannel
from transition_runtime import DurableRuntime, TransitionAuthority
from verify_transition_evidence import verify


def channels():
    result = [MemoryChannel(), MemoryChannel()]
    for channel in result:
        channel.worker = FencedWorker()
    return result


class TransitionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.path = Path(self.temp.name) / "world.db"
        self.a = TransitionAuthority.create(self.path, "a" * 64, fixture(), [0, 1, 2], GROUPS)

    def tearDown(self):
        self.a.close()
        self.temp.cleanup()

    def request(self, key="b", direction=1, action=1):
        cur = self.a.save()
        return {"op": "act", "world": cur["world"], "id": key * 64,
                "tick": cur["tick"] + 1, "parent": cur["cut_root"],
                "direction": direction, "action": action}

    def test_exact_pending_and_committed_replay(self):
        req = self.request()
        queued = self.a.submit(0, req)
        self.assertEqual(queued, self.a.submit(0, req))
        self.assertEqual(self.a.view(0)["inventory"], 5)
        runtime = DurableRuntime(self.a, channels())
        runtime.advance()
        committed = self.a.submit(0, req)
        self.assertEqual(committed["inventory_delta"], 1)
        self.assertEqual(committed["view"]["position"], 512)
        before = self.a.save()
        self.a.close()
        self.a = TransitionAuthority(self.path)
        self.assertEqual(committed, self.a.submit(0, req))
        self.assertEqual(self.a.save(), before)

    def test_invalid_or_conflicting_input_no_mutation(self):
        req = self.request()
        self.a.submit(0, req)
        baseline = self.a.audit()
        for player, bad in [(1, req), (0, {**req, "direction": -1}),
                            (0, {**req, "id": "c"*64}), (3, req),
                            (True, req), (1, {**req, "id": "c"*64, "action": True}),
                            (1, {**req, "id": "c"*64, "world": "c"*64}),
                            (1, {**req, "id": "c"*64, "tick": 2}),
                            (1, {**req, "id": "c"*64, "parent": "c"*64}),
                            (1, {**req, "id": "c"*64, "player": 0})]:
            with self.subTest(player=player, bad=bad), self.assertRaises(Rejected):
                self.a.submit(player, bad)
        self.assertEqual(self.a.audit(), baseline)

    def test_batch_queue_atomic_and_exact_pending_retry(self):
        first, second = self.request("b"), self.request("c")
        baseline = self.a.audit()
        # The second entry conflicts with an existing player's command.
        # A valid first insertion must roll back with it.
        self.a.submit(1, self.request("d"))
        pending = self.a.audit()
        with self.assertRaises(Rejected):
            self.a.submit_batch([(0, first), (1, second)])
        self.assertEqual(self.a.audit(), pending)
        for inputs in ([], [(0, first), (0, second)], [(0, first), (1, first)],
                       [(0, first), (1, {**second, "tick": 2})],
                       [(0, first), (3, second)], [(0, first), (True, second)]):
            with self.assertRaises(Rejected):
                self.a.submit_batch(inputs)
            self.assertEqual(self.a.audit(), pending)
        inputs = [(0, first), (1, self.request("d"))]
        queued = self.a.submit_batch(inputs)
        self.assertEqual([r["op"] for r in queued], ["queued", "queued"])
        self.assertEqual(self.a.submit_batch(inputs), queued)
        self.assertEqual(len(self.a.audit()["commands"]), len(baseline["commands"])+2)
        with self.assertRaises(Rejected):
            self.a.committed_receipts(inputs)

    def test_atomic_queue_and_seal_survive_restart_and_refuse_changes(self):
        inputs = [(0, self.request("b")), (1, self.request("c"))]
        runtime = DurableRuntime(self.a, channels())
        before = self.a.audit()
        with self.assertRaises(Rejected):
            runtime.seal(inputs=[inputs[0], (1, {**inputs[1][1], "parent": "0"*64})])
        self.assertEqual(self.a.audit(), before)
        controls = runtime.seal(inputs=inputs)
        self.assertEqual(runtime.seal(inputs=list(reversed(inputs))), controls)
        journal = self.a.audit()
        self.assertEqual(len(journal["commands"]), 2)
        self.assertEqual(len(journal["batches"]), 1)
        self.assertTrue(all(r["response"] is None for r in journal["commands"]))
        for invalid in ([(0, {**inputs[0][1], "action": -1})], [(2, self.request("d"))]):
            with self.assertRaises(Rejected):
                runtime.seal(inputs=invalid)
            self.assertEqual(self.a.audit(), journal)
        before = self.a.save()
        self.a.close()
        self.a = TransitionAuthority(self.path)
        self.assertEqual(self.a.audit(), journal)
        self.a.rotate(before["generation"], before["tick"], before["runtime_root"], SWAPPED)
        runtime = DurableRuntime(self.a, channels())
        runtime.advance(inputs=inputs)
        self.assertEqual([r["view"]["tick"] for r in self.a.committed_receipts(inputs)], [1, 1])

    def test_fused_seal_rolls_back_insertions_if_sealing_fails(self):
        from unittest.mock import patch
        inputs = [(0, self.request("b")), (1, self.request("c"))]
        runtime = DurableRuntime(self.a, channels())
        before = self.a.audit()
        with patch("transition_runtime.batch_controls", side_effect=RuntimeError("injected seal failure")):
            with self.assertRaisesRegex(RuntimeError, "injected seal failure"):
                runtime.seal(inputs=inputs)
        self.assertEqual(self.a.audit(), before)
        runtime.advance(inputs=inputs)
        self.assertEqual(len(self.a.committed_receipts(inputs)), 2)

    def test_batched_seal_process_death_before_and_after_durable_commit(self):
        inputs = [(0, self.request("b")), (1, self.request("c"))]
        DurableRuntime(self.a, channels())
        baseline = self.a.save()
        journal = self.a.audit()
        program = """
import json, os, signal, sys
import transition_runtime as tr
authority = tr.TransitionAuthority(sys.argv[1])
inputs = json.loads(sys.argv[2])
current = authority.read()
def die(*args):
    os.kill(os.getpid(), signal.SIGKILL)
if sys.argv[3] == "before":
    tr.batch_controls = die
authority.seal_next(current["generation"], current["tick"], current["runtime_root"], inputs=inputs)
die()
"""
        for phase in ("before", "after"):
            child = subprocess.run(
                [sys.executable, "-c", program, str(self.path), json.dumps(inputs), phase],
                cwd=Path(__file__).parent, capture_output=True, timeout=15)
            self.assertEqual(child.returncode, -signal.SIGKILL, child.stderr.decode(errors="replace"))
            self.a.close()
            self.a = TransitionAuthority(self.path)
            self.assertEqual(self.a.save(), baseline)
            if phase == "before":
                self.assertEqual(self.a.audit(), journal)
            else:
                self.assertEqual(len(self.a.audit()["commands"]), len(inputs))
                self.assertIsNone(self.a.audit()["batches"][0]["result"])
        self.a.rotate(baseline["generation"], baseline["tick"], baseline["runtime_root"], SWAPPED)
        DurableRuntime(self.a, channels()).advance(inputs=inputs)
        self.assertEqual(len(self.a.committed_receipts(inputs)), 2)

    def test_read_only_receipts_after_commit_and_reopen(self):
        inputs = [(0, self.request("b")), (1, self.request("c"))]
        runtime = DurableRuntime(self.a, channels())
        runtime.advance(inputs=inputs)
        original = self.a.audit()
        statements = []
        self.a.db.set_trace_callback(statements.append)
        try:
            result = self.a.committed_receipts(inputs)
        finally:
            self.a.db.set_trace_callback(None)
        self.assertEqual([s for s in statements if s in ("BEGIN", "BEGIN IMMEDIATE", "COMMIT")],
                         ["BEGIN", "COMMIT"])
        self.assertTrue(all(s.startswith("SELECT") or s in ("BEGIN", "COMMIT") for s in statements))
        self.assertEqual(self.a.audit(), original)
        self.assertEqual(result, [self.a.submit(p, q) for p,q in inputs])
        for bad in ([(1, inputs[0][1])], [(0, {**inputs[0][1], "direction": -1})],
                    [(0, self.request("d"))], [(0, {**inputs[0][1], "id": "e"*64})]):
            with self.assertRaises(Rejected):
                self.a.committed_receipts(bad)
        runtime.advance()
        self.assertEqual(self.a.committed_receipts(list(reversed(inputs))), list(reversed(result)))
        self.a.close()
        self.a = TransitionAuthority(self.path)
        self.assertEqual(self.a.committed_receipts(inputs), result)

    def test_real_transaction_boundaries_for_individual_and_batched_ticks(self):
        from scale_profile import PhaseClock
        runtime = DurableRuntime(self.a, channels())
        clock = PhaseClock()
        boundary = PhaseClock()
        self.a.db.set_trace_callback(clock.trace)
        self.a.transaction_event = boundary.boundary
        try:
            for batched, keys in ((False, ("b","c")), (True, ("d","e"))):
                inputs = [(p,self.request(k)) for p,k in enumerate(keys)]
                clock.begin()
                boundary.begin()
                if batched:
                    runtime.advance(inputs=inputs)
                    self.a.committed_receipts(inputs)
                else:
                    for p,q in inputs:
                        self.a.submit(p,q)
                    runtime.advance()
                    for p,q in inputs:
                        self.a.submit(p,q)
                clock.finish(1)
                boundary.finish(1)
                expected = {"write":2,"read":2,"commit":4,"rollback":0} if batched else {
                    "write":6,"read":1,"commit":7,"rollback":0}
                self.assertEqual(clock.transactions, expected)
                self.assertEqual(boundary.transactions, expected)
        finally:
            self.a.db.set_trace_callback(None)
            self.a.transaction_event = None

    def test_receipt_lookup_refuses_uncommitted_and_corrupt_batch(self):
        inputs = [(0, self.request("b")), (1, self.request("c"))]
        runtime = DurableRuntime(self.a, channels())
        coordinator = runtime.runtime.coordinator
        coordinator.step(controls=runtime.seal(inputs=inputs))
        reader = TransitionAuthority(self.path)
        try:
            def observe():
                with self.assertRaises(Rejected):
                    reader.committed_receipts(inputs)
            self.a.commit(candidate_receipts(coordinator), before_commit=observe)
            expected = reader.committed_receipts(inputs)
            self.a.db.execute("UPDATE batches SET result=? WHERE tick=1", ("0"*64,))
            with self.assertRaises(Rejected):
                reader.committed_receipts(inputs)
            self.a.db.execute("UPDATE batches SET result=? WHERE tick=1", (expected[0]["view"]["root"],))
            self.a.db.execute("UPDATE commands SET response=? WHERE player=1", (b'{}',))
            with self.assertRaises(Rejected):
                reader.committed_receipts(inputs)
        finally:
            reader.close()

    def test_batched_runtime_refuses_retired_generation_before_queue(self):
        inputs = [(0, self.request("b")), (1, self.request("c"))]
        runtime = DurableRuntime(self.a, channels())
        before = self.a.save()
        self.a.rotate(before["generation"], before["tick"], before["runtime_root"], GROUPS)
        journal = self.a.audit()
        with self.assertRaisesRegex(Rejected, "retired runtime"):
            runtime.advance(inputs=inputs)
        self.assertTrue(runtime.runtime.frozen)
        self.assertEqual(self.a.audit(), journal)

    def test_sealed_batch_is_immutable_and_survives_restart(self):
        req = self.request()
        self.a.submit(0, req)
        runtime = DurableRuntime(self.a, channels())
        first = runtime.seal()
        self.assertEqual(first, runtime.seal())
        with self.assertRaises(Rejected):
            self.a.submit(1, self.request("c"))
        before, journal = self.a.save(), self.a.audit()
        self.a.close()
        self.a = TransitionAuthority(self.path)
        self.assertEqual(journal, self.a.audit())
        self.a.rotate(before["generation"], before["tick"], before["runtime_root"], SWAPPED)
        runtime = DurableRuntime(self.a, channels())
        self.assertEqual(first, runtime.seal())
        runtime.advance()
        self.assertEqual(self.a.submit(0, req)["inventory_delta"], 1)
        self.assertEqual(self.a.view(1)["position"], 1023)

    def test_unsealed_and_wrong_candidate_refused_even_with_current_receipts(self):
        for sealed in (False, True):
            with self.subTest(sealed=sealed):
                if sealed:
                    cur = self.a.save()
                    self.a.rotate(cur["generation"], cur["tick"], cur["runtime_root"], GROUPS)
                    self.a.submit(0, self.request())
                runtime = DurableRuntime(self.a, channels())
                if sealed:
                    runtime.seal()
                coordinator = runtime.runtime.coordinator
                coordinator.step()  # authenticated but ignores durable input
                before, journal = self.a.save(), self.a.audit()
                with self.assertRaises(Rejected):
                    self.a.commit(candidate_receipts(coordinator))
                self.assertEqual(self.a.save(), before)
                self.assertEqual(self.a.audit(), journal)

    def test_legacy_authority_cannot_open_transaction_database(self):
        with self.assertRaises(Rejected):
            Authority(self.path)
        other = Path(self.temp.name) / "legacy.db"
        legacy = Authority.create(other, "d"*64, fixture(), [0, 1, 2], GROUPS)
        legacy.close()
        with self.assertRaises(Rejected):
            TransitionAuthority(other)

    def test_reader_cannot_observe_command_result_before_commit(self):
        req = self.request()
        self.a.submit(0, req)
        runtime = DurableRuntime(self.a, channels())
        coordinator = runtime.runtime.coordinator
        coordinator.step(controls=runtime.seal())
        reader = TransitionAuthority(self.path)
        try:
            def observe():
                self.assertEqual(reader.save()["tick"], 0)
                self.assertIsNone(reader.audit()["commands"][0]["response"])
            self.a.commit(candidate_receipts(coordinator), before_commit=observe)
            self.assertEqual(reader.save()["tick"], 1)
            self.assertEqual(reader.audit()["commands"][0]["response"]["inventory_delta"], 1)
        finally:
            reader.close()

    def test_corrupted_history_refuses_reopen(self):
        self.a.submit(0, self.request())
        DurableRuntime(self.a, channels()).advance()
        self.a.db.execute("UPDATE commands SET response=? WHERE player=0", (b'{}',))
        with self.assertRaises(Rejected):
            TransitionAuthority(self.path)

    def test_replay_after_later_ticks_returns_original_view(self):
        req = self.request()
        self.a.submit(0, req)
        runtime = DurableRuntime(self.a, channels())
        runtime.advance()
        response = self.a.submit(0, req)
        runtime.advance()
        self.assertEqual(self.a.save()["tick"], 2)
        self.assertEqual(self.a.submit(0, req), response)
        self.assertEqual(response["view"]["tick"], 1)

    def test_control_rows_action_bounds(self):
        for bad in ([0, 1, True], [0, 1, 2], [0, 1, 0, 0]):
            with self.assertRaises(Rejected):
                checked_controls([bad], {0})
        self.assertEqual(checked_controls([[0, 1, -1]], {0}), {0: (1, -1)})


class CellInteractionTests(unittest.TestCase):
    def run_fixture(self, bank, inventory, action):
        pos = 4 * WIDTH - 1
        state = {r: Region(r, (0,) * WIDTH, (0,) * WIDTH, ()) for r in range(REGIONS)}
        state[3] = Region(3, (0,) * (WIDTH - 1) + (41,), (0,) * (WIDTH - 1) + (bank,),
                          ((0, pos, inventory), (1, pos, inventory)))
        with tempfile.TemporaryDirectory() as tmp:
            ledger = TransitionAuthority.create(Path(tmp) / "w.db", "a"*64, state, [0, 1], GROUPS)
            try:
                for player, key in enumerate(("b", "c")):
                    view = ledger.view(player)
                    ledger.submit(player, {"op": "act", "world": view["world"], "id": key*64,
                                           "tick": 1, "parent": view["root"], "direction": 1, "action": action})
                DurableRuntime(ledger, channels()).advance()
                accepted = ledger.read()["state"]
                self.assertEqual(totals(region_roots(state)), totals(region_roots(accepted)))
                self.assertEqual(accepted[4].signals[0], 41)
                self.assertEqual([a[1] for a in accepted[4].agents], [pos+1, pos+1])
                return [a[2] for a in accepted[4].agents], accepted[3].banks[-1], accepted[4].banks[0]
            finally:
                ledger.close()

    def test_competing_pickups_cannot_duplicate_token(self):
        self.assertEqual(self.run_fixture(1, 0, 1), ([1, 0], 0, 0))

    def test_empty_drop_cannot_create_token(self):
        self.assertEqual(self.run_fixture(0, 0, -1), ([0, 0], 0, 0))

    def test_drop_respects_bank_capacity_and_boundary_resource_flow(self):
        # The source starts full; both drops are refused before the periodic
        # resource debit flows to the adjacent region.
        self.assertEqual(self.run_fixture(MAX_BANK, 1, -1), ([1, 1], MAX_BANK-1, 1))


class TransitionEvidenceTests(unittest.TestCase):
    def test_retained_runs_and_negative_evidence(self):
        root = Path(__file__).parent / "evidence"
        for suffix in (1, 2):
            data = json.loads((root / f"2026-11-26-transition-{suffix}.json").read_text())
            verify(data)
            mutations = [
                lambda d: d["phases"][1]["clients"][0]["observed"].update(reply={"op": "committed"}),
                lambda d: d["phases"][4]["clients"][0]["observed"]["reply"].update(inventory_delta=2),
                lambda d: d["phases"][2]["clients"][1]["request"].update(id="0"*64),
                lambda d: d["checkpoints"][2]["journal"]["batches"][1].update(result="0"*64),
                lambda d: d["checkpoints"][5]["save"].update(tick=4),
                lambda d: d["faults"][0].update(exitcode=0),
                lambda d: d["workers"][1]["pids"].__setitem__(0, d["workers"][0]["pids"][0]),
                lambda d: d["checkpoints"][7]["save"]["regions"][0][2].__setitem__(0, -1),
            ]
            for mutation in mutations:
                bad = copy.deepcopy(data)
                mutation(bad)
                with self.assertRaises(Rejected):
                    verify(bad)
