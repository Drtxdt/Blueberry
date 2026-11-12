"""Summarize acknowledged frames in one burst trace; never pool different sessions or delay settings."""
import argparse
import json
import math
from pathlib import Path
import statistics


def analyze(events, window):
    arrivals, sends = {}, {}
    painted = set()
    for e in events:
        if e["event"] == "editor_menu_written" and window["begin_qpc"] <= e["qpc"] <= window["end_qpc"]:
            painted.add((e["revision"], e["frame_id"]))
        if e["event"] not in ("direct_presented_queue", "direct_presented_send"):
            continue
        key = (e["revision"], e["frame_id"])
        target = arrivals if e["event"] == "direct_presented_queue" else sends
        if key in target or any(not isinstance(i, int) for i in key):
            raise ValueError("duplicate or missing acknowledgement identity")
        target[key] = e
    paired = painted & sends.keys() & arrivals.keys()
    if not paired:
        raise ValueError("no matched acknowledgements in this trace")

    def stats(values):
        values = sorted(values)
        return dict(p50=statistics.median(values), p95=values[math.ceil(.95 * len(values)) - 1], max=values[-1])

    return dict(diagnostic_only=True, window=window, painted=len(painted), matched=len(paired),
                missing_send=len(painted - sends.keys()), missing_arrival=len(painted - arrivals.keys()),
                synchronous_send_ms=stats(sends[k]["duration_ms"] for k in paired),
                reader_to_dispatch_ms=stats(arrivals[k]["duration_ms"] for k in paired),
                newer_frames_at_dispatch=stats(arrivals[k]["count"] for k in paired))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("trace", type=Path)
    args = parser.parse_args()
    window = json.loads(args.trace.with_name("burst-window.json").read_bytes())
    with args.trace.open(encoding="utf-8") as stream:
        print(json.dumps(analyze((json.loads(line) for line in stream), window), indent=2))
