"""Run unchanged upstream tests against the exact prepared private editor DLLs."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "target/private-editor"


def run(version, dotnet, require_coverage):
    source = OUTPUT / "src" / version / "test"
    test = OUTPUT / "upstream-tests" / version
    shutil.copytree(source, test, dirs_exist_ok=True, ignore=shutil.ignore_patterns("bin", "obj", "TestResults"))
    project = test / "PSReadLine.Tests.csproj"
    tree = ET.parse(project)
    root = tree.getroot()
    framework = "net461" if version == "2.0.0" else "net472"
    root.find("PropertyGroup/TargetFrameworks").text = framework
    for group in root.findall("ItemGroup"):
        for item in list(group):
            if item.tag == "ProjectReference":
                group.remove(item)
            elif item.tag == "PackageReference" and item.get("Include") == "PowerShellStandard.Library":
                item.set("version", "5.1.0")
    refs = ET.SubElement(root, "ItemGroup")
    module = OUTPUT / "modules" / version / "PSReadLine" / version
    for dll in list(module.glob("*.dll")) + list((module / "netstd").glob("*.dll")):
        ref = ET.SubElement(refs, "Reference", Include=dll.stem)
        ET.SubElement(ref, "HintPath").text = str(dll)
    if version == "2.0.0":
        ET.SubElement(refs, "PackageReference", Include="Microsoft.NET.Test.Sdk", Version="16.11.0")
    tree.write(project, encoding="utf-8", xml_declaration=True)
    env = os.environ.copy()
    env.update(NUGET_PACKAGES=str(OUTPUT / "nuget"), DOTNET_CLI_HOME=str(OUTPUT / "dotnet-home"),
               DOTNET_CLI_TELEMETRY_OPTOUT="1", PSREADLINE_TESTRUN="1")
    subprocess.run([dotnet, "restore", str(project), "--source", "https://api.nuget.org/v3/index.json"],
                   env=env, check=True)
    subprocess.run([dotnet, "test", str(project), "--no-restore", "-c", "Release",
                    "--filter", "FullyQualifiedName~Test.en_US_Windows|FullyQualifiedName~Test.ScreenReader",
                    "--logger", "trx;LogFileName=upstream.trx"], env=env, check=True)
    results = ET.parse(test / "TestResults/upstream.trx")
    ns = {"t": "http://microsoft.com/schemas/VisualStudio/TeamTest/2010"}
    counters = results.find("t:ResultSummary/t:Counters", ns).attrib
    summary = {key: int(value) for key, value in counters.items()}
    summary["coverage_eligible"] = summary["executed"] >= 100 and summary["failed"] == 0
    (test / "summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    if require_coverage and not summary["coverage_eligible"]:
        raise RuntimeError("Upstream tests did not exercise editing; inspect keyboard layout and retained TRX")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dotnet", default="dotnet")
    parser.add_argument("--version", choices=["2.0.0", "2.4.5"])
    parser.add_argument("--require-coverage", action="store_true")
    args = parser.parse_args()
    for version in [args.version] if args.version else ["2.0.0", "2.4.5"]:
        run(version, args.dotnet, args.require_coverage)
