[CmdletBinding()]
param([ValidateSet('explorer','windows-terminal','vscode')][string]$InheritedHost='vscode')
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Join-Path ([IO.Path]::GetTempPath()) ('blueberry-launcher-' + [Guid]::NewGuid().ToString('N'))
$layout = Join-Path $root 'launch & unicode 中'
$working = Join-Path $root 'working & 中文'
$capture = Join-Path $root 'capture.txt'
$oldCapture = $env:BLUEBERRY_TEST_LAUNCH_CAPTURE
$oldActive = $env:BLUEBERRY_ACTIVE
$oldOverlay = $env:BLUEBERRY_NATIVE_OVERLAY
$hostEnvironment = @{}
foreach ($name in @('TERM_PROGRAM','TERM_PROGRAM_VERSION','WT_SESSION','WT_PROFILE_ID')) {
    $hostEnvironment[$name] = [Environment]::GetEnvironmentVariable($name,'Process')
}
$source = @'
using System;
using System.Collections.Generic;
using System.IO;
class Capture {
    static void Main(string[] args) {
        var lines = new List<string>();
        lines.Add(Environment.GetEnvironmentVariable("BLUEBERRY_NATIVE_OVERLAY") ?? "<null>");
        lines.Add(Environment.GetEnvironmentVariable("BLUEBERRY_ACTIVE") ?? "<null>");
        foreach (string name in new [] { "TERM_PROGRAM", "TERM_PROGRAM_VERSION", "WT_SESSION", "WT_PROFILE_ID" })
            lines.Add(Environment.GetEnvironmentVariable(name) ?? "<null>");
        lines.AddRange(args);
        lines.Add("PID=" + System.Diagnostics.Process.GetCurrentProcess().Id);
        string path = Environment.GetEnvironmentVariable("BLUEBERRY_TEST_LAUNCH_CAPTURE");
        File.WriteAllLines(path + ".tmp", lines);
        File.Move(path + ".tmp", path);
        DateTime deadline = DateTime.UtcNow.AddSeconds(30);
        while (!File.Exists(path + ".release") && DateTime.UtcNow < deadline)
            System.Threading.Thread.Sleep(20);
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
    foreach ($name in $hostEnvironment.Keys) { [Environment]::SetEnvironmentVariable($name,$null,'Process') }
    if ($InheritedHost -eq 'vscode') { $env:TERM_PROGRAM='vscode'; $env:TERM_PROGRAM_VERSION='fixture' }
    if ($InheritedHost -eq 'windows-terminal') { $env:WT_SESSION='stale-session'; $env:WT_PROFILE_ID='stale-profile' }
    $callerIdentity = @{}
    foreach ($name in $hostEnvironment.Keys) { $callerIdentity[$name]=[Environment]::GetEnvironmentVariable($name,'Process') }
    $info = New-Object Diagnostics.ProcessStartInfo
    $info.FileName = Join-Path $env:SystemRoot 'System32/cmd.exe'
    $info.Arguments = '/d /s /c ""' + (Join-Path $layout 'blueberry-terminal.cmd') + '""'
    $info.WorkingDirectory = $working
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $process = [Diagnostics.Process]::Start($info)
    try {
        if (-not $process.WaitForExit(5000)) {
            $process.Kill();
            throw 'Launcher waited for its GUI child instead of returning asynchronously'
        }
        if ($process.ExitCode -ne 0) { throw 'Launcher did not return successfully' }
    }
    finally { $process.Dispose() }
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while (-not (Test-Path -LiteralPath $capture) -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 20 }
    $actual = [IO.File]::ReadAllLines($capture)
    if ($actual[-1] -notmatch '^PID=\d+$') { throw 'Missing owned launcher fixture PID' }
    $expected = @('4', '<null>', '<null>', '<null>', '<null>', '<null>', '--window', 'new', 'new-tab', '--inheritEnvironment', '--startingDirectory',
        $working, (Join-Path $layout 'blueberry.exe'), 'run', '--host-mode', 'direct', '--transport', 'pipe', $actual[-1])
    if (($actual | ConvertTo-Json -Compress) -cne ($expected | ConvertTo-Json -Compress)) {
        throw ('Launcher environment/arguments differ: ' + ($actual | ConvertTo-Json -Compress))
    }
    if ($env:BLUEBERRY_ACTIVE -ne '1' -or $env:BLUEBERRY_NATIVE_OVERLAY -ne 'inherited wrong interface') {
        throw 'Launcher changed its caller environment'
    }
    foreach ($name in $hostEnvironment.Keys) {
        if ([Environment]::GetEnvironmentVariable($name,'Process') -cne $callerIdentity[$name]) { throw "Launcher changed caller $name" }
    }
    Write-Host "Portable launcher quoting, simulated $InheritedHost environment isolation and asynchronous return passed"
}
finally {
    if (Test-Path -LiteralPath $root) {
        $fixture = $null
        try {
            if (Test-Path -LiteralPath $capture) {
                $last = [IO.File]::ReadAllLines($capture)[-1]
                if ($last -match '^PID=(\d+)$') {
                    try {
                        $candidate = [Diagnostics.Process]::GetProcessById([int]$Matches[1])
                        if ($candidate.MainModule.FileName -ieq (Join-Path $layout 'terminal/WindowsTerminal.exe')) { $fixture = $candidate }
                        else { $candidate.Dispose() }
                    }
                    catch [ArgumentException] { } # Fixture already exited.
                    catch [InvalidOperationException] { }
                }
            }
        }
        finally { [IO.File]::WriteAllText(($capture + '.release'), 'release owned fixture') }
        if ($null -ne $fixture) {
            try { if (-not $fixture.WaitForExit(5000)) { throw 'Owned fixture did not stop after release' } }
            finally { $fixture.Dispose() }
        }
    }
    $env:BLUEBERRY_TEST_LAUNCH_CAPTURE = $oldCapture
    $env:BLUEBERRY_ACTIVE = $oldActive
    $env:BLUEBERRY_NATIVE_OVERLAY = $oldOverlay
    foreach ($name in $hostEnvironment.Keys) { [Environment]::SetEnvironmentVariable($name,$hostEnvironment[$name],'Process') }
    # This newly created, fixed root is the only cleanup target.
    if ([IO.Path]::GetFullPath($root).StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath()), [StringComparison]::OrdinalIgnoreCase) -and
        [IO.Path]::GetFileName($root).StartsWith('blueberry-launcher-') -and (Test-Path -LiteralPath $root)) {
        Remove-Item -LiteralPath $root -Recurse -Force
    }
}
