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
    return hashlib.file_digest(Path(path).open('rb'), 'sha256').hexdigest().upper()

def inventory(paths):
    files = set()
    for value in paths:
        path = Path(value).resolve(strict=True)
        files.update(item for item in path.rglob('*') if item.is_file()) if path.is_dir() else files.add(path)
    return [dict(path=str(path), sha256=sha(path)) for path in sorted(files)]

def stats(values):
    ordered = sorted(values)
    return dict(samples=values, median=statistics.median(values), p95=ordered[max(0, math.ceil(len(values)*.95)-1)], unit='ms')

def rotate(index):
    return list(MODES[index % 4:] + MODES[:index % 4])

def run(options):
    plan = read(options.plan)
    out = Path(options.output).resolve()
    out.mkdir(parents=True, exist_ok=False)
    write(out/'plan.json', plan)
    assert set(plan['modes']) == set(MODES), 'Four modes required'
    assert not options.formal or (options.samples >= 300 and options.startups >= 30), 'Formal sample minimums not met'
    build = json.loads(subprocess.check_output([plan['modes']['candidate']['program'], 'doctor', '--json']))['build']
    assert not options.formal or (not build['dirty'] and build['default_host_mode'] == 'direct' and build['profile'] == 'release'), 'Unfrozen candidate'
    env = dict(os.environ, **plan['environment'])
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
    frozen = inventory([plan['probe'], plan['shell'], plan['original_editor']]+[v['program'] for v in plan['modes'].values()])
    report = dict(schema=3, profile=plan['profile'], formal=options.formal, source_commit=build['commit'], source_dirty=build['dirty'],
        shell=plan['shell'], psreadline_version=plan['psreadline_version'], profile_mode='with_profile', profile_sha256=profile_hash.hexdigest().upper(),
        profile_inventory=profiles, machine_id=env['COMPUTERNAME'], power_policy=hashlib.sha256(subprocess.check_output(['powercfg.exe','/GETACTIVESCHEME'])).hexdigest().upper(),
        terminal_rows=30, terminal_columns=120, trace='disabled', os_cache_cleared=False, startup_pairs=options.startups,
        hot_samples_per_mode=options.samples, startup_orders=[], hot_session_orders={}, modes={}, probe_sha256=sha(plan['probe']), passed=False)
    for mode, spec in plan['modes'].items():
        dependencies = inventory(spec['dependency_paths'])
        frozen.extend(dependencies)
        report['modes'][mode] = dict(sha256=None if mode == 'plain' else sha(spec['program']),
            entry=dict(path=spec['program'],sha256=sha(spec['program'])),dependencies=dependencies,
            transport={'candidate':'pipe','beta6':'osc'}.get(mode,'not_applicable'),
            host_mode={'candidate':'direct','beta6':'nested'}.get(mode,'not_applicable'),
            transport_degraded=False if mode in ('candidate','beta6') else None,startup=[],hot={},first_queries=[])
    write(out/'frozen.json', frozen)

    def session(name, mode, scenario, queries, data_dir, allow_timeout=False):
        folder = out/name
        folder.mkdir(parents=True, exist_ok=False)
        spec = plan['modes'][mode]
        data_dir.mkdir(parents=True, exist_ok=True)
        session_env = dict(plan['environment'], GIT_OPTIONAL_LOCKS='0')
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
        for scenario in CASES:
            probes = []
            for index in range(3):
                path,value = session(f'capability/{scenario}-{index}','inshellisense',scenario,[query(scenario,index)],out/'data/capability',True)
                probes.append((path,value))
            if all(value['passed'] for _,value in probes):
                continue
            assert all(not value['passed'] and len(value['queries']) == 1 and value['queries'][0].get('elapsed_ms',0) >= 20000
                       and value['queries'][0].get('input_echo') is not None for _,value in probes), f'{scenario}: inconsistent capability results'
            unavailable[scenario] = [dict(report=path.relative_to(out).as_posix(),sha256=sha(path),line=value['queries'][0]['line'],expected=value['queries'][0]['expected']) for path,value in probes]
        for index in range(options.startups):
            order = rotate(index)
            report['startup_orders'].append(order)
            for mode in order:
                _,value = session(f'startup/{index}-{mode}',mode,'root',[],out/f'data/startup/{index}/{mode}')
                report['modes'][mode]['startup'].append(value['startup'])
            print(f'startup {index+1}/{options.startups}',flush=True)
        for scenario in CASES:
            report['hot_session_orders'][scenario] = {cache:[] for cache in ('cache_miss','cache_hit')}
            for mode in MODES:
                report['modes'][mode]['hot'][scenario] = {cache:dict(input_echo=[],menu=[] if mode != 'plain' else None,
                    cache_capability='supported' if mode in ('candidate','beta6') else 'not_applicable') for cache in ('cache_miss','cache_hit')}
            for pair in range(math.ceil(options.samples/10)):
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
        for mode in MODES:
            report['modes'][mode]['startup']=stats(report['modes'][mode]['startup'])
            for groups in report['modes'][mode]['hot'].values():
                for measurement in groups.values():
                    measurement['input_echo']=stats(measurement['input_echo'])
                    if measurement['menu'] is not None: measurement['menu']=stats(measurement['menu'])
        assert all(sha(item['path']) == item['sha256'] for item in frozen), 'Frozen dependency changed'
        assert inventory([plan['fixtures']]) == fixture_files, 'Fixture changed'
        report['passed']=True
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
    run(parser.parse_args())
