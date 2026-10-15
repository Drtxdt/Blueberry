"""Verify every frozen shard and recompute a complete four-way report.

Each independently bounded section remains measurement_complete=false. Only
this merge can qualify all startup pairs and all six disjoint hot sequences.
No partial or interrupted section, changed dependency, or cached statistic is
accepted as a formal measurement.
"""
import argparse
import copy
import importlib.util
from pathlib import Path

SPEC = importlib.util.spec_from_file_location('comparison', Path(__file__).with_name('run-comparison.py'))
C = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(C)
IDENTITY = ('source_commit', 'source_dirty', 'formal', 'profile', 'profile_sha256', 'power_policy',
            'probe_sha256', 'collector_sha256', 'plan_sha256', 'fixture_sha256', 'frozen_sha256',
            'startup_pairs', 'hot_samples_per_mode', 'terminal_rows', 'terminal_columns', 'trace', 'probe_conpty')
IDENTITY += ('native_observer',)


def verified_samples(value):
    assert isinstance(value, dict) and C.stats(value['samples']) == value, 'Incorrect shard statistic'
    return value['samples']


def merge(entries):
    assert entries, 'No comparison sections'
    first = entries[0][1]
    assert first['formal'] and first['startup_pairs'] >= 30 and first['hot_samples_per_mode'] >= 300, 'Insufficient formal targets'
    assert first['hot_samples_per_mode'] % 10 == 0, 'Invalid hot target'
    for path, value in entries:
        assert value.get('section_complete') is True and value.get('measurement_complete') is False, 'Incomplete or unsharded input'
        assert not value.get('error'), 'Failed section'
        for key in IDENTITY:
            assert key in value and value[key] == first[key], 'Section identity mismatch: ' + key
        assert C.sha(path.parent/'frozen.json') == value['frozen_sha256'], 'Changed frozen inventory'
        assert C.sha(path.parent/'fixtures.json') == value['fixture_sha256'], 'Changed fixture inventory'
        for item in C.read(path.parent/'frozen.json') + C.read(path.parent/'fixtures.json'):
            assert C.sha(item['path']) == item['sha256'], 'Changed measurement dependency'
        assert C.profiles_unchanged(value['profile_inventory']), 'Changed profile'
        assert set(value['modes']) == set(C.MODES), 'Incomplete modes'
        for mode in C.MODES:
            for key in ('entry', 'dependencies', 'sha256', 'transport', 'host_mode', 'transport_degraded'):
                assert value['modes'][mode][key] == first['modes'][mode][key], 'Changed product identity'
    capability = [(p, v) for p, v in entries if v['section'] == 'capability']
    startup = [(p, v) for p, v in entries if v['section'] == 'startup']
    hot = [(p, v) for p, v in entries if v['section'] == 'hot']
    assert len(capability) == len(startup) == 1 and len(entries) == 2+len(hot), 'Duplicate or unknown section'
    cap_path, cap = capability[0]
    assert set(cap['capability_probes']) == set(C.CASES), 'Incomplete capability matrix'
    for scenario, evidence in cap['capability_probes'].items():
        probes = []
        for item in evidence:
            path = cap_path.parent/item['report']
            assert C.sha(path) == item['sha256'], 'Changed capability evidence'
            probes.append((path, C.read(path)))
        assert C.capability_unavailable(scenario, probes) == (scenario in cap['unavailable']), 'Incorrect capability status'
    result = copy.deepcopy(startup[0][1])
    result.update(section='merged', measurement_complete=False, section_complete=False, source_sections=[])
    for mode in C.MODES:
        measured = result['modes'][mode]
        measured['startup'] = verified_samples(measured['startup'])
        assert len(measured['startup']) == first['startup_pairs'], 'Insufficient startup pairs'
        measured.update(hot={}, first_queries=[])
    result['hot_session_orders'] = {}
    for scenario in C.CASES:
        shards = sorted([(p, v) for p, v in hot if v['scenario'] == scenario], key=lambda x:x[1]['pair_start'])
        next_pair = 0
        for path, value in shards:
            assert value['pair_start'] == next_pair and value['pair_count'] > 0, 'Missing, overlapping, or duplicated hot pairs'
            next_pair += value['pair_count']
            assert value['capability_evidence'] == dict(path=str(cap_path.resolve()), sha256=C.sha(cap_path)), 'Wrong capability batch'
            for mode in C.MODES:
                measured = value['modes'][mode]
                assert measured['startup'] == [] and set(measured['hot']) == {scenario}, 'Wrong shard scope'
                target = result['modes'][mode]['hot'].setdefault(scenario, {})
                groups = measured['hot'][scenario]
                assert set(groups) == {'cache_miss', 'cache_hit'}, 'Incomplete cache groups'
                for cache, group in groups.items():
                    echo = verified_samples(group['input_echo'])
                    assert len(echo) == value['pair_count']*10, 'Insufficient shard samples'
                    menu = group['menu']
                    if menu is not None:
                        menu = verified_samples(menu)
                        assert len(menu) == len(echo), 'Incomplete menu samples'
                    else:
                        assert mode == 'plain' or (mode == 'inshellisense' and scenario in cap['unavailable']), 'Unexplained missing menu'
                    if cache not in target:
                        target[cache] = copy.deepcopy(group)
                        target[cache].update(input_echo=[], menu=[] if menu is not None else None)
                    target[cache]['input_echo'].extend(echo)
                    if menu is not None: target[cache]['menu'].extend(menu)
                result['modes'][mode]['first_queries'].extend(measured['first_queries'])
            for cache, orders in value['hot_session_orders'][scenario].items():
                assert len(orders) == value['pair_count'], 'Missing session orders'
                result['hot_session_orders'].setdefault(scenario, {}).setdefault(cache, []).extend(orders)
        assert next_pair*10 == first['hot_samples_per_mode'], 'Incomplete hot scenario'
    assert set(v['scenario'] for _, v in hot) == set(C.CASES), 'Unknown hot scenario'
    C.summarize_measurements(result, first['startup_pairs'], first['hot_samples_per_mode'])
    result.update(measurement_complete=True, section_complete=True, unavailable=cap['unavailable'])
    result['source_sections'] = [dict(path=str(p.resolve()), sha256=C.sha(p), section=v['section']) for p, v in entries]
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reports', nargs='+', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    assert not args.output.exists(), 'Refusing to overwrite merged evidence'
    result = merge([(p.resolve(), C.read(p)) for p in args.reports])
    args.output.parent.mkdir(parents=True, exist_ok=True)
    C.write(args.output, result)
    print('Complete formal comparison: raw arrays verified and statistics recomputed')
