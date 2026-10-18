[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][ValidateSet('Install','Upgrade','Rollback','Uninstall','Status','Launch')][string]$Operation,
    [Parameter(Mandatory=$true)][string]$InstallRoot,
    [string]$PackagePath
)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$componentInstallRoot=$InstallRoot
$componentPackagePath=$PackagePath
. (Join-Path $PSScriptRoot 'manage-install.ps1') -Action Install -FunctionsOnly
$install=Assert-SafeInstallRoot $componentInstallRoot
$root=Join-Path $install 'components/terminal'
Assert-NoReparseComponents $root
$statePath=Join-Path $root 'component-state.json'
function Read-ComponentState {
    if (-not [IO.File]::Exists($statePath)) { return @{schema_version=1;product='Blueberry.Terminal';installed=$false;current=$null;history=@();versions=@{}} }
    $state=Read-JsonHashtable $statePath
    if ($state.schema_version -ne 1 -or $state.product -cne 'Blueberry.Terminal' -or
        $state.metadata_sha256 -cne (Get-ManifestIntegrityHash $state)) { throw 'Invalid Terminal installation state' }
    foreach ($id in $state.versions.Keys) {
        if ($id -notmatch '^[a-fA-F0-9]{64}$') { throw 'Invalid Terminal version identity' }
        foreach ($record in $state.versions[$id].files) { [void](Assert-SafeRelativePath $record.path) }
    }
    if ($state.installed -and (-not $state.current -or -not $state.versions.ContainsKey($state.current))) { throw 'Missing active Terminal version' }
    return $state
}
function Assert-ComponentFiles($state, [string]$id) {
    foreach ($record in $state.versions[$id].files) {
        $path=Get-ManagedPath -Root (Join-Path $root "versions/$id") -RelativePath $record.path
        $item=Get-RegularFileRecord -Path $path -Label 'Terminal runtime'
        if ($item.Length -ne $record.bytes -or (Get-Sha256 $path) -ine $record.sha256) { throw "Modified Terminal runtime: $($record.path)" }
    }
}
function Copy-ComponentSettings([string]$sourceId,[string]$targetId) {
    if ($sourceId -eq $targetId) { return }
    $source=if ($sourceId) { Join-Path $root "versions/$sourceId/settings" } else { Join-Path $install 'terminal/settings' }
    if (-not [IO.Directory]::Exists($source)) { return }
    Assert-NoReparseComponents $source
    foreach ($file in Get-ChildItem -LiteralPath $source -File -Recurse -Force) {
        Assert-NoReparseComponents $file.FullName
        $relative=Get-BlueberryRelativePath $source $file.FullName
        $destination=Get-ManagedPath -Root (Join-Path $root "versions/$targetId/settings") -RelativePath $relative
        Copy-FileAtomic -Source $file.FullName -Destination $destination -ExpectedHash (Get-Sha256 $file.FullName) -RelativePath $relative
    }
}
$lockHash=Get-ByteSha256 ([Text.Encoding]::UTF8.GetBytes($install.TrimEnd('\').ToUpperInvariant()))
$mutex=[Threading.Mutex]::new($false,('Local\Blueberry.Install.'+$lockHash))
$owned=$false
try {
    try { $owned=$mutex.WaitOne(0) } catch [Threading.AbandonedMutexException] { $owned=$true }
    if (-not $owned) { throw 'Another installation transaction is active' }
    $state=Read-ComponentState
    if ($Operation -eq 'Status') { $state | ConvertTo-Json -Depth 30; return }
    if ($Operation -eq 'Launch') {
        if (-not $state.installed) { throw 'Terminal component is not installed; use blueberry terminal install --package <zip>' }
        Assert-ComponentFiles $state $state.current
        $terminal=Join-Path $root "versions/$($state.current)/WindowsTerminal.exe"
        $exe=Join-Path $install 'blueberry.exe'
        if (-not [IO.File]::Exists($exe)) { throw 'Core executable is missing' }
        $env:BLUEBERRY_NATIVE_OVERLAY='4'
        foreach ($name in @('BLUEBERRY_ACTIVE','TERM_PROGRAM','TERM_PROGRAM_VERSION','WT_SESSION','WT_PROFILE_ID')) { [Environment]::SetEnvironmentVariable($name,$null,'Process') }
        & $terminal --window new new-tab --inheritEnvironment --startingDirectory (Get-Location).Path $exe run --host-mode direct --transport pipe
        exit $LASTEXITCODE
    }
    foreach ($process in Get-Process WindowsTerminal -ErrorAction SilentlyContinue) {
        if ($process.Path -and $process.Path.StartsWith(($root.TrimEnd('\')+'\'),[StringComparison]::OrdinalIgnoreCase)) { throw 'Close this component Terminal before maintenance' }
    }
    if ($Operation -eq 'Rollback') {
        if (-not $state.installed -or $state.history.Count -eq 0) { throw 'No Terminal rollback version' }
        $next=[string]$state.history[0]
        Assert-ComponentFiles $state $next
        Copy-ComponentSettings $state.current $next
        $old=$state.current
        $state.current=$next
        $state.history=@($old)+@($state.history | Select-Object -Skip 1)
        Write-Utf8JsonAtomic -Path $statePath -Value $state
    } elseif ($Operation -eq 'Uninstall') {
        # Mark inactive first. A crash or locked file leaves an explicit retryable
        # state, never an active launcher pointing at a partially deleted runtime.
        if ($state.installed) { foreach ($id in $state.versions.Keys) { Assert-ComponentFiles $state $id } }
        $state.installed=$false; $state.current=$null; $state.history=@()
        Write-Utf8JsonAtomic -Path $statePath -Value $state
        foreach ($id in @($state.versions.Keys)) {
            foreach ($record in @($state.versions[$id].files)) {
                $path=Get-ManagedPath -Root (Join-Path $root "versions/$id") -RelativePath $record.path
                if ([IO.File]::Exists($path)) {
                    if ((Get-Sha256 $path) -ine $record.sha256) { throw 'Modified Terminal runtime retained during uninstall' }
                    Remove-Item -LiteralPath $path -Force
                }
            }
            [void]$state.versions.Remove($id)
            Write-Utf8JsonAtomic -Path $statePath -Value $state
        }
    } else {
        if ($Operation -eq 'Install' -and $state.installed) { throw 'Terminal is already installed; use upgrade' }
        if ($Operation -eq 'Upgrade' -and -not $state.installed) { throw 'Terminal is not installed; use install' }
        $package=Resolve-ExistingFile -Path $componentPackagePath -Label 'Terminal component package'
        $id=(Get-Sha256 $package).ToLowerInvariant()
        if (-not $state.versions.ContainsKey($id)) {
            $stage=Join-Path $root ('.stage-'+[Guid]::NewGuid().ToString('N'))
            [IO.Directory]::CreateDirectory($stage) | Out-Null
            Add-Type -AssemblyName System.IO.Compression.FileSystem
            $archive=[IO.Compression.ZipFile]::OpenRead($package)
            try {
                if ($archive.Entries.Count -gt 4096) { throw 'Terminal package has too many entries' }
                [long]$totalBytes=0
                foreach ($entry in $archive.Entries) {
                    if ($entry.Length -gt 536870912) { throw 'Unsafe Terminal package entry' }
                    $totalBytes += $entry.Length
                    if ($totalBytes -gt 2147483648) { throw 'Terminal package exceeds total extracted size limit' }
                }
                $seen=[Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
                foreach ($entry in $archive.Entries) {
                    if ($entry.FullName.EndsWith('/')) { continue }
                    if ($entry.FullName -match '[:\\]' -or $entry.FullName -match '(^|/)[^/]*[. ](/|$)') { throw 'Unsafe Terminal member name' }
                    $name=Assert-SafeRelativePath $entry.FullName
                    if (-not $seen.Add($name) -or (($entry.ExternalAttributes -shr 16) -band 0xF000) -eq 0xA000 -or $entry.Length -gt 536870912) { throw 'Unsafe Terminal package entry' }
                    $path=Get-ManagedPath -Root $stage -RelativePath $name
                    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($path)) | Out-Null
                    [IO.Compression.ZipFileExtensions]::ExtractToFile($entry,$path,$false)
                    $stream=[IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::Read)
                    try { $stream.Flush($true) } finally { $stream.Dispose() }
                }
            } finally { $archive.Dispose() }
            $manifest=Read-JsonHashtable (Join-Path $stage 'component.json')
            if ($manifest.product -cne 'Blueberry.Terminal' -or $manifest.package_kind -cne 'terminal' -or $manifest.schema_version -ne 1 -or
                $manifest.metadata_sha256 -cne (Get-ManifestIntegrityHash $manifest) -or $manifest.terminal_dependency.interface_version -ne 4) { throw 'Invalid Terminal component manifest' }
            $ownedFiles=[Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
            [void]$ownedFiles.Add('component.json')
            foreach ($record in $manifest.files) {
                $name=Assert-SafeRelativePath $record.path
                if ($name -like 'settings/*' -or -not $ownedFiles.Add($name)) { throw 'Invalid Terminal owned path' }
                $path=Get-ManagedPath -Root $stage -RelativePath $name
                $file=Get-RegularFileRecord -Path $path -Label 'Terminal package file'
                if ($file.Length -ne $record.bytes -or (Get-Sha256 $path) -ine $record.sha256) { throw 'Terminal package hash mismatch' }
            }
            if ($seen.Count -ne $ownedFiles.Count) { throw 'Unlisted Terminal package files' }
            foreach ($name in @('WindowsTerminal.exe','Microsoft.Terminal.Settings.Model.dll','.portable')) { if (-not $ownedFiles.Contains($name)) { throw "Missing Terminal asset: $name" } }
            Assert-PeX64 (Join-Path $stage 'WindowsTerminal.exe') | Out-Null
            if ((Get-Sha256 (Join-Path $stage 'WindowsTerminal.exe')) -ine $manifest.terminal_dependency.executable_sha256) { throw 'Terminal executable identity mismatch' }
            foreach ($license in $manifest.terminal_dependency.licenses.Keys) {
                $name=[IO.Path]::GetFileName($license)
                if (-not $ownedFiles.Contains($name) -or (Get-Sha256 (Join-Path $stage $name)) -ine $manifest.terminal_dependency.licenses[$license]) { throw 'Terminal license identity mismatch' }
            }
            $target=Join-Path $root "versions/$id"
            [IO.Directory]::CreateDirectory((Split-Path -Parent $target)) | Out-Null
            if ([IO.Directory]::Exists($target)) {
                # A previous process may have stopped after publishing the
                # immutable version but before switching the state pointer.
                foreach ($record in $manifest.files) {
                    $existing=Get-ManagedPath -Root $target -RelativePath $record.path
                    if ((Get-Sha256 $existing) -ine $record.sha256) { throw 'Interrupted Terminal version has changed files' }
                }
                if ((Get-Sha256 (Join-Path $target 'component.json')) -ine (Get-Sha256 (Join-Path $stage 'component.json'))) { throw 'Interrupted Terminal manifest mismatch' }
                if ([IO.Path]::GetDirectoryName($stage) -cne $root) { throw 'Invalid stage cleanup boundary' }
                Remove-Item -LiteralPath $stage -Recurse -Force
            } else { [IO.Directory]::Move($stage,$target) }
            $state.versions[$id]=@{version=$manifest.version;dependency=$manifest.terminal_dependency;files=@($manifest.files)+@(@{path='component.json';sha256=(Get-Sha256 (Join-Path $target 'component.json'));bytes=(Get-Item -LiteralPath (Join-Path $target 'component.json')).Length})}
        }
        Assert-ComponentFiles $state $id
        Copy-ComponentSettings $state.current $id
        if ($state.current -and $state.current -ne $id) { $state.history=@($state.current)+@($state.history | Where-Object { $_ -ne $id }) }
        $state.current=$id; $state.installed=$true
        Write-Utf8JsonAtomic -Path $statePath -Value $state
    }
    $state | ConvertTo-Json -Depth 30
} finally {
    if ($owned) { $mutex.ReleaseMutex() }
    $mutex.Dispose()
}
