import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("readiness", Path(__file__).resolve().parents[1] / "scripts/analyze-static-readiness.py")
readiness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(readiness)


class StaticReadiness(unittest.TestCase):
    def test_derive_each_interval_and_delta_before_taking_medians(self):
        def metric(before, after):
            return {side: {"samples": values, "unit": "ms"}
                    for side, values in (("baseline", before), ("candidate", after))}
        batch = {"valid_pairs": 3, "metrics": {
            "first_visible_static_candidate": metric([1, 20, 100], [2, 30, 110]),
            "complete_static_candidates": metric([2, 120, 105], [5, 130, 150])}}
        self.assertEqual(readiness.gaps(batch), {"baseline": 5, "candidate": 40, "paired_change": 2})
        batch["metrics"]["complete_static_candidates"]["candidate"]["samples"][0] = 1
        with self.assertRaisesRegex(ValueError, "invalid"):
            readiness.gaps(batch)
        batch["valid_pairs"] = 4
        with self.assertRaisesRegex(ValueError, "incomplete"):
            readiness.gaps(batch)
