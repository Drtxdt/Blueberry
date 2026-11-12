[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$PreviousInstaller,
    [Parameter(Mandatory=$true)][string]$PreviousPackage,
    [Parameter(Mandatory=$true)][string]$PackagePath,
    [Parameter(Mandatory=$true)][string]$PreviousComponent,
    [string]$OriginalEditorManifest = (Join-Path $PSHOME 'Modules/PSReadLine/PSReadLine.psd1')
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
$oldModules=$env:PSModulePath
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
    if ($oldState.current.version -ne '0.5.3') { throw 'Expected real public 0.5.3 installation' }
    Copy-Item -LiteralPath (Join-Path $root 'install.json') -Destination (Join-Path $evidence 'original-install.json')
    $oldHash=(Get-FileHash -LiteralPath (Join-Path $root 'blueberry.exe')).Hash
    $config=Join-Path $configRoot 'config.toml'
    [IO.File]::AppendAllText($config,"`n# 中文 user setting retained`n",[Text.UTF8Encoding]::new($false))
    $configHash=(Get-FileHash -LiteralPath $config).Hash
    $registrationHash=(Get-FileHash -LiteralPath (Join-Path $root 'registration.json')).Hash
    $fixtureProfile = Join-Path $evidence 'profile.ps1'
    $profileText = "# user settings retained`r`nSet-PSReadLineKeyHandler -Chord F8 -Function HistorySearchBackward`r`n"
    [IO.File]::WriteAllText($fixtureProfile,$profileText,[Text.UTF8Encoding]::new($true))
    $profileOriginal = (Get-FileHash $fixtureProfile).Hash
    $exe = Join-Path $root 'blueberry.exe'
    & $exe startup enable --profile $fixtureProfile
    if ($LASTEXITCODE) { throw 'Public startup hook installation failed' }
    $profileInstalled = (Get-FileHash $fixtureProfile).Hash
    & $exe terminal install --install-root $root --package $PreviousComponent
    if ($LASTEXITCODE) { throw 'Public optional component installation failed' }
    $componentState = Join-Path $root 'components/terminal/component-state.json'
    $componentHash = (Get-FileHash $componentState).Hash
    foreach ($operation in @('Upgrade','Rollback','Upgrade')) {
        $arguments=@{Action=$operation;InstallRoot=$root}
        if ($operation -eq 'Upgrade') { $arguments.PackagePath=$candidate }
        & $manager @arguments
        if (-not $?) { throw "Public upgrade operation failed: $operation" }
        $state=Get-Content -LiteralPath (Join-Path $root 'install.json') -Raw -Encoding UTF8 | ConvertFrom-Json
        foreach ($property in $state.managed_hashes.PSObject.Properties) {
            if ((Get-FileHash -LiteralPath (Join-Path $root $property.Name)).Hash -ine $property.Value) { throw "Managed digest mismatch: $($property.Name)" }
        }
        if ((Get-FileHash $fixtureProfile).Hash -ne $profileInstalled -or (Get-FileHash $componentState).Hash -ne $componentHash) { throw 'Upgrade changed startup bindings or optional component state' }
        if ($operation -eq 'Rollback' -and (Get-FileHash -LiteralPath (Join-Path $root 'blueberry.exe')).Hash -ine $oldHash) { throw 'Rollback did not restore public executable' }
        if ((Get-FileHash -LiteralPath $config).Hash -ine $configHash) { throw 'Maintenance changed user configuration' }
        if ((Get-FileHash -LiteralPath (Join-Path $root 'registration.json')).Hash -ine $registrationHash) { throw 'Maintenance changed entry ownership' }
        if ([Environment]::GetEnvironmentVariable('PATH','User') -cne $oldPath) { throw 'Isolated validation changed user PATH' }
    }
    & $exe startup disable --profile $fixtureProfile
    if ($LASTEXITCODE -or (Get-FileHash $fixtureProfile).Hash -ne $profileOriginal) { throw 'Startup removal did not restore user profile bytes' }
    $priorModule = Join-Path $evidence 'Modules/PSReadLine'
    [IO.Directory]::CreateDirectory($priorModule) | Out-Null
    $source = (Resolve-Path -LiteralPath $OriginalEditorManifest).Path
    $v = (Import-PowerShellDataFile -LiteralPath $source).ModuleVersion
    Copy-Item -LiteralPath (Split-Path -LiteralPath $source) -Destination (Join-Path $priorModule $v) -Recurse
    $env:PSModulePath=(Split-Path -LiteralPath $priorModule)+';'+$oldModules
    function Read-FreshEditor {
        $json = & (Join-Path $PSHOME 'pwsh.exe') -NoProfile -NonInteractive -Command 'Import-Module PSReadLine -ErrorAction Stop; $t=[Microsoft.PowerShell.PSConsoleReadLine]; @{dll=$t.Assembly.Location;sha=(Get-FileHash $t.Assembly.Location).Hash;hooks=[bool]$t.GetProperty("EditorIntegrationVersion")} | ConvertTo-Json -Compress'
        if ($LASTEXITCODE) { throw 'Cannot inspect fresh shell after module change' }
        $json | ConvertFrom-Json
    }
    $prior = Read-FreshEditor
    if ($prior.hooks -or -not $prior.dll.StartsWith($priorModule+'\',[StringComparison]::OrdinalIgnoreCase)) { throw 'Original user module did not load' }
    & $exe startup enable --profile $fixtureProfile
    if ($LASTEXITCODE -or -not (Read-FreshEditor).hooks) { throw 'Candidate did not activate its editor' }
    # Exercise rollback after the user opted into the new module as well as
    # the earlier rollback that preserved the untouched 0.5.3 profile.
    & $manager -Action Rollback -InstallRoot $root
    $fallback = [IO.File]::ReadAllText($fixtureProfile)
    if ((Get-FileHash $exe).Hash -ine $oldHash -or (Read-FreshEditor).sha -ine $prior.sha -or
        -not $fallback.Contains('run --shell') -or $fallback.Contains('::AttachProfile(') -or
        -not $fallback.Contains($profileText)) { throw 'Rollback left an incompatible startup hook or editor' }
    & $exe startup disable --profile $fixtureProfile --owned-only
    if ($LASTEXITCODE -or (Get-FileHash $fixtureProfile).Hash -ine $profileOriginal) { throw 'Public executable could not remove the compatible hook' }
    & $exe startup enable --profile $fixtureProfile
    if ($LASTEXITCODE) { throw 'Public executable could not re-enable after rollback' }
    & $manager -Action Upgrade -InstallRoot $root -PackagePath $candidate
    & $exe startup enable --profile $fixtureProfile
    if ($LASTEXITCODE -or -not (Read-FreshEditor).hooks -or (Get-FileHash $componentState).Hash -ine $componentHash) { throw 'Re-upgrade did not preserve the component and recover the current editor' }
    & $exe terminal uninstall --install-root $root
    if ($LASTEXITCODE) { throw 'Optional component uninstall failed' }
    & $manager -Action Uninstall -InstallRoot $root
    if ((Test-Path $exe) -or (Get-FileHash $config).Hash -ne $configHash) { throw 'Core uninstall did not preserve user settings' }
    $after = Read-FreshEditor
    if ($after.hooks -or $after.dll -ine $prior.dll -or $after.sha -ine $prior.sha -or (Get-FileHash $fixtureProfile).Hash -ne $profileOriginal) { throw 'Uninstall did not restore original module and Profile' }
    $receipt['restored_editor'] = $after
    $receipt['active_profile_rollback'] = 'compatible hook removed and re-enabled by public 0.5.3 executable'
    $receipt['passed']=$true
    Write-Host "Public 0.5.3 upgrade/rollback/re-upgrade passed: $evidence"
} catch {
    $receipt['error']=$_.Exception.Message
    throw
} finally {
    $receipt | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $evidence 'receipt.json') -Encoding UTF8
    $env:LOCALAPPDATA=$oldLocal
    $env:APPDATA=$oldRoaming
    $env:PSModulePath=$oldModules
}
