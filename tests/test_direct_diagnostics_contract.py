import json
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]


class DirectDiagnosticsContract(unittest.TestCase):
    def test_every_literal_sender_stage_is_declared(self):
        events = json.loads((ROOT / "shell/direct-diagnostics.json").read_text())["events"]
        for path in ["shell/direct-bridge.cs", "shell/direct.ps1"]:
            source = (ROOT / path).read_text(encoding="utf-8")
            for call, stage in re.findall(r'''(TraceStage|TracePoint|StartupPoint)\(["']([a-z_]+)["']''', source):
                event = "trace" if call == "TraceStage" else "trace_point"
                self.assertIn(stage, events[event]["stages"], (path, call, stage))
        source = (ROOT / "shell/direct-bridge.cs").read_text(encoding="utf-8")
        stages = re.search(r"string\[\] stages=\{([^}]+)\}", source).group(1)
        for stage in re.findall(r'"([a-z_]+)"', stages):
            self.assertIn(stage, events["trace_point"]["stages"])

    def test_diagnostic_names_are_unique_and_policy_is_nonfatal(self):
        events = json.loads((ROOT / "shell/direct-diagnostics.json").read_text())["events"]
        for spec in events.values():
            self.assertEqual(len(spec["stages"]), len(set(spec["stages"])))
            self.assertEqual(spec["unknown_stage"], "ignore")
            self.assertEqual(spec["invalid_fields"], "ignore")


if __name__ == "__main__":
    unittest.main()
