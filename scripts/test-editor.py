"""Run unchanged upstream tests against the exact prepared private editor DLLs."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import uuid
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "target/private-editor"


def run(version, dotnet, require_coverage, original=None, layout="en-US"):
    source = OUTPUT / "src" / version / "test"
    test = OUTPUT / "upstream-tests" / (version + ("-original" if original else ""))
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
                group.remove(item)
    refs = ET.SubElement(root, "ItemGroup")
    module = Path(original).resolve() if original else OUTPUT / "modules" / version / "PSReadLine" / version
    for dll in list(module.glob("*.dll")) + list((module / "netstd").glob("*.dll")):
        ref = ET.SubElement(refs, "Reference", Include=dll.stem)
        ET.SubElement(ref, "HintPath").text = str(dll)
    shell = Path(os.environ["SystemRoot"]) / "System32/WindowsPowerShell/v1.0/powershell.exe"
    runtime = Path(subprocess.check_output([str(shell), "-NoProfile", "-NonInteractive", "-Command",
                   "[System.Management.Automation.PSObject].Assembly.Location"], text=True).strip())
    ref = ET.SubElement(refs, "Reference", Include="System.Management.Automation")
    ET.SubElement(ref, "HintPath").text = str(runtime)
    ET.SubElement(ref, "Private").text = "true"
    if version == "2.0.0":
        ET.SubElement(refs, "PackageReference", Include="Microsoft.NET.Test.Sdk", Version="16.11.0")
    # The net461 test SDK does not restore ObjectModel itself. Declare the
    # collector's compile dependency explicitly, independently of cache history
    # or whether the other upstream version has previously run on this machine.
    ET.SubElement(refs, "PackageReference", Include="Microsoft.TestPlatform.ObjectModel", Version="17.10.0")
    tree.write(project, encoding="utf-8", xml_declaration=True)
    env = os.environ.copy()
    env.update(NUGET_PACKAGES=str(OUTPUT / "nuget"), DOTNET_CLI_HOME=str(OUTPUT / "dotnet-home"),
               DOTNET_CLI_TELEMETRY_OPTOUT="1", PSREADLINE_TESTRUN="1",
               BLUEBERRY_UPSTREAM_LAYOUT="0000040c" if layout == "fr-FR" else "00000409")
    subprocess.run([dotnet, "restore", str(project), "--source", "https://api.nuget.org/v3/index.json"],
                   env=env, check=True)
    results_directory = test / "TestResults" / uuid.uuid4().hex
    test_filter = ("FullyQualifiedName~Test.fr_FR_Windows" if layout == "fr-FR" else
                   "FullyQualifiedName~Test.en_US_Windows|FullyQualifiedName~Test.ScreenReader|FullyQualifiedName~Test.KeyInfo")
    command = [dotnet, "test", str(project), "--no-restore", "-c", "Release",
                    "--filter", test_filter,
                    "--logger", "trx;LogFileName=upstream.trx", "--results-directory", str(results_directory)]
    if os.name == "nt":
        helper = OUTPUT / "editor-test-desktop.exe"
        compiler = Path(os.environ["SystemRoot"]) / "Microsoft.NET/Framework64/v4.0.30319/csc.exe"
        subprocess.run([str(compiler), "/nologo", "/target:exe", "/r:System.Windows.Forms.dll",
                        "/r:System.Drawing.dll", f"/out:{helper}",
                        str(ROOT / "scripts/editor-test-desktop.cs")], check=True)
        model = OUTPUT / "nuget/microsoft.testplatform.objectmodel/17.10.0/lib/net462/Microsoft.VisualStudio.TestPlatform.ObjectModel.dll"
        if not model.is_file():
            raise RuntimeError("Pinned VSTest collector compile dependency was not restored")
        collector = test / "EditorTestLayout.dll"
        subprocess.run([str(compiler), "/nologo", "/target:library", f"/r:{model}",
                        f"/out:{collector}", str(ROOT / "scripts/editor-test-layout.cs")], check=True)
        settings = ET.Element("RunSettings")
        ET.SubElement(ET.SubElement(settings, "RunConfiguration"), "TestSessionTimeout").text = "120000"
        collectors = ET.SubElement(ET.SubElement(settings, "InProcDataCollectionRunSettings"), "InProcDataCollectors")
        ET.SubElement(collectors, "InProcDataCollector", friendlyName="EditorTestLayout",
                      uri="InProcDataCollector://Blueberry/EditorTestLayout/1.0", codebase=str(collector),
                      assemblyQualifiedName="EditorTestLayout, EditorTestLayout, Version=0.0.0.0, Culture=neutral, PublicKeyToken=null")
        settings_path = test / "layout.runsettings"
        ET.ElementTree(settings).write(settings_path, encoding="utf-8", xml_declaration=True)
        command.extend(["--settings", str(settings_path)])
        command.insert(0, str(helper))
    process = subprocess.run(command, env=env)
    if not (results_directory / "upstream.trx").is_file():
        process.check_returncode()
        raise RuntimeError("Upstream runner produced no results")
    results = ET.parse(results_directory / "upstream.trx")
    ns = {"t": "http://microsoft.com/schemas/VisualStudio/TeamTest/2010"}
    counters = results.find("t:ResultSummary/t:Counters", ns).attrib
    summary = {key: int(value) for key, value in counters.items()}
    summary["runtime"] = {"path": str(runtime), "sha256": hashlib.sha256(runtime.read_bytes()).hexdigest()}
    summary["module"] = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in module.glob("*.dll")}
    for name, expected_hash in summary["module"].items():
        if "PSReadLine" in name:
            loaded_copy = test / "bin/Release" / framework / name
            if not loaded_copy.is_file() or hashlib.sha256(loaded_copy.read_bytes()).hexdigest() != expected_hash:
                raise RuntimeError("Upstream test output does not contain the exact candidate editor DLL")
    summary["results"] = str(results_directory / "upstream.trx")
    summary["layout"] = layout
    summary["skipped"] = [item.attrib["testName"] for item in results.findall("t:Results/t:UnitTestResult", ns)
                          if item.get("outcome") == "NotExecuted"]
    summary["coverage_eligible"] = summary["executed"] >= 100 and summary["failed"] == 0
    for path in [test / "summary.json", results_directory / "summary.json"]:
        path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
    process.check_returncode()
    if require_coverage and not summary["coverage_eligible"]:
        raise RuntimeError("Upstream tests did not exercise editing; inspect keyboard layout and retained TRX")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dotnet", default="dotnet")
    parser.add_argument("--version", choices=["2.0.0", "2.4.5"])
    parser.add_argument("--require-coverage", action="store_true")
    parser.add_argument("--layout", choices=["en-US", "fr-FR", "all"], default="all")
    parser.add_argument("--original-module", type=Path, help="Run the identical fixture against this original DLL directory")
    args = parser.parse_args()
    for version in [args.version] if args.version else ["2.0.0", "2.4.5"]:
        if args.original_module and not args.version:
            parser.error("--original-module requires --version")
        for layout in ["en-US", "fr-FR"] if args.layout == "all" else [args.layout]:
            run(version, args.dotnet, args.require_coverage, args.original_module, layout)
