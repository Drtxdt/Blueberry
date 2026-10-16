"""Actual worker failures, gate failures, timeouts, and stale-owner receipts."""
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/run-benchmark-batch.py"
SPEC = importlib.util.spec_from_file_location("batch", SCRIPT)
BATCH = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BATCH)


class BatchTests(unittest.TestCase):
    def execute(self, root, source, timeout=10):
        report = root / "report.json"
        task = dict(name="first", argv=[sys.executable, "-c", source], cwd=str(root),
                    timeout_seconds=timeout, report=str(report), completion_field="measurement_complete",
                    accepted_exit_codes=[0, 1])
        plan = root / "plan.json"
        BATCH.write(plan, dict(schema=1, tasks=[task]))
        process = subprocess.run([sys.executable, str(SCRIPT), "--plan", str(plan), "--output", str(root / "batch")],
                                 capture_output=True, timeout=20)
        return process, BATCH.read(root / "batch/batch.json")

    def test_complete_gate_failure_is_a_valid_measurement(self):
        with tempfile.TemporaryDirectory() as tmp:
            process, receipt = self.execute(Path(tmp), "import pathlib,sys; pathlib.Path('report.json').write_text('{\"measurement_complete\":true,\"passed\":false}'); sys.exit(1)")
            self.assertEqual(process.returncode, 0, process.stderr)
            self.assertTrue(receipt["measurement_complete"])
            self.assertEqual(receipt["results"][0]["exit_code"], 1)

    def test_missing_or_incomplete_report_is_invalid(self):
        for source in ["pass", "import pathlib; pathlib.Path('report.json').write_text('{\"measurement_complete\":false}')"]:
            with self.subTest(source=source), tempfile.TemporaryDirectory() as tmp:
                process, receipt = self.execute(Path(tmp), source)
                self.assertNotEqual(process.returncode, 0)
                self.assertFalse(receipt["measurement_complete"])
                self.assertEqual(receipt["status"], "invalid")

    def test_timeout_kills_owned_descendant(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = "import subprocess,sys,time; p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)']); open('child.json','w').write(str(p.pid)); time.sleep(60)"
            process, receipt = self.execute(root, source, 3)
            self.assertNotEqual(process.returncode, 0)
            self.assertEqual(receipt["status"], "invalid")
            pid = int((root / "child.json").read_text())
            self.assertIsNone(BATCH.process_identity(pid), "Descendant survived timeout")

    @unittest.skipUnless(os.name == "nt", "Windows Job owner-death semantics")
    def test_abrupt_coordinator_exit_cleans_tree_and_is_inspectable(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = "import subprocess,sys,time; p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)']); open('child.json','w').write(str(p.pid)); time.sleep(60)"
            task = dict(name="first", argv=[sys.executable, "-c", source], cwd=str(root),
                        timeout_seconds=60, report=str(root / "report.json"),
                        completion_field="measurement_complete")
            plan = root / "plan.json"
            BATCH.write(plan, dict(schema=1, tasks=[task]))
            process = subprocess.Popen([sys.executable, str(SCRIPT), "--plan", str(plan), "--output", str(root / "batch")],
                                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                import time
                deadline = time.monotonic() + 10
                while not (root / "child.json").exists():
                    self.assertLess(time.monotonic(), deadline, "Worker did not start")
                    time.sleep(.02)
                pid = int((root / "child.json").read_text())
            finally:
                process.kill()
                process.wait(timeout=10)
            # Kernel Job cleanup may finish just after the owner is signalled.
            deadline = time.monotonic() + 5
            while BATCH.process_identity(pid) is not None and time.monotonic() < deadline:
                time.sleep(.02)
            self.assertIsNone(BATCH.process_identity(pid), "Descendant survived owner exit")
            receipt = BATCH.inspect(root / "batch")
            self.assertEqual(receipt["status"], "interrupted")
            self.assertFalse(receipt["measurement_complete"])
            self.assertEqual(receipt["results"][0]["status"], "interrupted")
            self.assertFalse(receipt["results"][0]["worker_alive"])

    def test_inspection_reconciles_dead_owner_without_replacing_reports(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            receipt = dict(status="running", owner=dict(pid=4294967, created=0), measurement_complete=False, results=[])
            BATCH.write(root / "batch.json", receipt)
            result = BATCH.inspect(root)
            self.assertEqual(result["status"], "interrupted")
            self.assertFalse(result["measurement_complete"])

    def test_inspection_keeps_live_owner_and_rejects_recycled_identity(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            identity = BATCH.process_identity(os.getpid())
            BATCH.write(root / "batch.json", dict(status="running", owner=identity, measurement_complete=False))
            self.assertEqual(BATCH.inspect(root)["status"], "running")
            identity["created"] = 0
            BATCH.write(root / "batch.json", dict(status="running", owner=identity, measurement_complete=False))
            self.assertEqual(BATCH.inspect(root)["status"], "interrupted")

    def test_inspection_reconciles_legacy_interrupted_child(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            completed = dict(status="completed", report_sha256="retained")
            BATCH.write(root / "batch.json", dict(status="interrupted", owner=dict(pid=4294967, created=0),
                        interrupted_at_utc="retained-time", measurement_complete=False,
                        results=[completed, dict(status="running", worker=dict(pid=4294966, created=0))]))
            receipt = BATCH.inspect(root)
            self.assertEqual(receipt["results"][0], completed)
            self.assertEqual(receipt["results"][1]["status"], "interrupted")
            self.assertEqual(receipt["results"][1]["stopped_at_utc"], "retained-time")
            self.assertFalse(receipt["results"][1]["worker_alive"])

    def test_wrong_dependency_is_rejected_before_spawn(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            task = dict(name="first", argv=[sys.executable], cwd=str(root), timeout_seconds=1, report=str(root / "missing"))
            plan = dict(schema=1, tasks=[task], dependencies=[dict(path=str(SCRIPT), sha256="0" * 64)])
            with self.assertRaisesRegex(AssertionError, "identity mismatch"):
                BATCH.validate(plan)


if __name__ == "__main__":
    unittest.main()
