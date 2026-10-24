"""Fetch hash-pinned upstream PSReadLine without machine repository registration."""
import argparse
import hashlib
import io
from pathlib import Path
import urllib.request
import zipfile

HASHES = {
    "2.0.0": "99e34f07b930c763d58bd2ff2cc52cdd92641cc430c82b4bad22e972a3f9cf3b",
    "2.4.5": "cb9390e9733208456c234a7971d1ec4a917886c239502aab68f4b71aa4bba235",
}


def prepare(version, root):
    root = Path(root)
    root.mkdir(parents=True, exist_ok=True)
    archive = root / (version + ".nupkg")
    if archive.exists():
        data = archive.read_bytes()
    else:
        url = "https://www.powershellgallery.com/api/v2/package/PSReadLine/" + version
        with urllib.request.urlopen(url, timeout=120) as response:
            data = response.read()
    if hashlib.sha256(data).hexdigest() != HASHES[version]:
        raise ValueError("Original PSReadLine package digest mismatch: " + version)
    # Validate all paths before writing even one package member.
    with zipfile.ZipFile(io.BytesIO(data)) as package:
        destination = (root / "PSReadLine" / version).resolve()
        for entry in package.infolist():
            path = (destination / entry.filename).resolve()
            if not path.is_relative_to(destination):
                raise ValueError("Unsafe package member: " + entry.filename)
        archive.write_bytes(data)
        package.extractall(destination)
    return destination / "PSReadLine.psd1"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", choices=HASHES, required=True)
    parser.add_argument("--root", required=True)
    args = parser.parse_args()
    print(prepare(args.version, args.root))
