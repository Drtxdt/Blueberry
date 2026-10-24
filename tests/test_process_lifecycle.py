import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


@unittest.skipUnless(os.name == 'nt' and shutil.which('pwsh.exe'), 'Windows lifecycle observer')
class ProcessLifecycleTests(unittest.TestCase):
    def reconcile(self, events):
        script = Path(__file__).parents[1] / 'scripts/observe-process-lifecycle.ps1'
        quote = lambda value: "'" + str(value).replace("'", "''") + "'"
        code = (
            "$tokens=$null; $errors=$null; "
            f"$ast=[Management.Automation.Language.Parser]::ParseFile({quote(script)},[ref]$tokens,[ref]$errors); "
            "$function=$ast.Find({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] "
            "-and $node.Name -eq 'Resolve-LifecycleEvents'},$true); "
            ". ([scriptblock]::Create($function.Extent.Text)); "
            f"$records=ConvertFrom-Json {quote(json.dumps(events))}; "
            "Resolve-LifecycleEvents $records 9 | ConvertTo-Json -Depth 8 -Compress"
        )
        output = subprocess.check_output([shutil.which('pwsh.exe'), '-NoProfile', '-NonInteractive',
                                          '-Command', code], timeout=30)
        return json.loads(output)

    def test_transient_child_is_retained_and_unrelated_process_is_excluded(self):
        report = self.reconcile([
            dict(kind='start', time=100, pid=9, parent_pid=1, name='root.exe'),
            dict(kind='start', time=110, pid=10, parent_pid=9, name='helper.exe'),
            dict(kind='start', time=120, pid=11, parent_pid=1, name='unrelated.exe'),
            dict(kind='stop', time=130, pid=10),
            dict(kind='stop', time=200, pid=9),
        ])
        self.assertTrue(report['reconciled'])
        self.assertEqual([node['pid'] for node in report['processes']], [9, 10])
        self.assertEqual(report['processes'][1]['parent_started_utc'],
                         report['processes'][0]['started_utc'])

    def test_pid_reuse_cannot_hide_missing_exit(self):
        report = self.reconcile([
            dict(kind='start', time=100, pid=9, parent_pid=1, name='root.exe'),
            dict(kind='start', time=110, pid=10, parent_pid=9, name='first.exe'),
            dict(kind='start', time=150, pid=10, parent_pid=9, name='second.exe'),
            dict(kind='stop', time=180, pid=10),
            dict(kind='stop', time=200, pid=9),
        ])
        self.assertFalse(report['reconciled'])
        self.assertIsNone(report['processes'][1]['stopped_utc'])

    def test_subscription_failure_never_starts_workload_and_preserves_receipt(self):
        root = Path(__file__).parents[1]
        shell = shutil.which('pwsh.exe')
        quote = lambda value: "'" + str(value).replace("'", "''") + "'"
        with tempfile.TemporaryDirectory() as folder:
            folder = Path(folder)
            marker = folder / 'must-not-run.txt'
            output = folder / 'evidence'
            workload = f"[IO.File]::WriteAllText({quote(marker)},'started')"
            code = (
                "function Register-CimIndicationEvent { throw 'synthetic subscription denied' }; "
                f"& {quote(root / 'scripts/observe-process-lifecycle.ps1')} "
                f"-Executable {quote(shell)} -OutputDirectory {quote(output)} "
                f"-Arguments @('-NoProfile','-Command',{quote(workload)})"
            )
            result = subprocess.run([shell, '-NoProfile', '-NonInteractive', '-Command', code],
                                    capture_output=True, timeout=30)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(marker.exists())
            receipt = json.loads((output / 'lifecycle.json').read_text(encoding='utf-8-sig'))
            self.assertEqual(receipt['error'], 'synthetic subscription denied')
            self.assertIsNone(receipt['root_pid'])
            self.assertEqual(receipt['processes'], [])
            self.assertFalse(receipt['lifecycle_reconciled'])
            self.assertFalse(receipt['formal_timing_eligible'])


if __name__ == '__main__':
    unittest.main()
