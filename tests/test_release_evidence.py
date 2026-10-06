"""Checks that a release cannot pass with missing, slow or mismatched evidence."""

import hashlib
import importlib.util
import json
import os
import subprocess
import sys
import textwrap
import shutil
import unittest
import uuid
import zipfile
from pathlib import Path

MODULE_PATH = Path(__file__).resolve().parents[1] / "scripts/verify-release-evidence.py"
SPEC = importlib.util.spec_from_file_location("release_evidence", MODULE_PATH)
validator = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(validator)


def statistics(value, count):
    return {"samples": [value] * count, "median": value, "p95": value, "unit": "ms"}


class ReleaseEvidenceTests(unittest.TestCase):
    def test_real_ci_artifact_layout_preserves_bytes_and_rejects_duplicates(self):
        workflow = (MODULE_PATH.parent.parent / '.github/workflows/release.yml').read_text(encoding='utf-8')
        section = workflow.split('name: Normalize the original CI artifact layout', 1)[1]
        code = textwrap.dedent(section.split("python3 - <<'PY'", 1)[1].split('\n          PY', 1)[0])
        assets = self.root / 'release-assets'
        nested = assets / 'dist/ci'
        nested.mkdir(parents=True)
        tag = 'v' + validator.VERSION
        names = [f'blueberry-{tag}-windows-x64.zip', f'blueberry-{tag}-windows-x64.zip.sha256', f'blueberry-{tag}-release.json']
        for index, name in enumerate(names):
            (nested / name).write_bytes(bytes([index, 0, 255]))
        (assets / 'install.ps1').write_bytes(b'original installer')
        environment = dict(os.environ, RELEASE_TAG=tag)
        for _ in range(2):
            result = subprocess.run([sys.executable, '-c', code], cwd=self.root, env=environment, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            for index, name in enumerate(names):
                self.assertEqual((assets / name).read_bytes(), bytes([index, 0, 255]))
            self.assertEqual((assets / 'install.ps1').read_bytes(), b'original installer')
        (nested / names[0]).write_bytes(b'ambiguous')
        self.assertNotEqual(subprocess.run([sys.executable, '-c', code], cwd=self.root, env=environment, capture_output=True).returncode, 0)
        self.assertEqual((assets / names[0]).read_bytes(), bytes([0, 0, 255]))
        (nested / names[0]).unlink()
        (assets / names[0]).unlink()
        self.assertNotEqual(subprocess.run([sys.executable, '-c', code], cwd=self.root, env=environment, capture_output=True).returncode, 0)

    @unittest.skipIf(os.name == 'nt', 'Release shell simulation runs in the required Linux format job')
    def test_draft_creation_retry_and_published_release_protection(self):
        workflow = (MODULE_PATH.parent.parent / '.github/workflows/release.yml').read_text(encoding='utf-8')
        code = textwrap.dedent(workflow.split('name: Create or update unpublished draft with the measured bytes', 1)[1].split('run: |', 1)[1])
        binary = self.root / 'bin'
        binary.mkdir()
        gh = binary / 'gh'
        gh.write_text('''#!/usr/bin/env bash
set -euo pipefail
echo "$*" >> "$MOCK_ROOT/calls"
case "$1 $2" in
  'release view') test -f "$MOCK_ROOT/state" || exit 1; cat "$MOCK_ROOT/state" ;;
  'release create') [[ " $* " == *" --draft "* ]]; echo '{"isDraft":true}' > "$MOCK_ROOT/state" ;;
  'release upload') cp "$4" "$MOCK_ROOT/remote/$(basename "$4")" ;;
  'release download') cp "$MOCK_ROOT"/remote/* "$MOCK_ROOT/draft-verification/" ;;
  *) exit 90 ;;
esac
''', encoding='utf-8')
        gh.chmod(0o755)
        (self.root / 'remote').mkdir()
        assets = self.root / 'release-assets'
        assets.mkdir()
        (assets / 'package.zip').write_bytes(b'immutable CI bytes')
        (self.root / 'release-notes.md').write_text('notes')
        environment = dict(os.environ, PATH=str(binary)+os.pathsep+os.environ['PATH'],
                           MOCK_ROOT=str(self.root), RELEASE_TAG='v'+validator.VERSION)
        for _ in range(2):
            result = subprocess.run(['bash','-c',code], cwd=self.root, env=environment, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual((self.root / 'draft-verification/package.zip').read_bytes(), b'immutable CI bytes')
        (self.root / 'state').write_text('{"isDraft":false}')
        (self.root / 'calls').write_text('')
        result = subprocess.run(['bash','-c',code], cwd=self.root, env=environment, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn('release upload', (self.root / 'calls').read_text())
        self.assertNotIn('release create', (self.root / 'calls').read_text())

    def test_tag_and_manual_resolution_use_exact_evidence_run(self):
        workflow = (MODULE_PATH.parent.parent / '.github/workflows/release.yml').read_text(encoding='utf-8')
        section = workflow.split('name: Resolve exact CI run from pinned version evidence', 1)[1]
        code = textwrap.dedent(section.split("python3 - <<'PY'", 1)[1].split('\n          PY', 1)[0])
        fixture = {"version": validator.VERSION, "source_commit": self.commit, "ci": {"run_id": 123}}
        evidence = self.root / 'pinned-evidence.json'
        output = self.root / 'env'
        environment = dict(os.environ, RELEASE_TAG='v'+validator.VERSION, TAG_SHA=self.commit,
                           EVIDENCE_REF='b'*40, GITHUB_ENV=str(output))
        for manual, version, commit, passed in [('', validator.VERSION, self.commit, True),
                ('123', validator.VERSION, self.commit, True), ('456', validator.VERSION, self.commit, False),
                ('', '99.0.0', self.commit, False), ('', validator.VERSION, 'c'*40, False)]:
            fixture.update(version=version, source_commit=commit)
            evidence.write_text(json.dumps(fixture), encoding='utf-8')
            environment['MANUAL_CI_RUN_ID'] = manual
            result = subprocess.run([sys.executable, '-c', code], cwd=self.root, env=environment,
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode == 0, passed, result.stderr)
            if passed:
                self.assertIn('CI_RUN_ID=123\n', output.read_text())
        evidence.unlink()
        self.assertNotEqual(subprocess.run([sys.executable, '-c', code], cwd=self.root, env=environment,
                                          capture_output=True).returncode, 0)

    def setUp(self):
        scratch = Path(__file__).resolve().parents[1] / "target/release-evidence-tests"
        scratch.mkdir(parents=True, exist_ok=True)
        self.root = scratch / uuid.uuid4().hex
        self.root.mkdir()
        self.addCleanup(shutil.rmtree, self.root, ignore_errors=True)
        self.commit = "a" * 40
        self.conpty = validator.read_json(MODULE_PATH.parent.parent / "vendor/conpty/upstream.json")
        self.runtime = {**self.conpty, "mode": "pinned"}
        self.exe = b"MZ synthetic release fixture"
        self.exe_hash = hashlib.sha256(self.exe).hexdigest().upper()
        self.beta6_exe = b"MZ synthetic public beta.6 fixture"
        self.beta6_hash = hashlib.sha256(self.beta6_exe).hexdigest().upper()
        self.beta6_package = self.root / "blueberry-v0.5.0-beta.6-windows-x64.zip"
        with zipfile.ZipFile(self.beta6_package, "w") as archive:
            archive.writestr("blueberry.exe", self.beta6_exe)
        self.package = self.root / f"blueberry-v{validator.VERSION}-windows-x64.zip"
        editors = validator.read_json(MODULE_PATH.parent.parent / 'vendor/psreadline/upstream.json')
        editors['files'] = [{"version": version, "path": path, "sha256": 'F'*64} for version, path in
                            [('2.0.0', 'Microsoft.PowerShell.PSReadLine2.dll'), ('2.4.5', 'Microsoft.PowerShell.PSReadLine.dll')]]
        manifest = {
            "version": validator.VERSION,
            "platform": "windows-x64",
            "build": {"default_host_mode": "direct", "conpty": self.conpty, "commit":self.commit,
                      "dirty":False, "profile":"release", "private_editors":editors},
            "files": [{"path": "blueberry.exe", "sha256": self.exe_hash}],
        }
        with zipfile.ZipFile(self.package, "w") as archive:
            archive.writestr("blueberry.exe", self.exe)
            archive.writestr("release.json", json.dumps(manifest))
        self.external_manifest = self.root / f"blueberry-v{validator.VERSION}-release.json"
        self.external_manifest.write_text(json.dumps(manifest), encoding="utf-8")
        self.evidence = {
            "schema": 2,
            "version": validator.VERSION,
            "source_commit": self.commit,
            "build_identity": manifest['build'],
            "package_sha256": validator.sha256_file(self.package),
            "executable_sha256": self.exe_hash,
            "startup_reports": {},
            "hot_reports": {},
            "comparison_reports": {},
            "nested_compatibility_reports": {},
            "environments": {},
            "ci": {"source_commit": self.commit, "status": "success", "branch": "main", "event": "push", "run_id": 123, "package_sha256": validator.sha256_file(self.package)},
            "manual_acceptance": {
                "executable_sha256": self.exe_hash,
                "source_commit": self.commit,
                "source_dirty": False,
                "installation": {task: True for task in validator.INSTALL_TASKS},
                "windows_terminal": {task: True for task in validator.TERMINAL_TASKS},
                "user_trials": [
                    {
                        "id": f"participant-{index}",
                        "executable_sha256": self.exe_hash,
                        "tasks": {task: True for task in validator.USER_TASKS},
                    }
                    for index in range(8)
                ],
                "public_beta6_sha256": self.beta6_hash,
                "inshellisense_sha256": "B" * 64,
            },
        }
        for profile, (shell, shell_major, psreadline) in validator.PROFILES.items():
            environment = {"profile_sha256": "D" * 64, "machine_id": "fixed-test-machine", "power_policy": "fixed-test-policy", "terminal_rows": 30, "terminal_columns": 120}
            self.evidence["environments"][profile] = environment
            shell_version = shell_major + "1.0"
            order = list(validator.COMPARISON_MODES)
            rotated = lambda index: order[index % 4:] + order[:index % 4]
            comparison = {
                "probe_conpty": self.runtime,
                "schema": 2,
                **environment,
                "profile": profile,
                "source_commit": self.commit,
                "source_dirty": False,
                "shell": "C:\\Windows\\" + shell,
                "shell_version": shell_version,
                "psreadline_version": psreadline,
                "profile_mode": "with_profile",
                "trace": "disabled",
                "terminal_rows": 30,
                "terminal_columns": 120,
                "os_cache_cleared": False,
                "machine_id": "fixed-test-machine",
                "power_policy": "fixed-test-policy",
                "startup_pairs": 30,
                "hot_samples_per_mode": 300,
                "startup_orders": [rotated(index) for index in range(30)],
                "hot_session_orders": {
                    name: {
                        cache_mode: [rotated(index) for index in range(30)]
                        for cache_mode in ("cache_hit", "cache_miss")
                    }
                    for name in validator.SCENARIOS
                },
                "modes": {
                    mode: {
                        "sha256": {
                            "plain": None,
                            "beta6": self.beta6_hash,
                            "candidate": self.exe_hash,
                            "inshellisense": "B" * 64,
                        }[mode],
                        "transport": {"beta6": "osc", "candidate": "pipe"}.get(mode, "not_applicable"),
                        "host_mode": {"beta6": "nested", "candidate": "direct"}.get(mode, "not_applicable"),
                        "entry": {"path": mode + ".exe", "sha256": {"plain": "E" * 64, "beta6": self.beta6_hash, "candidate": self.exe_hash, "inshellisense": "B" * 64}[mode]},
                        "dependencies": [{"path": "PSReadLine.dll", "sha256": "F" * 64}],
                        "transport_degraded": False if mode in ("beta6", "candidate") else None,
                        "startup": statistics(10.0, 30),
                        "hot": {
                            name: {
                                cache_mode: {"input_echo": statistics(10.0, 300), "menu": None if mode == "plain" else statistics(10.0, 300), "cache_capability": "not_applicable" if mode == "plain" else "supported"}
                                for cache_mode in ("cache_hit", "cache_miss")
                            }
                            for name in validator.SCENARIOS
                        },
                    }
                    for mode in validator.COMPARISON_MODES
                },
            }
            comparison_name = f"comparison-{profile}.json"
            (self.root / comparison_name).write_text(json.dumps(comparison), encoding="utf-8")
            self.evidence["comparison_reports"][profile] = comparison_name
            startup = {
                "probe_conpty": self.runtime, "product_conpty": self.conpty,
                "schema": 3,
                "measurement": "complete_product",
                "source_commit": self.commit,
                "source_dirty": False,
                **environment,
                "trace": "disabled", "os_cache_cleared": False,
                "host_mode": "direct", "actual_host_modes": ["direct"] * 30, "actual_transports": ["pipe"] * 30,
                "build": "release",
                "platform": "windows",
                "arch": "x86_64",
                "executable_sha256": self.exe_hash,
                "shell": "C:\\Windows\\" + shell,
                "adapter_source": "embedded",
                "profile_mode": "with_profile",
                "no_profile": False,
                "iterations": 30,
                "shell_versions": [shell_version] * 30,
                "psreadline_versions": [psreadline] * 30,
                "paired_first_input_delta": statistics(10.0, 30),
                "plain_first_input": statistics(100.0, 30), "candidate_first_input": statistics(110.0, 30),
                "first_key_echo": statistics(10.0, 30), "first_static_candidate": statistics(10.0, 30),
                "first_dynamic_candidate": statistics(10.0, 30), "actual_automatic_menu": [True] * 30,
                "startup_orders": [["plain", "candidate"] if index % 2 == 0 else ["candidate", "plain"] for index in range(30)],
            }
            startup_name = f"startup-{profile}.json"
            (self.root / startup_name).write_text(json.dumps(startup), encoding="utf-8")
            self.evidence["startup_reports"][profile] = startup_name
            self.evidence["hot_reports"][profile] = {}
            for variant, (transport, descriptions) in validator.VARIANTS.items():
                scenarios = []
                for name in validator.SCENARIOS:
                    measurement = {
                        "shell_versions": [shell_version] * 30,
                        "psreadline_versions": [psreadline] * 30,
                        "actual_transport": [transport] * 30,
                        "actual_host_mode": ["direct"] * 30, "automatic_menu": [True] * 30,
                    }
                    acceptance = {
                        "status": "passed",
                        "observed_samples": 300,
                        "expected_samples": 300,
                        "statistics": statistics(10.0, 300),
                    }
                    scenarios.append({
                        "name": name,
                        "cache_miss": measurement,
                        "cache_hit": measurement,
                        "acceptance": {"cache_miss": acceptance, "cache_hit": acceptance},
                    })
                hot = {
                    "probe_conpty": self.runtime, "product_conpty": self.conpty,
                    "schema": 2,
                    "source_commit": self.commit, **environment,
                    "source_dirty": False,
                    "host_mode": "direct", "os_cache_cleared": False,
                    "build": "release",
                    "platform": "windows",
                    "arch": "x86_64",
                    "executable_sha256": self.exe_hash,
                    "shell": "C:\\Windows\\" + shell,
                    "transport": transport,
                    "descriptions": descriptions,
                    "trace": "disabled",
                    "profile_mode": "preserved",
                    "transport_degraded": False,
                    "target": {"status": "passed"},
                    "samples_per_cache_mode": 300,
                    "scenarios": scenarios,
                }
                hot_name = f"hot-{profile}-{variant}.json"
                (self.root / hot_name).write_text(json.dumps(hot), encoding="utf-8")
                self.evidence["hot_reports"][profile][variant] = hot_name
            self.evidence["nested_compatibility_reports"][profile] = {}
            for variant, transport in validator.NESTED_VARIANTS.items():
                nested = {
                    "probe_conpty": self.runtime, "product_conpty": self.conpty,
                    "source_commit": self.commit, **environment,
                    "source_dirty": False,
                    "build": "release", "platform": "windows", "arch": "x86_64",
                    "executable_sha256": self.exe_hash, "shell": "C:\\Windows\\" + shell,
                    "host_mode": "nested", "transport": transport, "transport_degraded": False,
                    "trace": "disabled", "os_cache_cleared": False,
                    "correctness_status": "passed", "samples_per_group": 30,
                    "groups": {name: {mode: statistics(32.0, 30) for mode in ("cache_hit", "cache_miss")} for name in validator.SCENARIOS},
                }
                filename = f"nested-{profile}-{variant}.json"
                (self.root / filename).write_text(json.dumps(nested), encoding="utf-8")
                self.evidence["nested_compatibility_reports"][profile][variant] = filename
        self.evidence_path = self.root / "evidence.json"
        self.save_evidence()

    def save_evidence(self):
        self.evidence_path.write_text(json.dumps(self.evidence), encoding="utf-8")

    def test_complete_matching_release_passes(self):
        self.assertEqual(
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package), self.exe_hash
        )

    def test_measured_private_dll_identity_mismatch_blocks_release(self):
        self.evidence['build_identity']['private_editors']['files'][0]['sha256'] = 'E'*64
        self.save_evidence()
        with self.assertRaisesRegex(ValueError, 'private dependency identity'):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)

    def test_wrong_frozen_build_and_missing_private_editor_matrix_block_release(self):
        original = json.loads(self.external_manifest.read_text(encoding='utf-8'))
        for mutation, error in [('commit', 'frozen release source'), ('editors', 'DLL matrix')]:
            manifest = json.loads(json.dumps(original))
            if mutation == 'commit':
                manifest['build']['commit'] = 'b'*40
            else:
                manifest['build']['private_editors']['files'].pop()
            self.evidence['build_identity'] = manifest['build']
            raw = json.dumps(manifest)
            with zipfile.ZipFile(self.package, 'w') as archive:
                archive.writestr('blueberry.exe', self.exe)
                archive.writestr('release.json', raw)
            self.external_manifest.write_text(raw, encoding='utf-8')
            self.evidence['package_sha256'] = validator.sha256_file(self.package)
            self.evidence['ci']['package_sha256'] = self.evidence['package_sha256']
            self.save_evidence()
            with self.assertRaisesRegex(ValueError, error):
                validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)

    def test_performance_deferral_preserves_functional_and_identity_gates(self):
        self.evidence["performance_waiver"] = {
            "version": validator.VERSION, "status": "deferred_by_user", "passed": False,
            "authorization": "如果没有bug先直接发版吧，性能以后再优化",
            "limitations": "Startup target missed; full performance matrix deferred.",
        }
        for field in ("startup_reports", "hot_reports", "comparison_reports", "nested_compatibility_reports"):
            self.evidence[field] = {}
        names = ["format", "Core / ubuntu-24.04", "Core / macos-15", "Core / windows-2022",
                 "Native ConPTY mouse / windows-2025", "Windows PowerShell 5.1 / PSReadLine 2.0.0",
                 "Windows PowerShell 5.1 / PSReadLine 2.4.5", "PowerShell 7 / PSReadLine 2.4.5 / osc",
                 "PowerShell 7 / PSReadLine 2.4.5 / pipe", "package"]
        data = {"run": {"id": 123, "head_sha": self.commit, "head_branch": "main",
                        "event": "push", "name": "Blueberry CI", "conclusion": "success"},
                "jobs": [{"name": name, "conclusion": "success"} for name in names]}
        path = self.root / "functional-ci.json"
        def save():
            path.write_text(json.dumps(data), encoding="utf-8")
            self.evidence["functional_ci"] = {"report": path.name, "sha256": validator.sha256_file(path)}
            self.save_evidence()
        save()
        self.assertEqual(validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package), self.exe_hash)
        validator.bundle(self.evidence_path, self.root / "evidence.zip")
        with zipfile.ZipFile(self.root / "evidence.zip") as archive:
            self.assertIn(path.name, archive.namelist())
        data["jobs"][-1]["conclusion"] = "failure"
        save()
        with self.assertRaisesRegex(ValueError, "CI matrix"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)
        data["jobs"][-1]["conclusion"] = "success"
        self.evidence["manual_acceptance"]["windows_terminal"]["ime"] = False
        save()
        with self.assertRaisesRegex(ValueError, "windows_terminal"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)
        self.evidence["manual_acceptance"]["windows_terminal"]["ime"] = True
        self.evidence["performance_waiver"]["passed"] = True
        save()
        with self.assertRaisesRegex(ValueError, "cannot claim a pass"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)
        self.evidence["performance_waiver"]["passed"] = False
        manual = self.evidence["manual_acceptance"]
        manual["windows_terminal"] = {}
        manual["windows_terminal_waiver"] = {
            "version": validator.VERSION, "status": "waived_by_user", "executed": False, "passed": False,
            "authorization": "本次也免除人工检查，按自动回归结果发布",
        }
        save()
        self.assertEqual(validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package), self.exe_hash)
        manual["installation"]["rollback"] = False
        save()
        with self.assertRaisesRegex(ValueError, "installation"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)
        manual["installation"]["rollback"] = True
        manual["windows_terminal_waiver"]["executed"] = True
        save()
        with self.assertRaisesRegex(ValueError, "non-execution"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)

    def test_conpty_fallback_or_changed_runtime_blocks_release(self):
        path = self.root / self.evidence["startup_reports"]["ps51-2.0.0"]
        report = validator.read_json(path)
        report["probe_conpty"]["mode"] = "system"
        path.write_text(json.dumps(report), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "verified probe ConPTY missing"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)
        report["probe_conpty"]["mode"] = "pinned"
        report["probe_conpty"]["sha256"] = "0" * 64
        path.write_text(json.dumps(report), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "probe ConPTY identity mismatch"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)

    def test_ci_record_must_match_the_downloaded_run(self):
        run_id=self.evidence["ci"]["run_id"]
        validator.verify(self.evidence_path,self.package,self.commit,self.beta6_package,run_id)
        with self.assertRaisesRegex(ValueError,"different CI run"):
            validator.verify(self.evidence_path,self.package,self.commit,self.beta6_package,run_id+1)

    def test_explicit_user_waiver_does_not_waive_terminal_or_install(self):
        manual = self.evidence["manual_acceptance"]
        manual["user_trials"] = []
        manual["user_trial_waiver"] = {"status": "waived_by_user", "version": validator.VERSION,
                                      "executed": False, "reason": "User explicitly skips target user trials for beta.7."}
        self.save_evidence()
        validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)
        manual["windows_terminal"]["ime"] = False
        self.save_evidence()
        with self.assertRaisesRegex(ValueError, "incomplete manual acceptance"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)

    def test_waiver_cannot_claim_pass_or_execution(self):
        manual = self.evidence["manual_acceptance"]
        manual["user_trials"] = []
        manual["user_trial_waiver"] = {"status": "passed", "version": validator.VERSION, "executed": False, "reason": "skipped"}
        self.save_evidence()
        with self.assertRaisesRegex(ValueError, "explicitly record"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)

    def test_nested_32_ms_is_reported_without_failing_direct_gate(self):
        validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)
        del self.evidence["nested_compatibility_reports"]["ps7-2.4.5"]["osc_without_descriptions"]
        self.save_evidence()
        with self.assertRaisesRegex(ValueError, "nested compatibility variants incomplete"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)

    def test_profile_or_source_change_invalidates_reports(self):
        path = self.root / self.evidence["hot_reports"]["ps7-2.4.5"]["pipe_with_descriptions"]
        report = json.loads(path.read_text(encoding="utf-8"))
        report["profile_sha256"] = "A" * 64
        path.write_text(json.dumps(report), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "environment changed"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)
        report["profile_sha256"] = "D" * 64
        report["source_commit"] = "b" * 40
        path.write_text(json.dumps(report), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "source commit mismatch"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)

    def test_false_plain_menu_and_adapter_only_startup_are_rejected(self):
        path = self.root / self.evidence["comparison_reports"]["ps7-2.4.5"]
        report = json.loads(path.read_text(encoding="utf-8"))
        report["modes"]["plain"]["hot"]["git"]["cache_hit"]["menu"] = statistics(10.0, 300)
        path.write_text(json.dumps(report), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "not applicable"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)
        path = self.root / self.evidence["startup_reports"]["ps7-2.4.5"]
        report = json.loads(path.read_text(encoding="utf-8"))
        report["measurement"] = "adapter_only"
        path.write_text(json.dumps(report), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "complete product"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)

    def test_disabled_direct_menu_and_unsuccessful_ci_reject_release(self):
        path = self.root / self.evidence["hot_reports"]["ps7-2.4.5"]["pipe_with_descriptions"]
        report = json.loads(path.read_text(encoding="utf-8"))
        report["scenarios"][0]["cache_hit"]["automatic_menu"][0] = False
        path.write_text(json.dumps(report), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "automatic direct menu unavailable"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)
        self.evidence["ci"]["status"] = "failure"
        self.save_evidence()
        with self.assertRaisesRegex(ValueError, "successful frozen main CI"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)

    def test_slow_startup_and_missing_variant_block_release(self):
        startup_path = self.root / self.evidence["startup_reports"]["ps7-2.4.5"]
        startup = json.loads(startup_path.read_text(encoding="utf-8"))
        startup["paired_first_input_delta"] = statistics(50.1, 30)
        startup["candidate_first_input"] = statistics(150.1, 30)
        startup_path.write_text(json.dumps(startup), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "exceeds 50"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)
        startup["paired_first_input_delta"] = statistics(10.0, 30)
        startup["candidate_first_input"] = statistics(110.0, 30)
        startup_path.write_text(json.dumps(startup), encoding="utf-8")
        del self.evidence["hot_reports"]["ps7-2.4.5"]["pipe_with_descriptions"]
        self.save_evidence()
        with self.assertRaisesRegex(ValueError, "variants incomplete"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)

    def test_package_or_report_hash_mismatch_blocks_release(self):
        self.evidence["executable_sha256"] = "B" * 64
        self.save_evidence()
        with self.assertRaisesRegex(ValueError, "EXE SHA-256 mismatch"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)
        self.evidence["executable_sha256"] = self.exe_hash
        self.evidence["package_sha256"] = "B" * 64
        self.save_evidence()
        with self.assertRaisesRegex(ValueError, "package SHA-256 mismatch"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)

    def test_startup_summary_cannot_hide_slow_product_or_missing_first_key(self):
        path = self.root / self.evidence["startup_reports"]["ps7-2.4.5"]
        report = json.loads(path.read_text(encoding="utf-8"))
        report["candidate_first_input"] = statistics(300.0, 30)
        path.write_text(json.dumps(report), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "delta disagrees"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)
        report["candidate_first_input"] = statistics(110.0, 30)
        report["first_key_echo"]["samples"].pop()
        path.write_text(json.dumps(report), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "expected 30 samples"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)

    def test_startup_requires_alternation_and_enabled_menu(self):
        path = self.root / self.evidence["startup_reports"]["ps7-2.4.5"]
        report = json.loads(path.read_text(encoding="utf-8"))
        report["startup_orders"][1] = ["plain", "candidate"]
        path.write_text(json.dumps(report), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "not alternating"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)
        report["startup_orders"][1] = ["candidate", "plain"]
        report["actual_automatic_menu"][0] = False
        path.write_text(json.dumps(report), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "menu unavailable"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)

    def test_slow_hot_group_blocks_release(self):
        hot_path = self.root / self.evidence["hot_reports"]["ps7-2.4.5"]["pipe_with_descriptions"]
        hot = json.loads(hot_path.read_text(encoding="utf-8"))
        hot["scenarios"][0]["acceptance"]["cache_hit"]["statistics"] = statistics(20.1, 300)
        hot_path.write_text(json.dumps(hot), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "exceeds 20"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)

    def test_unlisted_package_file_blocks_release(self):
        with zipfile.ZipFile(self.package, "a") as archive:
            archive.writestr("unexpected.txt", "extra")
        self.evidence["package_sha256"] = validator.sha256_file(self.package)
        self.evidence["ci"]["package_sha256"] = self.evidence["package_sha256"]
        self.save_evidence()
        with self.assertRaisesRegex(ValueError, "manifest file list mismatch"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)

    def test_missing_manual_task_blocks_release(self):
        self.evidence["manual_acceptance"]["user_trials"][0]["tasks"]["exit_restore"] = False
        self.save_evidence()
        with self.assertRaisesRegex(ValueError, "incomplete tasks"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)

    def test_manual_acceptance_from_another_exe_blocks_release(self):
        self.evidence["manual_acceptance"]["user_trials"][0]["executable_sha256"] = "C" * 64
        self.save_evidence()
        with self.assertRaisesRegex(ValueError, "wrong EXE"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)

    def test_external_manifest_must_match_packaged_bytes(self):
        self.external_manifest.write_text('{"version":"changed"}', encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "manifests differ"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)

    def test_report_paths_reject_windows_and_parent_traversal(self):
        self.evidence["startup_reports"]["ps7-2.4.5"] = "..\\outside.json"
        self.save_evidence()
        with self.assertRaisesRegex(ValueError, "safe relative"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)
        self.evidence["startup_reports"]["ps7-2.4.5"] = "../outside.json"
        self.save_evidence()
        with self.assertRaisesRegex(ValueError, "safe relative"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)

    def test_missing_or_mismatched_comparison_blocks_release(self):
        del self.evidence["comparison_reports"]["ps7-2.4.5"]
        self.save_evidence()
        with self.assertRaisesRegex(ValueError, "comparison profile matrix incomplete"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)
        self.evidence["comparison_reports"]["ps7-2.4.5"] = "comparison-ps7-2.4.5.json"
        comparison_path = self.root / self.evidence["comparison_reports"]["ps7-2.4.5"]
        comparison = json.loads(comparison_path.read_text(encoding="utf-8"))
        comparison["modes"]["beta6"]["sha256"] = "C" * 64
        comparison_path.write_text(json.dumps(comparison), encoding="utf-8")
        self.save_evidence()
        with self.assertRaisesRegex(ValueError, "binary hash mismatch"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)

    def test_public_beta6_bytes_must_match_recorded_release(self):
        with zipfile.ZipFile(self.beta6_package, "w") as archive:
            archive.writestr("blueberry.exe", b"MZ altered beta.6")
        with self.assertRaisesRegex(ValueError, "public beta.6 EXE hash mismatch"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)

    def test_missing_comparison_samples_or_false_transport_blocks_release(self):
        comparison_path = self.root / self.evidence["comparison_reports"]["ps7-2.4.5"]
        comparison = json.loads(comparison_path.read_text(encoding="utf-8"))
        comparison["modes"]["plain"]["hot"]["git"]["cache_hit"]["input_echo"]["samples"].pop()
        comparison_path.write_text(json.dumps(comparison), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "expected 300 samples"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)
        comparison["modes"]["plain"]["hot"]["git"]["cache_hit"] = {"input_echo": statistics(10.0, 300), "menu": None, "cache_capability": "not_applicable"}
        comparison["modes"]["inshellisense"]["transport"] = "pipe"
        comparison_path.write_text(json.dumps(comparison), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "incorrect transport label"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)
        comparison["modes"]["inshellisense"]["transport"] = "not_applicable"
        comparison["modes"]["candidate"]["transport_degraded"] = True
        comparison_path.write_text(json.dumps(comparison), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "transport degraded"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)

    def test_comparator_without_candidates_retains_timeouts_not_fake_latencies(self):
        path = self.root / self.evidence["comparison_reports"]["ps7-2.4.5"]
        comparison = json.loads(path.read_text(encoding="utf-8"))
        comparison["schema"] = 3
        comparison.update(formal=True, passed=True)
        measurement = comparison["modes"]["inshellisense"]["hot"]["root"]["cache_miss"]
        measurement.update(menu=None, cache_capability="not_applicable", menu_status="not_observed_in_capability_probe", capability_probes=[])
        for index in range(3):
            line = f"ssbeta-root-{index}"
            raw = {"passed": False, "program_sha256": "B" * 64,
                   "actual_shell": {"shell": "7.6.0", "psreadline": "2.4.5"},
                   "queries": [{"line": line, "expected": line, "failed": True, "menu": None,
                                "elapsed_ms": 20001, "input_echo": 12.0, "observations": [{"ms": 12.0, "screen": line}]}]}
            raw_path = self.root / f"capability-{index}.json"
            raw_path.write_text(json.dumps(raw), encoding="utf-8")
            measurement["capability_probes"].append({"report": raw_path.name, "sha256": validator.sha256_file(raw_path), "line": line, "expected": line})
        path.write_text(json.dumps(comparison), encoding="utf-8")
        validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)
        bundled = self.root / 'capability-evidence.zip'
        validator.bundle(self.evidence_path, bundled)
        with zipfile.ZipFile(bundled) as archive:
            self.assertTrue(all(f'capability-{index}.json' in archive.namelist() for index in range(3)))
        comparison["modes"]["candidate"]["hot"]["root"]["cache_miss"] = measurement
        path.write_text(json.dumps(comparison), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "product menu samples cannot be waived"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)
        comparison["modes"]["candidate"]["hot"]["root"]["cache_miss"] = {"input_echo": statistics(10.0, 300), "menu": statistics(10.0, 300), "cache_capability": "supported"}
        path.write_text(json.dumps(comparison), encoding="utf-8")
        raw_path.write_text("{}", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "capability evidence hash mismatch"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)


if __name__ == "__main__":
    unittest.main()
