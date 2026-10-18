"""Collect independent idle sessions from a frozen four-way comparison plan.

Run one plan for each supported Shell/editor combination under the owned
benchmark coordinator. Raw snapshots, failed sessions and identities survive
errors. Formal sessions always observe at least 30 seconds, five times/mode.
--candidate-only isolates an optimization A/B side or an additional public
reference; its report cannot replace the final four-way resource matrix.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import struct
import subprocess


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


COMPARISON = module('comparison', 'run-comparison.py')
RESOURCES = module('resources', 'resource-statistics.py')
sha, read, inventory = COMPARISON.sha, COMPARISON.read, COMPARISON.inventory


def write(path, value):
    temporary = path.with_name(path.name + '.tmp')
    temporary.write_text(json.dumps(value, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')
    os.replace(temporary, path)


def run(options):
    plan = read(options.plan)
    assert set(plan['modes']) == set(COMPARISON.MODES), 'Four comparator modes required'
    assert options.sessions >= (5 if options.formal else 1), 'Five independent formal sessions required'
    out = options.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    write(out / 'plan.json', plan)
    environment = dict(os.environ, **plan['environment'], GIT_OPTIONAL_LOCKS='0')
    assert not any(environment.get(name) not in (None, '', '0') for name in
                   ('BLUEBERRY_DIRECT_TRACE', 'BLUEBERRY_TEST_FRAME_DELAY_MS', 'BLUEBERRY_TEST_DISABLE_PREJIT')), 'Diagnostic environment'
    candidate = plan['modes']['candidate']['program']
    build = json.loads(subprocess.check_output([candidate, 'doctor', '--json']))['build']
    assert build.get('allocation_diagnostics') is not True, 'Allocation diagnostics cannot qualify as formal resources'
    assert not options.formal or (build['dirty'] is False and build['profile'] == 'release'), 'Unfrozen candidate'
    profile_paths = json.loads(subprocess.check_output([plan['shell'], '-NoProfile', '-NonInteractive', '-Command',
        '[Console]::WriteLine((@($PROFILE.AllUsersAllHosts,$PROFILE.AllUsersCurrentHost,$PROFILE.CurrentUserAllHosts,$PROFILE.CurrentUserCurrentHost)|ConvertTo-Json -Compress))'], env=environment))
    profiles = []
    profile_hash = hashlib.sha256()
    for value in profile_paths:
        encoded = value.encode()
        profile_hash.update(struct.pack('<Q', len(encoded)) + encoded)
        path = Path(value)
        if path.exists():
            data = path.read_bytes()
            profile_hash.update(b'\1' + struct.pack('<Q', len(data)) + data)
            profiles.append(dict(path=value, sha256=sha(path)))
        else:
            profile_hash.update(b'\0')
            profiles.append(dict(path=value, sha256=None))
    power = subprocess.check_output(['powercfg.exe', '/GETACTIVESCHEME'])
    (out / 'power-policy.txt').write_bytes(power)
    frozen = inventory([__file__, Path(__file__).with_name('run-comparison.py'),
                        Path(__file__).with_name('resource-statistics.py'), options.plan, options.sampler,
                        plan['probe'], plan['shell'], plan['original_editor'], plan['fixtures']]
                       + [spec['program'] for spec in plan['modes'].values()]
                       + [path for spec in plan['modes'].values() for path in spec['dependency_paths']])
    manifest = dict(schema=2, formal=options.formal, profile=plan['profile'],
        source_commit=build['commit'], source_dirty=build['dirty'], executable_sha256=sha(candidate),
        probe_sha256=sha(plan['probe']), sampler_sha256=sha(options.sampler), collector_sha256=sha(__file__),
        profile_inventory=profiles, profile_sha256=profile_hash.hexdigest().upper(),
        machine_id=environment['COMPUTERNAME'], power_policy=hashlib.sha256(power).hexdigest().upper(),
        terminal_rows=30, terminal_columns=120, trace='disabled', os_cache_cleared=False,
        sessions_per_mode=options.sessions, requested_seconds=30, measurement_complete=False,
        mode_scope='candidate_only' if options.candidate_only else 'four_way',
        performance_passed=None, observed_stages=dict(memory='process_tree_snapshots', cpu='one_core_process_tree'),
        orders=[], sessions=[], dependencies=frozen)
    receipt = out / 'resources.json'
    write(receipt, manifest)
    try:
        for repeat in range(options.sessions):
            order = COMPARISON.rotate(repeat)
            if options.candidate_only:
                order = [mode for mode in order if mode == 'candidate']
            manifest['orders'].append(order)
            for mode in order:
                spec = plan['modes'][mode]
                folder = out / f'{repeat}-{mode}'
                folder.mkdir()
                data = folder / 'data'
                data.mkdir()
                env = dict(plan['environment'], GIT_OPTIONAL_LOCKS='0')
                if mode == 'inshellisense': env['ISTERM'] = ''
                description = dict(program=spec['program'], style=spec['style'],
                    args=[arg.replace('{data_dir}', str(data)) for arg in spec['args']],
                    environment=env, cwd=plan['fixtures'], queries=[],
                    metadata_path=str(folder / 'actual-shell.json'),
                    resource_sampler=str(options.sampler.resolve()), resource_output=str(folder / 'raw-resources.json'))
                if mode in ('candidate', 'beta6'):
                    description.update(data_dir=str(data), expected_host_mode={'candidate': 'direct', 'beta6': 'nested'}[mode])
                write(folder / 'session.json', description)
                entry = dict(mode=mode, repeat=repeat, status='running', report=(folder / 'result.json').relative_to(out).as_posix())
                manifest['sessions'].append(entry)
                write(receipt, manifest)
                with (folder / 'probe.log').open('wb') as log:
                    result = subprocess.run([plan['probe'], '--session', str(folder / 'session.json'),
                                             '--output', str(folder / 'result.json')], env=environment,
                                            stdout=log, stderr=subprocess.STDOUT, timeout=150)
                value = read(folder / 'result.json')
                assert value.get('diagnostic_only') is not True, 'Diagnostic editor hooks cannot qualify as resource samples'
                assert result.returncode == 0 and value['passed'] is True, 'Resource session failed'
                actual = value['actual_shell']
                assert actual['psreadline'] == plan['psreadline_version'], 'Resource editor version mismatch'
                prefix = '7.' if plan['profile'].startswith('ps7-') else '5.1.'
                assert actual['shell'].startswith(prefix), 'Resource Shell version mismatch'
                original_directory = Path(plan['original_editor']).parent
                dll_name = 'Microsoft.PowerShell.PSReadLine2.dll' if plan['psreadline_version'] == '2.0.0' else 'Microsoft.PowerShell.PSReadLine.dll'
                expected = (next(item['sha256'] for item in build['private_editors']['files']
                                 if item['version'] == plan['psreadline_version'] and item['path'] == dll_name)
                            if mode == 'candidate' else sha(original_directory / dll_name))
                assert actual['dll_sha256'].upper() == expected.upper(), 'Resource loaded DLL mismatch'
                assert value['program_sha256'].upper() == sha(spec['program']), 'Resource executable identity mismatch'
                runtime = value.get('probe_conpty', {})
                assert runtime.get('mode') == 'pinned', 'Resource probe runtime is unverified'
                for field in ('package', 'version', 'sha256', 'files'):
                    assert runtime.get(field) == build['conpty'].get(field), 'Resource probe ConPTY identity mismatch'
                raw = read(folder / 'raw-resources.json')
                if 'resource_report_sha256' in value:
                    assert value['resource_report_sha256'] == sha(folder / 'raw-resources.json'), 'Raw resource digest mismatch'
                assert RESOURCES.snapshots_match(raw, value['resources']), 'Raw resource snapshot mismatch'
                summary = RESOURCES.summarize(raw)
                entry.update(status='completed', summary=summary, actual_shell=actual, program_sha256=sha(spec['program']),
                             report_sha256=sha(folder / 'result.json'),
                             raw_report=(folder / 'raw-resources.json').relative_to(out).as_posix(), raw_sha256=sha(folder / 'raw-resources.json'))
                write(receipt, manifest)
        assert COMPARISON.profiles_unchanged(profiles), 'Profile changed during resources'
        assert subprocess.check_output(['powercfg.exe', '/GETACTIVESCHEME']) == power, 'Power policy changed during resources'
        assert all(sha(item['path']) == item['sha256'] for item in frozen), 'Resource dependency changed'
        manifest['measurement_complete'] = True
    except BaseException as error:
        manifest['collection_error'] = str(error) or type(error).__name__
        if manifest['sessions'] and manifest['sessions'][-1]['status'] == 'running':
            manifest['sessions'][-1]['status'] = 'invalid'
        raise
    finally:
        write(receipt, manifest)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plan', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--sampler', type=Path, default=Path(__file__).with_name('measure-process-tree.ps1'))
    parser.add_argument('--sessions', type=int, default=5)
    parser.add_argument('--formal', action='store_true')
    parser.add_argument('--candidate-only', action='store_true',
                        help='Optimization/reference side only; does not satisfy final four-way qualification')
    run(parser.parse_args())
