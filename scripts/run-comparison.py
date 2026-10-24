#!/usr/bin/env python3
"""Frozen four-way ConPTY comparison; retains every session and failed result.

The JSON plan supplies probe, shell, original_editor, profile, fixtures,
environment, and modes (program, args, style, dependency_paths). Product args
use the literal {data_dir} placeholder. No user installation is changed.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import statistics
import struct
import subprocess

MODES = ('plain', 'beta6', 'candidate', 'inshellisense')
CASES = {
    'root': ('', 'ssbeta-root-', 'ssbeta-root-'),
    'git': ('git', 'git switch ssbeta-git-', 'ssbeta-git-'),
    'cargo': ('cargo/app', 'cargo build --features ssbeta-cargo-', 'ssbeta-cargo-'),
    'js': ('javascript', 'npm run ssbeta-js-', 'ssbeta-js-'),
    'path': ('paths', 'cd ssbeta-path-', 'ssbeta-path-'),
    'fuzzy': ('', 'ssbf', 'ssbeta-fuzzy-'),
}

def read(path):
    return json.loads(Path(path).read_text(encoding='utf-8-sig'))

def write(path, value):
    Path(path).write_text(json.dumps(value, indent=2, ensure_ascii=False), encoding='utf-8')

def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest().upper()

def probe_identity(value, formal):
    assert value.get('diagnostic_only') is not True, 'Diagnostic probe cannot qualify as timing samples'
    fields = ('probe_build_profile', 'probe_source_commit', 'probe_source_dirty')
    if not any(field in value for field in fields):
        return None  # Historical frozen probes predate the additive metadata.
    assert all(field in value for field in fields), 'Incomplete probe build identity'
    identity = {field: value[field] for field in fields}
    if formal:
        assert identity['probe_build_profile'] == 'release' and identity['probe_source_dirty'] is False, 'Unfrozen probe build'
        commit = identity['probe_source_commit']
        assert isinstance(commit, str) and len(commit) == 40 and all(c in '0123456789abcdef' for c in commit.lower()), 'Invalid probe source commit'
    return identity

def profiles_unchanged(profiles):
    return all((sha(item['path']) if Path(item['path']).exists() else None) == item['sha256'] for item in profiles)

def inventory(paths):
    files = set()
    for value in paths:
        path = Path(value).resolve(strict=True)
        files.update(item for item in path.rglob('*') if item.is_file()) if path.is_dir() else files.add(path)
    return [dict(path=str(path), sha256=sha(path)) for path in sorted(files)]

def stats(values):
    assert values and all(type(value) in (int, float) and math.isfinite(value) and value >= 0 for value in values), 'Invalid latency samples'
    ordered = sorted(values)
    return dict(samples=values, median=statistics.median(values), p95=ordered[max(0, math.ceil(len(values)*.95)-1)], unit='ms')

def rotate(index):
    return list(MODES[index % 4:] + MODES[:index % 4])

def capability_unavailable(scenario, probes):
    assert len(probes) == 3, f'{scenario}: insufficient capability probes'
    outcomes = []
    for path, value in probes:
        queries = value.get('queries', [])
        assert len(queries) == 1, f'{scenario}: invalid capability collection in {path}'
        query = queries[0]
        echo = query.get('input_echo')
        assert type(echo) in (int, float) and math.isfinite(echo) and echo >= 0, f'{scenario}: invalid input echo in {path}; not a missing capability or slow sample'
        passed = value.get('passed')
        assert type(passed) is bool, f'{scenario}: missing capability outcome in {path}'
        if passed:
            menu = query.get('menu')
            assert type(menu) in (int, float) and math.isfinite(menu) and menu >= 0, f'{scenario}: invalid menu observation in {path}'
        else:
            elapsed = query.get('elapsed_ms')
            assert query.get('failed') is True and type(elapsed) in (int, float) and math.isfinite(elapsed) and elapsed >= 20000, f'{scenario}: collection failure in {path}; not a missing capability'
        outcomes.append(passed)
    assert all(outcomes) or not any(outcomes), f'{scenario}: inconsistent capability results'
    return not any(outcomes)

def summarize_measurements(report, startups, samples, cases=None):
    cases = CASES if cases is None else cases
    assert set(report['modes']) == set(MODES), 'Incomplete comparison modes'
    for mode in MODES:
        measured = report['modes'][mode]
        assert len(measured['startup']) == startups, f'{mode}: insufficient startup samples'
        if startups:
            measured['startup'] = stats(measured['startup'])
        assert set(measured['hot']) == set(cases), f'{mode}: incomplete scenario matrix'
        for scenario, groups in measured['hot'].items():
            assert set(groups) == {'cache_miss', 'cache_hit'}, f'{mode}/{scenario}: incomplete cache matrix'
            for measurement in groups.values():
                assert len(measurement['input_echo']) == samples, f'{mode}/{scenario}: insufficient echo samples'
                measurement['input_echo'] = stats(measurement['input_echo'])
                if measurement['menu'] is None:
                    assert mode == 'plain' or (mode == 'inshellisense' and
                        measurement.get('menu_status') == 'not_observed_in_capability_probe' and
                        len(measurement.get('capability_probes', [])) == 3), f'{mode}/{scenario}: unexplained missing menu'
                else:
                    assert len(measurement['menu']) == samples, f'{mode}/{scenario}: insufficient menu samples'
                    measurement['menu'] = stats(measurement['menu'])

def run(options):
    section = options.section
    if section == 'hot':
        assert options.scenario and options.capability_report, 'Hot shards require scenario and capability evidence'
        assert options.samples % 10 == 0 and options.pair_count and options.pair_start >= 0, 'Invalid hot shard'
        assert options.pair_start + options.pair_count <= options.samples // 10, 'Hot shard exceeds sample target'
    plan = read(options.plan)
    out = Path(options.output).resolve()
    out.mkdir(parents=True, exist_ok=False)
    write(out/'plan.json', plan)
    assert set(plan['modes']) == set(MODES), 'Four modes required'
    assert not options.formal or (options.samples >= 300 and options.startups >= 30), 'Formal sample minimums not met'
    build = json.loads(subprocess.check_output([plan['modes']['candidate']['program'], 'doctor', '--json']))['build']
    assert build.get('allocation_diagnostics') is not True, 'Allocation diagnostics cannot qualify as comparison timing'
    assert not options.formal or (not build['dirty'] and build['default_host_mode'] == 'direct' and build['profile'] == 'release'), 'Unfrozen candidate'
    env = dict(os.environ, **plan['environment'])
    native_plan = plan.get('native_observer')
    if native_plan:
        assert native_plan['abi_version'] == 1 and native_plan['interface_version'] == 4, 'Unsupported native observer contract'
        assert sha(native_plan['dll']) == native_plan['sha256'].upper(), 'Native DLL identity mismatch'
    if plan.get('candidate_terminal_restore') == 'native_overlay_v4':
        assert native_plan and plan['environment'].get('BLUEBERRY_NATIVE_OVERLAY') == '4', 'Native candidate requires an explicit frozen native observer'
    env['GIT_OPTIONAL_LOCKS'] = '0'
    assert not any(env.get(name) not in (None, '', '0') for name in ('BLUEBERRY_DIRECT_TRACE', 'BLUEBERRY_TEST_FRAME_DELAY_MS', 'BLUEBERRY_TEST_DISABLE_PREJIT')), 'Diagnostic timing environment'
    profile_paths = json.loads(subprocess.check_output([plan['shell'], '-NoProfile', '-NonInteractive', '-Command',
        '[Console]::WriteLine((@($PROFILE.AllUsersAllHosts,$PROFILE.AllUsersCurrentHost,$PROFILE.CurrentUserAllHosts,$PROFILE.CurrentUserCurrentHost)|ConvertTo-Json -Compress))'], env=env))
    profile_hash = hashlib.sha256()
    profiles = []
    for value in profile_paths:
        encoded = value.encode()
        profile_hash.update(struct.pack('<Q', len(encoded))+encoded)
        path = Path(value)
        if path.exists():
            data = path.read_bytes()
            profile_hash.update(b'\1'+struct.pack('<Q', len(data))+data)
            profiles.append(dict(path=value, sha256=sha(path)))
        else:
            profile_hash.update(b'\0')
            profiles.append(dict(path=value, sha256=None))
    fixture_files = inventory([plan['fixtures']])
    write(out/'fixtures.json', fixture_files)
    frozen = inventory([__file__, options.plan, plan['probe'], plan['shell'], plan['original_editor']]+[v['program'] for v in plan['modes'].values()])
    if native_plan:
        frozen.extend(inventory([Path(native_plan['dll']).parent]))
    report = dict(schema=3, profile=plan['profile'], formal=options.formal, source_commit=build['commit'], source_dirty=build['dirty'],
        shell=plan['shell'], psreadline_version=plan['psreadline_version'], profile_mode='with_profile', profile_sha256=profile_hash.hexdigest().upper(),
        profile_inventory=profiles, machine_id=env['COMPUTERNAME'], power_policy=hashlib.sha256(subprocess.check_output(['powercfg.exe','/GETACTIVESCHEME'])).hexdigest().upper(),
        terminal_rows=30, terminal_columns=120, trace='disabled', os_cache_cleared=False, startup_pairs=options.startups,
        hot_samples_per_mode=options.samples, startup_orders=[], hot_session_orders={}, modes={}, probe_sha256=sha(plan['probe']),
        collector_sha256=sha(__file__), plan_sha256=sha(options.plan),
        measurement_complete=False, performance_passed=None, performance_evaluation='separate_direct_matrix',
        observed_stages=dict(input_echo='probe_model', menu='first_expected_candidate_in_probe_model',
            complete_candidates='not_measured', screen_pixels='not_measured'), passed=False)
    report['native_observer'] = None
    report['probe_build_identity'] = None
    if native_plan:
        report['observed_stages'].update(input_echo='production_terminalcore_buffer', menu='first_expected_candidate_in_native_buffer')
    for mode, spec in plan['modes'].items():
        dependencies = inventory(spec['dependency_paths'])
        frozen.extend(dependencies)
        report['modes'][mode] = dict(sha256=None if mode == 'plain' else sha(spec['program']),
            entry=dict(path=spec['program'],sha256=sha(spec['program'])),dependencies=dependencies,
            transport={'candidate':'pipe','beta6':'osc'}.get(mode,'not_applicable'),
            host_mode={'candidate':'direct','beta6':'nested'}.get(mode,'not_applicable'),
            transport_degraded=False if mode in ('candidate','beta6') else None,startup=[],hot={},first_queries=[])
    write(out/'frozen.json', frozen)
    report['section'] = section
    report['section_complete'] = False
    report['fixture_sha256'] = sha(out/'fixtures.json')
    report['frozen_sha256'] = sha(out/'frozen.json')
    if section == 'hot':
        report.update(scenario=options.scenario, pair_start=options.pair_start, pair_count=options.pair_count)

    def session(name, mode, scenario, queries, data_dir, allow_timeout=False):
        folder = out/name
        folder.mkdir(parents=True, exist_ok=False)
        spec = plan['modes'][mode]
        data_dir.mkdir(parents=True, exist_ok=True)
        session_env = dict(plan['environment'], GIT_OPTIONAL_LOCKS='0')
        if native_plan:
            session_env.update(BLUEBERRY_NATIVE_MODEL_DLL=native_plan['dll'], BLUEBERRY_NATIVE_MODEL_SHA256=native_plan['sha256'])
        if mode == 'inshellisense':
            session_env['ISTERM'] = ''
        description = dict(program=spec['program'], args=[arg.replace('{data_dir}', str(data_dir)) for arg in spec['args']],
            style=spec['style'], environment=session_env, cwd=str(Path(plan['fixtures'])/CASES[scenario][0]),
            queries=queries, metadata_path=str(folder/'actual-shell.json'))
        if mode in ('candidate','beta6'):
            description.update(data_dir=str(data_dir),expected_host_mode=report['modes'][mode]['host_mode'])
        write(folder/'session.json', description)
        with (folder/'probe.log').open('wb') as log:
            process = subprocess.run([plan['probe'],'--session',str(folder/'session.json'),'--output',str(folder/'result.json')],
                stdout=log,stderr=subprocess.STDOUT,timeout=120+25*len(queries))
        value = read(folder/'result.json')
        observed_probe = probe_identity(value, options.formal)
        if report['probe_build_identity'] is None:
            report['probe_build_identity'] = observed_probe
            if observed_probe: report.update(observed_probe)
        assert report['probe_build_identity'] == observed_probe, 'Probe build changed within collection'
        observed = value.get('native_observer')
        if native_plan:
            assert observed and observed['dll_sha256'].upper() == native_plan['sha256'].upper(), 'Native runtime DLL mismatch'
            assert observed['abi_version'] == 1 and observed['companion_interface_version'] == 4, 'Native runtime ABI mismatch'
            assert observed['backend'] == 'production_terminalcore_buffer' and observed['gui_renderer'] is False, 'Unexpected native observation stage'
            if report['native_observer'] is None: report['native_observer'] = observed
            assert report['native_observer'] == observed, 'Native observer changed within collection'
        else:
            assert observed is None, 'Unplanned native observer; collection invalid'
        if mode == 'candidate' and plan.get('candidate_terminal_restore'):
            assert value.get('adapter', {}).get('terminal_restore') == plan['candidate_terminal_restore'], 'Candidate used another restoration backend'
        runtime = value.get('probe_conpty', {})
        assert runtime.get('mode') == 'pinned' and runtime.get('sha256') == build['conpty']['sha256'] and runtime.get('files') == build['conpty']['files'], f'{name}: ConPTY identity mismatch'
        report.setdefault('probe_conpty', runtime)
        assert report['probe_conpty'] == runtime, f'{name}: ConPTY changed within comparison'
        actual = value.get('actual_shell', {})
        assert actual.get('psreadline') == plan['psreadline_version'], f'{name}: editor mismatch {actual}'
        dll_name='Microsoft.PowerShell.PSReadLine2.dll' if plan['psreadline_version'] == '2.0.0' else 'Microsoft.PowerShell.PSReadLine.dll'
        expected_dll = next(item['sha256'].upper() for item in build['private_editors']['files'] if item['version'] == plan['psreadline_version'] and item['path'] == dll_name) if mode == 'candidate' else sha(Path(plan['original_editor']).parent/dll_name)
        assert actual.get('dll_sha256') == expected_dll, f'{name}: actual DLL identity mismatch'
        assert actual.get('shell','').startswith('5.' if plan['profile'].startswith('ps51') else '7.'), f'{name}: shell mismatch'
        report.setdefault('shell_version', actual['shell'])
        assert actual['shell'] == report['shell_version'], 'Shell changed within comparison'
        if mode in ('candidate','beta6'):
            adapter = value.get('adapter',{})
            assert adapter['transport'] == report['modes'][mode]['transport'], f'{name}: transport degraded'
            if mode == 'candidate':
                assert adapter.get('host_mode') == 'direct' and adapter.get('editor_mode') == 'editor_hooks_v1' and adapter.get('automatic_menu') is True, f'{name}: editor fallback'
        if not allow_timeout:
            assert process.returncode == 0 and value['passed'], f'{name}: retained failure'
        return folder/'result.json', value

    def query(scenario,index,**extra):
        _, line, expected = CASES[scenario]
        return dict(line=line+str(index),expected=expected+str(index),**extra)

    try:
        unavailable = {}
        report['capability_probes'] = {}
        for scenario in CASES if section in ('full', 'capability') else ():
            probes = []
            for index in range(3):
                path,value = session(f'capability/{scenario}-{index}','inshellisense',scenario,[query(scenario,index)],out/'data/capability',True)
                probes.append((path,value))
            report['capability_probes'][scenario] = [dict(report=path.relative_to(out).as_posix(), sha256=sha(path)) for path, _ in probes]
            if not capability_unavailable(scenario, probes):
                continue
            unavailable[scenario] = [dict(report=path.relative_to(out).as_posix(),sha256=sha(path),line=value['queries'][0]['line'],expected=value['queries'][0]['expected']) for path,value in probes]
        report['unavailable'] = unavailable
        if section == 'hot':
            capability_path = Path(options.capability_report).resolve()
            capability = read(capability_path)
            assert capability.get('section') == 'capability' and capability.get('section_complete') is True, 'Incomplete capability evidence'
            for key in ('source_commit', 'source_dirty', 'formal', 'profile', 'profile_sha256', 'power_policy',
                        'probe_sha256', 'collector_sha256', 'plan_sha256', 'fixture_sha256', 'frozen_sha256'):
                assert capability.get(key) == report.get(key), f'Capability identity mismatch: {key}'
            cap_native = capability.get('native_observer')
            assert (cap_native is not None) == bool(native_plan), 'Capability observation backend mismatch'
            if cap_native:
                assert cap_native['dll_sha256'].upper() == native_plan['sha256'].upper(), 'Capability native DLL mismatch'
            report['native_observer'] = cap_native
            unavailable = capability['unavailable']
            assert set(capability['capability_probes']) == set(CASES), 'Incomplete capability scenarios'
            for scenario, evidence in capability['capability_probes'].items():
                probes = []
                for item in evidence:
                    path = capability_path.parent / item['report']
                    assert sha(path) == item['sha256'], 'Capability probe changed'
                    probes.append((path, read(path)))
                assert capability_unavailable(scenario, probes) == (scenario in unavailable), 'Capability summary mismatch'
            for evidence in unavailable.values():
                for item in evidence:
                    path = capability_path.parent / item['report']
                    assert sha(path) == item['sha256'], 'Capability probe changed'
                    item['report'] = str(path)
            report['capability_evidence'] = dict(path=str(capability_path), sha256=sha(capability_path))
            report['unavailable'] = unavailable
        for index in range(options.startups if section in ('full', 'startup') else 0):
            order = rotate(index)
            report['startup_orders'].append(order)
            for mode in order:
                _,value = session(f'startup/{index}-{mode}',mode,'root',[],out/f'data/startup/{index}/{mode}')
                report['modes'][mode]['startup'].append(value['startup'])
            print(f'startup {index+1}/{options.startups}',flush=True)
        scenarios = CASES if section == 'full' else (options.scenario,) if section == 'hot' else ()
        for scenario in scenarios:
            report['hot_session_orders'][scenario] = {cache:[] for cache in ('cache_miss','cache_hit')}
            for mode in MODES:
                report['modes'][mode]['hot'][scenario] = {cache:dict(input_echo=[],menu=[] if mode != 'plain' else None,
                    cache_capability='supported' if mode in ('candidate','beta6') else 'not_applicable') for cache in ('cache_miss','cache_hit')}
            pairs = range(math.ceil(options.samples/10)) if section == 'full' else range(options.pair_start, options.pair_start + options.pair_count)
            for pair in pairs:
                order = rotate(pair)
                before = {}
                for cache in ('cache_miss','cache_hit'):
                    report['hot_session_orders'][scenario][cache].append(order)
                    for mode in order:
                        data_dir = out/f'data/hot/{scenario}/{pair}/{mode}'
                        echo_only = mode == 'inshellisense' and scenario in unavailable
                        queries = [query(scenario,pair%10,warmup=True,echo_only=echo_only)] + [query(scenario,(pair+offset)%10,echo_only=echo_only) for offset in range(min(10,options.samples-pair*10))]
                        _,value = session(f'hot/{scenario}/{pair}-{cache}-{mode}',mode,scenario,queries,data_dir)
                        measured = report['modes'][mode]['hot'][scenario][cache]
                        report['modes'][mode]['first_queries'].append(dict(scenario=scenario,cache=cache,session=pair,observation=value['queries'][0]))
                        for item in value['queries'][1:]:
                            measured['input_echo'].append(item['input_echo'])
                            if mode != 'plain' and not echo_only:
                                measured['menu'].append(item['menu'])
                        if echo_only:
                            measured.update(menu=None,menu_status='not_observed_in_capability_probe',capability_probes=unavailable[scenario])
                        if mode in ('candidate','beta6'):
                            cache_file=data_dir/'commands.json'
                            assert read(cache_file).get('complete') is True, 'Incomplete command cache'
                            identity=(sha(cache_file),cache_file.stat().st_mtime_ns)
                            if cache == 'cache_miss': before[mode]=identity
                            else: assert identity == before[mode], 'Cache-hit rewrote the cache'
                print(f'{scenario} pair {pair+1}/{math.ceil(options.samples/10)}',flush=True)
                write(out/'progress.json',report)
        if section != 'capability':
            summarize_measurements(report, options.startups if section in ('full', 'startup') else 0,
                                   options.samples if section == 'full' else options.pair_count * 10 if section == 'hot' else 0,
                                   scenarios)
        assert all(sha(item['path']) == item['sha256'] for item in frozen), 'Frozen dependency changed'
        assert inventory([plan['fixtures']]) == fixture_files, 'Fixture changed'
        assert profiles_unchanged(profiles), 'Profile changed during comparison'
        report['section_complete']=True
        report['measurement_complete']=section == 'full'
        report['passed']=True  # Historical alias for collection validity, not a performance gate.
    except Exception as error:
        report['error']=str(error)
        raise
    finally:
        write(out/'comparison.json',report)

if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plan',required=True)
    parser.add_argument('--output',required=True)
    parser.add_argument('--samples',type=int,default=30)
    parser.add_argument('--startups',type=int,default=10)
    parser.add_argument('--formal',action='store_true')
    parser.add_argument('--section', choices=('full', 'capability', 'startup', 'hot'), default='full')
    parser.add_argument('--scenario', choices=CASES)
    parser.add_argument('--pair-start', type=int, default=0)
    parser.add_argument('--pair-count', type=int)
    parser.add_argument('--capability-report')
    run(parser.parse_args())
