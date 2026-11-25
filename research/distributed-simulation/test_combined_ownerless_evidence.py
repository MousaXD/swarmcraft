"""Reject forged local #68 qualification claims and corrupted replay evidence."""
import copy
import json
from pathlib import Path
import unittest

from region_runtime import Rejected
from verify_combined_ownerless_evidence import verify_observations


class CombinedEvidenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = json.loads((Path(__file__).parent/"evidence"/
                               "2026-11-26-combined-ownerless.json").read_bytes())

    def test_retained_combined_run(self):
        self.assertEqual(verify_observations(self.data),
                         {"ticks":3,"decisions":13,"workers":4,"client_receipts":4,"role_proofs":9})

    def test_reject_incorrect_claims_and_state(self):
        for mutate in (
            lambda r:r.update(physical_machines=2),
            lambda r:r.update(live_minecraft_players=2),
            lambda r:r.update(canonical_minecraft_authority=True),
            lambda r:r.update(founder_journal_unchanged_after_departure=False),
            lambda r:r["certified_cuts"][9].update(voter_slots=[0,1]),
            lambda r:r["certified_cuts"][10]["checkpoint"].update(tick=3),
            lambda r:r["stages"][1].update(player_cross_worker=[1,1]),
            lambda r:r["stages"][1].update(stale_receipt_refusal="invalid bounded sequence"),
            lambda r:r["stages"][1]["worker_proofs"][0]["result"].update(write_authority=True),
            lambda r:r["stages"][2]["pre_kernel"][0].update(kernel_memory_max_bytes=0),
            lambda r:r["stages"][2]["clients"][0]["observed"]["reply"]["view"].update(inventory=-1),
            lambda r:r["stages"][2].update(worker_cleanup=False),
            lambda r:r["withdrawal_refusal"].update(result="accepted"),
            lambda r:r["minority"]["copy_report"].update(write_authority=True),
            lambda r:r["minority"].update(copy_sha256="0"*64),
        ):
            altered = copy.deepcopy(self.data)
            mutate(altered)
            with self.assertRaises(Rejected):
                verify_observations(altered)


if __name__ == "__main__":
    unittest.main()
