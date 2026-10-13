"""Recompute complete idle process-tree measurements from all raw snapshots."""
import argparse
import json
import math
from pathlib import Path
import statistics


def snapshots_match(original, encoded):
    """Legacy probes reserialize f64s; allow only a single representable step.

    All integer counters, identities, keys and array lengths stay exact. The
    original file remains authoritative for statistics and its byte digest.
    """
    if type(original) is not type(encoded):
        return False
    if isinstance(original, dict):
        return original.keys() == encoded.keys() and all(snapshots_match(value, encoded[key])
                                                        for key, value in original.items())
    if isinstance(original, list):
        return len(original) == len(encoded) and all(snapshots_match(a, b) for a, b in zip(original, encoded))
    if isinstance(original, float):
        return math.isfinite(original) and math.isfinite(encoded) and abs(original - encoded) <= math.ulp(original)
    return original == encoded


def finite(value):
    assert type(value) in (int, float) and math.isfinite(value) and value >= 0, 'Invalid resource sample'
    return value


def summarize(report, minimum_seconds=30):
    assert report.get('schema') == 2 and report.get('measurement_complete') is True, 'Resource collection incomplete'
    assert report.get('diagnostic_only') is not True, 'Thread-counter diagnostics cannot qualify as formal resources'
    assert report.get('collection_errors') == [], 'Resource collection has errors'
    assert report.get('warmup_complete', True) is True, 'Resource initialization did not settle'
    frequency = finite(report['qpc_frequency'])
    assert frequency > 0
    samples = report['samples']
    assert len(samples) >= 2, 'Resource snapshots missing'
    elapsed = (finite(samples[-1]['qpc']) - finite(samples[0]['qpc'])) / frequency
    assert elapsed >= minimum_seconds and math.isclose(elapsed, report['elapsed_seconds'], abs_tol=.0001), 'Resource duration mismatch'
    identities = None
    previous = None
    working_sets, private_bytes = [], []
    for sample in samples:
        tree = {}
        for process in sample['processes']:
            assert not process.get('error'), 'Process sample failed'
            key = (process['pid'], process['started_utc'])
            assert key not in tree, 'Duplicate process identity'
            tree[key] = process
            for field in ('cpu_seconds', 'working_set_bytes', 'private_bytes'):
                finite(process[field])
        assert (report['root_pid'], report['root_started_utc']) in tree, 'Root identity absent'
        if identities is None:
            identities = set(tree)
        assert set(tree) == identities, 'Process tree changed; exited/new CPU cannot be treated as zero'
        if previous:
            assert sample['qpc'] > previous[0], 'Non-monotonic resource timestamps'
            assert all(tree[key]['cpu_seconds'] >= previous[1][key]['cpu_seconds'] for key in identities), 'CPU counter regressed'
        previous = (sample['qpc'], tree)
        working_sets.append(sum(process['working_set_bytes'] for process in tree.values()))
        private_bytes.append(sum(process['private_bytes'] for process in tree.values()))
    first = {(p['pid'], p['started_utc']): p for p in samples[0]['processes']}
    cpu = 100 * sum(previous[1][key]['cpu_seconds'] - first[key]['cpu_seconds'] for key in identities) / elapsed
    deltas = report.get('deltas')
    assert isinstance(deltas, list) and len(deltas) == len(identities), 'CPU deltas incomplete'
    assert len({item['pid'] for item in deltas}) == len(identities), 'Duplicate CPU delta'
    for item in deltas:
        key = next((key for key in identities if key[0] == item['pid']), None)
        assert key is not None, 'Unexpected delta process'
        expected = 100 * (previous[1][key]['cpu_seconds'] - first[key]['cpu_seconds']) / elapsed
        assert math.isclose(finite(item['cpu_percent_one_core']), expected, abs_tol=.000001), 'CPU delta disagrees with raw counters'
    return dict(elapsed_seconds=elapsed, cpu_percent_one_core=cpu,
                working_set_bytes=dict(samples=working_sets, median=statistics.median(working_sets), peak=max(working_sets)),
                private_bytes=dict(samples=private_bytes, median=statistics.median(private_bytes), peak=max(private_bytes)))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('report', type=Path)
    args = parser.parse_args()
    print(json.dumps(summarize(json.loads(args.report.read_text(encoding='utf-8-sig')))))
