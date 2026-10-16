"""Reject a companion package or manual receipt from another native build."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import zipfile

SPEC = importlib.util.spec_from_file_location('release', Path(__file__).parents[1] / 'scripts/verify-release-evidence.py')
V = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(V)
STAGE_SPEC = importlib.util.spec_from_file_location('stage_terminal', Path(__file__).parents[1] / 'scripts/stage-terminal-release-asset.py')
STAGE = importlib.util.module_from_spec(STAGE_SPEC)
STAGE_SPEC.loader.exec_module(STAGE)


class TerminalDependencyTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.package = self.root / 'terminal.zip'
        prefix = 'bin/WindowsTerminal/'
        self.exe = b'MZ synthetic Terminal fixture'
        exe_hash = hashlib.sha256(self.exe).hexdigest().upper()
        licenses = {prefix + name: b'synthetic license' for name in (
            'BLUEBERRY-PATCH-LICENSE.txt', 'TERMINAL-LICENSE.txt', 'TERMINAL-THIRD-PARTY-NOTICES.md')}
        with zipfile.ZipFile(self.package, 'w') as archive:
            archive.writestr(prefix + 'WindowsTerminal.exe', self.exe)
            archive.writestr(prefix + '.portable', b'')
            archive.writestr(prefix + 'Microsoft.Terminal.Settings.Model.dll', b'fixture')
            for name, data in licenses.items(): archive.writestr(name, data)
        package_hash = V.sha256_file(self.package)
        source = 'b' * 40
        ci = {
            'run': {'id': 42, 'head_sha': source, 'conclusion': 'success', 'path': '.github/workflows/wt-overlay-prototype.yml'},
            'artifact': {'id': 43, 'workflow_run': {'id': 42, 'head_sha': source}, 'digest': 'sha256:' + package_hash.lower()},
            'jobs': [{'name': 'native-buffer-proof', 'run_id': 42, 'conclusion': 'success'}],
        }
        runtime = {'diagnostic_only': True, 'pixel_visibility_measured': False, 'results': [
            {'kind': 'native_menu', 'exit_code': 0, 'terminal_sha256': exe_hash,
             'result': {'supported': True, 'interface_version': 4, 'reader_stopped': True, 'output_mode_restored': True}}]}
        self.dependency = {
            'interface_version': 4, 'package_sha256': package_hash, 'source_commit': source,
            'upstream_commit': '7ff11ee107c984e236960ec948b450ccbc10187b', 'run_id': 42, 'artifact_id': 43,
            'executable_member': prefix + 'WindowsTerminal.exe', 'executable_sha256': exe_hash,
            'licenses': {name: hashlib.sha256(data).hexdigest().upper() for name, data in licenses.items()},
        }
        model = self.root / 'model.zip'
        with zipfile.ZipFile(model, 'w') as archive:
            archive.writestr('Blueberry.NativeModel.dll', b'synthetic observer')
        model_hash = V.sha256_file(model)
        ci['native_model_artifact'] = {'id': 44, 'workflow_run': {'id': 42, 'head_sha': source}, 'digest': 'sha256:' + model_hash.lower()}
        self.dependency['native_model'] = {
            'artifact_id': 44, 'package': model.name, 'package_sha256': model_hash,
            'dll_member': 'Blueberry.NativeModel.dll', 'dll_sha256': hashlib.sha256(b'synthetic observer').hexdigest().upper()}
        for field, value in [('ci_report', ci), ('runtime_report', runtime)]:
            path = self.root / (field + '.json')
            path.write_text(json.dumps(value), encoding='utf-8')
            self.dependency[field] = {'report': path.name, 'sha256': V.sha256_file(path)}
        self.evidence = {'terminal_dependency': self.dependency, 'manual_acceptance': {'windows_terminal_matrix': {
            profile: {'terminal_package_sha256': package_hash, 'terminal_executable_sha256': exe_hash} for profile in V.PROFILES}}}

    def check(self):
        V.check_terminal_dependency(self.evidence, self.root, self.package)

    def test_original_package_and_bound_manual_receipts_pass(self):
        self.check()

    def test_missing_dependency_or_package_is_rejected(self):
        with self.assertRaisesRegex(ValueError, 'package is required'):
            V.check_terminal_dependency(self.evidence, self.root, None)
        with self.assertRaisesRegex(ValueError, 'fixed Terminal dependency'):
            V.check_terminal_dependency({}, self.root, self.package)

    def test_changed_package_or_executable_is_rejected(self):
        for field in ('package_sha256', 'executable_sha256'):
            with self.subTest(field=field):
                original = self.dependency[field]
                self.dependency[field] = '0' * 64
                with self.assertRaises(ValueError): self.check()
                self.dependency[field] = original

    def test_wrong_source_run_artifact_license_or_interface_is_rejected(self):
        for field, value in [('source_commit', 'c' * 40), ('run_id', 99), ('artifact_id', 99),
                             ('licenses', {}), ('interface_version', 3)]:
            with self.subTest(field=field):
                original = self.dependency[field]
                self.dependency[field] = value
                with self.assertRaises(ValueError): self.check()
                self.dependency[field] = original

    def test_runtime_from_another_executable_is_rejected(self):
        record = self.dependency['runtime_report']
        path = self.root / record['report']
        value = json.loads(path.read_text())
        value['results'][0]['terminal_sha256'] = '0' * 64
        path.write_text(json.dumps(value), encoding='utf-8')
        record['sha256'] = V.sha256_file(path)
        with self.assertRaisesRegex(ValueError, 'another EXE'): self.check()

    def test_manual_acceptance_cannot_be_inherited_from_another_terminal(self):
        original = copy.deepcopy(self.evidence['manual_acceptance'])
        for profile in V.PROFILES:
            self.evidence['manual_acceptance'] = copy.deepcopy(original)
            self.evidence['manual_acceptance']['windows_terminal_matrix'][profile]['terminal_executable_sha256'] = '0' * 64
            with self.subTest(profile=profile), self.assertRaisesRegex(ValueError, 'another Terminal package'):
                self.check()

    def test_legacy_or_unbound_native_observer_cannot_qualify(self):
        path = self.root / 'startup.json'
        self.evidence['startup_reports'] = {'fixture': path.name}
        path.write_text(json.dumps({'native_observers': []}), encoding='utf-8')
        with self.assertRaisesRegex(ValueError, 'observation boundary'): self.check()
        observer = {'dll_sha256': self.dependency['native_model']['dll_sha256'], 'abi_version': 1,
                    'companion_interface_version': 4, 'backend': 'production_terminalcore_buffer',
                    'snapshot_binding': 'per_received_chunk', 'screen_pixels': 'not_measured'}
        path.write_text(json.dumps({'native_observers': [observer]}), encoding='utf-8')
        self.check()
        observer['snapshot_binding'] = 'live_viewport'
        path.write_text(json.dumps({'native_observers': [observer]}), encoding='utf-8')
        with self.assertRaisesRegex(ValueError, 'observation boundary'): self.check()

    def test_stage_preserves_original_bytes_and_writes_checksum(self):
        self.dependency['asset_name'] = 'blueberry-terminal-fixture.zip'
        calls = []
        original = self.package.read_bytes()
        def download(descriptor, repository, token, destination, source):
            calls.append((descriptor['artifact_id'], descriptor['run_id'], source))
            destination.write_bytes(original)
        path = STAGE.stage(self.dependency, self.root / 'assets', download, 'fixture/repo', 'fixture-token')
        self.assertEqual(path.read_bytes(), original)
        self.assertEqual(calls, [(43, 42, 'b' * 40)])
        self.assertEqual(path.with_name(path.name + '.sha256').read_text().split()[0], self.dependency['package_sha256'])
        with self.assertRaisesRegex(ValueError, 'already exists'):
            STAGE.stage(self.dependency, self.root / 'assets', download, 'fixture/repo', 'fixture-token')

    def test_current_candidate_cannot_use_unpublished_legacy_setup(self):
        observer = {'dll_sha256': self.dependency['native_model']['dll_sha256'], 'abi_version': 1,
                    'companion_interface_version': 4, 'backend': 'production_terminalcore_buffer',
                    'snapshot_binding': 'per_received_chunk', 'screen_pixels': 'not_measured'}
        group = {'native_observers': [observer], 'hot_preparation_status': [{'source': 'adapter_json'}]}
        report = {'scenarios': [{'cache_miss': copy.deepcopy(group), 'cache_hit': copy.deepcopy(group)}]}
        path = self.root / 'hot.json'
        self.evidence['hot_reports'] = {'fixture': {'variant': path.name}}
        path.write_text(json.dumps(report), encoding='utf-8')
        self.check()
        report['scenarios'][0]['cache_hit']['hot_preparation_status'][0]['source'] = 'legacy_unpublished_status'
        path.write_text(json.dumps(report), encoding='utf-8')
        with self.assertRaisesRegex(ValueError, 'unpublished legacy status'): self.check()

    def test_failed_download_digest_never_becomes_a_release_asset(self):
        self.dependency['asset_name'] = 'blueberry-terminal-fixture.zip'
        def download(descriptor, repository, token, destination, source):
            destination.write_bytes(b'wrong artifact')
        output = self.root / 'assets'
        with self.assertRaisesRegex(ValueError, 'SHA-256 mismatch'):
            STAGE.stage(self.dependency, output, download, 'fixture/repo', 'fixture-token')
        self.assertFalse((output / self.dependency['asset_name']).exists())


if __name__ == '__main__': unittest.main()
