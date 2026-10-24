[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$PreviousInstaller,
    [Parameter(Mandatory=$true)][string]$PreviousPackage,
    [Parameter(Mandatory=$true)][string]$PackagePath
)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$previous=(Resolve-Path -LiteralPath $PreviousPackage).Path
$candidate=(Resolve-Path -LiteralPath $PackagePath).Path
$installer=(Resolve-Path -LiteralPath $PreviousInstaller).Path
$evidence=Join-Path (Split-Path -Parent $PSScriptRoot) ('target/public-upgrade-'+[Guid]::NewGuid().ToString('N'))
$root=Join-Path $evidence '中文 install'
$configRoot=Join-Path $evidence 'config'
$manager=Join-Path $PSScriptRoot 'manage-install.ps1'
$oldLocal=$env:LOCALAPPDATA
$oldRoaming=$env:APPDATA
$oldPath=[Environment]::GetEnvironmentVariable('PATH','User')
[IO.Directory]::CreateDirectory($evidence) | Out-Null
$env:LOCALAPPDATA=Join-Path $evidence 'local'
$env:APPDATA=Join-Path $evidence 'roaming'
$receipt=[ordered]@{schema=1;previous_package_sha256=(Get-FileHash -LiteralPath $previous).Hash;previous_installer_sha256=(Get-FileHash -LiteralPath $installer).Hash;candidate_package_sha256=(Get-FileHash -LiteralPath $candidate).Hash;install_root=$root;passed=$false}
try {
    # Use the released bootstrap and its own packaged manager, not a recreated
    # old manifest produced by today's installer.
    & $installer -PackagePath $previous -InstallRoot $root -ConfigRoot $configRoot -NoPrompt -NoPath
    if (-not $?) { throw 'Released bootstrap failed' }
    $oldState=Get-Content -LiteralPath (Join-Path $root 'install.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($oldState.current.version -ne '0.5.2') { throw 'Expected real public 0.5.2 installation' }
    Copy-Item -LiteralPath (Join-Path $root 'install.json') -Destination (Join-Path $evidence 'original-install.json')
    $oldHash=(Get-FileHash -LiteralPath (Join-Path $root 'blueberry.exe')).Hash
    $config=Join-Path $configRoot 'config.toml'
    [IO.File]::AppendAllText($config,"`n# 中文 user setting retained`n",[Text.UTF8Encoding]::new($false))
    $configHash=(Get-FileHash -LiteralPath $config).Hash
    $registrationHash=(Get-FileHash -LiteralPath (Join-Path $root 'registration.json')).Hash
    foreach ($operation in @('Upgrade','Rollback','Upgrade')) {
        $arguments=@{Action=$operation;InstallRoot=$root}
        if ($operation -eq 'Upgrade') { $arguments.PackagePath=$candidate }
        & $manager @arguments
        if (-not $?) { throw "Public upgrade operation failed: $operation" }
        $state=Get-Content -LiteralPath (Join-Path $root 'install.json') -Raw -Encoding UTF8 | ConvertFrom-Json
        foreach ($property in $state.managed_hashes.PSObject.Properties) {
            if ((Get-FileHash -LiteralPath (Join-Path $root $property.Name)).Hash -ine $property.Value) { throw "Managed digest mismatch: $($property.Name)" }
        }
        if ($operation -eq 'Rollback' -and (Get-FileHash -LiteralPath (Join-Path $root 'blueberry.exe')).Hash -ine $oldHash) { throw 'Rollback did not restore public executable' }
        if ((Get-FileHash -LiteralPath $config).Hash -ine $configHash) { throw 'Maintenance changed user configuration' }
        if ((Get-FileHash -LiteralPath (Join-Path $root 'registration.json')).Hash -ine $registrationHash) { throw 'Maintenance changed entry ownership' }
        if ([Environment]::GetEnvironmentVariable('PATH','User') -cne $oldPath) { throw 'Isolated validation changed user PATH' }
    }
    $receipt['passed']=$true
    Write-Host "Public 0.5.2 upgrade/rollback/re-upgrade passed: $evidence"
} catch {
    $receipt['error']=$_.Exception.Message
    throw
} finally {
    $receipt | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $evidence 'receipt.json') -Encoding UTF8
    $env:LOCALAPPDATA=$oldLocal
    $env:APPDATA=$oldRoaming
}
