[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Join-Path ([IO.Path]::GetTempPath()) ('blueberry-launcher-' + [Guid]::NewGuid().ToString('N'))
$layout = Join-Path $root 'launch & unicode 中'
$working = Join-Path $root 'working & 中文'
$capture = Join-Path $root 'capture.txt'
$oldCapture = $env:BLUEBERRY_TEST_LAUNCH_CAPTURE
$oldActive = $env:BLUEBERRY_ACTIVE
$oldOverlay = $env:BLUEBERRY_NATIVE_OVERLAY
$source = @'
using System;
using System.Collections.Generic;
using System.IO;
class Capture {
    static void Main(string[] args) {
        var lines = new List<string>();
        lines.Add(Environment.GetEnvironmentVariable("BLUEBERRY_NATIVE_OVERLAY") ?? "<null>");
        lines.Add(Environment.GetEnvironmentVariable("BLUEBERRY_ACTIVE") ?? "<null>");
        lines.AddRange(args);
        File.WriteAllLines(Environment.GetEnvironmentVariable("BLUEBERRY_TEST_LAUNCH_CAPTURE"), lines);
    }
}
'@
try {
    [IO.Directory]::CreateDirectory((Join-Path $layout 'terminal')) | Out-Null
    [IO.Directory]::CreateDirectory($working) | Out-Null
    $program = Join-Path $root 'capture.cs'
    [IO.File]::WriteAllText($program, $source)
    $compiler = Join-Path $env:SystemRoot 'Microsoft.NET/Framework64/v4.0.30319/csc.exe'
    & $compiler /nologo /target:winexe ("/out:" + (Join-Path $layout 'terminal/WindowsTerminal.exe')) $program
    if ($LASTEXITCODE -ne 0) { throw 'Launcher fixture compile failed' }
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot '../scripts/blueberry-terminal.cmd') -Destination $layout
    [IO.File]::WriteAllText((Join-Path $layout 'blueberry.exe'), 'fixture, never executed')
    $env:BLUEBERRY_TEST_LAUNCH_CAPTURE = $capture
    $env:BLUEBERRY_ACTIVE = '1'
    $env:BLUEBERRY_NATIVE_OVERLAY = 'inherited wrong interface'
    $info = New-Object Diagnostics.ProcessStartInfo
    $info.FileName = Join-Path $env:SystemRoot 'System32/cmd.exe'
    $info.Arguments = '/d /s /c ""' + (Join-Path $layout 'blueberry-terminal.cmd') + '""'
    $info.WorkingDirectory = $working
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $process = [Diagnostics.Process]::Start($info)
    try {
        if (-not $process.WaitForExit(10000) -or $process.ExitCode -ne 0) { throw 'Launcher did not return successfully' }
    }
    finally { $process.Dispose() }
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while (-not (Test-Path -LiteralPath $capture) -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 20 }
    $actual = [IO.File]::ReadAllLines($capture)
    $expected = @('4', '<null>', '--window', 'new', 'new-tab', '--inheritEnvironment', '--startingDirectory',
        $working, (Join-Path $layout 'blueberry.exe'), 'run', '--host-mode', 'direct', '--transport', 'pipe')
    if (($actual | ConvertTo-Json -Compress) -cne ($expected | ConvertTo-Json -Compress)) {
        throw ('Launcher environment/arguments differ: ' + ($actual | ConvertTo-Json -Compress))
    }
    if ($env:BLUEBERRY_ACTIVE -ne '1' -or $env:BLUEBERRY_NATIVE_OVERLAY -ne 'inherited wrong interface') {
        throw 'Launcher changed its caller environment'
    }
    Write-Host 'Portable launcher quoting, environment isolation and asynchronous return passed'
}
finally {
    $env:BLUEBERRY_TEST_LAUNCH_CAPTURE = $oldCapture
    $env:BLUEBERRY_ACTIVE = $oldActive
    $env:BLUEBERRY_NATIVE_OVERLAY = $oldOverlay
    # This newly created, fixed root is the only cleanup target.
    if ([IO.Path]::GetFullPath($root).StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath()), [StringComparison]::OrdinalIgnoreCase) -and
        [IO.Path]::GetFileName($root).StartsWith('blueberry-launcher-') -and (Test-Path -LiteralPath $root)) {
        Remove-Item -LiteralPath $root -Recurse -Force
    }
}
