"""Verify the sealed, local evidence tree before and after Actions upload."""
import hashlib
import json
import os
from pathlib import Path
import re
import stat


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest().upper()


def verify(root, manifest_name, expected_hash):
    if not re.fullmatch(r'[A-Za-z0-9_-]+\.json', manifest_name):
        raise ValueError('Expected a manifest basename')
    if not re.fullmatch(r'[A-Fa-f0-9]{64}', expected_hash):
        raise ValueError('Expected a manifest SHA-256')
    root = Path(root).absolute()
    if not root.is_dir():
        raise ValueError('Evidence root missing')
    def checked(entry):
        info = entry.lstat()
        if stat.S_ISLNK(info.st_mode) or getattr(info, 'st_file_attributes', 0) & 0x400:
            raise ValueError('Evidence must not contain links or reparse points')
        return entry
    entries = [checked(root)]
    for folder, directories, files in os.walk(root, followlinks=False):
        entries.extend(checked(Path(folder) / name) for name in directories + files)
    manifest = root / manifest_name
    if sha(manifest) != expected_hash.upper():
        raise ValueError('Upload manifest changed')
    data = json.loads(manifest.read_text(encoding='utf-8'))
    if data.get('schema') != 1 or not isinstance(data.get('files'), dict) or not data['files']:
        raise ValueError('Missing evidence file inventory')
    observed = {p.relative_to(root).as_posix() for p in entries if p.is_file() and p != manifest}
    if observed != set(data['files']):
        raise ValueError('Evidence file inventory changed')
    for name, expected in data['files'].items():
        if not re.fullmatch(r'[A-Fa-f0-9]{64}', expected) or sha(root / name) != expected.upper():
            raise ValueError('Evidence file changed: ' + name)
    return root


if __name__ == '__main__':
    root = verify(os.environ['BLUEBERRY_EVIDENCE_UPLOAD_ROOT'],
                  os.environ['EVIDENCE_MANIFEST'], os.environ['EVIDENCE_MANIFEST_SHA256'])
    if not re.fullmatch(r'blueberry-raw-[a-z0-9-]+', os.environ['EVIDENCE_NAME']):
        raise ValueError('Unexpected evidence artifact name')
    if not re.fullmatch(r'blueberry-evidence-[a-f0-9]{32}', os.environ['EVIDENCE_RUNNER_LABEL']):
        raise ValueError('Expected a unique one-job runner label')
    with open(os.environ['GITHUB_OUTPUT'], 'a', encoding='utf-8') as output:
        output.write('path=' + str(root) + '\n')
    print('Verified sealed evidence inventory and SHA-256 values')
