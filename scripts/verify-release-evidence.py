#!/usr/bin/env python3
"""Fail closed when beta.7 release measurements do not match the packaged EXE.

Usage: python scripts/verify-release-evidence.py EVIDENCE.json PACKAGE.zip --commit SHA --public-beta6-package BETA6.zip
The evidence file and its raw reports are release assets. They are produced only
after a source commit and its final package have been frozen.
"""

import argparse
import hashlib
import json
import math
import re
import statistics
import sys
import zipfile
from pathlib import Path, PurePosixPath, PureWindowsPath

VERSION = "0.5.0-beta.7"
PROFILES = {
    "ps51-2.0.0": ("powershell.exe", "5.", "2.0.0"),
    "ps51-2.4.5": ("powershell.exe", "5.", "2.4.5"),
    "ps7-2.4.5": ("pwsh.exe", "7.", "2.4.5"),
}
VARIANTS = {
    "osc_with_descriptions": ("osc", True),
    "osc_without_descriptions": ("osc", False),
    "pipe_with_descriptions": ("pipe", True),
}
SCENARIOS = {"root", "git", "cargo", "js", "path", "fuzzy"}
COMPARISON_MODES = ("plain", "beta6", "candidate", "inshellisense")
TERMINAL_TASKS = {"ime", "font_zoom", "selection", "paste", "nested_program"}
USER_TASKS = {"install", "explain", "project_parameters", "template", "exit_restore"}
INSTALL_TASKS = {"install", "upgrade", "rollback"}
HEX64 = re.compile(r"[0-9A-Fa-f]{64}\Z")
HEX40 = re.compile(r"[0-9A-Fa-f]{40}\Z")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def read_json(path):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            require(key not in result, f"duplicate JSON key {key!r} in {path}")
            result[key] = value
        return result

    with path.open("r", encoding="utf-8") as source:
        return json.load(source, object_pairs_hook=unique)


def sha256_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest().upper()


def sha256_zip_member(archive, name):
    digest = hashlib.sha256()
    with archive.open(name) as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest().upper()


def report_path(root, relative):
    require(isinstance(relative, str) and relative.endswith(".json"), "report path must be .json")
    parts = PurePosixPath(relative).parts
    require(
        "\\" not in relative and not PurePosixPath(relative).is_absolute()
        and all(part not in ("", ".", "..") for part in parts),
        "report path must be a safe relative POSIX path",
    )
    path = (root / relative).resolve(strict=True)
    require(path.is_relative_to(root), f"report leaves evidence directory: {relative}")
    return path


def samples(value, count, metric):
    require(isinstance(value, dict), f"{metric} is missing")
    series = value.get("samples")
    require(isinstance(series, list) and len(series) == count, f"{metric}: expected {count} samples")
    require(
        all(isinstance(item, (int, float)) and not isinstance(item, bool) and math.isfinite(item) for item in series),
        f"{metric}: samples must be finite numbers",
    )
    expected_median = statistics.median(series)
    expected_p95 = sorted(series)[math.ceil(0.95 * count) - 1]
    require(math.isclose(value.get("median", float("nan")), expected_median, abs_tol=0.0001), f"{metric}: median disagrees with raw samples")
    require(math.isclose(value.get("p95", float("nan")), expected_p95, abs_tol=0.0001), f"{metric}: P95 disagrees with raw samples")
    return expected_median, expected_p95


def check_identity(report, executable_hash, profile):
    shell_name, shell_major, psreadline = PROFILES[profile]
    require(report.get("build") == "release", f"{profile}: debug report")
    require(report.get("platform") == "windows" and report.get("arch") == "x86_64", f"{profile}: wrong platform")
    require(report.get("executable_sha256", "").upper() == executable_hash, f"{profile}: different executable")
    require(PureWindowsPath(report.get("shell", "")).name.lower() == shell_name, f"{profile}: wrong shell")
    return shell_major, psreadline


def check_versions(versions, expected, count, label):
    require(isinstance(versions, list) and len(versions) == count, f"{label}: missing version samples")
    require(all(isinstance(value, str) and value.startswith(expected) for value in versions), f"{label}: version mismatch")


def check_startup(path, executable_hash, profile):
    report = read_json(path)
    shell_major, psreadline = check_identity(report, executable_hash, profile)
    require(report.get("schema") == 2, f"{profile}: unsupported startup schema")
    require(report.get("adapter_source") == "embedded", f"{profile}: external adapter")
    require(report.get("profile_mode") == "with_profile" and report.get("no_profile") is False, f"{profile}: profile disabled")
    count = report.get("iterations")
    require(type(count) is int and count >= 30, f"{profile}: fewer than 30 startup pairs")
    check_versions(report.get("shell_versions"), shell_major, count, f"{profile} shell")
    check_versions(report.get("psreadline_versions"), psreadline, count, f"{profile} PSReadLine")
    median, _ = samples(report.get("paired_first_input_delta"), count, f"{profile} startup")
    require(median <= 50.0, f"{profile}: startup P50 {median:.3f} ms exceeds 50 ms")


def check_hot(path, executable_hash, profile, variant):
    report = read_json(path)
    shell_major, psreadline = check_identity(report, executable_hash, profile)
    transport, descriptions = VARIANTS[variant]
    require(report.get("schema") == 1, f"{profile}/{variant}: unsupported hot schema")
    require(report.get("transport") == transport and report.get("descriptions") is descriptions, f"{profile}/{variant}: wrong variant")
    require(report.get("trace") == "disabled" and report.get("profile_mode") == "preserved", f"{profile}/{variant}: trace or profile mismatch")
    require(report.get("transport_degraded") is False, f"{profile}/{variant}: transport degraded")
    require(report.get("target", {}).get("status") == "passed", f"{profile}/{variant}: target failed")
    count = report.get("samples_per_cache_mode")
    require(type(count) is int and count >= 300, f"{profile}/{variant}: fewer than 300 samples")
    scenarios = report.get("scenarios")
    require(isinstance(scenarios, list) and {item.get("name") for item in scenarios} == SCENARIOS and len(scenarios) == 6, f"{profile}/{variant}: six scenarios required")
    for scenario in scenarios:
        name = scenario["name"]
        for cache_mode in ("cache_miss", "cache_hit"):
            label = f"{profile}/{variant}/{name}/{cache_mode}"
            measurement = scenario.get(cache_mode, {})
            session_count = math.ceil(count / 10)
            check_versions(measurement.get("shell_versions"), shell_major, session_count, f"{label} shell")
            check_versions(measurement.get("psreadline_versions"), psreadline, session_count, f"{label} PSReadLine")
            require(measurement.get("actual_transport") == [transport] * session_count, f"{label}: transport mismatch")
            acceptance = scenario.get("acceptance", {}).get(cache_mode, {})
            require(acceptance.get("status") == "passed", f"{label}: acceptance failed")
            require(acceptance.get("observed_samples") == count and acceptance.get("expected_samples") == count, f"{label}: sample count mismatch")
            _, p95 = samples(acceptance.get("statistics"), count, label)
            require(p95 <= 20.0, f"{label}: P95 {p95:.3f} ms exceeds 20 ms")


def check_manual_acceptance(evidence, executable_hash, commit):
    manual = evidence.get("manual_acceptance")
    require(isinstance(manual, dict), "manual acceptance record missing")
    require(manual.get("executable_sha256", "").upper() == executable_hash, "manual acceptance belongs to another EXE")
    require(manual.get("source_commit", "").lower() == commit.lower(), "manual acceptance belongs to another source commit")
    for field, expected in (("installation", INSTALL_TASKS), ("windows_terminal", TERMINAL_TASKS)):
        tasks = manual.get(field)
        require(isinstance(tasks, dict) and set(tasks) == expected and all(value is True for value in tasks.values()), f"{field}: incomplete manual acceptance")
    trials = manual.get("user_trials")
    require(isinstance(trials, list) and 8 <= len(trials) <= 12, "need 8–12 target user trials")
    identifiers = [trial.get("id") for trial in trials]
    require(all(isinstance(identifier, str) and identifier for identifier in identifiers) and len(set(identifiers)) == len(identifiers), "user trial identifiers missing or duplicated")
    for trial in trials:
        require(trial.get("executable_sha256", "").upper() == executable_hash, f"user trial {trial['id']}: wrong EXE")
        tasks = trial.get("tasks")
        require(isinstance(tasks, dict) and set(tasks) == USER_TASKS and all(value is True for value in tasks.values()), f"user trial {trial['id']}: incomplete tasks")
    for field in ("public_beta6_sha256", "inshellisense_sha256"):
        require(HEX64.fullmatch(manual.get(field, "")) is not None, f"{field}: baseline file not frozen")


def check_public_beta6(package_path, expected_hash):
    package_path = package_path.resolve(strict=True)
    require(package_path.name == "blueberry-v0.5.0-beta.6-windows-x64.zip", "wrong public beta.6 asset name")
    with zipfile.ZipFile(package_path) as archive:
        require(archive.namelist().count("blueberry.exe") == 1, "public beta.6 package has no unique EXE")
        require(sha256_zip_member(archive, "blueberry.exe") == expected_hash.upper(), "public beta.6 EXE hash mismatch")


def check_comparison(path, executable_hash, commit, profile, manual):
    report = read_json(path)
    shell_name, shell_major, psreadline = PROFILES[profile]
    label = f"{profile} comparison"
    require(report.get("schema") == 1 and report.get("profile") == profile, f"{label}: wrong schema/profile")
    require(report.get("source_commit", "").lower() == commit.lower(), f"{label}: different source commit")
    require(PureWindowsPath(report.get("shell", "")).name.lower() == shell_name, f"{label}: wrong shell")
    require(str(report.get("shell_version", "")).startswith(shell_major), f"{label}: wrong shell version")
    require(report.get("psreadline_version") == psreadline, f"{label}: wrong PSReadLine version")
    require(report.get("profile_mode") == "with_profile" and report.get("terminal_rows") == 30 and report.get("terminal_columns") == 120, f"{label}: different profile or terminal")
    require(report.get("trace") == "disabled", f"{label}: diagnostic trace enabled")
    require(report.get("os_cache_cleared") is False, f"{label}: OS cache state differs")
    require(isinstance(report.get("machine_id"), str) and report["machine_id"].strip(), f"{label}: machine ID missing")
    require(isinstance(report.get("power_policy"), str) and report["power_policy"].strip(), f"{label}: power policy missing")
    startup_count = report.get("startup_pairs")
    hot_count = report.get("hot_samples_per_mode")
    require(type(startup_count) is int and startup_count >= 30, f"{label}: fewer than 30 startup pairs")
    require(type(hot_count) is int and hot_count >= 300, f"{label}: fewer than 300 hot samples")
    modes = report.get("modes")
    require(isinstance(modes, dict) and set(modes) == set(COMPARISON_MODES), f"{label}: comparator modes incomplete")
    expected_hashes = {
        "plain": None,
        "beta6": manual["public_beta6_sha256"].upper(),
        "candidate": executable_hash,
        "inshellisense": manual["inshellisense_sha256"].upper(),
    }
    for mode in COMPARISON_MODES:
        measured = modes[mode]
        require(isinstance(measured, dict), f"{label}/{mode}: mode missing")
        actual_hash = measured.get("sha256")
        require(
            actual_hash is None if expected_hashes[mode] is None
            else isinstance(actual_hash, str) and actual_hash.upper() == expected_hashes[mode],
            f"{label}/{mode}: binary hash mismatch",
        )
        transport = measured.get("transport")
        require(transport == ("osc" if mode in ("beta6", "candidate") else "not_applicable"), f"{label}/{mode}: incorrect transport label")
        require(measured.get("transport_degraded") is False if mode in ("beta6", "candidate") else measured.get("transport_degraded") is None, f"{label}/{mode}: transport degraded or falsely asserted")
        samples(measured.get("startup"), startup_count, f"{label}/{mode} startup")
        hot = measured.get("hot")
        require(isinstance(hot, dict) and set(hot) == SCENARIOS, f"{label}/{mode}: scenarios incomplete")
        for scenario in SCENARIOS:
            group = hot[scenario]
            require(isinstance(group, dict) and set(group) == {"cache_hit", "cache_miss"}, f"{label}/{mode}/{scenario}: cache modes incomplete")
            for cache_mode in ("cache_hit", "cache_miss"):
                samples(group[cache_mode], hot_count, f"{label}/{mode}/{scenario}/{cache_mode}")
    orders = report.get("startup_orders")
    require(isinstance(orders, list) and len(orders) == startup_count, f"{label}: startup ordering missing")
    require(all(isinstance(order, list) and set(order) == set(COMPARISON_MODES) and len(order) == 4 for order in orders), f"{label}: startup order is not four-way alternating")
    require(all(orders[index] == list(COMPARISON_MODES[index % 4:] + COMPARISON_MODES[:index % 4]) for index in range(startup_count)), f"{label}: startup order is not rotated")
    hot_orders = report.get("hot_session_orders")
    require(isinstance(hot_orders, dict) and set(hot_orders) == SCENARIOS, f"{label}: hot ordering missing")
    session_count = math.ceil(hot_count / 10)
    for scenario in SCENARIOS:
        require(set(hot_orders[scenario]) == {"cache_hit", "cache_miss"}, f"{label}/{scenario}: hot ordering incomplete")
        for cache_mode in ("cache_hit", "cache_miss"):
            orders = hot_orders[scenario][cache_mode]
            require(isinstance(orders, list) and len(orders) == session_count, f"{label}/{scenario}/{cache_mode}: hot ordering missing")
            require(all(order == list(COMPARISON_MODES[index % 4:] + COMPARISON_MODES[:index % 4]) for index, order in enumerate(orders)), f"{label}/{scenario}/{cache_mode}: hot order is not rotated")


def verify(evidence_path, package_path, commit, public_beta6_package):
    evidence_path = evidence_path.resolve(strict=True)
    package_path = package_path.resolve(strict=True)
    evidence = read_json(evidence_path)
    require(evidence.get("schema") == 1 and evidence.get("version") == VERSION, "wrong evidence version/schema")
    require(HEX40.fullmatch(commit) is not None, "--commit must be a 40-character Git SHA")
    require(evidence.get("source_commit", "").lower() == commit.lower(), "evidence source commit mismatch")
    executable_hash = evidence.get("executable_sha256", "").upper()
    package_hash = evidence.get("package_sha256", "").upper()
    require(HEX64.fullmatch(executable_hash) and HEX64.fullmatch(package_hash), "missing SHA-256")
    require(sha256_file(package_path) == package_hash, "package SHA-256 mismatch")
    with zipfile.ZipFile(package_path) as archive:
        names = [entry.filename for entry in archive.infolist() if not entry.is_dir()]
        require(len(names) == len(set(names)), "package contains duplicate file paths")
        require(names.count("blueberry.exe") == 1, "package must have exactly one blueberry.exe")
        require(names.count("release.json") == 1, "package must have exactly one release.json")
        manifest_bytes = archive.read("release.json")
        external_manifest = package_path.with_name(package_path.name.replace("-windows-x64.zip", "-release.json"))
        require(external_manifest != package_path and external_manifest.is_file(), "external package manifest missing")
        require(external_manifest.read_bytes() == manifest_bytes, "external and packaged manifests differ")
        manifest = json.loads(manifest_bytes)
        require(manifest.get("version") == VERSION and manifest.get("platform") == "windows-x64", "package manifest version/platform mismatch")
        require(sha256_zip_member(archive, "blueberry.exe") == executable_hash, "packaged EXE SHA-256 mismatch")
        files = manifest.get("files", [])
        require(isinstance(files, list), "package manifest has no file list")
        paths = [item.get("path") for item in files]
        require(len(paths) == len(set(paths)) and set(paths) == set(names) - {"release.json"}, "package manifest file list mismatch")
        for item in files:
            path = item["path"]
            require(
                isinstance(path, str) and path and "\\" not in path
                and not PurePosixPath(path).is_absolute()
                and ".." not in PurePosixPath(path).parts,
                "unsafe package path",
            )
            require(item.get("sha256", "").upper() == sha256_zip_member(archive, path), f"package file hash mismatch: {path}")
            if "bytes" in item:
                require(item["bytes"] == archive.getinfo(path).file_size, f"package file size mismatch: {path}")
        records = [item for item in files if item.get("path") == "blueberry.exe"]
        require(len(records) == 1 and records[0].get("sha256", "").upper() == executable_hash, "manifest EXE SHA-256 mismatch")
    startup = evidence.get("startup_reports")
    hot = evidence.get("hot_reports")
    comparisons = evidence.get("comparison_reports")
    require(isinstance(startup, dict) and set(startup) == set(PROFILES), "startup profile matrix incomplete")
    require(isinstance(hot, dict) and set(hot) == set(PROFILES), "hot profile matrix incomplete")
    require(isinstance(comparisons, dict) and set(comparisons) == set(PROFILES), "comparison profile matrix incomplete")
    root = evidence_path.parent
    for profile in PROFILES:
        check_startup(report_path(root, startup[profile]), executable_hash, profile)
        require(isinstance(hot[profile], dict) and set(hot[profile]) == set(VARIANTS), f"{profile}: hot variants incomplete")
        for variant in VARIANTS:
            check_hot(report_path(root, hot[profile][variant]), executable_hash, profile, variant)
    check_manual_acceptance(evidence, executable_hash, commit)
    check_public_beta6(public_beta6_package, evidence["manual_acceptance"]["public_beta6_sha256"])
    for profile in PROFILES:
        check_comparison(report_path(root, comparisons[profile]), executable_hash, commit, profile, evidence["manual_acceptance"])
    return executable_hash


def bundle(evidence_path, output_path):
    evidence_path = evidence_path.resolve(strict=True)
    evidence = read_json(evidence_path)
    reports = set(evidence["startup_reports"].values())
    reports.update(evidence["comparison_reports"].values())
    for variants in evidence["hot_reports"].values():
        reports.update(variants.values())
    output_path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output_path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        archive.write(evidence_path, evidence_path.name)
        for relative in sorted(reports):
            archive.write(report_path(evidence_path.parent, relative), relative)
    output_path.with_name(output_path.name + ".sha256").write_text(
        f"{sha256_file(output_path)}  {output_path.name}\n", encoding="utf-8"
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path)
    parser.add_argument("package", type=Path)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--public-beta6-package", required=True, type=Path,
                        help="original ZIP downloaded from this repository's public v0.5.0-beta.6 release")
    parser.add_argument("--bundle-out", type=Path, help="write a ZIP of verified raw reports")
    args = parser.parse_args()
    try:
        executable_hash = verify(args.evidence, args.package, args.commit, args.public_beta6_package)
        if args.bundle_out:
            bundle(args.evidence, args.bundle_out)
    except (OSError, ValueError, KeyError, TypeError, zipfile.BadZipFile) as error:
        parser.exit(1, f"release evidence rejected: {error}\n")
    print(f"release evidence accepted: {VERSION}, {args.commit}, EXE {executable_hash}")


if __name__ == "__main__":
    main()
