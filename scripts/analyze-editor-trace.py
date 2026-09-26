"""Correlate numeric QPC trace points with diagnostic VT observations."""
import argparse
import ctypes
import json
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("report", type=Path)
parser.add_argument("traces", type=Path)
parser.add_argument("--output", type=Path, required=True)
args = parser.parse_args()
report = json.loads(args.report.read_text(encoding="utf-8"))
frequency = ctypes.c_longlong()
if not ctypes.windll.kernel32.QueryPerformanceFrequency(ctypes.byref(frequency)):
    raise RuntimeError("QPC frequency unavailable")
rows = []
for scenario in report["scenarios"]:
    for cache, label in [("cache_miss", "miss"), ("cache_hit", "hit")]:
        for index, sample in enumerate(scenario[cache]["observations"]):
            trace = args.traces / f"{scenario['name']}-{label}-{index // 10}.jsonl"
            events = [json.loads(line) for line in trace.read_text(encoding="utf-8").splitlines()]
            points = [e for e in events if "qpc" in e]
            start, end = sample["input_qpc"], sample["menu_arrival_qpc"]
            # WriteConsole may complete just after its output reaches the VT
            # observer. Retain that signed difference, never clamp it to zero.
            painted = [e for e in points if e["event"] == "editor_menu_written"
                       and start <= e["qpc"] <= end + frequency.value // 1000]
            row = {"scenario": scenario["name"], "cache": label, "sample": index, **sample}
            if painted:
                paint = max(painted, key=lambda e: e["qpc"])
                rev, frame = paint["revision"], paint["frame_id"]
                stages = {e["event"]: e["qpc"] for e in points
                          if e["revision"] == rev and e["frame_id"] in (None, frame)}
                stages.update(input=start, vt_arrival=end)
                names = ["input", "editor_confirmed", "direct_frame_ready", "editor_frame_received",
                         "editor_refresh_enter", "editor_menu_written", "vt_arrival"]
                row.update(revision=rev, frame_id=frame, qpc=stages)
                row["spans_ms"] = {a + "__" + b: (stages[b] - stages[a]) * 1000 / frequency.value
                                   for a, b in zip(names, names[1:]) if a in stages and b in stages}
            else:
                row["correlation_error"] = "No menu write found in observation window"
            rows.append(row)
args.output.write_text(json.dumps({"diagnostic_only": True, "qpc_frequency": frequency.value,
                                  "samples": rows}, indent=2), encoding="utf-8")
for row in rows:
    latency = row["dynamic_complete_menu"] or row["first_candidate_menu"]
    if latency > 20:
        print(row["scenario"], row["cache"], row["sample"], round(latency, 3),
              {k: round(v, 3) for k, v in row.get("spans_ms", {}).items()})
