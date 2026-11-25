"""Placement selection, consent revisions and research-session boundaries."""
from dataclasses import replace
import unittest

from region_runtime import Coordinator, Rejected, Worker, initial_world, region_roots, serial_step, world_root
from test_region_runtime import MemoryChannel
from worker_placement import (ConsentLedger, Offer, Sample, MIB, MAX_SAMPLE_AGE_NS,
                              assess, choose, region_load, DEADLINE_US)

NOW = 20_000_000_000


def offer(slot=0, rev=1, mem=96, cpu=1000):
    return Offer(slot,rev,mem,cpu)


def sample(o, **changes):
    return replace(Sample(o.slot,o.revision,o.cpu_millicores,(o.process_memory_mib-8)*MIB,
                          100,NOW-1000,True),**changes)


class PlacementTests(unittest.TestCase):
    def setUp(self):
        self.load = region_load(initial_world("spread",16384,seed=58))

    def test_player_only_stale_missing_or_unenforced_never_admitted(self):
        o = offer()
        cases = [(offer(mem=0,cpu=0),sample(o),"player_only"),
                 (o,None,"unmeasured"),
                 (o,sample(o,revision=2),"stale_consent"),
                 (o,sample(o,slot=1),"stale_consent"),
                 (o,sample(o,observed_ns=NOW-MAX_SAMPLE_AGE_NS-1),"stale_measurement"),
                 (o,sample(o,observed_ns=NOW+1),"stale_measurement"),
                 (o,sample(o,verified_enforcement=False),"unsupported_enforcement"),
                 (o,sample(o,usable_memory_bytes=97*MIB),"inconsistent_measurement"),
                 (offer(cpu=500),sample(o),"inconsistent_measurement"),
                 (o,sample(o,effective_cpu_millicores=249),"insufficient_cpu"),
                 (o,sample(o,usable_memory_bytes=32*MIB),"insufficient_memory"),
                 (o,sample(o,rtt_us=8001),"path_too_slow")]
        for pref,measurement,reason in cases:
            with self.subTest(reason=reason):
                self.assertEqual(assess(pref,measurement,NOW),reason)
                decision=choose([pref],{pref.slot:measurement} if measurement else {},self.load,NOW)
                self.assertIsNone(decision["plan"])
                self.assertEqual(decision["reasons"]["0"],reason)

    def test_spread_admits_useful_second_worker_not_low_memory_or_slow_path(self):
        opts=[offer(0),offer(1),offer(2,mem=32,cpu=250),offer(3)]
        samples={o.slot:sample(o) for o in opts}
        samples[3]=sample(opts[3],rtt_us=50100)
        d=choose(opts,samples,self.load,NOW)
        plan=d["plan"]
        self.assertEqual(set(plan["owner_by_region"]),{0,1})
        self.assertEqual(d["reasons"],{"0":"admitted_for_experiment","1":"admitted_for_experiment",
                                     "2":"insufficient_memory","3":"path_too_slow"})
        self.assertLessEqual(plan["score_us"]*100,d["best_scores_by_worker_count"]["1"]*80)
        self.assertEqual(sorted(r for a in plan["allocations"].values() for r in a["regions"]),list(range(8)))
        self.assertEqual(d,choose(list(reversed(opts)),dict(reversed(list(samples.items()))),self.load,NOW))

    def test_clustered_extra_worker_has_no_useful_gain(self):
        opts=[offer(0),offer(1)]
        d=choose(opts,{o.slot:sample(o) for o in opts},region_load(initial_world("clustered",16384,59)),NOW)
        self.assertEqual(len(d["plan"]["allocations"]),1)
        self.assertIn("no_useful_feasible_placement",d["reasons"].values())
        self.assertIn("2",d["best_scores_by_worker_count"])

    def test_memory_can_require_two_even_if_one_would_have_enough_cpu(self):
        opts=[offer(0,mem=64),offer(1,mem=64)]
        d=choose(opts,{o.slot:sample(o) for o in opts},self.load,NOW)
        self.assertNotIn("1",d["best_scores_by_worker_count"])
        self.assertEqual(len(d["plan"]["allocations"]),2)

    def test_cpu_or_memory_shortage_never_returns_partial_world(self):
        for o in [offer(cpu=250),offer(mem=32),offer(mem=48)]:
            with self.subTest(o=o):
                d=choose([o],{0:sample(o)},self.load,NOW)
                self.assertIsNone(d["plan"])
                self.assertNotIn("admitted_for_experiment",d["reasons"].values())

    def test_freshness_boundary_and_invalid_offer_measurement(self):
        o=offer()
        self.assertEqual(assess(o,sample(o,observed_ns=NOW-MAX_SAMPLE_AGE_NS),NOW),"eligible")
        for invalid in [offer(slot=True),offer(rev=0),offer(mem=0),offer(cpu=0),offer(mem=129),offer(cpu=1001)]:
            with self.assertRaises(Rejected):
                invalid.validate()
        for invalid in [sample(o,rtt_us=-1),sample(o,effective_cpu_millicores=True),sample(o,verified_enforcement=1)]:
            with self.assertRaises(Rejected):
                assess(o,invalid,NOW)
        with self.assertRaises(Rejected):
            choose([o,o],{0:sample(o)},self.load,NOW)
        with self.assertRaises(Rejected):
            choose([o],{1:sample(o)},self.load,NOW)
        with self.assertRaises(Rejected):
            choose([o],{0:sample(o)},[{**r,"cpu_us":1} for r in self.load],NOW)
        with self.assertRaises(Rejected):
            choose([o],{0:sample(o)},self.load,True)

    def test_explicit_zero_and_revision_changes_are_not_implicit(self):
        ledger=ConsentLedger()
        opts=[offer(i,mem=0,cpu=0) for i in range(4)]
        ledger.accept(opts)
        ledger.accept(opts)
        with self.assertRaises(Rejected):
            ledger.accept(opts[:-1])
        changed=[offer(0),*opts[1:]]
        with self.assertRaises(Rejected):
            ledger.accept(changed)
        changed[0]=offer(0,rev=2)
        ledger.accept(changed)
        with self.assertRaises(Rejected):
            ledger.accept(opts)
        revoked=[offer(0,rev=3,mem=0,cpu=0),*opts[1:]]
        ledger.accept(revoked)
        with self.assertRaises(Rejected):
            ledger.accept(changed)

    def test_explicit_three_worker_layout_preserves_continuous_tick(self):
        initial=initial_world("boundary",512)
        control=Coordinator([MemoryChannel()],initial)
        control.step()
        cut=control.snapshot()[0]
        prior=control.root
        control.poisoned=True
        with self.assertRaises(Rejected):
            control.step()
        resumed=Coordinator([MemoryChannel() for _ in range(3)],cut,
                            placement={r:r%3 for r in range(8)},start_tick=1)
        self.assertEqual(resumed.root,prior)
        for tick in range(2,8):
            cut=serial_step(cut,tick)
            self.assertEqual(resumed.step()["root"],world_root(tick,region_roots(cut)))
        self.assertEqual(resumed.snapshot()[0],cut)

    def test_bad_resume_is_rejected_before_any_init_and_session_cannot_reinit(self):
        initial=initial_world("spread",64)
        for placement in [{r:0 for r in range(7)},{r:0 for r in range(8)},{r:2 for r in range(8)}]:
            channels=[MemoryChannel(),MemoryChannel()]
            with self.assertRaises(Rejected):
                Coordinator(channels,initial,placement=placement,start_tick=2)
            self.assertTrue(all(c.worker.phase=="new" for c in channels))
        channel=MemoryChannel()
        c=Coordinator([channel],initial,placement={r:0 for r in range(8)},start_tick=5)
        with self.assertRaises(Rejected):
            channel.worker.handle({"op":"init_at","version":1,"tick":1,"regions":[r.wire() for r in initial.values()]})
        self.assertEqual(c.tick,5)
        self.assertEqual(c.step()["tick"],6)


if __name__=="__main__":
    unittest.main()
