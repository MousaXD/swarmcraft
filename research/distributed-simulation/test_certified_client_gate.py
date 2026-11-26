"""Reference publication boundary with real signed CLI journals.

Set SWARMCRAFT_TEST_CLI to an already built swarmcraft binary. Workers
in these tests are deterministic in-process fixtures; the separate
combined lab supplies real TCP/process and kernel scope evidence.
"""
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock

from combined_ownerless_lab import (CertifiedClientGate, Quorum, artifact_manifest,
                                    combined_authority, observed_snapshot, FULL)
from ownerless_permission import PolicyChannel, PolicyWorker, parts
from player_runtime import fixture
from region_runtime import Rejected
from test_region_runtime import MemoryChannel
from transition_lab import Clients
from transition_runtime import DurableRuntime


class ReceiptSendTests(unittest.TestCase):
    def test_already_committed_replay_is_rechecked_before_send(self):
        # No socket or process is needed to exercise the send boundary:
        # a stale cached receipt must not reach the channel.
        authority = Mock()
        authority.committed_receipts.side_effect = Rejected("retired reference cut")
        channel = Mock()
        request = {"id":"a"*64}
        clients = Clients.__new__(Clients)
        clients.authority = authority
        clients.sessions = [{"channel":channel,"player":1,"request":request,
                             "submission":{"op":"committed","id":request["id"]}}]
        with self.assertRaisesRegex(Rejected,"retired reference cut"):
            clients.finish()
        authority.committed_receipts.assert_called_once_with([(1,request)])
        channel.send.assert_not_called()
        authority.submit.assert_not_called()


@unittest.skipUnless(os.environ.get("SWARMCRAFT_TEST_CLI"),
                     "set SWARMCRAFT_TEST_CLI to exercise actual signed journals")
class CertifiedGateTests(unittest.TestCase):
    def test_uncertified_restart_exact_receipt_replay_and_revocation(self):
        cli = Path(os.environ["SWARMCRAFT_TEST_CLI"]).resolve(strict=True)
        with tempfile.TemporaryDirectory(prefix="sc68-gate-test-") as temp:
            root = Path(temp)
            q = Quorum(cli,root,artifact_manifest(cli))
            q.player("invite_survivor",5)
            # A role check is separate from the checkpoint gate. The
            # synthetic player slot is mapped by our trusted fixture.
            q.role("player-proof",5)
            Authority = combined_authority(q.binding)
            path = root/"staged.db"
            ledger = Authority.create(path,parts(q.binding)[0],fixture(),[0,1],[FULL])
            try:
                gate = CertifiedClientGate(ledger,q,[1])
                joined = gate.view(1)
                base = ledger.save()
                for players in ([True],[1,1],[],[2]):
                    with self.assertRaises(Rejected):
                        CertifiedClientGate(ledger,q,players)
                with self.assertRaises(Rejected):
                    gate.view(0)
                req = {"op":"act","world":base["world"],"id":"a"*64,"tick":1,
                       "parent":base["cut_root"],"direction":1,"action":1}
                self.assertEqual(gate.submit(1,req)["op"],"queued")
                self.assertEqual(gate.view(1),joined)
                q.assert_state(observed_snapshot(ledger,q.mapping))
                before = ledger.audit()
                with self.assertRaisesRegex(Rejected,"no exact certified command receipt"):
                    gate.committed_receipts([(1,req)])
                self.assertEqual(ledger.audit(),before)
                channel = MemoryChannel()
                channel.worker = PolicyWorker(q.binding)
                runtime = DurableRuntime(ledger,[PolicyChannel(channel,q.binding)])
                runtime.advance()
                for attempt in (lambda:gate.view(1),lambda:gate.submit(1,req),
                                lambda:gate.committed_receipts([(1,req)])):
                    with self.assertRaisesRegex(Rejected,"uncertified staged tick"):
                        attempt()
                staged = ledger.save()
                checkpoint = observed_snapshot(ledger,q.mapping)
                # A restart cannot treat an independently committed SQL
                # transaction as a certified checkpoint.
                ledger.close()
                ledger = Authority(path)
                restarted = CertifiedClientGate(ledger,q,[1])
                for attempt in (lambda:restarted.view(1),lambda:restarted.submit(1,req),
                                lambda:restarted.committed_receipts([(1,req)])):
                    with self.assertRaisesRegex(Rejected,"checkpoint bytes do not match commitment"):
                        attempt()
                self.assertEqual(ledger.save(),staged)
                accepted = q.certify("certify_staged",["checkpoint"],checkpoint)
                # Reopen after certificate acceptance but before any
                # client acknowledgment. Recover exact prior result.
                ledger.close()
                ledger = Authority(path)
                restarted = CertifiedClientGate(ledger,q,[1])
                self.assertEqual(restarted.head,accepted)
                receipt, = restarted.committed_receipts([(1,req)])
                self.assertEqual(receipt["op"],"committed")
                self.assertEqual(receipt["view"],restarted.view(1))
                self.assertEqual(restarted.submit(1,req),receipt)
                before = ledger.audit()
                for bad in ({**req,"id":"b"*64},{**req,"action":-1},
                            {**req,"tick":2,"parent":staged["cut_root"]}):
                    with self.assertRaisesRegex(Rejected,"no exact certified command receipt"):
                        restarted.committed_receipts([(1,bad)])
                self.assertEqual(ledger.audit(),before)
                q.player("revoke_survivor",5,False)
                for attempt in (lambda:restarted.view(1),lambda:restarted.submit(1,req),
                                lambda:restarted.committed_receipts([(1,req)])):
                    with self.assertRaisesRegex(Rejected,"differs from expected pin"):
                        attempt()
                denied = CertifiedClientGate(ledger,q,[1])
                with self.assertRaisesRegex(Rejected,"no accepted private invitation"):
                    denied.committed_receipts([(1,req)])
                self.assertEqual(ledger.audit(),before)
                self.assertEqual(ledger.save(),staged)
                for profile in q.profiles:
                    self.assertFalse((profile/"control").exists())
                    self.assertFalse((profile/"runtime").exists())
                    self.assertFalse(any((profile/"worlds").iterdir()))
            finally:
                ledger.close()


if __name__ == "__main__":
    unittest.main()
