"""Signed role proofs and durable local lifecycle regressions."""
import os
from pathlib import Path
import tempfile
import time
import unittest

from combined_ownerless_lab import CertifiedClientGate, FULL, Quorum, artifact_manifest, combined_authority
from managed_ownerless import managed_authority
from ownerless_permission import PolicyChannel, PolicyWorker, parts
from player_runtime import fixture
from recovery_runtime import RecoveryAuthority
from region_runtime import Rejected
from regional_authority import candidate_receipts
from test_region_runtime import MemoryChannel
from transition_runtime import DurableRuntime
from worker_placement import MIB, MAX_SAMPLE_AGE_NS, Offer, Sample


def offers(revision=2,memory=80,cpu=750,extra=0):
    return [Offer(0,revision,memory,cpu),Offer(1,revision,extra,750 if extra else 0),
            Offer(2,1,0,0),Offer(3,1,0,0)]


@unittest.skipUnless(os.environ.get("SWARMCRAFT_TEST_CLI"),
                     "set SWARMCRAFT_TEST_CLI for signed managed lifecycle")
class ManagedTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="sc68-managed-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        cli = Path(os.environ["SWARMCRAFT_TEST_CLI"]).resolve(strict=True)
        self.q = Quorum(cli,self.root,artifact_manifest(cli))
        self.q.player("invite",5)
        self.q.worker("allow",3)
        self.Authority = managed_authority(self.q,(3,4,None,None))
        self.path = self.root/"managed.db"
        self.a = self.Authority.create(self.path,parts(self.q.binding)[0],fixture(),[0,1],[FULL])
        self.addCleanup(lambda:self.a.close())

    def probe(self):
        positive = self.a.begin_probe()
        now = time.monotonic_ns()
        samples = {o.slot:Sample(o.slot,o.revision,o.cpu_millicores,
                                (o.process_memory_mib-8)*MIB,100,now,True) for o in positive}
        return self.a.status(),samples,now

    def start(self,guard=lambda state,slots:True):
        status,samples,now = self.probe()
        self.a.admit(status["scope"],samples,[],now)
        self.a.attach_resource_guard(status["scope"],guard)
        channel = MemoryChannel()
        channel.worker = PolicyWorker(self.q.binding)
        return DurableRuntime(self.a,[PolicyChannel(channel,self.q.binding)])

    def drain(self):
        status = self.a.status()
        if status["phase"]!="draining":
            self.a.fence(status["scope"],status["generation"],"stop")
        status = self.a.status()
        self.a.release(status["scope"],status["generation"],status["probes"])

    def command(self,token):
        s = self.a.save()
        return {"op":"act","world":s["world"],"id":token*64,"tick":s["tick"]+1,
                "parent":s["cut_root"],"direction":1,"action":1}

    def test_zero_consent_and_signed_eligibility_before_probe(self):
        before = self.a.save()
        self.assertEqual(self.a.begin_probe(),[])
        with self.assertRaises(Rejected):
            self.a.rotate(before["generation"],before["tick"],before["runtime_root"],[FULL])
        for cls in (combined_authority(self.q.binding),RecoveryAuthority,
                    managed_authority(self.q,(4,3,None,None))):
            with self.assertRaises(Rejected):
                cls(self.path)
        self.a.configure(offers(extra=80))
        with self.assertRaisesRegex(Rejected,"worker has no accepted eligibility"):
            self.a.begin_probe()
        self.assertEqual(self.a.status()["phase"],"paused")
        self.assertEqual(self.a.status()["scope"],0)
        self.assertEqual(self.a.save(),before)

    def test_withdrawal_before_commit_preserves_input_and_restarts_fresh(self):
        self.a.configure(offers())
        runtime = self.start()
        req = self.command("a")
        gate = CertifiedClientGate(self.a,self.q,[1])
        self.assertEqual(gate.submit(1,req)["op"],"queued")
        before = self.a.save()
        def withdraw():
            other = self.Authority(self.path)
            try:
                other.configure(offers(3,0,0))
            finally:
                other.close()
        with self.assertRaisesRegex(Rejected,"contribution scope fenced"):
            runtime.advance(before_publish=withdraw)
        self.assertTrue(runtime.runtime.frozen)
        self.assertEqual(self.a.save(),before)
        self.assertEqual(self.a.status()["phase"],"draining")
        self.assertIsNone(self.a.audit()["batches"][0]["result"])
        with self.assertRaises(Rejected):
            gate.committed_receipts([(1,req)])
        with self.assertRaises(Rejected):
            self.a.begin_probe()
        self.drain()
        self.a.close()
        self.a = self.Authority(self.path)
        self.assertEqual(self.a.begin_probe(),[])
        self.a.configure(offers(4,80,750))
        fresh = self.start()
        self.assertGreater(fresh.runtime.generation,runtime.runtime.generation)
        fresh.advance()
        self.assertEqual(self.a.status()["phase"],"draining")
        self.assertEqual(self.a.audit()["commands"][0]["request"],req)
        with self.assertRaisesRegex(Rejected,"contribution scope fenced"):
            fresh.advance()
        with self.assertRaisesRegex(Rejected,"cleanup not released"):
            self.a.certification_candidate()
        self.drain()
        candidate = self.a.certification_candidate()
        self.assertEqual(candidate["tick"],1)
        # Local consent changes after staging are conservative refusal
        # before network certification; no replay or undo of a vote.
        self.a.configure(offers(5,0,0))
        with self.assertRaisesRegex(Rejected,"consent changed before certification"):
            self.a.certification_candidate()

    def test_ready_restart_and_expired_measurement_cannot_claim(self):
        self.a.configure(offers())
        status,samples,now = self.probe()
        expired = {0:Sample(0,2,750,72*MIB,100,now-MAX_SAMPLE_AGE_NS-1,True)}
        with self.assertRaisesRegex(Rejected,"measurement expired"):
            self.a.admit(status["scope"],expired,[],now)
        self.assertIsNone(self.a.scope_record()["samples"])
        self.a.admit(status["scope"],samples,[],now)
        before = self.a.save()
        with self.assertRaisesRegex(Rejected,"missing managed resource guard"):
            self.a.claim(before["generation"],before["tick"],before["runtime_root"])
        reopened = self.Authority(self.path)
        try:
            with self.assertRaisesRegex(Rejected,"retired controller"):
                reopened.attach_resource_guard(status["scope"],lambda *_:True)
            with self.assertRaisesRegex(Rejected,"retired controller"):
                reopened.claim(before["generation"],before["tick"],before["runtime_root"])
            self.assertEqual(reopened.save(),before)
            reopened.fence(status["scope"],before["generation"],"startup")
            reopened.release(status["scope"],before["generation"],[0])
        finally:
            reopened.close()
        self.assertEqual(self.a.status()["phase"],"paused")

    def test_governance_change_and_resource_guard_refuse_staged_receipts(self):
        self.a.configure(offers())
        live = [True]
        runtime = self.start(lambda state,slots:live[0])
        req = self.command("a")
        self.a.submit(1,req)
        controls = runtime.seal()
        runtime.runtime.coordinator.step(controls=controls)
        receipts = candidate_receipts(runtime.runtime.coordinator)
        before = self.a.save()
        live[0] = False
        with self.assertRaisesRegex(Rejected,"resource guard refused"):
            self.a.commit(receipts)
        self.assertEqual(self.a.save(),before)
        self.assertEqual(self.a.status()["phase"],"draining")
        live[0] = True
        with self.assertRaisesRegex(Rejected,"contribution scope fenced"):
            self.a.commit(receipts)
        self.drain()
        runtime = self.start()
        controls = runtime.seal()
        runtime.runtime.coordinator.step(controls=controls)
        receipts = candidate_receipts(runtime.runtime.coordinator)
        before = self.a.save()
        self.q.worker("revoke",3,True)
        with self.assertRaisesRegex(Rejected,"managed certified head changed"):
            self.a.commit(receipts)
        self.assertEqual(self.a.save(),before)
        self.assertEqual(self.a.status()["phase"],"draining")
        self.drain()
        with self.assertRaisesRegex(Rejected,"worker has no accepted eligibility"):
            self.a.begin_probe()
        self.assertEqual(self.a.status()["phase"],"paused")


if __name__=="__main__":
    unittest.main()
