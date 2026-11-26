import copy
import json
from pathlib import Path
import tempfile
import time
import unittest

from authority_lab import GROUPS
from player_runtime import fixture
from recovery_runtime import RecoveryAuthority
from regional_authority import FencedWorker, candidate_receipts
from region_runtime import Rejected
from test_transition_runtime import channels
from transition_lab import commands
from transition_runtime import DurableRuntime, TransitionAuthority
from verify_recovery_evidence import verify
from worker_placement import MIB, Offer, Sample


def choices(a=56, b=56, ra=2, rb=2):
    return [Offer(0,ra,a,1000 if a else 0), Offer(1,rb,b,1000 if b else 0),
            Offer(2,1,0,0), Offer(3,1,0,0)]


class RecoveryTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.path = Path(self.tmp.name)/"world.db"
        self.a = RecoveryAuthority.create(self.path,"a"*64,fixture(),[0,1,2],GROUPS)

    def tearDown(self):
        self.a.close()
        self.tmp.cleanup()

    def probe(self):
        positive = self.a.begin_probe()
        status = self.a.status()
        now = time.monotonic_ns()
        samples = {o.slot: Sample(o.slot,o.revision,1000,(o.process_memory_mib-8)*MIB,
                                  100,now,True) for o in positive}
        return status,samples,now

    def start(self):
        self.a.configure(choices())
        status,samples,now = self.probe()
        self.a.admit(status["scope"],samples,[],now)
        return DurableRuntime(self.a,channels())

    def drain(self, reason="stop", unavailable=()):
        s=self.a.status()
        self.a.fence(s["scope"],s["generation"],reason,unavailable)
        self.a.release(s["scope"],s["generation"],s["probes"])

    def test_no_contribution_no_worker_or_bypass(self):
        prior=self.a.save()
        self.assertEqual(self.a.begin_probe(),[])
        with self.assertRaises(Rejected):
            self.a.claim(prior["generation"],prior["tick"],prior["runtime_root"])
        with self.assertRaises(Rejected):
            self.a.rotate(prior["generation"],prior["tick"],prior["runtime_root"],GROUPS)
        with self.assertRaises(Rejected):
            TransitionAuthority(self.path)
        self.assertEqual(self.a.save(),prior)

    def test_immutable_consent_revision_and_probe_scope(self):
        self.a.configure(choices())
        status,samples,now=self.probe()
        with self.assertRaises(Rejected):
            self.a.configure(choices(a=96))
        with self.assertRaises(Rejected):
            self.a.begin_probe()
        self.a.configure(choices(a=96,ra=3))
        self.assertEqual(self.a.status()["phase"],"draining")
        with self.assertRaises(Rejected):
            self.a.admit(status["scope"],samples,[],now)
        self.a.release(status["scope"],status["generation"],status["probes"])
        new,_,_=self.probe()
        self.assertEqual(new["generation"],status["generation"])
        self.assertGreater(new["scope"],status["scope"])
        self.a.fence(new["scope"],new["generation"],"startup")
        with self.assertRaises(Rejected):
            self.a.release(status["scope"],status["generation"],status["probes"])
        self.a.release(new["scope"],new["generation"],new["probes"])

    def test_withdrawal_between_receipt_and_commit(self):
        runtime=self.start()
        request=commands(self.a,1,1)
        for p,q in request.items():
            self.a.submit(p,q)
        prior=self.a.save()
        def withdraw():
            # Another independent connection writes consent under the same
            # serialization lock, immediately before the publication call.
            other=RecoveryAuthority(self.path)
            try:
                other.configure(choices(a=96,b=0,ra=3,rb=3))
            finally:
                other.close()
        with self.assertRaisesRegex(Rejected,"contribution scope fenced"):
            runtime.advance(before_publish=withdraw)
        self.assertEqual(self.a.save(),prior)
        self.assertTrue(runtime.runtime.frozen)
        self.assertEqual(self.a.status()["phase"],"draining")
        self.assertIsNone(self.a.audit()["batches"][0]["result"])
        for p,q in request.items():
            self.assertEqual(self.a.submit(p,q)["op"],"queued")
        with self.assertRaises(Rejected):
            self.a.begin_probe()

    def test_batched_seal_cannot_bypass_withdrawn_consent(self):
        runtime=self.start()
        inputs=list(commands(self.a,1,1).items())
        self.a.configure(choices(a=96,b=0,ra=3,rb=3))
        before=self.a.audit()
        with self.assertRaisesRegex(Rejected,"contribution scope fenced"):
            runtime.advance(inputs=inputs)
        self.assertEqual(self.a.audit(),before)
        self.assertTrue(runtime.runtime.frozen)

    def test_batched_inputs_survive_withdrawal_before_publication(self):
        runtime=self.start()
        inputs=list(commands(self.a,1,1).items())
        before=self.a.save()
        def withdraw():
            other=RecoveryAuthority(self.path)
            try:
                other.configure(choices(a=96,b=0,ra=3,rb=3))
            finally:
                other.close()
        with self.assertRaisesRegex(Rejected,"contribution scope fenced"):
            runtime.advance(inputs=inputs,before_publish=withdraw)
        self.assertEqual(self.a.save(),before)
        self.assertEqual(len(self.a.audit()["commands"]),len(inputs))
        self.assertIsNone(self.a.audit()["batches"][0]["result"])
        with self.assertRaises(Rejected):
            self.a.committed_receipts(inputs)
        self.assertTrue(runtime.runtime.frozen)

    def test_fence_after_staged_candidate_even_before_rotation(self):
        runtime=self.start()
        c=runtime.runtime.coordinator
        c.step(controls=runtime.seal())
        receipts=candidate_receipts(c)
        before=self.a.save()
        state=self.a.status()
        self.a.fence(state["scope"],state["generation"],"worker_loss",[1])
        with self.assertRaisesRegex(Rejected,"contribution scope fenced"):
            self.a.commit(receipts)
        with self.assertRaises(Rejected):
            runtime.seal()
        self.assertEqual(self.a.save(),before)
        with self.assertRaises(Rejected):
            self.a.release(state["scope"],state["generation"],[0])
        self.a.release(state["scope"],state["generation"],[0,1])
        with self.assertRaises(Rejected):
            self.a.commit(receipts)
        self.assertEqual(self.a.status()["blocked"],[1])

    def test_failed_worker_revision_persists_and_insufficient_capacity_pauses(self):
        self.start()
        self.drain("partition",[1])
        before=self.a.status()
        self.a.close()
        self.a=RecoveryAuthority(self.path)
        self.assertEqual(self.a.status(),before)
        self.a.configure(choices())
        self.assertEqual(self.a.status()["blocked"],[1])
        status,samples,now=self.probe()
        self.assertEqual(list(samples),[0])
        result=self.a.admit(status["scope"],samples,[0],now)
        self.assertIsNone(result["plan"])
        self.assertEqual(self.a.status()["generation"],before["generation"])
        self.a.release(status["scope"],status["generation"],[0])
        self.a.configure(choices(a=96,b=56,ra=3,rb=2))
        self.assertEqual(self.a.status()["blocked"],[1])
        self.a.configure(choices(a=96,b=56,ra=3,rb=3))
        self.assertEqual(self.a.status()["blocked"],[])

    def test_unselected_probe_must_be_retired_before_admission(self):
        self.a.configure(choices(a=96,b=96))
        status,samples,now=self.probe()
        samples[1]=Sample(1,2,1000,88*MIB,20_000,now,True)
        with self.assertRaisesRegex(Rejected,"unselected probe not retired"):
            self.a.admit(status["scope"],samples,[],now)
        self.assertEqual(self.a.status(),status)
        decision=self.a.admit(status["scope"],samples,[1],now)
        self.assertEqual(set(decision["plan"]["owner_by_region"]),{0})

    def test_faulted_scope_reopens_draining_without_auto_start(self):
        runtime=self.start()
        runtime.advance()
        status=self.a.status()
        self.a.fence(status["scope"],status["generation"],"policy")
        durable=self.a.status()
        self.a.close()
        self.a=RecoveryAuthority(self.path)
        self.assertEqual(self.a.status(),durable)
        with self.assertRaises(Rejected):
            self.a.begin_probe()
        with self.assertRaises(Rejected):
            self.a.release(status["scope"]+1,status["generation"],status["probes"])

    def test_fenced_worker_only_probes_before_grant(self):
        worker=FencedWorker()
        self.assertEqual(worker.handle({"op":"ping"}),{"op":"pong"})
        self.assertEqual(worker.handle({"op":"status"}),{"op":"status","tick":0,"phase":"new"})
        with self.assertRaises(Rejected):
            worker.handle({"op":"init","version":1,"regions":[]})
        with self.assertRaises(Rejected):
            worker.handle({"op":"fenced_snapshot"})


class EvidenceTests(unittest.TestCase):
    def test_retained_evidence_and_mutations(self):
        evidence=Path(__file__).parent/"evidence"
        for n in (1,2):
            data=json.loads((evidence/f"2026-11-26-recovery-{n}.json").read_text())
            verify(data)
            for mutate in (
                lambda d:d["stages"][0]["cleanup"][1].update(populated=1),
                lambda d:d["stages"][3]["ticks"].append({"tick":4}),
                lambda d:d["stages"][4]["probes"]["0"]["kernel"].update(kernel_memory_max_bytes=0),
                lambda d:d["stages"][0].update(withdrawal_workers_alive=False),
                lambda d:d["faults"][0].update(last_tick=4),
                lambda d:d["observations"][2]["journal"]["batches"][1].update(result="0"*64),
                lambda d:d["clients"][4]["sessions"][0]["observed"].update(reply={"op":"committed"}),
                lambda d:d["events"][4].update(phase="running"),
                lambda d:d["pauses"][1].update(spawned_slots=[0]),
                lambda d:d["final"].update(tick=7),
            ):
                altered=copy.deepcopy(data)
                mutate(altered)
                with self.assertRaises(Rejected):
                    verify(altered)


if __name__=="__main__":
    unittest.main()
