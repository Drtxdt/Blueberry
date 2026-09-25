"""Checks that a release cannot pass with missing, slow or mismatched evidence."""

import hashlib
import importlib.util
import json
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
    def setUp(self):
        scratch = Path(__file__).resolve().parents[1] / "target/release-evidence-tests"
        scratch.mkdir(parents=True, exist_ok=True)
        self.root = scratch / uuid.uuid4().hex
        self.root.mkdir()
        self.addCleanup(shutil.rmtree, self.root, ignore_errors=True)
        self.commit = "a" * 40
        self.exe = b"MZ synthetic release fixture"
        self.exe_hash = hashlib.sha256(self.exe).hexdigest().upper()
        self.beta6_exe = b"MZ synthetic public beta.6 fixture"
        self.beta6_hash = hashlib.sha256(self.beta6_exe).hexdigest().upper()
        self.beta6_package = self.root / "blueberry-v0.5.0-beta.6-windows-x64.zip"
        with zipfile.ZipFile(self.beta6_package, "w") as archive:
            archive.writestr("blueberry.exe", self.beta6_exe)
        self.package = self.root / "blueberry-v0.5.0-beta.7-windows-x64.zip"
        manifest = {
            "version": validator.VERSION,
            "platform": "windows-x64",
            "files": [{"path": "blueberry.exe", "sha256": self.exe_hash}],
        }
        with zipfile.ZipFile(self.package, "w") as archive:
            archive.writestr("blueberry.exe", self.exe)
            archive.writestr("release.json", json.dumps(manifest))
        self.external_manifest = self.root / "blueberry-v0.5.0-beta.7-release.json"
        self.external_manifest.write_text(json.dumps(manifest), encoding="utf-8")
        self.evidence = {
            "schema": 1,
            "version": validator.VERSION,
            "source_commit": self.commit,
            "package_sha256": validator.sha256_file(self.package),
            "executable_sha256": self.exe_hash,
            "startup_reports": {},
            "hot_reports": {},
            "comparison_reports": {},
            "manual_acceptance": {
                "executable_sha256": self.exe_hash,
                "source_commit": self.commit,
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
            shell_version = shell_major + "1.0"
            order = list(validator.COMPARISON_MODES)
            rotated = lambda index: order[index % 4:] + order[:index % 4]
            comparison = {
                "schema": 1,
                "profile": profile,
                "source_commit": self.commit,
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
                        "transport": "osc" if mode in ("beta6", "candidate") else "not_applicable",
                        "transport_degraded": False if mode in ("beta6", "candidate") else None,
                        "startup": statistics(10.0, 30),
                        "hot": {
                            name: {
                                cache_mode: statistics(10.0, 300)
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
                "schema": 2,
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
                    "schema": 1,
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
        self.evidence_path = self.root / "evidence.json"
        self.save_evidence()

    def save_evidence(self):
        self.evidence_path.write_text(json.dumps(self.evidence), encoding="utf-8")

    def test_complete_matching_release_passes(self):
        self.assertEqual(
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package), self.exe_hash
        )

    def test_slow_startup_and_missing_variant_block_release(self):
        startup_path = self.root / self.evidence["startup_reports"]["ps7-2.4.5"]
        startup = json.loads(startup_path.read_text(encoding="utf-8"))
        startup["paired_first_input_delta"] = statistics(50.1, 30)
        startup_path.write_text(json.dumps(startup), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "exceeds 50"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)
        startup["paired_first_input_delta"] = statistics(10.0, 30)
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

    def test_slow_hot_group_blocks_release(self):
        hot_path = self.root / self.evidence["hot_reports"]["ps7-2.4.5"]["osc_with_descriptions"]
        hot = json.loads(hot_path.read_text(encoding="utf-8"))
        hot["scenarios"][0]["acceptance"]["cache_hit"]["statistics"] = statistics(20.1, 300)
        hot_path.write_text(json.dumps(hot), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "exceeds 20"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)

    def test_unlisted_package_file_blocks_release(self):
        with zipfile.ZipFile(self.package, "a") as archive:
            archive.writestr("unexpected.txt", "extra")
        self.evidence["package_sha256"] = validator.sha256_file(self.package)
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
        comparison["modes"]["plain"]["hot"]["git"]["cache_hit"]["samples"].pop()
        comparison_path.write_text(json.dumps(comparison), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "expected 300 samples"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)
        comparison["modes"]["plain"]["hot"]["git"]["cache_hit"] = statistics(10.0, 300)
        comparison["modes"]["inshellisense"]["transport"] = "pipe"
        comparison_path.write_text(json.dumps(comparison), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "incorrect transport label"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)
        comparison["modes"]["inshellisense"]["transport"] = "not_applicable"
        comparison["modes"]["candidate"]["transport_degraded"] = True
        comparison_path.write_text(json.dumps(comparison), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "transport degraded"):
            validator.verify(self.evidence_path, self.package, self.commit, self.beta6_package)


if __name__ == "__main__":
    unittest.main()
