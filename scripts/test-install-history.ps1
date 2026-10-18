[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$PackagePath,[Parameter(Mandatory=$true)][string]$NextPackagePath)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$package=(Resolve-Path -LiteralPath $PackagePath).Path
$nextPackage=(Resolve-Path -LiteralPath $NextPackagePath).Path
$manager=Join-Path $PSScriptRoot 'manage-install.ps1'
$evidence=Join-Path (Split-Path -Parent $PSScriptRoot) ('target/install-history-'+[Guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($evidence) | Out-Null
$results=@()
foreach ($historyCount in @(0,1,5)) {
    $root=Join-Path $evidence "history-$historyCount"
    $timer=[Diagnostics.Stopwatch]::StartNew()
    & $manager -Action Install -PackagePath $package -InstallRoot $root
    for ($index=0; $index -lt $historyCount; $index++) {
        $seedPackage=if ($index % 2 -eq 0) { $nextPackage } else { $package }
        & $manager -Action Upgrade -PackagePath $seedPackage -InstallRoot $root
    }
    $before=Get-Content -LiteralPath (Join-Path $root 'install.json') -Raw | ConvertFrom-Json
    if (@($before.previous).Count -ne $historyCount) { throw 'History fixture count mismatch' }
    $sentinel=Join-Path $root 'components/terminal/settings/user.json'
    [IO.Directory]::CreateDirectory((Split-Path -Parent $sentinel)) | Out-Null
    [IO.File]::WriteAllText($sentinel,'{"user":"中文 untouched"}',[Text.UTF8Encoding]::new($false))
    $sentinelHash=(Get-FileHash -LiteralPath $sentinel).Hash
    $historical=@{}
    foreach ($snapshot in @($before.previous)) {
        $manifest=Join-Path $root $snapshot.snapshot_manifest
        $historical[$manifest]=(Get-FileHash -LiteralPath $manifest).Hash
        foreach ($record in $snapshot.files) {
            $path=Join-Path $root $record.snapshot_path
            $historical[$path]=(Get-FileHash -LiteralPath $path).Hash
        }
    }
    foreach ($operation in @('Upgrade','Rollback','Upgrade')) {
        $arguments=@{Action=$operation;InstallRoot=$root}
        if ($operation -eq 'Upgrade') { $arguments.PackagePath=if ($historyCount % 2 -eq 0) { $nextPackage } else { $package } }
        & $manager @arguments
        $state=Get-Content -LiteralPath (Join-Path $root 'install.json') -Raw | ConvertFrom-Json
        foreach ($property in $state.managed_hashes.PSObject.Properties) {
            if ((Get-FileHash -LiteralPath (Join-Path $root $property.Name)).Hash -ine $property.Value) { throw "Managed file changed: $($property.Name)" }
        }
        foreach ($path in $historical.Keys) {
            if ((Get-FileHash -LiteralPath $path).Hash -ine $historical[$path]) { throw 'Historical snapshot modified' }
        }
        if ((Get-FileHash -LiteralPath $sentinel).Hash -ine $sentinelHash) { throw 'Core maintenance modified component settings' }
    }
    $results+=@{initial_history=$historyCount;elapsed_ms=$timer.Elapsed.TotalMilliseconds;status='passed';install_root=$root;package_sha256=(Get-FileHash -LiteralPath $package).Hash}
    $results | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $evidence 'results.json') -Encoding UTF8
}
Write-Host "Installation history matrix passed; retained evidence: $evidence"
