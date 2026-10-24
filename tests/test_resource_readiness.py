"""Keep initialization readiness distinct from stable process identities."""
import os
from pathlib import Path
import shutil
import subprocess
import unittest


@unittest.skipUnless(os.name == 'nt', 'Windows resource sampler')
class ResourceReadinessTests(unittest.TestCase):
    def test_adapter_states_in_both_powershell_editions(self):
        script = Path(__file__).with_name('resource-readiness.tests.ps1')
        for name in ('powershell.exe', 'pwsh.exe'):
            shell = shutil.which(name)
            if shell is None:
                self.skipTest(f'{name} unavailable')
            with self.subTest(shell=name):
                result = subprocess.run(
                    [shell, '-NoProfile', '-NonInteractive', '-File', str(script)],
                    capture_output=True, timeout=30,
                    env=dict(os.environ, POWERSHELL_UPDATECHECK='Off'))
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == '__main__':
    unittest.main()
