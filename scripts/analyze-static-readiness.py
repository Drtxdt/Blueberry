"""Recover paired first-visible to complete-menu intervals from the sealed AB/BA summaries."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import statistics
import zipfile


def gaps(batch):
    count = batch["valid_pairs"]
    metrics = batch["metrics"]
    by_side = {}
    for side in ("baseline", "candidate"):
        first = metrics["first_visible_static_candidate"][side]
        complete = metrics["complete_static_candidates"][side]
        if first["unit"] != "ms" or complete["unit"] != "ms":
            raise ValueError("incompatible observation units")
        if not count or len(first["samples"]) != count or len(complete["samples"]) != count:
            raise ValueError("incomplete paired sample vectors")
        durations = []
        for a, b in zip(first["samples"], complete["samples"], strict=True):
            if not (math.isfinite(a) and math.isfinite(b) and 0 <= a <= b):
                raise ValueError("invalid first/complete observation")
            durations.append(b - a)
        by_side[side] = durations
    deltas = [b - a for a, b in zip(by_side["baseline"], by_side["candidate"], strict=True)]
    return {key: round(statistics.median(values), 2)
            for key, values in {**by_side, "paired_change": deltas}.items()}


def analyze(receipt, directory):
    result = []
    for group in receipt["formal_ab_ba"]:
        with zipfile.ZipFile(directory / (str(group["artifact_id"]) + ".zip")) as archive:
            raw = archive.read("summary.json")
        if hashlib.sha256(raw).hexdigest() != group["summary_sha256"]:
            raise ValueError("summary does not match the sealed receipt")
        summary = json.loads(raw)
        if summary["formal"] is not True or summary["observer_sha256"] != summary["executable_sha256"]["candidate"]:
            raise ValueError("ineligible observer or batch")
        for batch in summary["batches"]:
            result.append(dict(group=group["name"], batch=batch["batch"], cache=batch["cache"],
                               pairs=batch["valid_pairs"], gap_p50_ms=gaps(batch)))
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archives", type=Path, help="directory containing the original Actions artifact ZIPs by ID")
    parser.add_argument("--receipt", type=Path, default=Path(__file__).resolve().parents[1] / "docs/review-hardening-remote-validation.json")
    args = parser.parse_args()
    print(json.dumps(analyze(json.loads(args.receipt.read_bytes()), args.archives), indent=2))
