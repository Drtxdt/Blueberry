"""Freeze volatile command directories without shrinking the command workload.

Copies actual command file bytes from System32 and temporary Codex arg0 shims.
Other PATH directories retain their normal location (e.g. Git's relative DLLs).
The local OS-file snapshot is a test dependency, not a Release attachment;
publish inventories and checksums, not copied Windows executables.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest().upper()


def command_files(directory, extensions):
    if not directory.is_dir():
        return []
    return sorted((p for p in directory.iterdir() if p.is_file() and p.suffix.lower() in extensions),
                  key=lambda p: p.name.lower())


def snapshot(source, destination, extensions):
    destination.mkdir(parents=True, exist_ok=False)
    files = command_files(source, extensions)
    rows = []
    for path in files:
        checksum = sha(path)
        target = destination / path.name
        shutil.copy2(path, target)
        assert sha(target) == checksum, "Command changed while copying"
        rows.append(dict(name=path.name, source=str(path), sha256=checksum, bytes=path.stat().st_size))
    after = command_files(source, extensions)
    assert [p.name for p in after] == [row["name"] for row in rows], "Command directory changed during snapshot"
    assert all(sha(path) == row["sha256"] for path, row in zip(after, rows)), "Command changed during snapshot"
    assert [p.name for p in command_files(destination, extensions)] == [p.name for p in files], "Incomplete copied command set"
    return rows


def freeze(output, path_value, extensions, selected):
    output = Path(output).resolve()
    output.mkdir(parents=True, exist_ok=False)
    mappings = []
    paths = []
    for index, directory in enumerate(path_value.split(os.pathsep)):
        if selected(directory):
            target = output / f"commands-{index}"
            files = snapshot(Path(directory), target, extensions)
            mappings.append(dict(original=directory, snapshot=str(target), commands=files))
            paths.append(str(target))
        else:
            paths.append(directory)
    manifest = dict(schema=1, original_path=path_value, path=os.pathsep.join(paths),
                    pathext=sorted(extensions), mappings=mappings,
                    release_payload="inventory_only; copied OS binaries are local test dependencies")
    (output / "path-snapshot.json").write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    system = os.path.normcase(str(Path(os.environ["SystemRoot"]) / "System32"))
    def selected(path):
        normalized = os.path.normcase(path.rstrip("\\/"))
        return normalized == system or "/.codex/tmp/arg0/" in path.replace("\\", "/").lower()
    extensions = {x.lower() for x in os.environ.get("PATHEXT", ".COM;.EXE;.BAT;.CMD").split(";") if x}
    extensions = {x if x.startswith(".") else "." + x for x in extensions}
    result = freeze(args.output, os.environ["PATH"], extensions, selected)
    print(json.dumps(dict(directories=len(result["mappings"]),
                          commands=sum(len(x["commands"]) for x in result["mappings"]),
                          bytes=sum(f["bytes"] for x in result["mappings"] for f in x["commands"]))))


if __name__ == "__main__":
    main()
