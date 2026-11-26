"""Reject forged local #68 qualification claims and corrupted replay evidence."""
import copy
import json
from pathlib import Path
import unittest

from combined_ownerless_lab import review_checkpoint
from region_runtime import Rejected
from verify_combined_ownerless_evidence import verify_observations, verify_sync_signatures


class CombinedEvidenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = json.loads((Path(__file__).parent/"evidence"/
                               "2026-11-26-combined-ownerless.json").read_bytes())

    def test_retained_combined_run(self):
        self.assertEqual(verify_observations(self.data),
                         {"ticks":3,"decisions":13,"workers":4,"client_receipts":4,"role_proofs":9})

    def test_retained_certified_gate_run_and_mutations(self):
        data = json.loads((Path(__file__).parent/"evidence"/
                           "2026-11-26-combined-certified-gate.json").read_bytes())
        self.assertEqual(data["schema"],2)
        self.assertEqual(verify_observations(data),
                         {"ticks":3,"decisions":13,"workers":4,"client_receipts":4,"role_proofs":9})
        for mutate in (
            lambda r:r.update(schema=1),
            lambda r:r.update(schema=True),
            lambda r:r["stages"][0].pop("publication_checks"),
            lambda r:r["stages"][0]["publication_checks"].pop(),
            lambda r:r["stages"][0].update(uncertified_client_access_refused=False),
            lambda r:r["stages"][1]["publication_checks"][0].update(expected_parent="0"*64),
            lambda r:r["stages"][1]["publication_checks"][0].update(reference_fence=0),
            lambda r:r["stages"][1]["publication_checks"][0].update(checkpoint="0"*64),
            lambda r:r["stages"][1]["publication_checks"][0].update(player_key=r["participant_keys"][0]),
            lambda r:r["stages"][2]["publication_checks"][0].update(local_validator=False),
            lambda r:r["stages"][2]["publication_checks"][0].update(pending_proposal="a"*64),
            lambda r:r["stages"][2]["publication_checks"][0].update(freshness_lease=True),
            lambda r:r["stages"][2]["publication_checks"][0].update(write_authority=True),
        ):
            altered = copy.deepcopy(data)
            mutate(altered)
            with self.assertRaises(Rejected):
                verify_observations(altered)

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

    def test_validator_review_independently_replays_exact_successor(self):
        records = self.data["certified_cuts"]
        binding = self.data["binding"]
        keys = self.data["participant_keys"]
        mapping = [keys[0],keys[5]]
        for seq in (6,10,13):
            prior, candidate = records[seq-1]["checkpoint"], records[seq]["checkpoint"]
            self.assertEqual(review_checkpoint(binding,mapping,prior,candidate),candidate["runtime_root"])
        prior, candidate = records[9]["checkpoint"],records[10]["checkpoint"]
        for mutate in (
            lambda c:c.update(tick=3),
            lambda c:c["player_keys"].reverse(),
            lambda c:c.update(runtime_root="0"*64),
            lambda c:c["regions"].reverse(),
            lambda c:c["transaction_journal"]["genesis"].reverse(),
            lambda c:c["transaction_journal"]["commands"][0]["response"].update(inventory_delta=123),
            lambda c:c["transaction_journal"]["commands"][-1]["request"].update(parent="0"*64),
            lambda c:c["transaction_journal"]["commands"][-1]["request"].update(id=c["transaction_journal"]["commands"][0]["request"]["id"]),
            lambda c:c["transaction_journal"]["commands"][-1]["response"].update(inventory_delta=123),
            lambda c:c["transaction_journal"]["batches"][-1]["batch"][0].__setitem__(1,1),
            lambda c:c["transaction_journal"]["batches"][-1].update(result="0"*64),
        ):
            altered = copy.deepcopy(candidate)
            mutate(altered)
            with self.assertRaises(Rejected):
                review_checkpoint(binding,mapping,prior,altered)
        with self.assertRaises(Rejected):
            review_checkpoint(binding,mapping,{**prior,"tick":True},candidate)

    def test_networked_certification_and_receipt_mutations(self):
        data = json.loads((Path(__file__).parent/"evidence"/
                           "2026-11-26-combined-networked-cut.json").read_bytes())
        self.assertEqual(data["schema"],3)
        self.assertEqual(verify_observations(data),
                         {"ticks":3,"decisions":13,"workers":4,"client_receipts":4,"role_proofs":9})
        verify_sync_signatures(data)
        for mutate in (
            lambda r:r.update(schema=2),
            lambda r:r["certified_cuts"][6]["network"].update(collector_slot=0),
            lambda r:r["certified_cuts"][6]["network"]["reviews"][0].update(replayed_root="0"*64),
            lambda r:r["certified_cuts"][6]["network"]["vote_listener"].update(validator="scpeer:"+"0"*64),
            lambda r:r["certified_cuts"][6]["network"]["sync"].pop(),
            lambda r:r["certified_cuts"][10]["network"]["minority_attempt"]["collector_status"].update(pending_proposal=None),
            lambda r:r["certified_cuts"][10]["network"]["minority_attempt"].update(client_access_refused=False),
            lambda r:r["certified_cuts"][13]["network"]["sync"][0]["result"]["receipt"]["body"].update(fence=1),
            lambda r:r["certified_cuts"][13]["network"]["sync"][0]["result"].update(write_authority=True),
            lambda r:r["certified_cuts"][13]["network"].update(pre_sync_client_access_refused=False),
            lambda r:r["stages"][1].update(workers_retired_before_certification=False),
        ):
            altered = copy.deepcopy(data)
            mutate(altered)
            with self.assertRaises(Rejected):
                verify_observations(altered)
        altered = copy.deepcopy(data)
        altered["certified_cuts"][6]["network"]["sync"][0]["result"]["receipt"]["signature"][0] ^= 1
        with self.assertRaisesRegex(Rejected,"invalid durable sync receipt signature"):
            verify_sync_signatures(altered)


if __name__ == "__main__":
    unittest.main()
