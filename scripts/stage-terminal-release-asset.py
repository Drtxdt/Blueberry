"""Download the exact separately pinned companion artifact without repacking it."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re


def stage(dependency, output, download, repository, token):
    name = dependency['asset_name']
    if not isinstance(name, str) or not re.fullmatch(r'blueberry-terminal-[0-9A-Za-z.-]+\.zip', name):
        raise ValueError('unsafe Terminal release asset name')
    source = dependency['source_commit']
    if not re.fullmatch(r'[0-9a-fA-F]{40}', source):
        raise ValueError('Terminal source commit is not pinned')
    output.mkdir(parents=True, exist_ok=True)
    destination = output / name
    checksum = output / (name + '.sha256')
    if destination.exists() or checksum.exists():
        raise ValueError('Terminal release asset already exists')
    partial = destination.with_suffix('.partial')
    if partial.exists():
        raise ValueError('Terminal download partial already exists')
    download(dependency, repository, token, partial, source)
    with partial.open('rb') as stream:
        digest = hashlib.file_digest(stream, 'sha256').hexdigest().upper()
    if digest != dependency['package_sha256'].upper():
        raise ValueError('original Terminal artifact SHA-256 mismatch')
    partial.rename(destination)
    with checksum.open('x', encoding='utf-8', newline='\n') as stream:
        stream.write(f'{digest}  {name}\n')
    return destination.resolve()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('evidence', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    evidence = json.loads(args.evidence.read_text(encoding='utf-8-sig'))
    dependency = evidence.get('terminal_dependency')
    if dependency is None:
        # Historical releases did not use the companion. The package validator
        # separately requires this record for builds with native interface 4.
        print('')
        return
    spec = importlib.util.spec_from_file_location('materialize', Path(__file__).with_name('materialize-release-artifacts.py'))
    materialize = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(materialize)
    print(stage(dependency, args.output, materialize.download, os.environ['GITHUB_REPOSITORY'], os.environ['GH_TOKEN']))


if __name__ == '__main__':
    main()
