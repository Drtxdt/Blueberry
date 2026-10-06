"""Fetch and verify the pinned Microsoft ConPTY runtime before Windows builds.

Only the two named x64 files are extracted. Cargo never downloads dependencies
at build time, and the product never downloads a runtime at session startup.
"""
import hashlib
import io
import json
from pathlib import Path
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parent.parent


def prepare():
    spec = json.loads((ROOT / "vendor/conpty/upstream.json").read_text())
    output = ROOT / "target/conpty" / spec["version"]
    output.mkdir(parents=True, exist_ok=True)
    archive = output / "package.nupkg"
    if not archive.is_file():
        with urllib.request.urlopen(spec["url"], timeout=120) as response:
            data = response.read()
        if hashlib.sha256(data).hexdigest() != spec["sha256"].lower():
            raise ValueError("ConPTY package SHA-256 mismatch")
        archive.write_bytes(data)
    data = archive.read_bytes()
    if hashlib.sha256(data).hexdigest() != spec["sha256"].lower():
        raise ValueError("Cached ConPTY package SHA-256 mismatch")
    with zipfile.ZipFile(io.BytesIO(data)) as package:
        for entry in spec["files"]:
            content = package.read(entry["member"])
            if hashlib.sha256(content).hexdigest() != entry["sha256"].lower():
                raise ValueError("ConPTY payload SHA-256 mismatch: " + entry["path"])
            path = output / entry["path"]
            if not path.is_file() or path.read_bytes() != content:
                path.write_bytes(content)
    print("Verified " + spec["package"] + " " + spec["version"] + " (x64)")


if __name__ == "__main__":
    prepare()
