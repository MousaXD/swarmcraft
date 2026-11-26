"""The paired local candidate experiment has a separate evidence contract."""
import copy
import json
from pathlib import Path
import unittest

from region_runtime import Rejected
from scale_candidate_lab import COUNTS, MODES, REPEATS, SAMPLES, summarize, verify


class CandidateCostEvidenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        path = Path(__file__).parent / "evidence" / "2026-11-26-scale-candidate-cost.json"
        cls.report = json.loads(path.read_text())

    def test_retained_complete_paired_matrix(self):
        result = summarize(verify(self.report))
        self.assertEqual(len(result), len(COUNTS)*len(MODES))
        self.assertTrue(all(r["samples"] == REPEATS*SAMPLES for r in result))

    def test_negative_evidence(self):
        for mutate in (
            lambda d: d["runs"].pop(),
            lambda d: d["runs"][0].update(candidate_mode="reuse"),
            lambda d: d["runs"][0].update(journal_hash="0"*64),
            lambda d: d["runs"][0]["samples"][0].update(root="0"*64),
            lambda d: d["runs"][0]["samples"][0].update(cpu_ns=-1),
            lambda d: d["runs"][0]["samples"][0]["phase_ns"].update(other=-1),
            lambda d: d["runs"][0]["samples"][0]["transactions"].update(write=True),
        ):
            report = copy.deepcopy(self.report)
            mutate(report)
            with self.assertRaises(Rejected):
                verify(report)


if __name__ == "__main__":
    unittest.main()
