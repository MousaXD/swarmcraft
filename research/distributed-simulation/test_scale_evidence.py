"""Regression tests for the paced reference-runtime measurement contract."""
import copy
import json
from pathlib import Path
import unittest
from unittest.mock import patch

from region_runtime import Rejected, region_roots, serial_step, world_root
from scale_lab import PERIOD_NS, WINDOW_NS, PLAYERS, SCENARIOS, fixture, summarize
from scale_profile import PHASES, PhaseClock
from verify_scale_evidence import verify, verify_run, verify_timing


class TimingTests(unittest.TestCase):
    def timed_rows(self, step_ns):
        rows=[]
        finish=0
        for tick in range(1,WINDOW_NS//PERIOD_NS+1):
            scheduled=(tick-1)*PERIOD_NS
            start=max(finish,scheduled)
            if start>=WINDOW_NS:
                break
            finish=start+step_ns
            rows.append({"tick":tick,"scheduled_ns":scheduled,"begin_ns":start,"finish_ns":finish,
                         "root":"a"*64,"cut_root":"b"*64})
        return rows

    def test_full_paced_target(self):
        rows=self.timed_rows(5_000_000)
        stats=summarize(rows)
        self.assertTrue(stats["qualifies_20hz"])
        self.assertEqual(stats["within_window"],160)
        self.assertEqual(stats["window_tps"],20)
        verify_timing({"ticks":rows,"summary":stats,"coordinator_cpu_ns":1})

    def test_overload_cannot_claim_success_using_service_rate(self):
        rows=self.timed_rows(60_000_000)
        stats=summarize(rows)
        self.assertFalse(stats["qualifies_20hz"])
        self.assertEqual(stats["within_window"],133)
        self.assertEqual(stats["on_time_ticks"],0)
        self.assertEqual(stats["target_slot_failures"],160)
        verify_timing({"ticks":rows,"summary":stats,"coordinator_cpu_ns":1})
        altered={**stats,"qualifies_20hz":True}
        with self.assertRaises(Rejected):
            verify_timing({"ticks":rows,"summary":altered,"coordinator_cpu_ns":1})

    def test_short_run_or_overlapping_tick_refused(self):
        rows=self.timed_rows(5_000_000)[:10]
        with self.assertRaises(Rejected):
            verify_timing({"ticks":rows,"summary":summarize(rows),"coordinator_cpu_ns":1})
        rows=self.timed_rows(5_000_000)
        rows[1]["begin_ns"]=rows[0]["finish_ns"]-1
        with self.assertRaises(Rejected):
            verify_timing({"ticks":rows,"summary":summarize(rows),"coordinator_cpu_ns":1})

    def test_exclusive_profile_never_double_counts_nested_phases(self):
        clock=PhaseClock()
        clock.begin()
        with patch("scale_profile.time.monotonic_ns",side_effect=[10,20,35,50]):
            with clock.phase("ledger_read"):
                with clock.phase("state_decode_roots"):
                    pass
        values=clock.finish(100)
        self.assertEqual(values["ledger_read"],25)
        self.assertEqual(values["state_decode_roots"],15)
        self.assertEqual(values["other"],60)
        self.assertEqual(sum(values.values()),100)

    def test_profile_rejects_forged_phase_accounting(self):
        rows=self.timed_rows(5_000_000)
        for row in rows:
            phases=dict.fromkeys(PHASES,1)
            phases["other"]=5_000_000-len(PHASES)+1
            row["phase_ns"]=phases
        def verify_profile():
            verify_timing({"ticks":rows,"summary":summarize(rows),"coordinator_cpu_ns":1},True)
        verify_profile()
        for mutate in (
            lambda p:p.update(other=-1),
            lambda p:p.update(worker_tick=0),
            lambda p:p.update(other=p["other"]+1),
            lambda p:p.update(unknown_phase=1),
            lambda p:p.pop("ledger_read"),
        ):
            saved=rows[0]["phase_ns"].copy()
            mutate(rows[0]["phase_ns"])
            with self.assertRaises(Rejected):
                verify_profile()
            rows[0]["phase_ns"]=saved

    def test_transaction_boundary_evidence(self):
        rows=self.timed_rows(5_000_000)
        for row in rows:
            phases=dict.fromkeys(PHASES,1)
            phases["other"]=5_000_000-len(PHASES)+1
            row["phase_ns"]=phases
            row["transactions"]={"write":2,"read":2,"commit":4,"rollback":0}
        evidence={"ticks":rows,"summary":summarize(rows),"coordinator_cpu_ns":1}
        verify_timing(evidence,True,"batched")
        with self.assertRaises(Rejected):
            verify_timing(evidence,True,"individual")
        for bad in ({"write":6}, {"rollback":1}, {"commit":3}, {"read":True}, {"extra":0}):
            saved=rows[0]["transactions"].copy()
            rows[0]["transactions"].update(bad)
            with self.assertRaises(Rejected):
                verify_timing(evidence,True,"batched")
            rows[0]["transactions"]=saved

    def test_trace_counts_only_current_tick_boundaries(self):
        clock=PhaseClock()
        clock.trace("BEGIN IMMEDIATE")
        clock.begin()
        for statement in ("BEGIN IMMEDIATE", "INSERT INTO commands VALUES(1)", "COMMIT",
                          "BEGIN", "SELECT * FROM world", "COMMIT"):
            clock.trace(statement)
        clock.finish(1)
        clock.trace("BEGIN")
        self.assertEqual(clock.transactions,{"write":1,"read":1,"commit":2,"rollback":0})

    def test_worker_round_trip_evidence(self):
        for mode,rounds in (("fused",3),("separate",4)):
            rows=self.timed_rows(5_000_000)
            for row in rows:
                row["worker_messages"]=[[rounds,rounds],[rounds,rounds]]
            evidence={"ticks":rows,"summary":summarize(rows),"coordinator_cpu_ns":1,"workers":2}
            verify_timing(evidence,worker_exchange=mode)
            for invalid in ([[rounds,rounds]], [[rounds,rounds],[rounds,rounds+1]],
                            [[True,rounds],[rounds,rounds]], [[rounds,rounds]]*3):
                rows[0]["worker_messages"]=invalid
                with self.assertRaises(Rejected):
                    verify_timing(evidence,worker_exchange=mode)


class RetainedEvidenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        path=Path(__file__).parent/"evidence"/"2026-11-26-scale.json"
        cls.report=json.loads(path.read_text())
        cls.row=cls.report["runs"][0]
        n=len(cls.row["ticks"])
        cls.states=[fixture(SCENARIOS[0][1],SCENARIOS[0][2])]
        for tick in range(1,n+1):
            d=1 if tick%2 else -1
            cls.states.append(serial_step(cls.states[-1],tick,controls={p:(d,d) for p in PLAYERS}))
        cls.roots=[world_root(t,region_roots(s)) for t,s in enumerate(cls.states)]

    def test_complete_matrix(self):
        result=verify(self.report)
        self.assertEqual(result["runs"],24)
        self.assertEqual(result["worker_processes"],36)
        self.assertEqual(result["client_processes"],48)

    def test_tampered_single_run(self):
        for mutate in (
            lambda r:r["summary"].update(qualifies_20hz=not r["summary"]["qualifies_20hz"]),
            lambda r:r["ticks"][0].update(root="0"*64),
            lambda r:r["clients"][0]["receipts"][0]["view"].update(inventory=-1),
            lambda r:r["journal"]["commands"][0].update(response=None),
            lambda r:r["kernel_before"][0].update(kernel_memory_max_bytes=0),
            lambda r:r["worker_frame_bytes"][0].__setitem__(0,0),
            lambda r:r.update(scope_released=False),
            lambda r:r["final"]["grants"][0].update(regions=[]),
        ):
            row=copy.deepcopy(self.row)
            mutate(row)
            with self.assertRaises(Rejected):
                verify_run(row,SCENARIOS[0],1,1,self.report["hardware"],self.states,self.roots)

    def test_incomplete_matrix(self):
        altered={**self.report,"runs":self.report["runs"][:-1]}
        with self.assertRaises(Rejected):
            verify(altered)

    def test_profiled_cached_matrix(self):
        path=Path(__file__).parent/"evidence"/"2026-11-26-scale-profile-cached.json"
        report=json.loads(path.read_text())
        result=verify(report)
        self.assertEqual(result["state_cache"],"cached")
        self.assertEqual(result["runs"],24)
        self.assertEqual(result["accepted_ticks"],1883)
        self.assertEqual(result["client_commands"],3766)
        self.assertEqual(result["qualified_runs"],0)
        self.assertEqual(len(result["phase_profile"]),12)

    def test_batched_transaction_matrix(self):
        path=Path(__file__).parent/"evidence"/"2026-11-26-scale-batched.json"
        report=json.loads(path.read_text())
        result=verify(report)
        self.assertEqual(result["state_cache"],"cached")
        self.assertEqual(result["input_mode"],"batched")
        self.assertEqual(result["runs"],24)
        self.assertEqual(result["accepted_ticks"],2069)
        self.assertEqual(result["client_commands"],4138)
        self.assertEqual(result["qualified_runs"],0)
        self.assertEqual(result["transactions"],{"write":4138,"read":4138,"commit":8276,"rollback":0})

    def test_candidate_reuse_matrix(self):
        path=Path(__file__).parent/"evidence"/"2026-11-26-scale-reuse.json"
        report=json.loads(path.read_text())
        result=verify(report)
        self.assertEqual(result["candidate_mode"],"reuse")
        self.assertEqual(result["transaction_counts"],"boundary")
        self.assertEqual(result["runs"],24)
        self.assertEqual(result["accepted_ticks"],2834)
        self.assertEqual(result["within_window_ticks"],2818)
        self.assertEqual(result["client_commands"],5668)
        self.assertEqual(result["qualified_runs"],5)
        self.assertEqual(result["transactions"],{"write":5668,"read":5668,"commit":11336,"rollback":0})
        for field,value in (("candidate_mode","unknown"),("transaction_counts","unknown")):
            with self.assertRaises(Rejected):
                verify({**report,field:value})

    def test_fused_worker_exchange_matrix(self):
        path=Path(__file__).parent/"evidence"/"2026-11-26-scale-fused.json"
        report=json.loads(path.read_text())
        result=verify(report)
        self.assertEqual(result["worker_exchange"],"fused")
        self.assertEqual(result["runs"],24)
        self.assertEqual(result["accepted_ticks"],2185)
        self.assertEqual(result["within_window_ticks"],2162)
        self.assertEqual(result["client_commands"],4370)
        self.assertEqual(result["qualified_runs"],0)
        self.assertEqual(result["worker_frames"],[9711,9711])
        self.assertEqual(result["transactions"],{"write":4370,"read":4370,"commit":8740,"rollback":0})
        for value in ("unknown","separate"):
            with self.assertRaises(Rejected):
                verify({**report,"worker_exchange":value})
        for field in ("worker_messages","worker_frame_counts"):
            bad=copy.deepcopy(report)
            row=bad["runs"][0]
            target=row["ticks"][0] if field=="worker_messages" else row
            target[field][0][0]+=1
            with self.assertRaises(Rejected):
                verify(bad)


if __name__=="__main__":
    unittest.main()
