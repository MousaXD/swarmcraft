"""Non-destructive reference staging restore against actual signed CLI journals."""
import os
from pathlib import Path
import signal
import sqlite3
import tempfile
import unittest

from combined_ownerless_lab import (CertifiedClientGate, FULL, Quorum, artifact_manifest,
                                    combined_authority, observed_snapshot, restore_certified_staging,
                                    restoration_source_pin)
from ownerless_permission import PolicyChannel, PolicyWorker, parts
from player_runtime import fixture
from region_runtime import Rejected, canonical
from regional_authority import candidate_receipts
from test_region_runtime import MemoryChannel
from transition_runtime import DurableRuntime


def runtime_for(ledger, binding):
    channel = MemoryChannel()
    channel.worker = PolicyWorker(binding)
    return DurableRuntime(ledger,[PolicyChannel(channel,binding)])


def command(ledger, token, direction=1):
    base = ledger.save()
    return {"op":"act","world":base["world"],"id":token*64,"tick":base["tick"]+1,
            "parent":base["cut_root"],"direction":direction,"action":direction}


@unittest.skipUnless(os.environ.get("SWARMCRAFT_TEST_CLI"),
                     "set SWARMCRAFT_TEST_CLI to exercise signed restore")
class CertifiedStagingRestoreTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="sc68-restore-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        cli = Path(os.environ["SWARMCRAFT_TEST_CLI"]).resolve(strict=True)
        self.q = Quorum(cli,self.root,artifact_manifest(cli))
        self.q.player("invite_survivor",5)
        self.Authority = combined_authority(self.q.binding)

    def stage(self, name, *, certified=False, direction=1):
        path = self.root/name
        ledger = self.Authority.create(path,parts(self.q.binding)[0],fixture(),[0,1],[FULL])
        try:
            runtime = runtime_for(ledger,self.q.binding)
            first = command(ledger,"a",direction)
            ledger.submit(1,first)
            runtime.advance()
            if certified:
                self.q.certify("first_checkpoint",["checkpoint"],observed_snapshot(ledger,self.q.mapping))
            receipt, = ledger.committed_receipts([(1,first)])
            second = command(ledger,"b")
            ledger.submit(1,second)
            runtime.advance()
            stale = candidate_receipts(runtime.runtime.coordinator)
            # Also leave unsealed, unacknowledged input beyond the staged
            # head, and rotate to a later already-consumed generation.
            pending = command(ledger,"c")
            ledger.submit(1,pending)
            old = ledger.save()
            ledger.rotate(old["generation"],old["tick"],old["runtime_root"],[FULL])
            retired = runtime_for(ledger,self.q.binding)
            retired.runtime.frozen = retired.runtime.coordinator.poisoned = True
            runtime.runtime.frozen = runtime.runtime.coordinator.poisoned = True
            return path, restoration_source_pin(ledger.save()), receipt, second, pending, stale, ledger.history()
        finally:
            ledger.close()

    def restore(self,path,pin,name="restored.db",**kwargs):
        return restore_certified_staging(self.q,path,self.root/name,expected_source=pin,
                                        source_retired=True,**kwargs)

    def test_restore_exact_prefix_and_resume_with_new_generation(self):
        self.q.worker("allow_worker",3)
        source,pin,receipt,second,pending,stale,history = self.stage("staged.db",certified=True)
        original = source.read_bytes()
        checkpoint = self.q.checkpoint.read_bytes()
        result = self.restore(source,pin)
        self.assertEqual(result["restored_tick"],1)
        self.assertEqual(result["discarded_ticks"],1)
        self.assertEqual(result["quarantine_generation"],pin["generation"]+1)
        self.assertTrue(result["rotation_required"])
        for key in ("runtime_activation","write_authority","canonical_authority",
                    "live_quorum_verified","worker_admission","player_admission"):
            self.assertIs(result[key],False)
        self.assertEqual(source.read_bytes(),original)
        self.assertEqual((self.root/"restored.db").stat().st_mode & 0o777,0o600)
        recovered = self.Authority(self.root/"restored.db")
        try:
            self.assertEqual(canonical(observed_snapshot(recovered,self.q.mapping)),checkpoint)
            restored_history = recovered.history()
            self.assertEqual(restored_history["commits"],history["commits"][:2])
            self.assertEqual(restored_history["epochs"][:-1],history["epochs"][:1])
            self.assertEqual(restored_history["epochs"][-1][0],pin["generation"]+1)
            self.assertTrue(restored_history["epochs"][-1][-1])
            gate = CertifiedClientGate(recovered,self.q,[1])
            first = recovered.audit()["commands"][0]["request"]
            self.assertEqual(gate.committed_receipts([(1,first)]),[receipt])
            self.assertEqual(gate.view(1),receipt["view"])
            for request in (second,pending):
                with self.assertRaisesRegex(Rejected,"no exact certified command receipt"):
                    gate.committed_receipts([(1,request)])
            # No old session or quarantined generation can be used for
            # a new worker runtime. A fresh explicit rotation is needed.
            before = recovered.save()
            with self.assertRaisesRegex(Rejected,"generation already claimed"):
                runtime_for(recovered,self.q.binding)
            with self.assertRaisesRegex(Rejected,"stale ownerless receipt session"):
                recovered.commit(stale)
            self.assertEqual(recovered.save(),before)
            self.q.role("fresh_player",5)
            self.q.role("fresh_worker",3,750)
            recovered.rotate(before["generation"],before["tick"],before["runtime_root"],[FULL])
            fresh = runtime_for(recovered,self.q.binding)
            replacement = command(recovered,"d",-1)
            self.assertEqual(gate.submit(1,replacement)["op"],"queued")
            fresh.advance()
            with self.assertRaisesRegex(Rejected,"uncertified staged tick"):
                gate.committed_receipts([(1,replacement)])
            accepted = self.q.certify("replacement_checkpoint",["checkpoint"],
                                      observed_snapshot(recovered,self.q.mapping))
            gate.publish(accepted)
            result, = gate.committed_receipts([(1,replacement)])
            self.assertEqual(result["view"]["tick"],2)
            self.assertNotEqual(replacement,second)
            with self.assertRaisesRegex(Rejected,"no exact certified command receipt"):
                gate.committed_receipts([(1,second)])
            self.assertEqual(gate.committed_receipts([(1,first)]),[receipt])
        finally:
            recovered.close()
        self.assertEqual(source.read_bytes(),original)
        # A newer certified history may differ from the abandoned local
        # branch at the same tick; never manufacture a matching prefix.
        with self.assertRaisesRegex(Rejected,"certified checkpoint differs from audited source prefix"):
            self.restore(source,pin,"diverged.db")
        self.assertFalse((self.root/"diverged.db").exists())

    def test_killed_restore_and_no_clobber_from_genesis(self):
        source,pin,_,_,_,_,_ = self.stage("staged.db")
        original = source.read_bytes()
        dest = self.root/"restored.db"
        with self.assertRaisesRegex(Rejected,"explicitly retired"):
            restore_certified_staging(self.q,source,dest,expected_source=pin,source_retired=False)
        with self.assertRaisesRegex(Rejected,"source pin changed"):
            self.restore(source,{**pin,"generation":pin["generation"]-1})
        with self.assertRaisesRegex(Rejected,"destination or sidecar already exists"):
            self.restore(source,pin,source.name)
        (self.root/"restored.db-journal").write_bytes(b"preserve me")
        with self.assertRaisesRegex(Rejected,"destination or sidecar already exists"):
            self.restore(source,pin)
        self.assertEqual((self.root/"restored.db-journal").read_bytes(),b"preserve me")
        (self.root/"restored.db-journal").unlink()
        # Real process interruption after building/fsyncing the private
        # database, before publishing a destination link.
        pid = os.fork()
        if pid == 0:
            try:
                self.restore(source,pin,before_publish=lambda:os.kill(os.getpid(),signal.SIGKILL))
            except BaseException:
                os._exit(2)
            os._exit(3)
        _, status = os.waitpid(pid,0)
        self.assertTrue(os.WIFSIGNALED(status))
        self.assertEqual(os.WTERMSIG(status),signal.SIGKILL)
        self.assertFalse(dest.exists())
        self.assertEqual(source.read_bytes(),original)
        result = self.restore(source,pin)
        self.assertEqual(result["restored_tick"],0)
        self.assertEqual(result["discarded_ticks"],2)
        restored = self.Authority(dest,read_only=True)
        try:
            self.assertEqual(canonical(observed_snapshot(restored,self.q.mapping)),self.q.checkpoint.read_bytes())
            self.assertEqual(len(restored.history()["commits"]),1)
            self.assertEqual(len(restored.history()["epochs"]),1)
            save = restored.save()
            with self.assertRaisesRegex(Rejected,"read-only authority inspection"):
                restored.rotate(save["generation"],save["tick"],save["runtime_root"],[FULL])
            with self.assertRaises(sqlite3.OperationalError):
                restored.db.execute("DELETE FROM world")
        finally:
            restored.close()
        durable = dest.read_bytes()
        with self.assertRaisesRegex(Rejected,"destination or sidecar already exists"):
            self.restore(source,pin)
        self.assertEqual(dest.read_bytes(),durable)
        self.assertEqual(source.read_bytes(),original)

    def test_pending_vote_and_authority_advance_refuse_publication(self):
        source,pin,_,_,_,_,_ = self.stage("staged.db")
        original = source.read_bytes()
        # A different certified governance head during restoration must
        # abort publication even when it retains identical checkpoint bytes.
        with self.assertRaisesRegex(Rejected,"restoration authority pin, identity or pending vote differs"):
            self.restore(source,pin,before_publish=lambda:self.q.certify("advance",["--recover"]))
        self.assertFalse((self.root/"restored.db").exists())
        proposal = self.q.proposal(["--recover"])
        p = self.q.write_json("restore-pending.json",proposal)
        self.q.auth(1,"vote",*self.q.context(),"--proposal",p,"--expected-proposal",proposal["proposal_hash"],
                    "--checkpoint",self.q.checkpoint,"--confirm")
        before = self.q.auth(1,"status","--expected-world",self.q.world)
        with self.assertRaisesRegex(Rejected,"restoration authority pin, identity or pending vote differs"):
            self.restore(source,pin)
        self.assertEqual(self.q.auth(1,"status","--expected-world",self.q.world),before)
        self.assertFalse((self.root/"restored.db").exists())
        self.assertEqual(source.read_bytes(),original)


if __name__ == "__main__":
    unittest.main()
