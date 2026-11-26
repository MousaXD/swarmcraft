"""Signed role proofs and durable local lifecycle regressions."""
import os
import signal
import socket
from pathlib import Path
import tempfile
import time
import unittest

from combined_ownerless_lab import (CertifiedClientGate, FULL, Quorum, artifact_manifest,
                                    combined_authority, observed_snapshot)
from managed_certification import expected_head, resume_managed_certification
from managed_ownerless import certify_managed_checkpoint, managed_authority, restore_managed_staging
from ownerless_permission import PolicyChannel, PolicyWorker, parts
from player_runtime import fixture
from recovery_runtime import RecoveryAuthority
from region_runtime import Rejected, canonical
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

    def restore(self,name,pin=None,**kwargs):
        return restore_managed_staging(self.q,(3,4,None,None),self.path,self.root/name,
            expected_pin=self.a.restoration_pin() if pin is None else pin,
            source_retired=True,**kwargs)

    def test_revoked_sealed_input_cannot_resurrect_after_reinvite(self):
        self.a.configure(offers())
        runtime = self.start()
        gate = CertifiedClientGate(self.a,self.q,[1])
        request = self.command("a")
        gate.submit(1,request)
        controls = runtime.seal()
        runtime.runtime.coordinator.step(controls=controls)
        stale = candidate_receipts(runtime.runtime.coordinator)
        before = self.a.save()
        self.q.player("revoke_player",5,False)
        with self.assertRaisesRegex(Rejected,"managed certified head changed"):
            self.a.commit(stale)
        self.assertEqual(self.a.save(),before)
        self.drain()
        with self.assertRaisesRegex(Rejected,"no current private invitation"):
            self.a.begin_probe()
        self.q.player("reinvite_player",5)
        with self.assertRaisesRegex(Rejected,"invitation was revoked after queueing"):
            self.a.begin_probe()
        with self.assertRaisesRegex(Rejected,"invitation was revoked after queueing"):
            self.a.submit(1,request)
        self.assertIsNone(self.a.audit()["batches"][0]["result"])
        self.assertEqual(self.a.status()["phase"],"paused")
        old = self.a.restoration_pin()
        original = self.path.read_bytes()
        result = self.restore("restored.db")
        self.assertEqual(result["restored_tick"],0)
        self.assertEqual(result["discarded_ticks"],0)
        self.assertEqual(result["abandoned_request_ids"],[request["id"]])
        self.assertEqual(self.path.read_bytes(),original)
        restored = self.Authority(self.root/"restored.db")
        try:
            self.assertEqual(restored.status()["offers"],old["offers"])
            self.assertEqual(restored.status()["scope"],old["scope"])
            self.assertEqual(restored.audit()["commands"],[])
            self.assertEqual(restored.audit()["batches"],[])
            self.assertEqual(canonical(observed_snapshot(restored,self.q.mapping)),self.q.checkpoint.read_bytes())
            with self.assertRaisesRegex(Rejected,"abandoned managed command ID"):
                restored.submit(1,request)
            with self.assertRaisesRegex(Rejected,"no released managed staged cut"):
                restored.certification_candidate()
            with self.assertRaises(Rejected):
                restored.claim(result["quarantine_generation"],0,before["runtime_root"])
            self.assertEqual(restored.events()[:-1],self.a.events())
            self.assertEqual(restored.events()[-1]["event"],"certified_restore_quarantine")
            # Fresh commands need new authorization and a fresh scope.
            replacement = {**request,"id":"b"*64,"direction":-1}
            self.assertEqual(restored.submit(1,replacement)["op"],"queued")
            self.assertEqual(restored.db.execute("SELECT COUNT(*) FROM command_authorizations").fetchone()[0],1)
        finally:
            restored.close()

    def test_restore_abandoned_advance_preserves_latest_consent_and_blocks(self):
        self.a.configure(offers())
        runtime = self.start()
        first = self.command("a")
        first_gate = CertifiedClientGate(self.a,self.q,[1])
        self.a.submit(1,first)
        runtime.advance()
        self.drain()
        certify_managed_checkpoint(self.a,self.q,first_gate,"first")
        before = self.a.save()
        runtime = self.start()
        second = self.command("b")
        self.a.submit(1,second)
        runtime.advance()
        status = self.a.status()
        self.a.fence(status["scope"],status["generation"],"policy",[0])
        self.drain()
        # A later consent change of a different slot must not erase the
        # original worker's blocked revision on the restored database.
        self.a.configure([Offer(0,2,80,750),Offer(1,3,0,0),Offer(2,1,0,0),Offer(3,1,0,0)])
        pin = self.a.restoration_pin()
        self.assertEqual(pin["blocked"],[0])
        stale = {**pin,"metadata_sha256":"0"*64}
        with self.assertRaisesRegex(Rejected,"consent or journal pin changed"):
            self.restore("wrong.db",stale)
        self.assertFalse((self.root/"wrong.db").exists())
        original = self.path.read_bytes()
        result = self.restore("restored.db",pin)
        self.assertEqual(result["restored_tick"],1)
        self.assertEqual(result["discarded_ticks"],1)
        self.assertEqual(result["abandoned_request_ids"],[second["id"]])
        self.assertEqual(self.path.read_bytes(),original)
        restored = self.Authority(self.root/"restored.db")
        try:
            self.assertEqual(restored.status()["blocked"],[0])
            self.assertEqual(restored.status()["offers"],pin["offers"])
            self.assertEqual(restored.status()["scope"],pin["scope"])
            self.assertEqual(restored.save()["runtime_root"],before["runtime_root"])
            self.assertEqual(restored.begin_probe(),[])
            gate = CertifiedClientGate(restored,self.q,[1])
            self.assertEqual(gate.committed_receipts([(1,first)])[0]["view"]["tick"],1)
            with self.assertRaisesRegex(Rejected,"abandoned managed command ID"):
                restored.submit(1,second)
            with self.assertRaisesRegex(Rejected,"consent changed before certification|staged cut differs"):
                restored.certification_candidate()
            # A newer revision clears only its own failed-worker block.
            restored.configure(offers(4))
            self.assertEqual(restored.status()["blocked"],[])
            pos = restored.begin_probe()
            self.assertEqual([o.slot for o in pos],[0])
            self.assertEqual(restored.status()["scope"],pin["scope"]+1)
        finally:
            restored.close()

    def test_batched_invalid_invitation_and_corrupt_authorization_refuse(self):
        self.a.configure(offers())
        runtime = self.start()
        request = self.command("a")
        other = {**request,"id":"b"*64}
        before = self.a.audit()
        # Player zero has no invitation; the fused write rolls back
        # even the eligible first request and its provenance row.
        with self.assertRaisesRegex(Rejected,"no current private invitation"):
            runtime.advance(inputs=[(1,request),(0,other)])
        self.assertEqual(self.a.audit(),before)
        self.assertEqual(self.a.db.execute("SELECT COUNT(*) FROM command_authorizations").fetchone()[0],0)
        self.a.submit(1,request)
        with self.a.transaction(write=True):
            self.a.db.execute("DELETE FROM command_authorizations")
        with self.assertRaisesRegex(Rejected,"orphaned managed command authorization"):
            self.Authority(self.path)

    def staged(self):
        self.q.depart()
        self.q.certify("recover",["--recover"])
        self.a.configure(offers())
        runtime = self.start()
        req = self.command("a")
        gate = CertifiedClientGate(self.a,self.q,[1])
        gate.submit(1,req)
        runtime.advance()
        self.drain()
        return gate,req

    def interrupt(self,phase):
        # The child owns a freshly opened SQLite connection; parent
        # controller memory and fixture head never observe its progress.
        self.a.close()
        pid = os.fork()
        if pid==0:
            try:
                ledger = self.Authority(self.path)
                gate = CertifiedClientGate(ledger,self.q,[1])
                def kill():
                    os.kill(os.getpid(),signal.SIGKILL)
                certify_managed_checkpoint(ledger,self.q,gate,"interrupted",
                    **({"before_retry":kill} if phase=="vote" else {"before_sync":kill}))
            except BaseException:
                import traceback
                traceback.print_exc()
            os._exit(1)
        _,status = os.waitpid(pid,0)
        self.a = self.Authority(self.path)
        self.assertTrue(os.WIFSIGNALED(status),status)
        self.assertEqual(os.WTERMSIG(status),signal.SIGKILL)
        raw,done = self.a.db.execute("SELECT plan,completion FROM managed_certifications").fetchone()
        self.assertIsNone(done)
        import json
        return json.loads(raw)

    def test_pending_vote_survives_process_kill_and_exact_retry(self):
        gate,req = self.staged()
        plan = self.interrupt("vote")
        target = plan["proposal"]["proposal_hash"]
        before = [self.q.auth(i,"status","--expected-world",self.q.world) for i in self.q.active]
        self.assertEqual([s["pending_proposal"] for s in before],[target,None])
        cold = CertifiedClientGate(self.a,self.q,[1])
        with self.assertRaisesRegex(Rejected,"reconciliation incomplete"):
            cold.committed_receipts([(1,req)])
        with self.assertRaisesRegex(Rejected,"reconciliation incomplete"):
            self.a.prepare_certification("replacement")
        with self.assertRaisesRegex(Rejected,"target pin differs"):
            resume_managed_certification(self.a,self.q,expected_target="0"*64)
        with self.assertRaisesRegex(Rejected,"new votes forbidden"):
            resume_managed_certification(self.a,self.q,expected_target=target,recover_only=True)
        self.assertEqual(before,[self.q.auth(i,"status","--expected-world",self.q.world) for i in self.q.active])
        result = resume_managed_certification(self.a,self.q,expected_target=target)
        self.assertEqual(result["mode"],"retry_exact_vote")
        self.assertEqual(result["head"],expected_head(plan))
        self.assertEqual(result["before"],before)
        fresh = CertifiedClientGate(self.a,self.q,[1])
        receipt, = fresh.committed_receipts([(1,req)])
        self.assertEqual(receipt["view"]["tick"],1)
        self.a.close()
        self.a = self.Authority(self.path)
        again = resume_managed_certification(self.a,self.q,expected_target=target)
        self.assertEqual(again["mode"],"already_reconciled")
        self.assertEqual(again["completion"],result["completion"])
        self.assertEqual(CertifiedClientGate(self.a,self.q,[1]).committed_receipts([(1,req)]),[receipt])

    def test_accepted_certificate_survives_kill_and_withdrawal_without_new_votes(self):
        gate,req = self.staged()
        plan = self.interrupt("accepted")
        target = plan["proposal"]["proposal_hash"]
        before = [self.q.auth(i,"status","--expected-world",self.q.world) for i in self.q.active]
        self.assertEqual([s["head"] for s in before],[target,plan["base"]["head"]])
        self.assertEqual(before[1]["pending_proposal"],target)
        self.a.configure(offers(3,0,0))
        self.assertFalse(self.a.certification_intent(target)[2])
        # Even a controller that knows the accepted target cannot expose
        # the staged result while the local dissemination barrier is open.
        self.q.head = expected_head(plan)
        with self.assertRaisesRegex(Rejected,"reconciliation incomplete"):
            CertifiedClientGate(self.a,self.q,[1]).committed_receipts([(1,req)])
        self.q.head = plan["base"]
        def blocked():
            with self.assertRaisesRegex(Rejected,"reconciliation incomplete"):
                CertifiedClientGate(self.a,self.q,[1]).committed_receipts([(1,req)])
        result = resume_managed_certification(self.a,self.q,expected_target=target,before_sync=blocked)
        self.assertEqual(result["mode"],"accepted_certificate")
        self.assertEqual(result["before"],before)
        self.assertEqual(self.a.status()["offers"][0]["process_memory_mib"],0)
        self.assertEqual(self.a.begin_probe(),[])
        receipt, = CertifiedClientGate(self.a,self.q,[1]).committed_receipts([(1,req)])
        self.assertEqual(receipt["view"]["tick"],1)
        self.a.close()
        self.a = self.Authority(self.path)
        self.assertEqual(CertifiedClientGate(self.a,self.q,[1]).committed_receipts([(1,req)]),[receipt])

    def test_withdrawn_minority_cannot_cast_new_vote_or_restore(self):
        gate,req = self.staged()
        plan = self.interrupt("vote")
        target = plan["proposal"]["proposal_hash"]
        self.a.configure(offers(3,0,0))
        before = [self.q.auth(i,"status","--expected-world",self.q.world) for i in self.q.active]
        with self.assertRaisesRegex(Rejected,"new votes forbidden"):
            resume_managed_certification(self.a,self.q,expected_target=target)
        self.assertEqual(before,[self.q.auth(i,"status","--expected-world",self.q.world) for i in self.q.active])
        with self.assertRaisesRegex(Rejected,"reconciliation incomplete|pending"):
            self.restore("forbidden.db")
        self.assertFalse((self.root/"forbidden.db").exists())
        self.assertIsNone(self.a.certification_intent(target)[1])
        self.assertEqual(self.a.begin_probe(),[])

    def test_existing_majority_recovers_after_withdrawal_without_new_votes(self):
        gate,req = self.staged()
        plan = self.interrupt("vote")
        target = plan["proposal"]["proposal_hash"]
        # Persist a second matching reviewed vote before withdrawal.
        p = self.q.write_json("existing-remote-proposal.json",plan["proposal"])
        c = self.q.write_json("existing-remote-candidate.bin",plan["candidate"])
        self.q.auth(2,"vote","--expected-world",self.q.world,
                    "--expected-head",plan["base"]["head"],"--proposal",p,
                    "--expected-proposal",target,"--checkpoint",c,"--confirm")
        self.a.configure(offers(3,0,0))
        before = [self.q.auth(i,"status","--expected-world",self.q.world) for i in self.q.active]
        self.assertEqual([s["pending_proposal"] for s in before],[target,target])
        result = resume_managed_certification(self.a,self.q,expected_target=target,recover_only=True)
        self.assertEqual(result["mode"],"recover_existing_evidence")
        self.assertEqual(result["before"],before)
        self.assertEqual(self.a.begin_probe(),[])
        self.assertEqual(CertifiedClientGate(self.a,self.q,[1]).committed_receipts([(1,req)])[0]["view"]["tick"],1)


if __name__=="__main__":
    unittest.main()
