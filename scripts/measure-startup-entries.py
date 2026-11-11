"""Pair the managed Profile and dedicated executable entry, each with a native Shell control."""
import argparse
import json
from pathlib import Path
import statistics
import subprocess


def identity(report):
    # A shared isolated fixture is installed for both entries. Only the profile
    # arm enables autostart; the direct and native arms use its recursion guard.
    fields = ("machine_id", "power_policy", "profile_sha256", "parent_path_pathext_sha256", "shell", "shell_versions",
              "psreadline_versions", "terminal_rows", "terminal_columns", "actual_transports",
              "actual_host_modes", "executable_sha256", "probe_executable_sha256", "native_observers",
              "probe_conpty", "plain_editors", "editors", "local_cache_state", "trace", "build")
    return {key: report[key] for key in fields}


def attribute(report, trace):
    events = [json.loads(line) for line in trace.read_text(encoding="utf-8").splitlines()]
    frequency = report["qpc_frequency_hz"]
    points = dict(report["startup_qpc"][0]["candidate"])
    for event in events:
        if event.get("qpc") is not None:
            points.setdefault(event["event"], event["qpc"])
    spans = {}
    for start, end in [
        ("launch", "profile_hook_entry"), ("profile_hook_entry", "blueberry_process_entry"),
        ("launch", "blueberry_process_entry"), ("blueberry_process_entry", "run_maintenance_begin"),
        ("run_maintenance_begin", "run_maintenance_end"), ("run_maintenance_end", "run_options_ready"),
        ("run_options_ready", "direct_start"), ("direct_start", "direct_resources_ready"),
        ("direct_spawn_begin", "direct_spawn_return"), ("direct_spawn_return", "direct_command_enter"),
        ("direct_command_enter", "direct_module_loaded"), ("direct_module_loaded", "direct_hello"),
        ("launch", "prompt"), ("key_sent", "echo"), ("child_environment_published", "direct_environment_received"),
        ("direct_environment_received", "worker_index_started"), ("worker_index_started", "worker_index_finished"),
        ("visible_static", "worker_index_finished"), ("worker_index_finished", "complete_static"),
        ("visible_static", "complete_static"),
    ]:
        a, b = points.get(start), points.get(end)
        # Signed intervals distinguish an index already finished before display.
        spans[f"{start}__{end}"] = None if a is None or b is None else (b - a) * 1000 / frequency
    return spans


def measure(args):
    if args.pairs < 2 or args.pairs % 2:
        raise ValueError("choose an even number of pairs for balanced AB/BA order")
    args.output.mkdir(parents=True, exist_ok=False)
    frozen = None
    pairs = []
    for pair in range(args.pairs):
        records = {}
        for entry in (("profile", "direct") if pair % 2 == 0 else ("direct", "profile")):
            name = f"{pair:03}-{entry}"
            output = args.output / (name + ".json")
            traces = args.output / (name + "-trace")
            cmd = [str(args.probe or args.executable), "product-probe", "--host-executable", str(args.executable),
                   "--shell", str(args.shell), "--iterations", "1", "--cache-state", args.cache, "--output", str(output)]
            if entry == "profile":
                cmd += ["--profile-entry", "--profile-fixture", str(args.fixture)]
            if args.trace:
                cmd += ["--diagnostic-trace-directory", str(traces)]
            with (args.output / (name + ".log")).open("w", encoding="utf-8") as log:
                subprocess.run(cmd, check=True, stdout=log, stderr=subprocess.STDOUT, timeout=180)
            report = json.loads(output.read_bytes())
            current = identity(report)
            if (frozen is not None and current != frozen) or not report["editor_eligible"]:
                raise ValueError(f"incompatible identity in {name}; no combined result")
            if (report["actual_entry"] != {"profile": "isolated_automatic_profile", "direct": "direct_executable"}[entry]
                    or report["local_cache_state"] != args.cache or not report["local_appdata_isolated"]
                    or report["trace"] != ("enabled" if args.trace else "disabled")):
                raise ValueError(f"unexpected measurement mode in {name}")
            frozen = current
            metrics = ("plain_first_input", "candidate_first_input", "paired_first_input_delta",
                       "first_visible_static_candidate", "complete_static_candidates")
            records[entry] = {field: report[field]["samples"][0] for field in metrics}
            if args.trace:
                records[entry]["spans_ms"] = attribute(report, traces / "direct-product-0.jsonl")
        pairs.append(records)
    summary = {"diagnostic_only": True, "identity": frozen, "pairs": pairs, "unit": "ms",
               "profile_minus_direct_first_input_p50": statistics.median(
                   p["profile"]["candidate_first_input"] - p["direct"]["candidate_first_input"] for p in pairs)}
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("shell", "executable", "fixture", "output", "probe"):
        parser.add_argument("--" + name, type=Path, required=name != "probe")
    parser.add_argument("--pairs", type=int, default=2)
    parser.add_argument("--cache", choices=("cold", "warm"), default="warm")
    parser.add_argument("--trace", action="store_true")
    measure(parser.parse_args())
