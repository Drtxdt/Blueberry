import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
import uuid


@unittest.skipUnless(os.name == 'nt' and shutil.which('pwsh.exe'), 'Windows resource sampler')
class ResourceSamplerTests(unittest.TestCase):
    def test_startup_child_exit_is_retained_outside_complete_idle_interval(self):
        root = Path(__file__).parents[1]
        with tempfile.TemporaryDirectory() as folder:
            output = Path(folder) / 'resources.json'
            code = ('import os,subprocess,sys,time; '
                    'child=subprocess.Popen([sys.executable,"-c","import time;time.sleep(3)"]); '
                    'print(os.getpid(),flush=True);child.wait();time.sleep(60)')
            process = subprocess.Popen([sys.executable, '-c', code], stdout=subprocess.PIPE, text=True)
            try:
                pid = int(process.stdout.readline())
                result = subprocess.run([shutil.which('pwsh.exe'), '-NoProfile', '-NonInteractive', '-File',
                    str(root / 'scripts/measure-process-tree.ps1'), '-RootProcessId', str(pid),
                    '-OutputPath', str(output), '-Seconds', '10'], capture_output=True, timeout=55)
                report = json.loads(output.read_text(encoding='utf-8-sig'))
                if result.returncode:
                    evidence = root / 'target/resource-sampler-evidence'
                    evidence.mkdir(parents=True, exist_ok=True)
                    retained = evidence / (str(uuid.uuid4()) + '.json')
                    shutil.copyfile(output, retained)
                    self.fail(f"{report.get('collection_errors')}; raw snapshots: {retained}")
                self.assertTrue(report['warmup_complete'])
                self.assertTrue(report['measurement_complete'])
                self.assertGreaterEqual(report['elapsed_seconds'], 10)
                self.assertTrue(any(len(sample['processes']) > 1 for sample in report['warmup_samples']))
                self.assertTrue(all([item['pid'] for item in sample['processes']] == [pid]
                                    for sample in report['samples']))
                self.assertEqual(report['collection_errors'], [])
            finally:
                process.terminate()
                process.wait(timeout=10)
                process.stdout.close()


if __name__ == '__main__':
    unittest.main()
