import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


@unittest.skipUnless(os.name == 'nt' and shutil.which('pwsh.exe'), 'Windows lifecycle observer')
class ProcessLifecycleTests(unittest.TestCase):
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
