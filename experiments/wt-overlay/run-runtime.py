"""Run the fixed CI prototype in its own new portable directory; no installation."""
import argparse
import hashlib
import json
from pathlib import Path
import stat
import subprocess
import sys
import time
import uuid
import zipfile

EXPECTED_ZIP = "aa983ff4b99953133d967fd157edad5fa1dc55336f8e42a7e0301b6162ae8335"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--zip", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True,
                        help="New dedicated directory, including portable layout and evidence")
    args = parser.parse_args()
    if sys.platform != "win32":
        raise SystemExit("Windows only")
    archive = args.zip.resolve(strict=True)
    root = args.output.resolve()
    if root.exists():
        raise SystemExit("Refusing to reuse or overwrite a runtime directory")
    if hashlib.sha256(archive.read_bytes()).hexdigest() != EXPECTED_ZIP:
        raise SystemExit("Prototype ZIP identity mismatch")
    # Validate the whole inventory before creating/extracting a directory.
    with zipfile.ZipFile(archive) as z:
        names = set()
        for entry in z.infolist():
            name = entry.filename.replace("\\", "/")
            if (name.startswith("/") or ":" in name or ".." in Path(name).parts
                    or name.casefold() in names or stat.S_ISLNK(entry.external_attr >> 16)):
                raise SystemExit("Unsafe or duplicate ZIP member")
            names.add(name.casefold())
        root.mkdir(parents=True)
        z.extractall(root / "portable")
    executables = list((root / "portable").rglob("WindowsTerminal.exe"))
    if len(executables) != 1:
        raise SystemExit("Expected exactly one prototype executable")
    executable = executables[0]
    layout = executable.parent
    for member in (".portable", "Microsoft.Terminal.Settings.Model.dll", "OpenConsole.exe",
                   "TERMINAL-LICENSE.txt", "PROTOTYPE-README.txt"):
        if not (layout / member).is_file():
            raise SystemExit("Incomplete prototype portable layout: " + member)
    settings = layout / "settings"
    settings.mkdir()  # Existing settings in the archive must not be silently reused.
    output = root / "runtime.json"
    probe = Path(__file__).with_name("probe-runtime.py").resolve(strict=True)
    profile = "{" + str(uuid.uuid4()) + "}"
    command = subprocess.list2cmdline([sys.executable, str(probe), "--output", str(output)])
    config = {"defaultProfile": profile, "firstWindowPreference": "defaultProfile",
              "initialCols": 100, "initialRows": 24,
              "disabledProfileSources": ["Windows.Terminal.PowershellCore",
                                         "Windows.Terminal.Wsl", "Windows.Terminal.Azure"],
              "profiles": {"defaults": {"closeOnExit": "always"}, "list": [
                  {"guid": profile, "name": "Blueberry independent interface probe",
                   "commandline": command, "startingDirectory": str(root)}]}}
    (settings / "settings.json").write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")
    startup = subprocess.STARTUPINFO()
    startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
    startup.wShowWindow = 0
    receipt = {"schema": 1, "diagnostic_only": True, "zip_sha256": EXPECTED_ZIP,
               "executable": str(executable),
               "executable_sha256": hashlib.sha256(executable.read_bytes()).hexdigest(),
               "probe_sha256": hashlib.sha256(probe.read_bytes()).hexdigest(),
               "local_settings": str(settings / "settings.json"),
               "installed_terminal_modified": False, "user_profile_modified": False}
    process = None
    try:
        with (root / "launcher.log").open("wb") as log:
            process = subprocess.Popen([str(executable), "--window", "new", "new-tab",
                                        "--profile", profile], startupinfo=startup,
                                       stdout=log, stderr=log)
            receipt["pid"] = process.pid
            deadline = time.monotonic() + 30
            while not output.exists() and time.monotonic() < deadline and process.poll() is None:
                time.sleep(.2)
            if output.exists():
                receipt["result"] = json.loads(output.read_text(encoding="utf-8"))
            else:
                receipt["error"] = "Prototype did not produce runtime evidence"
            if process.poll() is None:
                try:
                    process.wait(10)
                except subprocess.TimeoutExpired:
                    process.terminate()
                    process.wait(5)
                    receipt["own_launcher_terminated"] = True
            receipt["exit_code"] = process.returncode
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            process.wait(5)
        (root / "launcher.json").write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    result = receipt.get("result", {})
    print(json.dumps({"supported": result.get("supported", False), "evidence": str(output),
                      "error": result.get("error", receipt.get("error"))}, ensure_ascii=False))
    return 0 if (result.get("supported") and result.get("reader_cleaned_up")
                 and result.get("console_modes_restored") and receipt.get("exit_code") == 0) else 1


if __name__ == "__main__":
    raise SystemExit(main())
