"""Build the two pinned, session-private PSReadLine modules (never at product startup)."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parent.parent
VENDOR = ROOT / "vendor" / "psreadline"
OUTPUT = ROOT / "target" / "private-editor"


def source_digest():
    digest = hashlib.sha256()
    inputs = [VENDOR / "upstream.json", VENDOR / "EditorIntegration.cs",
              VENDOR / "patches/2.0.0.patch", VENDOR / "patches/2.4.5.patch", Path(__file__)]
    for path in sorted(inputs):
        digest.update((path.relative_to(ROOT).as_posix() + "\0").encode())
        digest.update(path.read_bytes())
    return digest.hexdigest()


def prepare(version, spec, dotnet):
    source = OUTPUT / "src" / version
    archive = ROOT / "target" / "psreadline-upstream" / (version + ".zip")
    archive.parent.mkdir(parents=True, exist_ok=True)
    if not archive.exists():
        with urllib.request.urlopen(
            "https://codeload.github.com/PowerShell/PSReadLine/zip/" + spec["commit"], timeout=120
        ) as response:
            archive.write_bytes(response.read())
    data = archive.read_bytes()
    if hashlib.sha256(data).hexdigest() != spec["archive_sha256"]:
        raise ValueError("Upstream archive digest mismatch: " + version)
    # Re-extract only known archive members; never reuse a previously patched tree.
    with zipfile.ZipFile(io.BytesIO(data)) as z:
        for entry in z.infolist():
            relative = Path(entry.filename).relative_to("PSReadLine-" + spec["commit"])
            if not relative.parts or entry.is_dir():
                continue
            if ".." in relative.parts or relative.is_absolute():
                raise ValueError("Unsafe archive path")
            path = source / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(z.read(entry))
    patch = VENDOR / "patches" / (version + ".patch")
    subprocess.run(["git", "apply", "--ignore-space-change", str(patch)], cwd=source, check=True)
    shutil.copyfile(VENDOR / "EditorIntegration.cs", source / "PSReadLine" / "EditorIntegration.cs")
    project = source / "PSReadLine" / "PSReadLine.csproj"
    if version == "2.0.0":
        text = project.read_text(encoding="utf-8-sig")
        text = text.replace('version="5.1.0-*"', 'version="5.1.0"')
        text = text.replace('version="4.5.0-*"', 'version="4.5.0"')
        project.write_text(text, encoding="utf-8")
    env = os.environ.copy()
    env.update(DOTNET_CLI_TELEMETRY_OPTOUT="1", DOTNET_NOLOGO="1",
               DOTNET_CLI_HOME=str(OUTPUT / "dotnet-home"), NUGET_PACKAGES=str(OUTPUT / "nuget"))
    framework = "net461" if version == "2.0.0" else "netstandard2.0"
    subprocess.run([dotnet, "build", str(project), "-c", "Release", "-f", framework,
                    "--source", "https://api.nuget.org/v3/index.json",
                    "-p:TargetFrameworks=" + framework, "-p:RestoreIgnoreFailedSources=false",
                    "-p:InformationalVersion=" + version + "+blueberry.editor.v1.2"],
                   cwd=ROOT, env=env, check=True)
    built = source / "PSReadLine" / "bin" / "Release" / framework
    module = OUTPUT / "modules" / version / "PSReadLine" / version
    module.mkdir(parents=True, exist_ok=True)
    # Match upstream's module layout, not its SDK build output directory.
    # .NET Framework builds also emit dozens of reference facades which are
    # not runtime assets and must not become session startup work.
    allowed = {"Microsoft.PowerShell.PSReadLine2.dll", "System.Runtime.InteropServices.RuntimeInformation.dll"} if version == "2.0.0" else {"Microsoft.PowerShell.PSReadLine.dll", "Microsoft.PowerShell.Pager.dll"}
    for file in module.glob("*.dll"):
        if file.name not in allowed:
            file.unlink()
    for file in built.iterdir():
        if file.name in allowed or file.suffix in (".psd1", ".psm1", ".ps1xml"):
            shutil.copyfile(file, module / file.name)
    stale_polyfill = module / "Microsoft.PowerShell.PSReadLine.Polyfiller.dll"
    if stale_polyfill.is_file():
        stale_polyfill.unlink()
    manifest = module / "PSReadLine.psd1"
    text = manifest.read_text(encoding="utf-8-sig").replace("ModuleVersion = '2.0'", "ModuleVersion = '2.0.0'")
    manifest.write_text(text, encoding="utf-8-sig")
    if version == "2.4.5":
        for framework, dest in [("netstandard2.0", "netstd"), ("net6.0", "net6plus")]:
            subprocess.run([dotnet, "build", str(source / "Polyfill" / "Polyfill.csproj"),
                            "-c", "Release", "-f", framework,
                            "--source", "https://api.nuget.org/v3/index.json"], cwd=ROOT, env=env, check=True)
            target = module / dest
            target.mkdir(exist_ok=True)
            shutil.copyfile(source / "Polyfill" / "bin" / "Release" / framework /
                            "Microsoft.PowerShell.PSReadLine.Polyfiller.dll",
                            target / "Microsoft.PowerShell.PSReadLine.Polyfiller.dll")
        pager = OUTPUT / "nuget" / "microsoft.powershell.pager" / "1.0.0" / "lib" / "netstandard2.0" / "Microsoft.PowerShell.Pager.dll"
        shutil.copyfile(pager, module / pager.name)
    licenses = list(source.glob("LICENSE*")) + list(source.glob("PSReadLine/License.txt"))
    if not licenses:
        raise ValueError("Upstream license missing")
    shutil.copyfile(licenses[0], module / "License.txt")
    if version == "2.4.5":
        shutil.copyfile(VENDOR / "PAGER-LICENSE.txt", module / "Pager-LICENSE.txt")
    else:
        for name in ("RUNTIME-LICENSE.txt", "RUNTIME-NOTICES.txt"):
            shutil.copyfile(VENDOR / name, module / name)
    return {"version": version, "commit": spec["commit"], "root": str(module),
            "files": [{"path": str(p.relative_to(module)).replace("\\", "/"),
                       "sha256": hashlib.sha256(p.read_bytes()).hexdigest()}
                      for p in sorted(module.rglob("*")) if p.is_file()]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dotnet", default="dotnet")
    args = parser.parse_args()
    lock = json.loads((VENDOR / "upstream.json").read_text())
    modules = [prepare(version, spec, args.dotnet) for version, spec in lock["versions"].items()]
    (OUTPUT / "manifest.json").write_text(json.dumps({"patch": lock["patch"], "source_sha256": source_digest(), "modules": modules}, indent=2), encoding="utf-8")
    print("Private editor modules ready: " + str(OUTPUT / "manifest.json"))


if __name__ == "__main__":
    main()
