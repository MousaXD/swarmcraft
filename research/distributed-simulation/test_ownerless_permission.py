import copy
import tempfile
import unittest
from pathlib import Path

from ownerless_permission import POLICY_HASH, PolicyWorker, PolicyChannel, authority_for, parts
from player_runtime import fixture
from region_runtime import Rejected
from regional_authority import Authority, FencedRuntime, candidate_receipts, new_grants
from test_region_runtime import MemoryChannel

BINDING = "sc65v1:"+"a"*64+":"+POLICY_HASH+":"+"b"*64
GROUPS = [[0,1,2,3],[4,5,6,7]]


class OwnerlessTests(unittest.TestCase):
    def test_wrong_pins_and_legacy_grants(self):
        for binding in (None,"",BINDING.replace(POLICY_HASH,"c"*64),BINDING.replace("a"*64,"0"*64)):
            with self.assertRaises(Rejected):
                parts(binding)
        worker = PolicyWorker(BINDING)
        grant = new_grants("a"*64,1,0,"d"*64,GROUPS)[0]
        self.assertEqual(worker.handle({"op":"bind_grant","grant":grant})["op"],"refused")
        self.assertEqual(worker.handle({"op":"bind_policy_grant","grant":grant,
                                       "binding":BINDING[:-1]+"c"})["op"],"refused")
        self.assertIsNone(worker.grant)
        self.assertEqual(worker.handle({"op":"bind_policy_grant","grant":grant,"binding":BINDING})["binding"],BINDING)
        for op in ("op","rcon","console","set_policy","grant_vote","bind_grant"):
            self.assertEqual(worker.handle({"op":op})["op"],"refused")
        self.assertEqual(worker.handle({"op":"bind_policy_grant","grant":grant,"binding":BINDING})["op"],"refused")
        self.assertEqual(worker.grant,grant)

    def test_receipt_pin_and_mac_required_inside_atomic_commit(self):
        with tempfile.TemporaryDirectory() as tmp:
            path=Path(tmp)/"ledger"
            Ledger=authority_for(BINDING)
            ledger=Ledger.create(path,"a"*64,fixture(),[0,1,2],GROUPS)
            try:
                channels=[MemoryChannel(),MemoryChannel()]
                for c in channels:
                    c.worker=PolicyWorker(BINDING)
                runtime=FencedRuntime(ledger,[PolicyChannel(c,BINDING) for c in channels])
                runtime.coordinator.step()
                receipts=candidate_receipts(runtime.coordinator)
                before=ledger.save()
                for mutate in (
                    lambda r:r[0].pop("binding"),
                    lambda r:r[0].update(binding=BINDING[:-1]+"c"),
                    lambda r:r[0].update(policy_mac="0"*64),
                    lambda r:r[0]["body"].update(tick=2),
                    lambda r:r[0].update(policy_mac=r[1]["policy_mac"]),
                ):
                    altered=copy.deepcopy(receipts)
                    mutate(altered)
                    with self.assertRaises(Rejected):
                        ledger.commit(altered)
                    self.assertEqual(ledger.save(),before)
                self.assertEqual(ledger.commit(receipts)["tick"],1)
                with self.assertRaises(Rejected):
                    ledger.commit(receipts)
                self.assertEqual(ledger.save()["tick"],1)
            finally:
                ledger.close()
            with self.assertRaises(Rejected):
                Authority(path)
            with self.assertRaises(Rejected):
                authority_for(BINDING[:-1]+"c")(path)


if __name__=="__main__":
    unittest.main()
