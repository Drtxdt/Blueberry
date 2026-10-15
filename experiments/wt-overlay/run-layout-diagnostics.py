"""CI-only dedicated portable copies: stream and Win32-record reply diagnostics.

No installed app, real user settings or shell profile is used. A hosted runner
without a usable WinUI desktop must report failure, never infer support.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import time
import uuid


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--layout", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    layout = args.layout.resolve(strict=True)
    output = args.output.resolve()
    if output.exists() or (layout / "settings").exists():
        raise SystemExit("Dedicated diagnostic directories must be new")
    for name in ("WindowsTerminal.exe", ".portable", "Microsoft.Terminal.Settings.Model.dll", "PROTOTYPE-README.txt"):
        if not (layout / name).is_file():
            raise SystemExit("Incomplete independent layout: " + name)
    output.mkdir(parents=True)
    summary = {"schema": 1, "diagnostic_only": True, "installed_terminal_modified": False,
               "pixel_visibility_measured": False, "results": [], "passed": False}
    for kind in ("stream", "records", "native_menu"):
        root = output / kind
        copy = root / "terminal"
        shutil.copytree(layout, copy)
        settings = copy / "settings"
        settings.mkdir()
        receipt = root / "runtime.json"
        probe = Path(__file__).with_name("probe-runtime.py" if kind == "stream" else "probe-win32-reply.py" if kind == "records" else "probe-native-menu.cs").resolve()
        if kind == 'native_menu':
            executable = root/'native-menu-probe.exe'
            repository = Path(__file__).resolve().parents[2]
            import os
            compiler = Path(os.environ['SystemRoot'])/'Microsoft.NET/Framework64/v4.0.30319/csc.exe'
            subprocess.run([str(compiler), '/nologo', '/target:exe', '/optimize+', '/codepage:65001',
                            '/out:'+str(executable), '/reference:System.Core.dll', str(probe),
                            str(repository/'shell/direct-bridge.cs'), str(repository/'shell/native-overlay.cs')], check=True)
            args = [str(executable), str(receipt)]
        else:
            args = [sys.executable, str(probe)]
            args += ["--output", str(receipt)] if kind == "stream" else [str(receipt)]
        profile = "{" + str(uuid.uuid4()) + "}"
        config = {"defaultProfile": profile, "firstWindowPreference": "defaultProfile",
                  "profiles": {"defaults": {"closeOnExit": "always"}, "list": [
                      {"guid": profile, "name": "Independent native reply probe",
                       "commandline": subprocess.list2cmdline(args), "startingDirectory": str(root)}]},
                  "disabledProfileSources": ["Windows.Terminal.PowershellCore", "Windows.Terminal.Wsl", "Windows.Terminal.Azure"]}
        (settings / "settings.json").write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")
        startup = subprocess.STARTUPINFO()
        startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = 0
        row = {"kind": kind, "probe_sha256": hashlib.sha256(probe.read_bytes()).hexdigest(),
               "terminal_sha256": hashlib.sha256((copy / "WindowsTerminal.exe").read_bytes()).hexdigest()}
        with (root / "launcher.log").open("wb") as log:
            process = subprocess.Popen([str(copy / "WindowsTerminal.exe"), "--window", "new", "new-tab",
                                        "--profile", profile], startupinfo=startup, stdout=log, stderr=log)
            row["pid"] = process.pid
            try:
                deadline = time.monotonic() + 45
                while not receipt.exists() and time.monotonic() < deadline and process.poll() is None:
                    time.sleep(.2)
                row["result"] = json.loads(receipt.read_text(encoding="utf-8")) if receipt.exists() else None
                if process.poll() is None:
                    try:
                        process.wait(10)
                    except subprocess.TimeoutExpired:
                        row["own_launcher_terminated"] = True
            finally:
                if process.poll() is None:
                    process.terminate()
                    process.wait(5)
            row["exit_code"] = process.returncode
        summary["results"].append(row)
    summary["passed"] = all(row["result"] and row["result"]["supported"] and row["exit_code"] == 0 for row in summary["results"])
    (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"passed": summary["passed"], "modes": [
        {"kind": row["kind"], "supported": bool(row["result"] and row["result"]["supported"]), "exit_code": row["exit_code"]}
        for row in summary["results"]]}))
    return 0 if summary["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
