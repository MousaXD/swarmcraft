"""Retained managed restoration evidence and mutation regressions."""
import copy
import json
from pathlib import Path
import unittest

from region_runtime import Rejected
from verify_combined_ownerless_evidence import verify_sync_signatures
from verify_managed_restore_evidence import verify_observations


class ManagedRestoreEvidenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = json.loads((Path(__file__).parent/"evidence"/
                               "2026-11-26-managed-certified-restore.json").read_bytes())

    def test_retained_managed_restore(self):
        self.assertEqual(verify_observations(self.data),
            {"certified_ticks":2,"abandoned_ticks":1,"restored_tick":1,"workers":3,
             "recovered_receipts":1,"tombstoned_commands":1,"signed_decisions":7,"signed_sync_receipts":2})
        verify_sync_signatures({**self.data,"schema":3})

    def test_v3_certification_lineage_and_mutations(self):
        data = json.loads((Path(__file__).parent/"evidence"/
                           "2026-11-26-managed-v3-restore-regression.json").read_bytes())
        self.assertEqual(verify_observations(data),verify_observations(self.data))
        verify_sync_signatures({**data,"schema":3})
        for mutate in (
            lambda r:r["certifications"][1]["plan"].update(generation=3),
            lambda r:r["certifications"][1]["plan"]["proposal"].update(proposal_hash="a"*64),
            lambda r:r["certifications"][1]["plan"]["active"].pop(),
            lambda r:r["certifications"][1]["completion"]["sync"].clear(),
            lambda r:r["certifications"][0]["completion"]["sync"][0]["result"]["receipt"]["signature"].__setitem__(0,0),
            lambda r:r["restored_certifications"].clear(),
            lambda r:r["artifact_manifest"].pop("managed_certification.py"),
        ):
            altered = copy.deepcopy(data)
            mutate(altered)
            with self.assertRaises(Rejected):
                verify_observations(altered)

    def test_refuse_mutated_managed_restore(self):
        for mutate in (
            lambda r:r.update(runtime_activation=True),
            lambda r:r.update(source_sha256_after="0"*64),
            lambda r:r["source_pin"].update(blocked=[]),
            lambda r:r["source_pin"]["offers"][1].update(revision=2),
            lambda r:r["restored_status"].update(blocked=[]),
            lambda r:r["restoration"].update(abandoned_request_ids=[]),
            lambda r:r["quarantine"].update(claimed=False),
            lambda r:r["restored_history"]["commits"].append(r["source_history"]["commits"][-1]),
            lambda r:r["restored_events"].pop(0),
            lambda r:r["stages"][0]["authorizations"][0].__setitem__(3,4),
            lambda r:r["stages"][1]["client"]["observed"].update(reply=r["stages"][0]["client"]["observed"]["reply"]),
            lambda r:r["stages"][2]["guards"].pop(),
            lambda r:r["stages"][2]["after"].update(generation=3),
            lambda r:r["stages"][2]["publication_checks"][0].update(freshness_lease=True),
            lambda r:r["restored_authorizations"].clear(),
            lambda r:r["recovered_client"]["observed"]["reply"].update(inventory_delta=42),
            lambda r:r["history"]["records"][4]["proposal"]["action"]["set_player"].update(allowed=True),
            lambda r:r["certified_cuts"][7]["network"]["sync"][0]["result"]["receipt"]["body"].update(fence=5),
        ):
            altered = copy.deepcopy(self.data)
            mutate(altered)
            with self.assertRaises(Rejected):
                verify_observations(altered)
        altered = copy.deepcopy(self.data)
        altered["certified_cuts"][4]["network"]["sync"][0]["result"]["receipt"]["signature"][0] ^= 1
        with self.assertRaisesRegex(Rejected,"invalid durable sync receipt signature"):
            verify_sync_signatures({**altered,"schema":3})


if __name__=="__main__":
    unittest.main()
