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
            "schema": 2,
            "version": validator.VERSION,
            "source_commit": self.commit,
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


if __name__ == "__main__":
    unittest.main()
