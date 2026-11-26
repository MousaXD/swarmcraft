"""Retained managed contribution evidence and mutation regressions."""
import copy
import json
from pathlib import Path
import unittest

from region_runtime import Rejected
from verify_combined_ownerless_evidence import verify_sync_signatures
from verify_managed_ownerless_evidence import verify_observations


class ManagedEvidenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = json.loads((Path(__file__).parent/"evidence"/
                               "2026-11-26-managed-ownerless-contribution.json").read_bytes())

    def test_retained_managed_contribution(self):
        self.assertEqual(verify_observations(self.data),
                         {"certified_ticks":2,"withdrawn_ticks":1,"worker_processes":5,"scopes":3,
                          "signed_decisions":7,"signed_sync_receipts":2,"role_proofs":8})
        verify_sync_signatures({**self.data,"schema":3})

    def test_reject_modified_lifecycle_and_publication_observations(self):
        for mutate in (
            lambda r:r.update(runtime_activation=True),
            lambda r:r.update(zero_consent_no_spawn=False),
            lambda r:r.update(revoked_role_refused=False),
            lambda r:r["slot_subjects"].reverse(),
            lambda r:r["stages"][0]["scope_record"].update(released=False),
            lambda r:r["stages"][0]["scope_record"]["proofs"][0].update(revision=3),
            lambda r:r["stages"][0]["scope_record"]["proofs"][0]["proof"]["result"].update(worker_admission=True),
            lambda r:r["stages"][0]["guards"].pop(),
            lambda r:r["stages"][0]["guards"][0]["kernel"][0].update(kernel_memory_max_bytes=1),
            lambda r:r["stages"][0]["cleanup"][0].update(leaf_removed=False),
            lambda r:r["stages"][1]["draining"].update(phase="running"),
            lambda r:r["stages"][1]["journal"]["batches"][-1].update(result="a"*64),
            lambda r:r["stages"][1]["client"]["observed"].update(reply=r["stages"][0]["client"]["observed"]["reply"]),
            lambda r:r["stages"][1].update(certification_refusal="accepted"),
            lambda r:r["stages"][2]["after"].update(generation=3),
            lambda r:r["stages"][2]["client"]["request"].update(id="a"*64),
            lambda r:r["stages"][2]["publication_checks"][0].update(freshness_lease=True),
            lambda r:r["certified_cuts"][7]["network"]["sync"][0]["result"]["receipt"]["body"].update(fence=5),
            lambda r:r["events"].pop(),
            lambda r:r["events"][4].update(tick=1),
        ):
            altered = copy.deepcopy(self.data)
            mutate(altered)
            with self.assertRaises(Rejected):
                verify_observations(altered)
        altered = copy.deepcopy(self.data)
        altered["certified_cuts"][5]["network"]["sync"][0]["result"]["receipt"]["signature"][0] ^= 1
        with self.assertRaisesRegex(Rejected,"invalid durable sync receipt signature"):
            verify_sync_signatures({**altered,"schema":3})


if __name__=="__main__":
    unittest.main()
