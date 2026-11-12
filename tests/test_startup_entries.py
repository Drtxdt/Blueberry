import importlib.util
import json
from pathlib import Path
import unittest
from unittest.mock import Mock

spec = importlib.util.spec_from_file_location("entries", Path(__file__).resolve().parents[1] / "scripts/measure-startup-entries.py")
entries = importlib.util.module_from_spec(spec)
spec.loader.exec_module(entries)


class StartupAttribution(unittest.TestCase):
    def test_profile_requires_a_hook_from_this_launch_and_the_file_bootstrap(self):
        stages = ("profile_hook_entry blueberry_process_entry run_maintenance_begin run_maintenance_end "
                  "run_options_ready direct_start direct_resources_ready direct_spawn_begin direct_spawn_return "
                  "direct_script_enter direct_module_lookup direct_module_imported direct_module_loaded direct_hello "
                  "child_environment_published direct_environment_received worker_index_started worker_index_finished").split()
        events = [{"event": stage, "qpc": i + 10} for i, stage in enumerate(stages)]
        report = {"qpc_frequency_hz": 1000, "actual_entry": "isolated_automatic_profile",
                  "startup_qpc": [{"candidate": {"launch": 5, "prompt": 25, "key_sent": 30, "echo": 32,
                                                  "visible_static": 40, "complete_static": 50}}]}
        trace = Mock(spec=Path)
        trace.name = "trace.jsonl"
        trace.read_text.side_effect = ["\n".join(map(json.dumps, rows)) for rows in
                                      (events, events[1:], [dict(e, qpc=e["qpc"] - 100) for e in events])]
        spans = entries.attribute(report, trace)
        self.assertEqual(spans["direct_script_enter__direct_module_loaded"], 3)
        self.assertLess(spans["visible_static__worker_index_finished"], 0)
        with self.assertRaisesRegex(ValueError, "missing=.*profile_hook_entry"):
            entries.attribute(report, trace)
        with self.assertRaisesRegex(ValueError, "does not match"):
            entries.attribute(report, trace)
