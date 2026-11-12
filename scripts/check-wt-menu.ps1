# Run manually inside an official Windows Terminal tab. The tester operates the UI.
[CmdletBinding()]
param([string]$Executable = (Join-Path $PSScriptRoot '../target/release/blueberry.exe'),
      [string]$Shell = 'pwsh.exe',
      [ValidateSet('direct','nested')][string]$HostMode='direct',
      [ValidateSet('2.0.0','2.4.5')][string]$Editor='2.4.5',
      [string]$OriginalEditorManifest)
$ErrorActionPreference = 'Stop'
if (-not $env:WT_SESSION) { throw 'Open a Windows Terminal tab before running this check.' }
$binary = (Resolve-Path -LiteralPath $Executable).Path
if ($OriginalEditorManifest) { $OriginalEditorManifest = (Resolve-Path -LiteralPath $OriginalEditorManifest).Path }
$shellPath = (Get-Command $Shell -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
$fixture = Join-Path ([IO.Path]::GetTempPath()) ('blueberry-wt-menu-' + [Guid]::NewGuid().ToString('N'))
$null = [IO.Directory]::CreateDirectory((Join-Path $fixture 'many'))
[IO.File]::WriteAllText((Join-Path $fixture 'blocked'), 'This regular file is deliberately not a directory.')
foreach ($i in 1..4096) { [IO.File]::Create((Join-Path $fixture "many/entry-$i")).Dispose() }
$config = Join-Path $fixture 'config.toml'
[IO.File]::WriteAllLines($config, @('[completion]', 'auto_trigger = false', '[keys]', "trigger = 'F6'"))
$priorHistory = $env:BLUEBERRY_NO_HISTORY
$priorModule = $env:BLUEBERRY_TEST_PSREADLINE_MODULE
Push-Location -LiteralPath $fixture
try {
    $env:BLUEBERRY_NO_HISTORY = '1'
    Write-Host "Manual fixture: $fixture"
    Write-Host 'Type Get-ChildItem .\many\absent then F6 for loading/empty; use .\blocked\value then F6 for error. Exit the child shell when finished.'
    $editorArgs = @()
    if ($HostMode -eq 'direct') { $editorArgs = @('--psreadline-version',$Editor) }
    else {
        if (-not $OriginalEditorManifest) { throw 'Nested checks require the selected original PSReadLine manifest.' }
        $module = Test-ModuleManifest -Path $OriginalEditorManifest -ErrorAction Stop
        if ($module.Version.ToString() -ne $Editor) { throw 'Original PSReadLine version differs from the selected editor' }
        $env:BLUEBERRY_TEST_PSREADLINE_MODULE = (Resolve-Path $OriginalEditorManifest).Path
    }
    Write-Host "$HostMode / $Editor / EXE SHA-256: $((Get-FileHash $binary).Hash)"
    & $binary run --host-mode $HostMode --shell $shellPath --no-profile @editorArgs --config $config --data-dir (Join-Path $fixture 'data') --trace (Join-Path $fixture 'trace.jsonl')
    if ($LASTEXITCODE) { throw "Child shell exited with $LASTEXITCODE" }
} finally {
    Pop-Location
    $env:BLUEBERRY_NO_HISTORY = $priorHistory
    $env:BLUEBERRY_TEST_PSREADLINE_MODULE = $priorModule
}
