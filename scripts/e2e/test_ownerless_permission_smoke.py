"""Negative replay checks on the retained real Fabric/worker observations."""
import copy
import json
from pathlib import Path
import unittest

from ownerless_permission_smoke import verify_observations
from region_runtime import Rejected

RECEIPT = Path(__file__).resolve().parents[2] / "research/distributed-simulation/evidence/2026-11-26-ownerless-permission.json"


class PermissionEvidenceTests(unittest.TestCase):
    def test_retained_observations_and_mutations(self):
        receipt = json.loads(RECEIPT.read_text())
        self.assertEqual(verify_observations(receipt), {
            "fabric_checks": 57, "unsafe_launches_refused": 5,
            "worker_refusals": 34, "worker_ticks": 2,
        })
        mutations = (
            lambda r: r["fabric_runs"][0]["checks"].pop(),
            lambda r: r["fabric_runs"][1].update(saved_ops=[{"name": "Founder"}]),
            lambda r: r["fabric_runs"][3].update(game_listener_started=True),
            lambda r: r["workers"]["runs"][0].update(root="0"*64),
            lambda r: r["workers"]["runs"][1]["receipt_bindings"].__setitem__(0, "wrong"),
            lambda r: r["workers"].update(all_workers_exited=False),
            lambda r: r["artifact_manifest"]["bridge"].update(sha256="0"*64),
            lambda r: r["creation_certificate"]["genesis"]["initial_state_commitment"].__setitem__(0, 999),
            lambda r: r["workers"]["refusals"].pop(),
        )
        for mutation in mutations:
            bad = copy.deepcopy(receipt)
            mutation(bad)
            with self.subTest(mutation=mutation), self.assertRaises(Rejected):
                verify_observations(bad)


if __name__ == "__main__":
    unittest.main()
