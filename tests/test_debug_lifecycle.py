"""Real Windows debug events, short-lived descendants and failure receipts."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'scripts/observe-debug-lifecycle.py'


@unittest.skipUnless(os.name == 'nt', 'Windows debug events')
class DebugLifecycleTests(unittest.TestCase):
    def observe(self, directory, command, timeout=10):
        output = Path(directory) / 'evidence'
        result = subprocess.run([sys.executable, str(SCRIPT), '--output', str(output),
                                 '--timeout', str(timeout), '--', *command], capture_output=True, timeout=20)
        return result, json.loads((output / 'lifecycle.json').read_text(encoding='utf-8'))

    def test_short_lived_child_has_parent_and_exit(self):
        with tempfile.TemporaryDirectory() as directory:
            result, receipt = self.observe(directory, [sys.executable, '-c',
                'import subprocess,sys; subprocess.run([sys.executable,"-c","raise SystemExit(7)"])'])
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(receipt['lifecycle_reconciled'])
            self.assertFalse(receipt['formal_timing_eligible'])
            child = next(row for row in receipt['processes'] if row['exit_code'] == 7)
            self.assertEqual(child['parent_pid'], receipt['root_pid'])
            self.assertGreater(child['created_filetime'], 0)
            self.assertTrue(all(row['observed_exit_ns'] is not None for row in receipt['processes']))

    def test_timeout_does_not_claim_complete_lifecycle(self):
        with tempfile.TemporaryDirectory() as directory:
            result, receipt = self.observe(directory, [sys.executable, '-c', 'import time; time.sleep(30)'], 0.4)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(receipt['lifecycle_reconciled'])
            self.assertIn('timed out', receipt['error'])
            # Opening for synchronization is sufficient to verify exit; no
            # permission to inspect unrelated processes is requested.
            import ctypes
            from ctypes import wintypes
            kernel = ctypes.WinDLL('kernel32', use_last_error=True)
            kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
            kernel.OpenProcess.restype = wintypes.HANDLE
            kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
            kernel.CloseHandle.argtypes = [wintypes.HANDLE]
            handle = kernel.OpenProcess(0x00100000, False, receipt['root_pid'])
            if handle:
                try:
                    self.assertEqual(kernel.WaitForSingleObject(handle, 2000), 0)
                finally:
                    kernel.CloseHandle(handle)

    def test_missing_executable_retains_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            result, receipt = self.observe(directory, [str(Path(directory) / 'absent.exe')])
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(receipt['lifecycle_reconciled'])
            self.assertEqual(receipt['processes'], [])
            self.assertTrue(receipt['error'])


if __name__ == '__main__':
    unittest.main()
