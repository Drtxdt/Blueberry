"""Download hash-pinned raw evidence from this repository's Actions artifacts."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath, PureWindowsPath
import stat
import tempfile
import urllib.error
import urllib.request
import zipfile


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest().upper()


def extract_verified(archive_path, descriptor, root):
    if digest(archive_path) != descriptor['sha256'].upper():
        raise ValueError('raw artifact archive SHA-256 mismatch')
    root = root.resolve(strict=True)
    files = descriptor['files']
    if not isinstance(files, dict) or not files:
        raise ValueError('raw artifact file manifest missing')
    with zipfile.ZipFile(archive_path) as archive:
        members = [item for item in archive.infolist() if not item.is_dir()]
        if len(members) != len(files) or {item.filename for item in members} != set(files):
            raise ValueError('raw artifact members differ from manifest')
        if sum(item.file_size for item in members) > 2 * 1024**3:
            raise ValueError('raw artifact exceeds 2 GiB unpacked limit')
        # Validate every path and byte before publishing any file.
        with tempfile.TemporaryDirectory() as staging:
            prepared = []
            destinations = set()
            for index, item in enumerate(members):
                name = item.filename
                path = PurePosixPath(name)
                if (path.is_absolute() or PureWindowsPath(name).drive or '\\' in name
                        or any(part in ('..', '.') or ':' in part for part in path.parts)
                        or stat.S_ISLNK(item.external_attr >> 16)):
                    raise ValueError('unsafe raw artifact path')
                destination = (root / name).resolve()
                if not destination.is_relative_to(root) or destination == root:
                    raise ValueError('raw artifact escapes evidence directory')
                key = destination.as_posix().casefold()
                if key in destinations:
                    raise ValueError('raw artifact paths collide')
                destinations.add(key)
                staged = Path(staging) / str(index)
                with archive.open(item) as source, staged.open('wb') as output:
                    while chunk := source.read(1024 * 1024):
                        output.write(chunk)
                expected = files[name].upper()
                if digest(staged) != expected:
                    raise ValueError('raw artifact member SHA-256 mismatch')
                if destination.exists() and (not destination.is_file() or digest(destination) != expected):
                    raise ValueError('raw artifact would replace existing evidence')
                prepared.append((staged, destination))
            for staged, destination in prepared:
                destination.parent.mkdir(parents=True, exist_ok=True)
                if not destination.exists():
                    with staged.open('rb') as source, destination.open('xb') as output:
                        while chunk := source.read(1024 * 1024):
                            output.write(chunk)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def download(descriptor, repository, token, destination, source_commit):
    artifact_id, run_id = descriptor['artifact_id'], descriptor['run_id']
    if type(artifact_id) is not int or artifact_id <= 0 or type(run_id) is not int or run_id <= 0:
        raise ValueError('artifact and run IDs must be positive integers')
    api = f'https://api.github.com/repos/{repository}/actions/artifacts/{artifact_id}'
    headers = {'Authorization': f'Bearer {token}', 'Accept': 'application/vnd.github+json', 'User-Agent': 'Blueberry-evidence'}
    with urllib.request.urlopen(urllib.request.Request(api, headers=headers), timeout=60) as response:
        metadata = json.load(response)
    run = metadata.get('workflow_run', {})
    if metadata.get('expired') or metadata.get('id') != artifact_id or run.get('id') != run_id or run.get('head_sha') != source_commit:
        raise ValueError('artifact run, source commit, or retention mismatch')
    opener = urllib.request.build_opener(NoRedirect)
    try:
        response = opener.open(urllib.request.Request(api + '/zip', headers=headers), timeout=60)
    except urllib.error.HTTPError as error:
        if error.code != 302 or not error.headers.get('Location', '').startswith('https://'):
            raise
        # Never forward the GitHub token to the signed storage URL.
        response = urllib.request.urlopen(error.headers['Location'], timeout=60)
    with response, destination.open('wb') as output:
        total = 0
        while chunk := response.read(1024 * 1024):
            total += len(chunk)
            if total > 2 * 1024**3:
                raise ValueError('raw artifact download exceeds 2 GiB')
            output.write(chunk)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('evidence', type=Path)
    args = parser.parse_args()
    evidence = json.loads(args.evidence.read_text(encoding='utf-8-sig'))
    for descriptor in evidence.get('raw_artifacts', []):
        with tempfile.TemporaryDirectory() as folder:
            archive = Path(folder) / 'artifact.zip'
            download(descriptor, os.environ['GITHUB_REPOSITORY'], os.environ['GH_TOKEN'], archive, evidence['source_commit'])
            extract_verified(archive, descriptor, args.evidence.parent)


if __name__ == '__main__':
    main()
