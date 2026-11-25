"""Regression tests for the paced reference-runtime measurement contract."""
import copy
import json
from pathlib import Path
import unittest

from region_runtime import Rejected, region_roots, serial_step, world_root
from scale_lab import PERIOD_NS, WINDOW_NS, PLAYERS, SCENARIOS, fixture, summarize
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


if __name__=="__main__":
    unittest.main()
