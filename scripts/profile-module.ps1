# The private binary is versioned separately so an active CLR can retain its DLL
# while a new PowerShell resolves the original manifest.
function Assert-EditorDirectory([string]$Path) {
    $item = Get-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue
    if ($item -and (-not $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint))) {
        throw "Expected an ordinary directory: $Path"
    }
}
function Get-EditorTreeDigest([string]$Root) {
    # Do not traverse a redirected subdirectory, even when comparing a module
    # that was originally staged by us.
    $queue = [Collections.Generic.Queue[string]]::new()
    $queue.Enqueue($Root)
    $records = [Collections.Generic.List[string]]::new()
    while ($queue.Count) {
        $dir = $queue.Dequeue()
        foreach ($entry in Get-ChildItem -LiteralPath $dir -Force) {
            if ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) { return 'modified' }
            $relative = $entry.FullName.Substring($Root.Length+1)
            if ($entry.PSIsContainer) { $records.Add("D $relative"); $queue.Enqueue($entry.FullName) }
            else { $records.Add("F $relative " + (Get-FileHash -LiteralPath $entry.FullName).Hash) }
        }
    }
    $records.Sort([StringComparer]::Ordinal)
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return [Convert]::ToBase64String($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes(($records -join "`n")))) }
    finally { $sha.Dispose() }
}
function Restore-OriginalEditorDirectory {
    param($Owner,[string]$Active,[string]$Store)
    # Candidates predating manifest-based installation moved the original
    # directory. Recover only the directory recorded by that layout.
    $original = Join-Path $Store 'original'
    Assert-EditorDirectory $original
    if ($Owner.had_original -and -not [IO.Directory]::Exists($original)) {
        if (-not [IO.Directory]::Exists($Active)) { throw "Missing original module at $original" }
    } else {
        if ([IO.Directory]::Exists($Active)) {
            $retired = Join-Path $Store ('retired-'+[guid]::NewGuid().ToString('N'))
            [IO.Directory]::Move($Active,$retired)
            Write-Warning "Preserved previous managed directory at $retired"
        }
        if ($Owner.had_original) { [IO.Directory]::Move($original,$Active) }
    }
}
function Write-EditorFile([string]$Path,[byte[]]$Bytes) {
    $stage = $Path+'.'+[guid]::NewGuid().ToString('N')+'.tmp'
    try {
        [IO.File]::WriteAllBytes($stage,$Bytes)
        if ([IO.File]::Exists($Path)) { [IO.File]::Replace($stage,$Path,[NullString]::Value) }
        else { [IO.File]::Move($stage,$Path) }
    } finally { if ([IO.File]::Exists($stage)) { [IO.File]::Delete($stage) } }
}
function Remove-EmptyEditorDirectories($Record,[string]$Directory,[string]$Module,[string]$Modules) {
    $paths = @()
    if (-not $Record.had_directory) { $paths += $Directory }
    if ($Record.had_module_dir -eq $false) { $paths += $Module }
    if ($Record.had_modules_root -eq $false) { $paths += $Modules }
    foreach ($path in $paths) {
        Assert-EditorDirectory $path
        if ([IO.Directory]::Exists($path) -and -not @(Get-ChildItem -LiteralPath $path -Force).Count) {
            try { [IO.Directory]::Delete($path) } catch [IO.IOException] { }
        }
    }
}
function Assert-EditorFile([string]$Path) {
    $item = Get-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue
    if ($item -and ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint))) {
        throw "Expected an ordinary file: $Path"
    }
}
function Remove-OwnedEditorPayload([string]$Directory,[string]$Name,[string]$Digest) {
    if ($Name -cnotmatch '^Blueberry-Editor-[0-9a-f]{32}$') { throw 'Invalid managed editor directory name' }
    $target = [IO.Path]::GetFullPath((Join-Path $Directory $Name))
    if ((Split-Path -LiteralPath $target) -ine $Directory) { throw 'Unexpected editor payload path' }
    Assert-EditorDirectory $target
    if (-not [IO.Directory]::Exists($target)) { return 'removed' }
    if ((Get-EditorTreeDigest $target) -cne $Digest) { Write-Warning "Preserved modified editor payload at $target"; return 'preserved' }
    try {
        # Do not partially remove the module under a running CLR. All assets
        # are ordinary files in the packaged version directory.
        foreach ($file in [IO.Directory]::GetFiles($target,'*',[IO.SearchOption]::AllDirectories)) {
            $handle = [IO.File]::Open($file,'Open','Read','None')
            $handle.Dispose()
        }
        [IO.Directory]::Delete($target,$true)
        return 'removed'
    } catch [IO.IOException] { Write-Warning "Preserved in-use editor payload at $target"; return 'in-use' }
      catch [UnauthorizedAccessException] { Write-Warning "Preserved in-use editor payload at $target"; return 'in-use' }
}
function Update-BlueberryProfileEditor {
    param([string]$Action,[string]$ProfileFile,[string]$Executable,[string]$Payload,[string]$Version,[switch]$OwnedOnly)
    $parent = Split-Path -LiteralPath $ProfileFile
    $modules = Join-Path $parent 'Modules'
    $active = Join-Path $modules 'PSReadLine'
    $store = Join-Path $parent '.blueberry-editor'
    $receipt = Join-Path $store 'owner.json'
    if ($Action -eq 'disable' -and -not [IO.Directory]::Exists($store)) { return }
    foreach ($dir in @($parent,$store)) { Assert-EditorDirectory $dir }
    Assert-EditorFile $receipt
    $owner = if ([IO.File]::Exists($receipt)) { [IO.File]::ReadAllText($receipt) | ConvertFrom-Json }
    if ($owner -and ($owner.exe -ine $Executable -or $owner.profile -ine $ProfileFile)) {
        if ($OwnedOnly) { return }
        throw "Another installation manages $active"
    }
    foreach ($dir in @($modules,$active)) { Assert-EditorDirectory $dir }
    if ([IO.Directory]::Exists($store)) {
        foreach ($note in Get-ChildItem -LiteralPath $store -Filter 'retire-*.json' -File) {
            Assert-EditorFile $note.FullName
            $pending = [IO.File]::ReadAllText($note.FullName) | ConvertFrom-Json
            if ($pending.exe -ine $Executable -or $pending.profile -ine $ProfileFile) { continue }
            if ($pending.version -notin '2.0.0','2.4.5') { throw 'Invalid deferred editor version' }
            $where = if ($pending.flat) { $active } else { Join-Path $active $pending.version }
            Assert-EditorDirectory $where
            if ((Remove-OwnedEditorPayload $where $pending.payload $pending.payload_digest) -ne 'in-use') {
                [IO.File]::Delete($note.FullName)
                Remove-EmptyEditorDirectories $pending $where $active $modules
            }
        }
    }
    if ($owner -and $owner.schema -eq 1) {
        Restore-OriginalEditorDirectory $owner $active $store
        [IO.File]::Delete($receipt)
        $owner = $null
    }
    $backup = Join-Path $store 'original.psd1'
    Assert-EditorFile $backup
    if ($owner) {
        if ($owner.schema -ne 2 -or $owner.flat -isnot [bool] -or $owner.had_manifest -isnot [bool] -or
            $owner.version -notin '2.0.0','2.4.5' -or $owner.managed_hash -notmatch '^[0-9a-f]{64}$' -or
            $owner.payload -cnotmatch '^Blueberry-Editor-[0-9a-f]{32}$') { throw 'Invalid editor ownership record' }
        $dir = if ($owner.flat) { $active } else { Join-Path $active $owner.version }
        Assert-EditorDirectory $dir
        $manifest = Join-Path $dir 'PSReadLine.psd1'
        Assert-EditorFile $manifest
        $currentHash = if ([IO.File]::Exists($manifest)) { (Get-FileHash -LiteralPath $manifest).Hash }
        if ($Action -eq 'enable') {
            if ($currentHash -ine $owner.managed_hash) { throw "Managed manifest changed; run startup disable to restore it: $manifest" }
            $installed = Join-Path $dir $owner.payload
            Assert-EditorDirectory $installed
            if ((Get-EditorTreeDigest $installed) -cne $owner.payload_digest) { throw "Managed assets changed; run startup disable to preserve them before reinstalling" }
            if ($owner.version -eq $Version -and (Get-EditorTreeDigest (Join-Path $Payload $Version)) -ceq $owner.package_digest) { return }
        }
        if ($owner.had_manifest -and $currentHash -ine $owner.original_hash -and
            (-not [IO.File]::Exists($backup) -or (Get-FileHash $backup).Hash -ine $owner.original_hash)) { throw "Cannot verify original module manifest at $backup" }
        if ($currentHash -and $currentHash -ine $owner.managed_hash -and $currentHash -ine $owner.original_hash) {
            $saved = Join-Path $store ('modified-manifest-'+[guid]::NewGuid().ToString('N')+'.psd1')
            [IO.File]::Copy($manifest,$saved)
            Write-Warning "Preserved edited manifest at $saved"
        }
        if ($owner.had_manifest) {
            if ($currentHash -ine $owner.original_hash) { Write-EditorFile $manifest ([IO.File]::ReadAllBytes($backup)) }
        }
        elseif ([IO.File]::Exists($manifest)) { [IO.File]::Delete($manifest) }
        $clean = Remove-OwnedEditorPayload $dir $owner.payload $owner.payload_digest
        if ($clean -eq 'in-use') {
            $note = Join-Path $store ('retire-'+$owner.payload+'.json')
            Write-EditorFile $note ([Text.Encoding]::UTF8.GetBytes(($owner | ConvertTo-Json)))
        }
        if ($owner.had_manifest -and [IO.File]::Exists($backup)) { [IO.File]::Delete($backup) }
        [IO.File]::Delete($receipt)
        if ($clean -eq 'removed') { Remove-EmptyEditorDirectories $owner $dir $active $modules }
    }
    if ($Action -eq 'disable') {
        if ([IO.Directory]::Exists($store) -and -not @(Get-ChildItem -LiteralPath $store -Force).Count) { [IO.Directory]::Delete($store) }
        return
    }
    if ($Version -notin '2.0.0','2.4.5') { throw "Unsupported editor $Version" }
    $source = Join-Path $Payload $Version
    $template = Join-Path $source 'PSReadLine.psd1'
    if (-not [IO.File]::Exists($template)) { throw "Missing packaged editor $Version" }
    $flat = [IO.File]::Exists((Join-Path $active 'PSReadLine.psd1')) -or [IO.File]::Exists((Join-Path $active 'PSReadLine.psm1'))
    $dir = if ($flat) { $active } else { Join-Path $active $Version }
    Assert-EditorDirectory $dir
    $manifest = Join-Path $dir 'PSReadLine.psd1'
    Assert-EditorFile $manifest
    $hadDirectory = [IO.Directory]::Exists($dir)
    $hadManifest = [IO.File]::Exists($manifest)
    $hadModuleDir = [IO.Directory]::Exists($active)
    $hadModulesRoot = [IO.Directory]::Exists($modules)
    [IO.Directory]::CreateDirectory($dir) | Out-Null
    [IO.Directory]::CreateDirectory($store) | Out-Null
    $name = 'Blueberry-Editor-'+[guid]::NewGuid().ToString('N')
    $private = [IO.Directory]::CreateDirectory((Join-Path $dir $name)).FullName
    $packageDigest = Get-EditorTreeDigest $source
    if ($packageDigest -ceq 'modified') { throw 'Unexpected link in packaged editor' }
    foreach ($item in Get-ChildItem -LiteralPath $source -Force) {
        if ($item.Name -ne 'PSReadLine.psd1') { Copy-Item -LiteralPath $item.FullName -Destination $private -Recurse }
    }
    # CLR 4 and CoreCLR use different file sharing modes for mapped DLLs.
    # Hold an explicit lease for this generation even after Remove-Module.
    [IO.File]::WriteAllBytes((Join-Path $private 'blueberry-module.use'),[byte[]]@())
    $leaseScript = @'
& {
$leasePath = [IO.Path]::Combine($PSScriptRoot,'blueberry-module.use')
$leaseKey = 'Blueberry.Editor.Lease:' + $leasePath
if ($null -eq [AppDomain]::CurrentDomain.GetData($leaseKey)) {
    [AppDomain]::CurrentDomain.SetData($leaseKey,[IO.File]::Open($leasePath,'Open','Read','Read'))
}
}
'@
    [IO.File]::WriteAllText((Join-Path $private 'Blueberry.EditorLease.ps1'),$leaseScript)
    $text = [IO.File]::ReadAllText($template)
    $dll = if ($Version -eq '2.0.0') { 'Microsoft.PowerShell.PSReadLine2.dll' } else { 'Microsoft.PowerShell.PSReadLine.dll' }
    $text = $text.Replace('RootModule =', "ScriptsToProcess = '$name/Blueberry.EditorLease.ps1'`r`nRootModule =")
    foreach ($member in @('PSReadLine.psm1','PSReadLine.format.ps1xml',$dll)) {
        if (-not $text.Contains($member)) { throw "Missing manifest member: $member" }
        $text = $text.Replace($member,"$name/$member")
    }
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes($text)
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $hash = [BitConverter]::ToString($sha.ComputeHash($bytes)).Replace('-','') } finally { $sha.Dispose() }
    $originalHash = if ($hadManifest) { (Get-FileHash $manifest).Hash }
    if ([IO.File]::Exists($backup)) {
        if (-not $hadManifest -or (Get-FileHash $backup).Hash -ine $originalHash) { throw "Unclaimed original manifest at $backup; preserved" }
    } elseif ($hadManifest) { [IO.File]::Copy($manifest,$backup) }
    $record = @{schema=2;exe=$Executable;profile=$ProfileFile;version=$Version;flat=$flat;had_directory=$hadDirectory;had_module_dir=$hadModuleDir;had_modules_root=$hadModulesRoot;had_manifest=$hadManifest;original_hash=$originalHash;managed_hash=$hash;payload=$name;payload_digest=(Get-EditorTreeDigest $private);package_digest=$packageDigest}
    Write-EditorFile $receipt ([Text.Encoding]::UTF8.GetBytes(($record | ConvertTo-Json)))
    try { Write-EditorFile $manifest $bytes } catch {
        Update-BlueberryProfileEditor disable $ProfileFile $Executable $Payload $Version -OwnedOnly
        throw
    }
}
