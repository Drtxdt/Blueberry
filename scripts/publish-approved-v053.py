"""One-version release exception explicitly approved by the maintainer, 2026-10-26.

Promotes original CI bytes only; never builds, merges, or changes published assets.
"""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import zipfile

REPO = 'Drtxdt/Blueberry'
TAG = 'v0.5.3'
SOURCE = '8b97ebbc02d4b4908b060a939f418e1ac9d2507c'
RELEASE_ID = 423917048


def api(path, method='GET', data=None):
    args = ['gh', 'api', f'repos/{REPO}/{path}', '--method', method]
    if data is not None:
        args += ['--input', '-']
    result = subprocess.run(args, input=json.dumps(data) if data is not None else None,
                            text=True, capture_output=True, check=True, timeout=90)
    return json.loads(result.stdout) if result.stdout.strip() else None


def main():
    evidence = json.loads(Path('docs/benchmarks/v0.6/review-20261018/finalization/v0.5.3-manual-release.json').read_text())
    assert evidence['source_commit'] == SOURCE
    assert evidence['manual_release_exception']['authorization'] == '我测试没有问题。直接发版吧'
    run = api('actions/runs/40097143597')
    assert run['conclusion'] == 'success' and run['head_sha'] == '83963b120016c8f7b9fef2425a73297246535914'
    assert api('commits/' + SOURCE)['sha'] == SOURCE
    release = api(f'releases/{RELEASE_ID}')
    assert release['tag_name'] == TAG and release['draft'], 'Refuse to modify a published release'
    assert release['target_commitish'] == SOURCE
    stage = Path('release-assets')
    stage.mkdir()
    for directory in ('original-core', 'original-terminal'):
        for path in Path(directory).rglob('*'):
            if path.is_file():
                destination = stage / path.name
                assert not destination.exists(), 'Duplicate asset name'
                shutil.copyfile(path, destination)
    for package in evidence['packages']:
        path = stage / package['name']
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        assert digest.upper() == package['sha256']
        assert digest in path.with_suffix(path.suffix + '.sha256').read_text().lower()
        with zipfile.ZipFile(path) as archive:
            manifest = json.loads(archive.read('component.json' if 'terminal-component' in path.name else 'release.json'))
            if 'terminal-component' not in path.name:
                assert manifest['build']['commit'] == SOURCE and manifest['version'] == '0.5.3'
            assert len(manifest['files']) == package['verified_manifest_files']
            for entry in manifest['files']:
                assert hashlib.sha256(archive.read(entry['path'])).hexdigest().upper() == entry['sha256'].upper()
    (stage / 'blueberry-v0.5.3-validation-summary.json').write_text(json.dumps(evidence, ensure_ascii=False, indent=2))
    files = sorted(stage.iterdir())
    allowed = {p['name'] for p in evidence['packages']}
    allowed |= {name + '.sha256' for name in list(allowed)}
    allowed |= {'install.ps1', 'blueberry-v0.5.3-release.json', 'blueberry-v0.5.3-validation-summary.json'}
    assert {p.name for p in files} == allowed
    for asset in api(f'releases/{RELEASE_ID}/assets?per_page=100'):
        assert asset['name'] in allowed
        if asset['state'] != 'uploaded':
            api('releases/assets/' + str(asset['id']), 'DELETE')
        else:
            assert asset['digest'] == 'sha256:' + hashlib.sha256((stage / asset['name']).read_bytes()).hexdigest()
    for path in files:
        existing = api(f'releases/{RELEASE_ID}/assets?per_page=100')
        if not any(a['name'] == path.name for a in existing):
            subprocess.run(['gh', 'release', 'upload', TAG, str(path), '--repo', REPO], check=True, timeout=180)
    assets = api(f'releases/{RELEASE_ID}/assets?per_page=100')
    assert {a['name'] for a in assets} == allowed
    for asset in assets:
        path = stage / asset['name']
        assert asset['state'] == 'uploaded' and asset['size'] == path.stat().st_size
        assert asset['digest'] == 'sha256:' + hashlib.sha256(path.read_bytes()).hexdigest()
    published = api(f'releases/{RELEASE_ID}', 'PATCH', {
        'body': Path('docs/v0.5.3-release-notes.md').read_text(),
        'draft': False, 'prerelease': False, 'make_latest': 'true'})
    assert not published['draft'] and published['published_at']
    obj = api('git/ref/tags/' + TAG)['object']
    if obj['type'] == 'tag':
        obj = api('git/tags/' + obj['sha'])['object']
    assert obj['sha'] == SOURCE
    Path('publication-receipt.json').write_text(json.dumps({
        'url': published['html_url'], 'source_commit': SOURCE,
        'published_at': published['published_at'],
        'assets': [{'name': a['name'], 'digest': a['digest'], 'url': a['browser_download_url']} for a in assets]
    }, indent=2))
    print('Published:', published['html_url'])


if __name__ == '__main__':
    main()
