[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$PackageOne,[Parameter(Mandatory=$true)][string]$PackageTwo)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$root=Join-Path ([IO.Path]::GetTempPath()) ('blueberry-component-test-'+[Guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($root) | Out-Null
$manager=Join-Path $PSScriptRoot 'manage-terminal.ps1'
function Run-Component([string]$action,[string]$package) {
    $arguments=@{Operation=$action;InstallRoot=$root}
    if ($package) { $arguments.PackagePath=$package }
    & $manager @arguments | Out-Null
}
function State { Get-Content -LiteralPath (Join-Path $root 'components/terminal/component-state.json') -Raw | ConvertFrom-Json }
function Check([bool]$condition,[string]$message) { if (-not $condition) { throw $message } }
[IO.File]::WriteAllText((Join-Path $root 'core-user-file.txt'),'untouched')
Run-Component Install $PackageOne
$first=(State).current
$settings=Join-Path $root "components/terminal/versions/$first/settings/settings.json"
[IO.Directory]::CreateDirectory((Split-Path -Parent $settings)) | Out-Null
[IO.File]::WriteAllText($settings,'{"font":"中文 Test font"}',[Text.UTF8Encoding]::new($false))
Run-Component Upgrade $PackageTwo
$second=(State).current
Check ($second -ne $first) 'Upgrade did not select independent version'
$secondSettings=Join-Path $root "components/terminal/versions/$second/settings/settings.json"
Check ([IO.File]::ReadAllText($secondSettings) -ceq [IO.File]::ReadAllText($settings)) 'Settings lost during upgrade'
[IO.File]::WriteAllText($secondSettings,'{"font":"Latest preference"}')
Run-Component Rollback
Check ((State).current -eq $first) 'Rollback did not select previous version'
Check ([IO.File]::ReadAllText($settings) -ceq '{"font":"Latest preference"}') 'Rollback lost latest user settings'
Run-Component Upgrade $PackageTwo
Check ((State).current -eq $second) 'Re-upgrade failed'
# Invalid input must not change the active component or its state bytes.
$statePath=Join-Path $root 'components/terminal/component-state.json'
$stateBytes=[IO.File]::ReadAllBytes($statePath)
$stateHash=(Get-FileHash -LiteralPath $statePath -Algorithm SHA256).Hash
$oversized=Join-Path $root 'too-many-entries.zip'
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive=[IO.Compression.ZipFile]::Open($oversized,[IO.Compression.ZipArchiveMode]::Create)
try { foreach ($index in 0..4096) { [void]$archive.CreateEntry("entry-$index") } } finally { $archive.Dispose() }
$failed=$false
try { Run-Component Upgrade $oversized } catch { $failed=$_.Exception.Message -like '*too many entries*' }
Check $failed 'Oversized entry count was not rejected before extraction'
Check ((Get-FileHash -LiteralPath $statePath -Algorithm SHA256).Hash -ceq $stateHash) 'Rejected package changed active state'
[IO.File]::WriteAllText($statePath,'{"schema_version":1,"product":"Blueberry.Terminal","metadata_sha256":"damaged"}')
$failed=$false
try { Run-Component Status } catch { $failed=$true } finally { [IO.File]::WriteAllBytes($statePath,$stateBytes) }
Check $failed 'Damaged component state was accepted'
Check ((State).current -eq $second) 'State recovery selected another component'
# A blocked deletion leaves a retryable inactive state and retains settings.
$locked=Join-Path $root "components/terminal/versions/$second/WindowsTerminal.exe"
$handle=[IO.File]::Open($locked,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
$failed=$false
try { Run-Component Uninstall } catch { $failed=$true } finally { $handle.Dispose() }
Check $failed 'Locked runtime unexpectedly uninstalled'
Run-Component Uninstall
Check (-not (State).installed) 'Uninstall remained active'
Check ([IO.File]::Exists($settings) -and [IO.File]::Exists($secondSettings)) 'Uninstall deleted user settings'
Check ([IO.File]::ReadAllText((Join-Path $root 'core-user-file.txt')) -ceq 'untouched') 'Component modified core user files'
Write-Output "Terminal component lifecycle passed; retained evidence: $root"
