"""Retained restoration evidence and fail-closed mutation checks."""
import copy
import json
from pathlib import Path
import unittest

from region_runtime import Rejected
from verify_certified_restore_evidence import verify_observations
from verify_combined_ownerless_evidence import verify_sync_signatures


class CertifiedRestoreEvidenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = json.loads((Path(__file__).parent/"evidence"/
                               "2026-11-26-certified-staging-restore.json").read_bytes())

    def test_restored_reference_run_and_signatures(self):
        self.assertEqual(verify_observations(self.data),
                         {"certified_ticks":2,"abandoned_ticks":1,"restored_tick":1,"workers":3,
                          "recovered_receipts":1,"signed_decisions":5,"signed_sync_receipts":3})
        verify_sync_signatures({**self.data,"schema":3})

    def test_reject_changed_restore_observations(self):
        for mutate in (
            lambda r:r.update(source_sha256_after="0"*64),
            lambda r:r.update(physical_machines=2),
            lambda r:r["restoration"].update(write_authority=True),
            lambda r:r["restoration"].update(rotation_required=False),
            lambda r:r["restoration"]["source"].update(generation=1),
            lambda r:r["restored_checkpoint"].update(tick=2),
            lambda r:r["quarantine"].update(claimed=False),
            lambda r:r["quarantine"].update(cut_root="0"*64),
            lambda r:r["restored_history"]["commits"].append(r["source_history"]["commits"][-1]),
            lambda r:r["restored_history"]["commits"][1].__setitem__(5,"0"*64),
            lambda r:r["source_history"]["epochs"][0][3][0].update(session="0"*64),
            lambda r:r["stages"][1]["client"]["observed"].update(reply=r["stages"][0]["client"]["observed"]["reply"]),
            lambda r:r["stages"][1].update(worker_retired=False),
            lambda r:r["stages"][2]["after"].update(generation=2),
            lambda r:r["stages"][2]["checkpoint"]["transaction_journal"]["commands"][-1]["response"].update(inventory_delta=42),
            lambda r:r["recovered_client"]["observed"]["reply"].update(inventory_delta=42),
            lambda r:r["certified_cuts"][5]["network"]["sync"][0]["result"]["receipt"]["body"].update(fence=3),
            lambda r:r["certified_cuts"][5]["network"]["vote_listener"].update(validator="scpeer:"+"0"*64),
        ):
            altered = copy.deepcopy(self.data)
            mutate(altered)
            with self.assertRaises(Rejected):
                verify_observations(altered)
        altered = copy.deepcopy(self.data)
        altered["certified_cuts"][3]["network"]["sync"][0]["result"]["receipt"]["signature"][0] ^= 1
        with self.assertRaisesRegex(Rejected,"invalid durable sync receipt signature"):
            verify_sync_signatures({**altered,"schema":3})


if __name__=="__main__":
    unittest.main()
