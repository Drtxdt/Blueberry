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
        self.run_initialization_case(3, False)

    def test_stable_but_initializing_child_does_not_enter_idle_interval(self):
        self.run_initialization_case(12, True)

    def run_initialization_case(self, seconds, require_metadata):
        root = Path(__file__).parents[1]
        with tempfile.TemporaryDirectory() as folder:
            output = Path(folder) / 'resources.json'
            adapter = Path(folder) / 'adapter.json'
            metadata = dict(host_mode='direct', session_commands=dict(complete=True, error=None),
                            system_commands=dict(count=10, error=None))
            code = ('import os,subprocess,sys,time;from pathlib import Path; '
                    f'child=subprocess.Popen([sys.executable,"-c","import time;time.sleep({seconds})"]); '
                    'print(os.getpid(),flush=True);child.wait();'
                    f'Path({str(adapter)!r}).write_text({json.dumps(metadata)!r});time.sleep(60)')
            process = subprocess.Popen([sys.executable, '-c', code], stdout=subprocess.PIPE, text=True)
            try:
                pid = int(process.stdout.readline())
                argv = [shutil.which('pwsh.exe'), '-NoProfile', '-NonInteractive', '-File',
                    str(root / 'scripts/measure-process-tree.ps1'), '-RootProcessId', str(pid),
                    '-OutputPath', str(output), '-Seconds', '10']
                if require_metadata:
                    argv += ['-ReadyAdapterDirectory', folder]
                result = subprocess.run(argv, capture_output=True, timeout=55)
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
                if require_metadata:
                    self.assertTrue(any(not sample['initialization']['ready']
                                        for sample in report['warmup_samples']))
                    self.assertTrue(report['samples'][0]['initialization']['ready'])
            finally:
                process.terminate()
                process.wait(timeout=10)
                process.stdout.close()


if __name__ == '__main__':
    unittest.main()
