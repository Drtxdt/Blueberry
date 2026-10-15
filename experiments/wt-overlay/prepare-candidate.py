"""Prepare a fixed two-package manual candidate without installing or opening it.

The manifest must pin both original ZIPs, digests and the embedded Blueberry
source commit. Output is new and confined to this workspace's artifacts tree.
The generated launcher opens a dedicated portable profile only when the user
runs it; existing Terminal settings and PowerShell profiles remain untouched.
"""
import argparse
import hashlib
import json
from pathlib import Path
import stat
import subprocess
import uuid
import zipfile


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest().upper()


def extract(archive, expected, destination):
    assert digest(archive) == expected.upper(), 'Original package SHA-256 mismatch'
    with zipfile.ZipFile(archive) as source:
        names = set()
        assert sum(x.file_size for x in source.infolist()) <= 2 * 1024**3, 'Package exceeds extraction bound'
        for item in source.infolist():
            name = item.filename.replace('\\', '/')
            path = Path(name)
            assert name and all(part not in ('', '.', '..') for part in name.rstrip('/').split('/')), 'Noncanonical package path'
            assert not path.is_absolute() and ':' not in name and '..' not in path.parts, 'Unsafe package path'
            assert name.casefold() not in names and not stat.S_ISLNK(item.external_attr >> 16), 'Duplicate or linked member'
            for part in path.parts:
                assert part and not part.endswith(('.', ' ')), 'Windows path normalization alias'
                stem = part.split('.')[0].upper()
                assert stem not in {'CON', 'PRN', 'AUX', 'NUL'} and not (len(stem) == 4 and stem[:3] in {'COM', 'LPT'} and stem[3] in '123456789'), 'Reserved Windows member'
            names.add(name.casefold())
        destination.mkdir()
        source.extractall(destination)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    repository = Path(__file__).resolve().parents[2]
    output = args.output.resolve()
    assert output.is_relative_to((repository/'artifacts').resolve()) and not output.exists(), 'Output must be new and inside workspace artifacts'
    manifest = json.loads(args.manifest.read_text(encoding='utf-8-sig'))
    assert manifest['schema'] == 1 and manifest['interface_version'] == 4 and manifest['host_mode'] == 'direct', 'Unsupported candidate contract'
    shell = Path(manifest['shell']).resolve(strict=True)
    output.mkdir(parents=True)
    extract(Path(manifest['blueberry_zip']).resolve(strict=True), manifest['blueberry_zip_sha256'], output/'blueberry')
    extract(Path(manifest['terminal_zip']).resolve(strict=True), manifest['terminal_zip_sha256'], output/'terminal')
    products = list((output/'blueberry').rglob('blueberry.exe'))
    terminals = list((output/'terminal').rglob('WindowsTerminal.exe'))
    assert len(products) == len(terminals) == 1, 'Ambiguous package executable'
    product, terminal = products[0], terminals[0]
    doctor = json.loads(subprocess.check_output([str(product), 'doctor', '--json'], timeout=30))
    build = doctor['build']
    assert build['commit'] == manifest['blueberry_source_commit'] and not build['dirty'], 'Embedded source identity mismatch'
    assert build['profile'] == 'release' and build.get('allocation_diagnostics') is not True, 'Diagnostic build cannot be a manual candidate'
    assert build.get('companion_overlay_interface') == 4, 'Blueberry package lacks the fenced companion interface'
    runtime_path = Path(manifest['terminal_runtime_summary']).resolve(strict=True)
    assert digest(runtime_path) == manifest['terminal_runtime_summary_sha256'].upper(), 'Changed native runtime evidence'
    runtime = json.loads(runtime_path.read_text(encoding='utf-8-sig'))
    native = [row for row in runtime['results'] if row['kind'] == 'native_menu']
    assert len(native) == 1 and native[0]['terminal_sha256'].upper() == digest(terminal), 'Native runtime belongs to another Terminal'
    proof = native[0]['result']
    assert native[0]['exit_code'] == 0 and proof['supported'] is True and proof.get('interface_version') == 4, 'Native interface not proven'
    assert proof.get('reader_stopped') is True and proof.get('output_mode_restored') is True, 'Native runtime cleanup unverified'
    layout = terminal.parent
    assert (layout/'.portable').is_file() and (layout/'Microsoft.Terminal.Settings.Model.dll').is_file(), 'Incomplete portable layout'
    settings = layout/'settings'
    settings.mkdir()  # Do not reuse settings hidden inside a downloaded package.
    profile = '{'+str(uuid.uuid4())+'}'
    command = [str(product), 'run', '--shell', str(shell), '--host-mode', 'direct',
               '--psreadline-version', manifest['psreadline_version'], '--data-dir', str(output/'data')]
    config = dict(defaultProfile=profile, initialCols=120, initialRows=30,
                  disabledProfileSources=['Windows.Terminal.PowershellCore', 'Windows.Terminal.Wsl', 'Windows.Terminal.Azure'],
                  profiles=dict(defaults=dict(closeOnExit='never'), list=[dict(guid=profile,
                      name='Blueberry native candidate (isolated)', commandline=subprocess.list2cmdline(command),
                      startingDirectory=str(repository), environment=dict(BLUEBERRY_NATIVE_OVERLAY='4', BLUEBERRY_ACTIVE='',
                          BLUEBERRY_DIRECT_TRACE='0', BLUEBERRY_TRACE='0', BLUEBERRY_NO_HISTORY=''))]))
    if manifest.get('font_face'):
        config['profiles']['defaults']['font'] = dict(face=manifest['font_face'])
    (settings/'settings.json').write_text(json.dumps(config, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
    launcher = output/'launch.ps1'
    literal = lambda value: "'"+str(value).replace("'", "''")+"'"
    launcher.write_text('& '+literal(terminal)+' --window new new-tab --profile '+literal(profile)+'\n', encoding='utf-8-sig')
    receipt = dict(schema=1, kind='prepared manual diagnostic; not final release acceptance',
                   manifest=manifest, installed_terminal_modified=False, user_profile_modified=False,
                   program_sha256=digest(product), terminal_sha256=digest(terminal),
                   shell_sha256=digest(shell), doctor=doctor, launcher=str(launcher),
                   settings_sha256=digest(settings/'settings.json'), launched=False,
                   native_handshake_verified=False, full_history_acceptance=False)
    (output/'candidate.json').write_text(json.dumps(receipt, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
    print(json.dumps(dict(prepared=True, launcher=str(launcher), launched=False)))


if __name__ == '__main__':
    main()
