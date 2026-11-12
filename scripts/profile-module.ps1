# Keep the original module on the same volume. Directory renames preserve
# module bytes, alternate streams and ACLs across enable and disable.
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
function Update-BlueberryProfileEditor {
    param([string]$Action,[string]$ProfileFile,[string]$Executable,[string]$Payload,[string]$Version,[switch]$OwnedOnly)
    $parent = Split-Path -LiteralPath $ProfileFile
    $modules = Join-Path $parent 'Modules'
    $active = Join-Path $modules 'PSReadLine'
    $store = Join-Path $parent '.blueberry-editor'
    $receipt = Join-Path $store 'owner.json'
    $original = Join-Path $store 'original'
    foreach ($dir in @($parent,$modules,$active,$store,$original)) { Assert-EditorDirectory $dir }
    $owner = $null
    if ([IO.File]::Exists($receipt)) { $owner = [IO.File]::ReadAllText($receipt) | ConvertFrom-Json }
    if ($owner -and ($owner.exe -ine $Executable -or $owner.profile -ine $ProfileFile)) {
        if ($OwnedOnly) { return }
        throw "Another installation manages $active"
    }
    if ($Action -eq 'disable') {
        if (-not $owner) { return }
        # The receipt is durable before publication. A retry after interruption
        # must not retire the original when its backup was never moved.
        if ($owner.had_original -and -not [IO.Directory]::Exists($original)) {
            if (-not [IO.Directory]::Exists($active)) { throw "Missing original module at $original" }
        } else {
            if ([IO.Directory]::Exists($active)) {
                $retired = Join-Path $store ('retired-'+[guid]::NewGuid().ToString('N'))
                [IO.Directory]::Move($active,$retired)
                if ((Get-EditorTreeDigest $retired) -ceq $owner.digest) {
                    # Only this renamed, unchanged managed tree is disposable.
                    if ((Split-Path -LiteralPath ([IO.Path]::GetFullPath($retired))) -ine $store) { throw 'Unexpected retirement directory' }
                    try { [IO.Directory]::Delete($retired,$true) } catch { Write-Warning "Preserved in-use module at $retired" }
                } else { Write-Warning "Preserved modified module at $retired" }
            }
            if ($owner.had_original) { [IO.Directory]::Move($original,$active) }
        }
        [IO.File]::Delete($receipt)
        if (-not @(Get-ChildItem -LiteralPath $store -Force).Count) { [IO.Directory]::Delete($store) }
        return
    }
    $source = Join-Path $Payload $Version
    if (-not [IO.File]::Exists((Join-Path $source 'PSReadLine.psd1'))) { throw "Missing packaged editor $Version" }
    if ($owner) {
        if (-not [IO.Directory]::Exists($active) -or (Get-EditorTreeDigest $active) -cne $owner.digest) {
            throw "Managed module changed or enable was interrupted. Run startup disable to recover: $active"
        }
        $installedVersion = Join-Path $active $owner.version
        if ($Version -eq $owner.version -and (Get-EditorTreeDigest $installedVersion) -ceq (Get-EditorTreeDigest $source)) { return }
        # A newer executable may carry a different bridge for the same editor
        # version. Recover the original before publishing the new payload.
        Update-BlueberryProfileEditor disable $ProfileFile $Executable $Payload $Version -OwnedOnly
    }
    if ([IO.Directory]::Exists($original)) { throw "Unclaimed original module at $original" }
    [IO.Directory]::CreateDirectory($modules) | Out-Null
    [IO.Directory]::CreateDirectory($store) | Out-Null
    $stage = Join-Path $store ('stage-'+[guid]::NewGuid().ToString('N'))
    [IO.Directory]::CreateDirectory($stage) | Out-Null
    Copy-Item -LiteralPath $source -Destination (Join-Path $stage $Version) -Recurse
    $had = [IO.Directory]::Exists($active)
    $record = @{schema=1;exe=$Executable;profile=$ProfileFile;version=$Version;had_original=$had;digest=(Get-EditorTreeDigest $stage)}
    $tmp = Join-Path $store ('receipt-'+[guid]::NewGuid().ToString('N'))
    [IO.File]::WriteAllText($tmp,($record | ConvertTo-Json))
    [IO.File]::Move($tmp,$receipt)
    try {
        if ($had) { [IO.Directory]::Move($active,$original) }
        [IO.Directory]::Move($stage,$active)
    } catch {
        if ([IO.Directory]::Exists($original) -and -not [IO.Directory]::Exists($active)) { [IO.Directory]::Move($original,$active) }
        [IO.File]::Delete($receipt)
        throw
    }
}
