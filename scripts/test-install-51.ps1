[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$PackagePath)
$ErrorActionPreference = 'Stop'
$root = Join-Path ([IO.Path]::GetTempPath()) ('blueberry-install-51-' + [Guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($root) | Out-Null
$legacy = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$installer = Join-Path $PSScriptRoot '../install.ps1'
$manager = Join-Path $PSScriptRoot 'manage-install.ps1'
$installRoot = Join-Path $root '中文 install'
$configRoot = Join-Path $root 'config'
$previousLocal = $env:LOCALAPPDATA
$previousRoaming = $env:APPDATA
$env:LOCALAPPDATA = Join-Path $root 'local'
$env:APPDATA = Join-Path $root 'roaming'
try {
    $payload = Join-Path $root 'payload'
    Expand-Archive -LiteralPath $PackagePath -DestinationPath $payload
    $version = (Get-Content -LiteralPath (Join-Path $payload 'release.json') -Raw | ConvertFrom-Json).version
    $notes = Join-Path $root 'previous-notes.md'
    [IO.File]::WriteAllText($notes, 'Previous package fixture')
    $firstOutput = Join-Path $root 'first'
    & (Join-Path $PSScriptRoot 'release.ps1') -ExePath (Join-Path $payload 'blueberry.exe') -Version $version -OutputDirectory $firstOutput -ReleaseNotesPath $notes -LicenseNoticesPath (Join-Path $payload 'THIRD-PARTY-NOTICES.txt') | Out-Null
    $firstPackage = (Get-ChildItem -LiteralPath $firstOutput -Filter '*.zip').FullName
    & $legacy -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $installer -PackagePath $firstPackage -InstallRoot $installRoot -ConfigRoot $configRoot -NoPrompt -NoPath
    if ($LASTEXITCODE -ne 0) { throw '5.1 bootstrap failed' }
    $config = Join-Path $configRoot 'config.toml'
    [IO.File]::AppendAllText($config, "`n# preserved across upgrades`n")
    $before = (Get-FileHash -LiteralPath $config).Hash
    & $manager -Action Upgrade -PackagePath $PackagePath -InstallRoot $installRoot
    & $legacy -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $manager -Action Rollback -InstallRoot $installRoot
    if ($LASTEXITCODE -ne 0) { throw 'Cross-runtime rollback failed' }
    & $legacy -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $installer -PackagePath $PackagePath -InstallRoot $installRoot -ConfigRoot $configRoot -NoPrompt -NoPath
    if ($LASTEXITCODE -ne 0) { throw 'Repeated bootstrap failed' }
    if ((Get-FileHash -LiteralPath $config).Hash -ne $before) { throw 'Upgrade changed user configuration' }
    & $legacy -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $manager -Action Uninstall -InstallRoot $installRoot
    if ($LASTEXITCODE -ne 0) { throw '5.1 uninstall failed' }
    if (-not [IO.File]::Exists($config)) { throw 'Uninstall removed user configuration' }
    $bad = Join-Path $root 'invalid.zip'
    [IO.File]::WriteAllText($bad, 'interrupted download')
    [IO.File]::WriteAllText(($bad + '.sha256'), (('0' * 64) + '  invalid.zip'))
    & $legacy -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $installer -PackagePath $bad -InstallRoot $installRoot -ConfigRoot $configRoot -NoPrompt -NoPath 2>$null
    if ($LASTEXITCODE -eq 0 -or [IO.File]::Exists((Join-Path $installRoot 'blueberry.exe'))) { throw 'Corrupt package was accepted' }
    Write-Output 'PowerShell 5.1 installation and cross-runtime lifecycle passed.'
} finally {
    $env:LOCALAPPDATA = $previousLocal
    $env:APPDATA = $previousRoaming
    $resolved = [IO.Path]::GetFullPath($root)
    $allowed = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\','/') + [IO.Path]::DirectorySeparatorChar
    if ($resolved.StartsWith($allowed, [StringComparison]::OrdinalIgnoreCase)) { Remove-Item -LiteralPath $resolved -Recurse -Force }
}
