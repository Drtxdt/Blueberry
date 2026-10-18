[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$BundledPackage,[Parameter(Mandatory=$true)][string]$CorePackage)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$root=Join-Path (Split-Path -Parent $PSScriptRoot) ('target/terminal-migration-'+[Guid]::NewGuid().ToString('N'))
$manager=Join-Path $PSScriptRoot 'manage-install.ps1'
& $manager -Action Install -PackagePath $BundledPackage -InstallRoot $root
$settings=Join-Path $root 'terminal/settings/settings.json'
[IO.Directory]::CreateDirectory((Split-Path -Parent $settings)) | Out-Null
[IO.File]::WriteAllText($settings,'{"font":"中文 legacy font"}',[Text.UTF8Encoding]::new($false))
$settingsHash=(Get-FileHash -LiteralPath $settings).Hash
$terminalHash=(Get-FileHash -LiteralPath (Join-Path $root 'terminal/WindowsTerminal.exe')).Hash
& $manager -Action Upgrade -PackagePath $CorePackage -InstallRoot $root
$statePath=Join-Path $root 'components/terminal/component-state.json'
$state=Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
$versionRoot=Join-Path $root "components/terminal/versions/$($state.current)"
if (-not $state.installed -or (Get-FileHash -LiteralPath (Join-Path $versionRoot 'WindowsTerminal.exe')).Hash -ne $terminalHash) { throw 'Migration changed Terminal identity' }
if ((Get-FileHash -LiteralPath (Join-Path $versionRoot 'settings/settings.json')).Hash -ne $settingsHash) { throw 'Migration lost user settings' }
$stateHash=(Get-FileHash -LiteralPath $statePath).Hash
foreach ($operation in @('Rollback','Upgrade')) {
    $arguments=@{Action=$operation;InstallRoot=$root}
    if ($operation -eq 'Upgrade') { $arguments.PackagePath=$CorePackage }
    & $manager @arguments
    if ((Get-FileHash -LiteralPath $statePath).Hash -ne $stateHash) { throw 'Core rollback or re-upgrade changed independent component state' }
    if ((Get-FileHash -LiteralPath (Join-Path $versionRoot 'settings/settings.json')).Hash -ne $settingsHash) { throw 'Core maintenance changed migrated settings' }
}
& (Join-Path $PSScriptRoot 'manage-terminal.ps1') -Operation Status -InstallRoot $root
Write-Host "Bundled Terminal migration passed; retained evidence: $root"
