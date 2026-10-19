#Requires -Version 5.1
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$CorePackage,
    [Parameter(Mandatory=$true)][string]$NextCorePackage,
    [Parameter(Mandatory=$true)][string]$TerminalPackage,
    [Parameter(Mandatory=$true)][string]$NextTerminalPackage
)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$identity=[Security.Principal.WindowsIdentity]::GetCurrent()
$principal=[Security.Principal.WindowsPrincipal]::new($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'This isolated VHD regression requires an administrative Windows CI runner.'
}
$workspace=[IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$evidence=Join-Path $workspace ('target/storage-failures-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($evidence)
$vhd=Join-Path $evidence 'owned-fault-volume.vhd'
$core=Join-Path $PSScriptRoot 'manage-install.ps1'
$terminal=Join-Path $PSScriptRoot 'manage-terminal.ps1'
$results=[Collections.Generic.List[object]]::new()
$attached=$false
$root=$null
function Save-Results {
    $results.ToArray() | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $evidence 'results.json') -Encoding UTF8
}
function Run-Manager([string]$Name,[scriptblock]$Action,[bool]$ExpectFailure=$false) {
    $timer=[Diagnostics.Stopwatch]::StartNew()
    $failed=$false
    try { & $Action *> (Join-Path $evidence ($Name+'.log')) }
    catch {
        $failed=$true
        $_ | Out-String | Add-Content -LiteralPath (Join-Path $evidence ($Name+'.log'))
    }
    $results.Add(@{name=$Name;expected_failure=$ExpectFailure;failed=$failed;elapsed_ms=$timer.Elapsed.TotalMilliseconds})
    Save-Results
    if ($failed -ne $ExpectFailure) { throw "Unexpected result for $Name; inspect retained evidence." }
}
function Snapshot {
    $paths=@('install.json','blueberry.exe','RELEASE-NOTES.md','user-settings.json','components/terminal/component-state.json')
    $manifest=Get-Content -LiteralPath (Join-Path $root 'install.json') -Raw | ConvertFrom-Json
    $paths+=@($manifest.managed_hashes.PSObject.Properties | ForEach-Object Name)
    $statePath=Join-Path $root 'components/terminal/component-state.json'
    if (Test-Path -LiteralPath $statePath) {
        $state=Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
        $version=$state.versions.PSObject.Properties[$state.current].Value
        $paths+=@($version.files | ForEach-Object { 'components/terminal/versions/'+$state.current+'/'+$_.path })
        $paths+='components/terminal/versions/'+$state.current+'/settings/settings.json'
    }
    $hashes=@{}
    foreach ($relative in $paths) {
        $path=Join-Path $root $relative
        if (Test-Path -LiteralPath $path -PathType Leaf) { $hashes[$relative]=(Get-FileHash -LiteralPath $path).Hash }
    }
    return $hashes
}
function Assert-Unchanged($Before) {
    foreach ($relative in $Before.Keys) {
        if ((Get-FileHash -LiteralPath (Join-Path $root $relative)).Hash -ne $Before[$relative]) {
            throw "Failed maintenance changed active state or user data: $relative"
        }
    }
}
function Fill-OwnedVolume {
    $path=Join-Path $root 'space-reservation.bin'
    $stream=[IO.FileStream]::new($path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None,1,[IO.FileOptions]::WriteThrough)
    $full=$false
    $lastCode=$null
    try {
        foreach ($size in @(1048576,65536,4096,512)) {
            $buffer=New-Object byte[] $size
            while ($true) {
                try { $stream.Write($buffer,0,$buffer.Length) }
                catch {
                    $errorDetail=$_.Exception.GetBaseException()
                    $code=$errorDetail.HResult -band 0xffff
                    if ($errorDetail -isnot [IO.IOException] -or $code -notin @(39,112)) { throw }
                    $full=$true
                    $lastCode=$code
                    break
                }
            }
        }
        $stream.Flush($true)
    } finally { $stream.Dispose() }
    if (-not $full) { throw 'The owned volume did not report disk full.' }
    $results.Add(@{name='volume-filled';disk_full_win32_code=$lastCode;reserved_bytes=(Get-Item -LiteralPath $path).Length})
    Save-Results
    return $path
}
function With-DeniedFile([string]$Path,[scriptblock]$Action) {
    $original=Get-Acl -LiteralPath $Path
    $denied=Get-Acl -LiteralPath $Path
    $parent=Split-Path -Parent $Path
    $originalParent=Get-Acl -LiteralPath $parent
    $deniedParent=Get-Acl -LiteralPath $parent
    $rule=[Security.AccessControl.FileSystemAccessRule]::new(
        $identity.User,[Security.AccessControl.FileSystemRights]'WriteData,AppendData,Delete',
        [Security.AccessControl.AccessControlType]::Deny)
    $denied.AddAccessRule($rule)
    $deniedParent.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
        $identity.User,[Security.AccessControl.FileSystemRights]'CreateFiles,DeleteSubdirectoriesAndFiles',
        [Security.AccessControl.AccessControlType]::Deny))
    try {
        Set-Acl -LiteralPath $Path -AclObject $denied
        Set-Acl -LiteralPath $parent -AclObject $deniedParent
        & $Action
    } finally {
        Set-Acl -LiteralPath $parent -AclObject $originalParent
        Set-Acl -LiteralPath $Path -AclObject $original
    }
}
try {
    # Only a newly created image below this unique workspace evidence directory
    # may be initialized/formatted. Never select an existing host disk by number.
    if (Test-Path -LiteralPath $vhd) { throw 'Refusing an existing disk image.' }
    $script=Join-Path $evidence 'create-owned-volume.txt'
    "create vdisk file=`"$vhd`" maximum=2048 type=expandable`r`nexit" | Set-Content -LiteralPath $script -Encoding ASCII
    & "$env:SystemRoot/System32/diskpart.exe" /s $script *> (Join-Path $evidence 'diskpart.log')
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $vhd -PathType Leaf)) { throw 'Owned disk image creation failed.' }
    $null=Mount-DiskImage -ImagePath $vhd -NoDriveLetter -PassThru
    $attached=$true
    $disk=Get-DiskImage -ImagePath $vhd | Get-Disk
    if ($disk.IsBoot -or $disk.IsSystem -or $disk.PartitionStyle -ne 'RAW' -or $disk.Size -gt 3GB) {
        throw 'Refusing to initialize a disk that is not the new empty owned image.'
    }
    $letters=@(Get-Volume | Where-Object DriveLetter | ForEach-Object { [string]$_.DriveLetter })
    $letter=@('Z','Y','X','W','V','U','T','S','R') | Where-Object { $_ -notin $letters } | Select-Object -First 1
    if (-not $letter) { throw 'No unused test drive letter.' }
    $null=Initialize-Disk -Number $disk.Number -PartitionStyle GPT
    $partition=New-Partition -DiskNumber $disk.Number -UseMaximumSize -DriveLetter $letter
    $volume=$partition | Format-Volume -FileSystem NTFS -NewFileSystemLabel 'BlueberryFault' -Confirm:$false
    @{image=$vhd;disk_unique_id=$disk.UniqueId;volume_unique_id=$volume.UniqueId;drive_letter=$letter} |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $evidence 'volume-identity.json') -Encoding UTF8
    $root=Join-Path ($letter+':\') 'isolated-install'
    Run-Manager 'core-install' { & $core -Action Install -PackagePath $CorePackage -InstallRoot $root }
    [IO.File]::WriteAllText((Join-Path $root 'user-settings.json'),'{"font":"中文 preserved"}')
    Run-Manager 'terminal-install' { & $terminal -Operation Install -PackagePath $TerminalPackage -InstallRoot $root }
    $state=Get-Content -LiteralPath (Join-Path $root 'components/terminal/component-state.json') -Raw | ConvertFrom-Json
    $settings=Join-Path $root ('components/terminal/versions/'+$state.current+'/settings/settings.json')
    $null=[IO.Directory]::CreateDirectory((Split-Path -Parent $settings))
    [IO.File]::WriteAllText($settings,'{"fontFace":"中文 retained"}')
    $before=Snapshot
    $reservation=Fill-OwnedVolume
    try {
        Run-Manager 'core-disk-full' { & $core -Action Upgrade -PackagePath $NextCorePackage -InstallRoot $root } $true
        Assert-Unchanged $before
        Run-Manager 'terminal-disk-full' { & $terminal -Operation Upgrade -PackagePath $NextTerminalPackage -InstallRoot $root } $true
        Assert-Unchanged $before
    } finally { Remove-Item -LiteralPath $reservation -Force }
    Run-Manager 'core-recover-space' { & $core -Action Recover -InstallRoot $root }
    Run-Manager 'core-retry-space' { & $core -Action Upgrade -PackagePath $NextCorePackage -InstallRoot $root }
    Run-Manager 'terminal-retry-space' { & $terminal -Operation Upgrade -PackagePath $NextTerminalPackage -InstallRoot $root }
    $before=Snapshot
    With-DeniedFile (Join-Path $root 'RELEASE-NOTES.md') {
        Run-Manager 'core-permission-denied' { & $core -Action Upgrade -PackagePath $CorePackage -InstallRoot $root } $true
        Assert-Unchanged $before
    }
    Run-Manager 'core-recover-permission' { & $core -Action Recover -InstallRoot $root }
    Run-Manager 'core-retry-permission' { & $core -Action Upgrade -PackagePath $CorePackage -InstallRoot $root }
    $before=Snapshot
    With-DeniedFile (Join-Path $root 'components/terminal/component-state.json') {
        Run-Manager 'terminal-permission-denied' { & $terminal -Operation Upgrade -PackagePath $TerminalPackage -InstallRoot $root } $true
        Assert-Unchanged $before
    }
    Run-Manager 'terminal-retry-permission' { & $terminal -Operation Upgrade -PackagePath $TerminalPackage -InstallRoot $root }
    $results.Add(@{name='final';status='passed';core_sha256=(Get-FileHash $CorePackage).Hash;terminal_sha256=(Get-FileHash $TerminalPackage).Hash})
    Save-Results
} finally {
    try {
        if ($root -and (Test-Path -LiteralPath $root)) {
            foreach ($file in @(Get-ChildItem -LiteralPath $root -Recurse -File | Where-Object {
                $_.Name -in @('install.json','component-state.json','transaction.json','transaction.wal','operation.json','user-settings.json')
            })) {
                $relative=$file.FullName.Substring($root.Length).TrimStart('\','/')
                $destination=Join-Path (Join-Path $evidence 'state') $relative
                $null=[IO.Directory]::CreateDirectory((Split-Path -Parent $destination))
                Copy-Item -LiteralPath $file.FullName -Destination $destination
            }
        }
    } finally {
        if ($attached) { Dismount-DiskImage -ImagePath $vhd | Out-Null }
        $resolvedImage=[IO.Path]::GetFullPath($vhd)
        $ownedPrefix=[IO.Path]::GetFullPath($evidence).TrimEnd('\','/')+[IO.Path]::DirectorySeparatorChar
        if ($resolvedImage.StartsWith($ownedPrefix,[StringComparison]::OrdinalIgnoreCase) -and
            (Test-Path -LiteralPath $resolvedImage -PathType Leaf)) {
            Remove-Item -LiteralPath $resolvedImage -Force
        }
    }
}
Write-Output "Real NTFS disk-full and ACL regressions passed: $evidence"
