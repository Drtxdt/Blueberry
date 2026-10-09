"""Exercise the actual matrix runner with deterministic fake probe reports."""
import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

SHELL = shutil.which('pwsh')
ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(SHELL, 'PowerShell 7 is required')
class MatrixTests(unittest.TestCase):
    def run_matrix(self, continue_on_failure=True, corrupt=''):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            probe = root / 'probe.ps1'
            probe.write_text('''
$hash='A'*64
if($args[0] -eq 'doctor') {
    @{build=@{dirty=$false;private_editors=@{present=$true};conpty=@{sha256=$hash}}}|ConvertTo-Json -Depth 10
    exit 0
}
$output=$args[[Array]::IndexOf($args,'--output')+1]
$count=2
$stats=@{samples=@(60,60);median=60;p95=60}
$report=@{probe_conpty=@{mode='pinned';sha256=$hash};product_conpty=@{sha256=$hash}}
if($args[0] -eq 'product-probe') {
    $module=$env:BLUEBERRY_TEST_PSREADLINE_MODULE
    $dll=Join-Path (Split-Path $module) 'Microsoft.PowerShell.PSReadLine.dll'
    if($module -like '*200*') {$dll=Join-Path (Split-Path $module) 'Microsoft.PowerShell.PSReadLine2.dll'}
    $actual=(Get-FileHash $dll).Hash
    $report.plain_editors=@(@{dll_sha256=$actual},@{dll_sha256=$actual})
    $report.editor_eligible=$true
    $report.paired_first_input_delta=$stats
} else {
    $report.transport_degraded=$false
    $report.scenarios=@(foreach($name in @('root','git','cargo','js','path','fuzzy')) {
        @{name=$name;acceptance=@{cache_miss=@{observed_samples=2;expected_samples=2;statistics=$stats;status='failed'};cache_hit=@{observed_samples=2;expected_samples=2;statistics=$stats;status='failed'}}}
    })
}
CORRUPTION
$report|ConvertTo-Json -Depth 20|Set-Content -LiteralPath $output -Encoding utf8
'''.replace('CORRUPTION', corrupt), encoding='utf-8')
            modules = []
            for version, dll in [('200', 'Microsoft.PowerShell.PSReadLine2.dll'), ('245', 'Microsoft.PowerShell.PSReadLine.dll')]:
                directory = root / version
                directory.mkdir()
                (directory / dll).write_bytes(b'original')
                module = directory / 'PSReadLine.psd1'
                module.write_text('@{}')
                modules.append(module)
            output = root / 'results'
            command = [SHELL, '-NoProfile', '-File', str(ROOT / 'scripts/run-editor-matrix.ps1'),
                       '-Executable', str(probe), '-Probe', str(probe), '-Original200', str(modules[0]),
                       '-Original245', str(modules[1]), '-OutputDirectory', str(output),
                       '-Shell7', 'unused', '-Shell51', 'unused', '-Samples', '2', '-StartupPairs', '2']
            if continue_on_failure:
                command.append('-ContinueOnGateFailure')
            result = subprocess.run(command, capture_output=True, timeout=60)
            self.assertNotEqual(result.returncode, 0)
            self.assertTrue((output / 'matrix.json').exists(), result.stderr.decode('utf-8', errors='replace'))
            return json.loads((output / 'matrix.json').read_text(encoding='utf-8-sig'))

    def test_slow_measurements_complete_all_groups_without_passing(self):
        result = self.run_matrix()
        self.assertTrue(result['measurement_complete'])
        self.assertFalse(result['automated_performance_passed'])
        self.assertEqual(len(result['results']), 9)
        self.assertEqual(len(result['gate_errors']), 75)

    def test_default_stops_after_first_threshold_failure(self):
        result = self.run_matrix(False)
        self.assertFalse(result['measurement_complete'])
        self.assertEqual(len(result['results']), 1)

    def test_invalid_identity_counts_and_statistics_are_fatal(self):
        for corrupt in ["$report.probe_conpty.sha256='B'*64",
                        "$stats.samples=@(60)", "$stats.median=1", "$stats.p95=[double]::NaN"]:
            with self.subTest(corrupt=corrupt):
                result = self.run_matrix(corrupt=corrupt)
                self.assertFalse(result['measurement_complete'])
                self.assertEqual(len(result['results']), 1)


if __name__ == '__main__':
    unittest.main()
