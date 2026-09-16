[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('Install', 'Upgrade', 'Uninstall', 'Rollback', 'PreviewSettings', 'ApplySettings')]
    [string]$Action,

    [string]$PackagePath,

    [string]$InstallRoot,

    [string]$SettingsPath,

    [string]$ProfileName = 'Blueberry',

    [string]$ExePath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'install-common.ps1')

$script:ManifestFileName = 'install.json'
$script:InstallManifestSchemaVersion = 2
$script:BlueberryProfileGuid = '{28E40931-78EB-4F61-8932-DAB5CB555972}'
$script:RequiredPackageFiles = @(
    'blueberry.exe',
    'VERSION.txt',
    'README.md',
    'LICENSE',
    'THIRD-PARTY-NOTICES.txt',
    'RELEASE-NOTES.md',
    'config.example.toml',
    'docs/installation.md',
    'docs/releasing.md',
    'docs/specifications.md',
    'docs/powershell-adapter.md',
    'specs/tools.toml',
    'specs/builtin/_catalog.toml',
    'specs/builtin/core.toml',
    'specs/builtin/vcs.toml',
    'specs/builtin/rust.toml',
    'specs/builtin/python.toml',
    'specs/builtin/javascript.toml',
    'specs/builtin/build.toml',
    'specs/builtin/containers.toml',
    'specs/builtin/devops.toml',
    'specs/builtin/remote.toml',
    'specs/builtin/windows.toml',
    'manage-install.ps1',
    'install-common.ps1',
    'install.ps1'
)

function Get-DefaultInstallRoot {
    $localAppData = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
    if ([string]::IsNullOrWhiteSpace($localAppData)) {
        throw '无法确定 LocalAppData；请显式传入 -InstallRoot'
    }
    return [IO.Path]::Combine($localAppData, 'Blueberry', 'bin')
}

function Get-FullPath {
    param([Parameter(Mandatory = $true)][string]$Path)
    if ([string]::IsNullOrWhiteSpace($Path)) {
        throw '路径不能为空'
    }
    return [IO.Path]::GetFullPath($Path)
}

function Test-SamePath {
    param(
        [Parameter(Mandatory = $true)][string]$Left,
        [Parameter(Mandatory = $true)][string]$Right
    )
    return [string]::Equals(
        (Get-FullPath $Left).TrimEnd([char[]]@('\', '/')),
        (Get-FullPath $Right).TrimEnd([char[]]@('\', '/')),
        [StringComparison]::OrdinalIgnoreCase)
}

function Assert-NoReparseComponents {
    param([Parameter(Mandatory = $true)][string]$Path)

    $full = Get-FullPath $Path
    $root = [IO.Path]::GetPathRoot($full)
    if ([string]::IsNullOrWhiteSpace($root)) {
        throw "无法确定路径根: $Path"
    }
    # Keep the volume separator.  A trimmed `C:\` becomes `C:` and is a
    # drive-relative path in .NET/PowerShell.
    $current = $root
    $remainder = $full.Substring($root.Length)
    foreach ($part in ($remainder.Split([char[]]@('\', '/'), [StringSplitOptions]::RemoveEmptyEntries))) {
        $current = Join-Path $current $part
        try {
            # Get-Item -Force still returns a dangling link, while Test-Path
            # alone treats that path as absent and would miss the reparse.
            $item = Get-Item -LiteralPath $current -Force -ErrorAction Stop
        }
        catch [System.Management.Automation.ItemNotFoundException] {
            break
        }
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw "路径包含符号链接、junction 或重解析点，拒绝访问: $current"
        }
    }
}

function Assert-SafeInstallRoot {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [switch]$Create
    )

    $full = Get-FullPath $Path
    Assert-NoReparseComponents -Path $full
    $volumeRoot = [IO.Path]::GetPathRoot($full)
    if ([string]::IsNullOrWhiteSpace($volumeRoot) -or (Test-SamePath $full $volumeRoot)) {
        throw "拒绝把磁盘根目录作为安装根: $full"
    }
    $userProfile = [Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)
    if (-not [string]::IsNullOrWhiteSpace($userProfile) -and (Test-SamePath $full $userProfile)) {
        throw "拒绝把用户目录作为安装根: $full"
    }
    if (Test-Path -LiteralPath $full) {
        $item = Get-Item -LiteralPath $full -Force
        if (-not $item.PSIsContainer) {
            throw "安装根不是目录: $full"
        }
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw "安装根不能是符号链接、junction 或重解析点: $full"
        }
    } elseif ($Create) {
        [IO.Directory]::CreateDirectory($full) | Out-Null
        Assert-NoReparseComponents -Path $full
    }
    return $full
}

function Resolve-ExistingFile {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Label
    )
    Assert-NoReparseComponents -Path $Path
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "$Label 不存在: $Path"
    }
    $item = Get-Item -LiteralPath $Path -Force
    if ($item.PSIsContainer -or
        ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
        $item.Length -le 0) {
        throw "$Label 必须是非空普通文件: $Path"
    }
    return $item.FullName
}

function Get-RegularFileRecord {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Label,
        [switch]$AllowEmpty
    )

    Assert-NoReparseComponents -Path $Path
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "$Label 不存在: $Path"
    }
    $item = Get-Item -LiteralPath $Path -Force
    if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "$Label 不是普通文件: $Path"
    }
    if (-not $AllowEmpty -and $item.Length -le 0) {
        throw "$Label 不能为空: $Path"
    }
    return $item
}

function Assert-SafeRelativePath {
    param([Parameter(Mandatory = $true)][string]$RelativePath)

    if ([string]::IsNullOrWhiteSpace($RelativePath)) {
        throw '包内路径不能为空'
    }
    $normalized = $RelativePath.Replace('/', '\')
    if ([IO.Path]::IsPathRooted($normalized) -or
        $normalized.StartsWith('\', [StringComparison]::Ordinal) -or
        $normalized -match '^[A-Za-z]:') {
        throw "包内路径必须是相对路径: $RelativePath"
    }
    if ($normalized -match '\\{2,}') {
        throw "包内路径含有空目录段: $RelativePath"
    }
    $parts = $normalized.Split('\', [StringSplitOptions]::RemoveEmptyEntries)
    if ($parts | Where-Object { $_ -eq '..' -or $_ -eq '.' }) {
        throw "包内路径含有不安全的目录段: $RelativePath"
    }
    return ($normalized.Replace('\', '/'))
}

function Get-MarkdownLinkTargets {
    param([Parameter(Mandatory = $true)][string]$Path)
    $item = Get-RegularFileRecord -Path $Path -Label '性能报告'
    try {
        $text = [Text.UTF8Encoding]::new($false, $true).GetString([IO.File]::ReadAllBytes($item.FullName))
    }
    catch {
        throw "性能报告不是有效的 UTF-8 文本: $Path"
    }
    # Links in fenced examples are documentation text, not declared release assets.
    $text = [Text.RegularExpressions.Regex]::Replace(
        $text, '(?ms)^[ \t]{0,3}```.*?^[ \t]{0,3}```[ \t]*$', '')
    $targets = [Collections.Generic.List[string]]::new()
    $inlinePattern = '(?<!\!)\[[^\]\r\n]+\]\(\s*(?:<([^>\r\n]+)>|([^\s)\r\n]+))'
    foreach ($match in [Text.RegularExpressions.Regex]::Matches($text, $inlinePattern)) {
        $target = if ($match.Groups[1].Success) { $match.Groups[1].Value } else { $match.Groups[2].Value }
        if (-not [string]::IsNullOrWhiteSpace($target)) { [void]$targets.Add($target.Trim()) }
    }
    $referencePattern = '(?m)^[ \t]{0,3}\[[^\]\r\n]+\]:\s*(?:<([^>\r\n]+)>|([^\s\r\n]+))'
    foreach ($match in [Text.RegularExpressions.Regex]::Matches($text, $referencePattern)) {
        $target = if ($match.Groups[1].Success) { $match.Groups[1].Value } else { $match.Groups[2].Value }
        if (-not [string]::IsNullOrWhiteSpace($target)) { [void]$targets.Add($target.Trim()) }
    }
    return @($targets.ToArray())
}

function Get-ManagedPath {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$RelativePath
    )

    $safe = Assert-SafeRelativePath $RelativePath
    $fullRoot = Get-FullPath $Root
    $rootName = $fullRoot.TrimEnd([char[]]@('\', '/'))
    $candidate = [IO.Path]::GetFullPath((Join-Path $rootName ($safe.Replace('/', '\'))))
    $prefix = "$rootName\"
    if (-not $candidate.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "包内路径跳出了安装根: $RelativePath"
    }
    # This is deliberately checked for every path, including paths that do not
    # exist yet.  Existing junctions at any ancestor are rejected before a
    # directory or file is created below them.
    Assert-NoReparseComponents -Path $candidate
    return $candidate
}

function Assert-PathAvailable {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$RelativePath,
        [switch]$AllowExistingFile
    )
    $path = Get-ManagedPath -Root $Root -RelativePath $RelativePath
    if (Test-Path -LiteralPath $path) {
        $item = Get-Item -LiteralPath $path -Force
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw "目标不能是重解析点: $RelativePath"
        }
        if (-not $AllowExistingFile -or $item.PSIsContainer) {
            throw "目标路径已存在或不是普通文件: $RelativePath"
        }
    }
    $parent = Split-Path -Parent $path
    if ($parent) {
        Assert-NoReparseComponents -Path $parent
        if (Test-Path -LiteralPath $parent) {
            $parentItem = Get-Item -LiteralPath $parent -Force
            if (-not $parentItem.PSIsContainer -or
                ($parentItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
                throw "目标父目录不是安全目录: $parent"
            }
        }
    }
    return $path
}

function Assert-Sha256 {
    param(
        [Parameter(Mandatory = $true)][AllowNull()][string]$Hash,
        [Parameter(Mandatory = $true)][string]$Label
    )
    if ([string]::IsNullOrWhiteSpace($Hash) -or $Hash -notmatch '^[0-9A-Fa-f]{64}$') {
        throw "$Label 的 SHA-256 无效"
    }
    return $Hash.ToUpperInvariant()
}

function Get-Sha256 {
    param([Parameter(Mandatory = $true)][string]$Path)
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToUpperInvariant()
}

function Get-ByteSha256 {
    param([Parameter(Mandatory = $true)][byte[]]$Bytes)
    $algorithm = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($algorithm.ComputeHash($Bytes))).Replace('-', '').ToUpperInvariant()
    }
    finally {
        $algorithm.Dispose()
    }
}

function Convert-ToArray {
    param([AllowNull()][object]$Value)
    if ($null -eq $Value) { return @() }
    if ($Value -is [Array]) { return @($Value) }
    return @($Value)
}

function Expand-Values {
    param([AllowNull()][object]$Value)
    if ($null -eq $Value) { return }
    if ($Value -is [string]) { return $Value }
    if ($Value -is [System.Collections.IEnumerable]) {
        foreach ($item in $Value) { Expand-Values $item }
    } else {
        return $Value
    }
}

function Test-VersionString {
    param([Parameter(Mandatory = $true)][string]$Version)
    return [Text.RegularExpressions.Regex]::IsMatch(
        $Version,
        '^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z][0-9A-Za-z.-]*)?$')
}

function Read-JsonHashtable {
    param([Parameter(Mandatory = $true)][string]$Path)
    try {
        $value = Get-Content -LiteralPath $Path -Raw -Encoding UTF8 | ConvertFrom-BlueberryInstallJson
    }
    catch {
        throw "无法解析 JSON 文件 $Path：$($_.Exception.Message)"
    }
    if (-not ($value -is [Collections.IDictionary])) {
        throw "JSON 根对象必须是对象: $Path"
    }
    return $value
}

function Get-ManifestIntegrityHash {
    param([Parameter(Mandatory = $true)][Collections.IDictionary]$Manifest)
    $copy = [ordered]@{}
    foreach ($key in $Manifest.Keys) {
        if ([string]$key -eq 'metadata_sha256') {
            $copy[$key] = ''
        } else {
            $copy[$key] = $Manifest[$key]
        }
    }
    if (-not $copy.Contains('metadata_sha256')) { $copy['metadata_sha256'] = '' }
    $json = ConvertTo-BlueberryCanonicalJson $copy
    return Get-ByteSha256 -Bytes ([Text.UTF8Encoding]::new($false).GetBytes("$json`n"))
}

function Write-Utf8JsonAtomic {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$Value
    )
    $Value['metadata_sha256'] = ''
    $Value['metadata_sha256'] = Get-ManifestIntegrityHash -Manifest $Value
    $temporary = "$Path.new-$([Guid]::NewGuid().ToString('N'))"
    try {
        $parent = Split-Path -Parent $Path
        if ($parent) {
            Assert-NoReparseComponents -Path $parent
            [IO.Directory]::CreateDirectory($parent) | Out-Null
        }
        $json = $Value | ConvertTo-Json -Depth 50
        [IO.File]::WriteAllText($temporary, "$json`n", [Text.UTF8Encoding]::new($false))
        Move-BlueberryFile $temporary $Path
    }
    finally {
        if (Test-Path -LiteralPath $temporary) {
            Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
        }
    }
}

function Assert-PeX64 {
    param([Parameter(Mandatory = $true)][string]$Path)
    $item = Get-RegularFileRecord -Path $Path -Label 'Windows x64 exe'
    if ($item.Length -lt 0x40) { throw "exe 不是有效的 PE x64 文件: $Path" }
    $bytes = [IO.File]::ReadAllBytes($item.FullName)
    if ($bytes.Length -lt 0x40 -or $bytes[0] -ne 0x4d -or $bytes[1] -ne 0x5a) {
        throw "exe 缺少有效的 MZ 头，拒绝作为 Windows x64 产品: $Path"
    }
    $peOffset = [BitConverter]::ToInt32($bytes, 0x3c)
    if ($peOffset -lt 0 -or $peOffset -gt ($bytes.Length - 24) -or
        $bytes[$peOffset] -ne 0x50 -or $bytes[$peOffset + 1] -ne 0x45 -or
        $bytes[$peOffset + 2] -ne 0 -or $bytes[$peOffset + 3] -ne 0) {
        throw "exe 缺少有效的 PE 签名: $Path"
    }
    $machine = [BitConverter]::ToUInt16($bytes, $peOffset + 4)
    $sectionCount = [BitConverter]::ToUInt16($bytes, $peOffset + 6)
    $optionalSize = [BitConverter]::ToUInt16($bytes, $peOffset + 20)
    $optionalOffset = $peOffset + 24
    if ($machine -ne 0x8664 -or $sectionCount -le 0 -or
        $optionalSize -lt 2 -or $optionalOffset -gt ($bytes.Length - $optionalSize) -or
        [BitConverter]::ToUInt16($bytes, $optionalOffset) -ne 0x20b) {
        throw "exe 必须是 PE32+ Windows x64 (AMD64): $Path"
    }
    return $item.FullName
}

function Assert-VersionFile {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$ExpectedVersion
    )
    $item = Get-RegularFileRecord -Path $Path -Label 'VERSION.txt'
    $text = [Text.UTF8Encoding]::new($false, $true).GetString([IO.File]::ReadAllBytes($item.FullName))
    $match = [Text.RegularExpressions.Regex]::Match($text, '(?m)^\s*Blueberry\s+([^\s\r\n]+)\s*$')
    if (-not $match.Success -or $match.Groups[1].Value -ne $ExpectedVersion -or
        $text -notmatch '(?m)^\s*Platform:\s*windows-x64\s*$' -or
        $text -notmatch '(?m)^\s*Signature:\s*unsigned\s*$') {
        throw "VERSION.txt 与发布版本或平台不一致: $Path"
    }
}

function Assert-ExternalPackageHash {
    param([Parameter(Mandatory = $true)][string]$PackagePath)
    $shaPath = "$PackagePath.sha256"
    try { Get-Item -LiteralPath $shaPath -Force -ErrorAction Stop | Out-Null }
    catch [System.Management.Automation.ItemNotFoundException] { return }
    $shaItem = Get-RegularFileRecord -Path $shaPath -Label 'ZIP 外部 SHA-256 文件'
    $lines = @([IO.File]::ReadAllLines($shaItem.FullName, [Text.UTF8Encoding]::new($false, $false)) |
        Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    if ($lines.Count -ne 1) { throw "ZIP 外部 SHA-256 文件必须只有一条非空记录: $shaPath" }
    $parts = $lines[0].Trim() -split '\s+'
    if ($parts.Count -lt 2 -or $parts[0] -notmatch '^[0-9A-Fa-f]{64}$' -or
        [IO.Path]::GetFileName($parts[1]) -ine [IO.Path]::GetFileName($PackagePath)) {
        throw "ZIP 外部 SHA-256 文件格式或文件名无效: $shaPath"
    }
    $actual = Get-Sha256 $PackagePath
    if ($actual -ne $parts[0].ToUpperInvariant()) { throw "ZIP 外部 SHA-256 不匹配: $PackagePath" }
}

function Read-Package {
    param([Parameter(Mandatory = $true)][string]$Path)

    $packagePath = Resolve-ExistingFile -Path $Path -Label '本地发布包'
    if ([IO.Path]::GetExtension($packagePath) -ine '.zip') {
        throw "-PackagePath 必须是本地 .zip 文件: $packagePath"
    }
    Assert-ExternalPackageHash -PackagePath $packagePath
    $stage = Join-Path ([IO.Path]::GetTempPath()) "blueberry-package-$([Guid]::NewGuid().ToString('N'))"
    try {
        $zip = [IO.Compression.ZipFile]::OpenRead($packagePath)
        $zipFiles = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        try {
            foreach ($entry in $zip.Entries) {
                $isDirectory = $entry.FullName.EndsWith('/') -or $entry.FullName.EndsWith('\')
                $entryName = $entry.FullName.TrimEnd('/', '\')
                if ($entryName.Length -eq 0) { continue }
                $relative = Assert-SafeRelativePath $entryName
                if (-not $isDirectory -and -not $zipFiles.Add($relative)) {
                    throw "ZIP 内存在重复文件路径: $relative"
                }
            }
        }
        finally { $zip.Dispose() }
        [IO.Directory]::CreateDirectory($stage) | Out-Null
        Assert-NoReparseComponents -Path $stage
        Expand-Archive -LiteralPath $packagePath -DestinationPath $stage -Force
        foreach ($directory in @(Get-ChildItem -LiteralPath $stage -Directory -Recurse -Force)) {
            if ($directory.Attributes -band [IO.FileAttributes]::ReparsePoint) {
                throw "发布包目录不能是重解析点: $($directory.FullName)"
            }
        }
        foreach ($file in @(Get-ChildItem -LiteralPath $stage -File -Recurse -Force)) {
            if ($file.Attributes -band [IO.FileAttributes]::ReparsePoint) {
                throw "发布包文件不能是重解析点: $($file.FullName)"
            }
        }
        $releaseManifestPath = Join-Path $stage 'release.json'
        $releaseRecord = Get-RegularFileRecord -Path $releaseManifestPath -Label '发布包 release.json'
        $releaseManifest = Read-JsonHashtable $releaseManifestPath
        if ([int]$releaseManifest.schema_version -ne 1 -or
            [string]$releaseManifest.product -ne 'Blueberry' -or
            [string]$releaseManifest.platform -ne 'windows-x64' -or
            [string]$releaseManifest.architecture -ne 'x64') {
            throw '发布包 release.json 不是受支持的 Blueberry Windows x64 清单'
        }
        $version = [string]$releaseManifest.version
        if (-not (Test-VersionString $version)) { throw "发布包版本号无效: $version" }
        if ([bool]$releaseManifest.signed -or [string]$releaseManifest.signature_status -ne 'unsigned') {
            throw '发布包签名状态不符合 Beta 交付约定；当前只接受明确标注 unsigned 的包'
        }
        if (-not $releaseManifest.ContainsKey('files')) { throw '发布包缺少 files 完整性清单' }
        $files = [Collections.Generic.List[object]]::new()
        $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        foreach ($record in (Convert-ToArray $releaseManifest.files)) {
            if (-not ($record -is [Collections.IDictionary])) { throw '发布包 files 包含无效项' }
            $relative = Assert-SafeRelativePath ([string]$record.path)
            if ($relative -ieq 'release.json' -or $relative -ieq $script:ManifestFileName -or
                $relative.StartsWith('previous/', [StringComparison]::OrdinalIgnoreCase) -or
                $relative -ieq 'config.toml') {
                throw "发布包包含保留或配置路径: $relative"
            }
            if (-not $seen.Add($relative)) { throw "发布包 files 存在重复路径: $relative" }
            if (-not $record.ContainsKey('sha256') -or -not $record.ContainsKey('bytes')) {
                throw "发布包 files 缺少 SHA-256 或长度: $relative"
            }
            $expectedHash = Assert-Sha256 ([string]$record.sha256) "发布包文件 $relative"
            $expectedBytes = [int64]$record.bytes
            if ($expectedBytes -le 0) { throw "发布包文件长度必须为正数: $relative" }
            $filePath = Join-Path $stage ($relative.Replace('/', '\'))
            $item = Get-RegularFileRecord -Path $filePath -Label "发布包文件 $relative"
            if ($item.Length -ne $expectedBytes) { throw "发布包长度不匹配: $relative" }
            $actualHash = Get-Sha256 $filePath
            if ($actualHash -ne $expectedHash) { throw "发布包 SHA-256 不匹配: $relative" }
            $files.Add([pscustomobject]@{
                    path   = $relative
                    source = $filePath
                    sha256 = $actualHash
                    bytes  = $item.Length
                })
        }
        $executable = @($files | Where-Object { $_.path -ieq 'blueberry.exe' })
        if ($executable.Count -ne 1) { throw '发布包必须恰好包含 blueberry.exe' }
        Assert-PeX64 -Path $executable[0].source | Out-Null
        Assert-VersionFile -Path (Join-Path $stage 'VERSION.txt') -ExpectedVersion $version
        foreach ($required in $script:RequiredPackageFiles) {
            if (-not $seen.Contains($required)) { throw "发布包缺少必要资产: $required" }
            $requiredPath = Join-Path $stage ($required.Replace('/', '\'))
            $requiredItem = Get-RegularFileRecord -Path $requiredPath -Label "必要资产 $required"
            if ($requiredItem.Length -le 0) { throw "必要资产不能为空: $required" }
        }
        $stageFiles = @(Get-ChildItem -LiteralPath $stage -File -Recurse -Force)
        $allowed = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        [void]$allowed.Add('release.json')
        foreach ($record in $files) { [void]$allowed.Add($record.path) }
        foreach ($file in $stageFiles) {
            $relative = (Get-BlueberryRelativePath $stage $file.FullName).Replace('\', '/')
            if (-not $allowed.Contains($relative)) { throw "发布包包含未列入 release.json 的文件: $relative" }
        }
        foreach ($zipFile in $zipFiles) {
            if (-not $allowed.Contains($zipFile)) { throw "ZIP 包含未列入 release.json 的文件: $zipFile" }
        }
        [pscustomobject]@{
            root           = $stage
            manifest_path  = $releaseManifestPath
            manifest       = $releaseManifest
            release_sha256 = Get-Sha256 $releaseManifestPath
            release_bytes  = $releaseRecord.Length
            files          = $files
        }
    }
    catch {
        if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue }
        throw
    }
}

function Add-TransactionTouched {
    param(
        [Parameter(Mandatory = $true)]$Transaction,
        [Parameter(Mandatory = $true)][string]$RelativePath
    )
    [void]$Transaction.touched.Add((Assert-SafeRelativePath $RelativePath))
}

function Copy-FileAtomic {
    param(
        [Parameter(Mandatory = $true)][string]$Source,
        [Parameter(Mandatory = $true)][string]$Destination,
        [Parameter(Mandatory = $true)][string]$ExpectedHash,
        [Parameter(Mandatory = $true)][string]$RelativePath,
        $Transaction
    )
    $sourceItem = Get-RegularFileRecord -Path $Source -Label '复制源文件'
    $parent = Split-Path -Parent $Destination
    if ($parent) {
        Assert-NoReparseComponents -Path $parent
        [IO.Directory]::CreateDirectory($parent) | Out-Null
        Assert-NoReparseComponents -Path $parent
    }
    if ($null -ne $Transaction) { Add-TransactionTouched -Transaction $Transaction -RelativePath $RelativePath }
    $temporary = "$Destination.new-$([Guid]::NewGuid().ToString('N'))"
    try {
        Copy-Item -LiteralPath $sourceItem.FullName -Destination $temporary -Force
        $actual = Get-Sha256 $temporary
        if ($actual -ne $ExpectedHash.ToUpperInvariant()) { throw "复制后 SHA-256 不匹配: $RelativePath" }
        Move-BlueberryFile $temporary $Destination
    }
    finally {
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue }
    }
}

function Copy-BytesAtomic {
    param(
        [Parameter(Mandatory = $true)][byte[]]$Bytes,
        [Parameter(Mandatory = $true)][string]$Destination,
        [Parameter(Mandatory = $true)][string]$RelativePath,
        $Transaction
    )
    $parent = Split-Path -Parent $Destination
    if ($parent) {
        Assert-NoReparseComponents -Path $parent
        [IO.Directory]::CreateDirectory($parent) | Out-Null
    }
    if ($null -ne $Transaction) { Add-TransactionTouched -Transaction $Transaction -RelativePath $RelativePath }
    $temporary = "$Destination.new-$([Guid]::NewGuid().ToString('N'))"
    try {
        [IO.File]::WriteAllBytes($temporary, $Bytes)
        Move-BlueberryFile $temporary $Destination
    }
    finally {
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue }
    }
}

function Assert-PackageDestinations {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)]$Package,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][Collections.Generic.HashSet[string]]$AllowedExisting
    )
    foreach ($record in $Package.files) {
        $path = Get-ManagedPath -Root $Root -RelativePath $record.path
        if (Test-Path -LiteralPath $path) {
            if (-not $AllowedExisting.Contains($record.path)) {
                throw "安装根已有未由 Blueberry 管理的文件，拒绝覆盖: $($record.path)"
            }
            $item = Get-Item -LiteralPath $path -Force
            if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
                throw "安装目标不能是目录或重解析点: $($record.path)"
            }
        }
    }
    $releasePath = Get-ManagedPath -Root $Root -RelativePath 'release.json'
    if (Test-Path -LiteralPath $releasePath) {
        if (-not $AllowedExisting.Contains('release.json')) { throw '安装根已有未由 Blueberry 管理的 release.json，拒绝覆盖' }
        $releaseItem = Get-Item -LiteralPath $releasePath -Force
        if ($releaseItem.PSIsContainer -or ($releaseItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw 'release.json 不能是目录或重解析点'
        }
    }
}

function Copy-PackageFiles {
    param(
        [Parameter(Mandatory = $true)]$Package,
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][Collections.Generic.HashSet[string]]$AllowedExisting,
        [Parameter(Mandatory = $true)]$Transaction,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][Collections.Generic.List[string]]$Copied
    )
    foreach ($record in $Package.files) {
        $destination = Get-ManagedPath -Root $Root -RelativePath $record.path
        if (Test-Path -LiteralPath $destination) {
            if (-not $AllowedExisting.Contains($record.path)) { throw "安装根已有未由 Blueberry 管理的文件，拒绝覆盖: $($record.path)" }
            $destinationItem = Get-Item -LiteralPath $destination -Force
            if ($destinationItem.PSIsContainer -or ($destinationItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
                throw "安装目标不能是目录或重解析点: $($record.path)"
            }
        }
        Copy-FileAtomic -Source $record.source -Destination $destination -ExpectedHash $record.sha256 `
            -RelativePath $record.path -Transaction $Transaction
        if (-not $Copied.Contains($record.path)) { $Copied.Add($record.path) }
    }
    $releaseDestination = Get-ManagedPath -Root $Root -RelativePath 'release.json'
    if (Test-Path -LiteralPath $releaseDestination) {
        if (-not $AllowedExisting.Contains('release.json')) { throw '安装根已有未由 Blueberry 管理的 release.json，拒绝覆盖' }
        $releaseItem = Get-Item -LiteralPath $releaseDestination -Force
        if ($releaseItem.PSIsContainer -or ($releaseItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw 'release.json 不能是目录或重解析点'
        }
    }
    Copy-FileAtomic -Source $Package.manifest_path -Destination $releaseDestination -ExpectedHash $Package.release_sha256 `
        -RelativePath 'release.json' -Transaction $Transaction
    if (-not $Copied.Contains('release.json')) { $Copied.Add('release.json') }
}

function Read-InstallManifest {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [switch]$Required
    )
    $path = Get-ManagedPath -Root $Root -RelativePath $script:ManifestFileName
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        if ($Required) { throw "安装清单不存在，无法确认这是 Blueberry 安装根: $path" }
        return $null
    }
    $item = Get-RegularFileRecord -Path $path -Label '安装清单'
    $manifest = Read-JsonHashtable $path
    if ([int]$manifest.schema_version -ne $script:InstallManifestSchemaVersion -or
        [string]$manifest.product -ne 'Blueberry' -or
        [string]$manifest.platform -ne 'windows-x64' -or
        [bool]$manifest.signed -or [string]$manifest.signature_status -ne 'unsigned') {
        throw "不是受支持的 Blueberry Windows x64 安装清单: $path"
    }
    if (-not $manifest.ContainsKey('install_root') -or -not (Test-SamePath ([string]$manifest.install_root) $Root)) {
        throw "安装清单的 canonical install_root 与传入路径不一致: $path"
    }
    if (-not $manifest.ContainsKey('metadata_sha256') -or
        (Assert-Sha256 ([string]$manifest.metadata_sha256) '安装清单 metadata_sha256') -ne (Get-ManifestIntegrityHash $manifest)) {
        throw "安装清单元数据完整性校验失败，拒绝修改安装根: $path"
    }
    if (-not $manifest.ContainsKey('managed_files') -or -not $manifest.ContainsKey('managed_hashes')) {
        throw "安装清单缺少完整受管文件记录: $path"
    }
    $managed = [Collections.Generic.List[string]]::new()
    $managedSet = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($relativeValue in (Convert-ToArray $manifest.managed_files)) {
        $relative = Assert-SafeRelativePath ([string]$relativeValue)
        if (-not $managedSet.Add($relative)) { throw "安装清单 managed_files 存在重复路径: $relative" }
        $managed.Add($relative)
    }
    if (-not $managedSet.Contains($script:ManifestFileName)) { throw '安装清单没有把自身列入 managed_files' }
    $managedHashes = [Collections.Generic.Dictionary[string, string]]::new([StringComparer]::OrdinalIgnoreCase)
    if (-not ($manifest.managed_hashes -is [Collections.IDictionary])) { throw '安装清单 managed_hashes 必须是对象' }
    foreach ($key in $manifest.managed_hashes.Keys) {
        $relative = Assert-SafeRelativePath ([string]$key)
        if ($relative -ieq $script:ManifestFileName) { throw '安装清单不能为自身伪造受管文件 SHA-256；使用 metadata_sha256' }
        if (-not $managedSet.Contains($relative)) { throw "安装清单 managed_hashes 包含未列入 managed_files 的路径: $relative" }
        $managedHashes[$relative] = Assert-Sha256 ([string]$manifest.managed_hashes[$key]) "安装清单文件 $relative"
    }
    foreach ($relative in $managed) {
        if ($relative -ne $script:ManifestFileName -and -not $managedHashes.ContainsKey($relative)) {
            throw "安装清单缺少受管文件 SHA-256: $relative"
        }
    }
    if (-not $manifest.ContainsKey('current') -or -not ($manifest.current -is [Collections.IDictionary])) {
        throw '安装清单缺少 current'
    }
    $currentPath = Assert-SafeRelativePath ([string]$manifest.current.path)
    $currentVersion = [string]$manifest.current.version
    $currentHash = Assert-Sha256 ([string]$manifest.current.sha256) '安装清单 current.sha256'
    if ($currentPath -ne 'blueberry.exe' -or -not (Test-VersionString $currentVersion) -or
        -not $managedHashes.ContainsKey('blueberry.exe') -or $managedHashes['blueberry.exe'] -ne $currentHash) {
        throw '安装清单 current 与 blueberry.exe 受管哈希不一致'
    }
    $previous = [Collections.Generic.List[object]]::new()
    foreach ($entry in (Convert-ToArray $manifest.previous)) {
        if (-not ($entry -is [Collections.IDictionary])) { throw '安装清单 previous 包含无效项' }
        $snapshotRoot = Assert-SafeRelativePath ([string]$entry.snapshot_root)
        $snapshotManifest = Assert-SafeRelativePath ([string]$entry.snapshot_manifest)
        if (-not $snapshotRoot.StartsWith('previous/', [StringComparison]::OrdinalIgnoreCase) -or
            -not $snapshotManifest.StartsWith("$snapshotRoot/", [StringComparison]::OrdinalIgnoreCase)) {
            throw "安装清单 previous 快照路径无效: $snapshotRoot"
        }
        $entryVersion = [string]$entry.version
        $entryHash = Assert-Sha256 ([string]$entry.sha256) '安装清单 previous.sha256'
        if (-not (Test-VersionString $entryVersion) -or -not $entry.ContainsKey('files') -or
            -not $entry.ContainsKey('snapshot_manifest_sha256')) {
            throw "安装清单 previous 元数据不完整: $snapshotRoot"
        }
        $snapshotManifestHash = Assert-Sha256 ([string]$entry.snapshot_manifest_sha256) '安装清单快照元数据 SHA-256'
        $snapshotFiles = [Collections.Generic.List[object]]::new()
        $snapshotSeen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        foreach ($record in (Convert-ToArray $entry.files)) {
            if (-not ($record -is [Collections.IDictionary])) { throw "安装清单 previous 快照文件无效: $snapshotRoot" }
            $originalPath = Assert-SafeRelativePath ([string]$record.path)
            $snapshotPath = Assert-SafeRelativePath ([string]$record.snapshot_path)
            if ($originalPath -eq $script:ManifestFileName -or
                -not $snapshotPath.StartsWith("$snapshotRoot/files/", [StringComparison]::OrdinalIgnoreCase) -or
                -not $snapshotSeen.Add($originalPath)) {
                throw "安装清单 previous 快照文件路径无效: $originalPath"
            }
            $hash = Assert-Sha256 ([string]$record.sha256) "安装清单快照文件 $originalPath"
            $bytes = [int64]$record.bytes
            if ($bytes -le 0) { throw "安装清单快照文件长度无效: $originalPath" }
            if (-not $managedSet.Contains($snapshotPath) -or -not $managedHashes.ContainsKey($snapshotPath) -or
                $managedHashes[$snapshotPath] -ne $hash) {
                throw "安装清单 previous 快照没有完整列入 managed_hashes: $snapshotPath"
            }
            $snapshotFiles.Add([pscustomobject]@{
                    path = $originalPath; snapshot_path = $snapshotPath; sha256 = $hash; bytes = $bytes
                })
        }
        if (-not $snapshotSeen.Contains('blueberry.exe')) { throw "安装清单 previous 快照缺少 blueberry.exe: $snapshotRoot" }
        $exeRecord = $snapshotFiles | Where-Object { $_.path -ieq 'blueberry.exe' }
        $entryPath = Assert-SafeRelativePath ([string]$entry.path)
        if ($entryPath -ne $exeRecord.snapshot_path -or $entryHash -ne $exeRecord.sha256) {
            throw "安装清单 previous exe 元数据不一致: $snapshotRoot"
        }
        if (-not $managedSet.Contains($snapshotManifest) -or -not $managedHashes.ContainsKey($snapshotManifest) -or
            $managedHashes[$snapshotManifest] -ne $snapshotManifestHash) {
            throw "安装清单 previous 元数据快照没有纳入受管文件: $snapshotManifest"
        }
        $previous.Add([pscustomobject]@{
                path = $entryPath; version = $entryVersion; sha256 = $entryHash; snapshot_root = $snapshotRoot
                snapshot_manifest = $snapshotManifest; snapshot_manifest_sha256 = $snapshotManifestHash; files = $snapshotFiles
            })
    }
    [pscustomobject]@{
        path = $path; raw_bytes = [IO.File]::ReadAllBytes($item.FullName); manifest = $manifest
        managed_files = $managed; managed_hashes = $managedHashes
        current = [pscustomobject]@{ path = $currentPath; version = $currentVersion; sha256 = $currentHash }
        previous = $previous
    }
}

function Get-ActiveManagedFiles {
    param([Parameter(Mandatory = $true)]$State)
    return @($State.managed_files | Where-Object {
            $_ -ne $script:ManifestFileName -and -not $_.StartsWith('previous/', [StringComparison]::OrdinalIgnoreCase)
        })
}

function Assert-InstalledState {
    param([Parameter(Mandatory = $true)][string]$Root, [Parameter(Mandatory = $true)]$State)
    foreach ($relative in $State.managed_files) {
        if ($relative -eq $script:ManifestFileName) { continue }
        $path = Get-ManagedPath -Root $Root -RelativePath $relative
        $record = Get-RegularFileRecord -Path $path -Label "受管文件 $relative"
        if ((Get-Sha256 $record.FullName) -ne $State.managed_hashes[$relative]) {
            throw "受管文件 SHA-256 与安装清单不一致，拒绝修改: $relative"
        }
    }
    $currentPath = Get-ManagedPath -Root $Root -RelativePath 'blueberry.exe'
    Assert-PeX64 -Path $currentPath | Out-Null
    if ((Get-Sha256 $currentPath) -ne $State.current.sha256) { throw '当前 blueberry.exe SHA-256 与安装清单不一致，拒绝修改' }
    Assert-VersionFile -Path (Get-ManagedPath -Root $Root -RelativePath 'VERSION.txt') -ExpectedVersion $State.current.version
    foreach ($entry in $State.previous) {
        foreach ($record in $entry.files) {
            $snapshotPath = Get-ManagedPath -Root $Root -RelativePath $record.snapshot_path
            $item = Get-RegularFileRecord -Path $snapshotPath -Label "上一版快照 $($record.snapshot_path)"
            if ($item.Length -ne $record.bytes -or (Get-Sha256 $item.FullName) -ne $record.sha256) {
                throw "上一版快照 SHA-256 或长度与安装清单不一致: $($record.snapshot_path)"
            }
        }
        $metadataPath = Get-ManagedPath -Root $Root -RelativePath $entry.snapshot_manifest
        $metadataItem = Get-RegularFileRecord -Path $metadataPath -Label "上一版安装清单快照 $($entry.snapshot_manifest)"
        if ((Get-Sha256 $metadataItem.FullName) -ne $entry.snapshot_manifest_sha256) {
            throw "上一版安装清单快照 SHA-256 不一致: $($entry.snapshot_manifest)"
        }
    }
}

function New-Transaction {
    param([Parameter(Mandatory = $true)][string]$Root, [Parameter(Mandatory = $true)]$State)
    $transactionRoot = Join-Path ([IO.Path]::GetTempPath()) "blueberry-transaction-$([Guid]::NewGuid().ToString('N'))"
    $backupRoot = Join-Path $transactionRoot 'files'
    [IO.Directory]::CreateDirectory($backupRoot) | Out-Null
    $records = [Collections.Generic.List[object]]::new()
    try {
        foreach ($relative in $State.managed_files) {
            if ($relative -eq $script:ManifestFileName) { continue }
            $source = Get-ManagedPath -Root $Root -RelativePath $relative
            $sourceItem = Get-RegularFileRecord -Path $source -Label "事务备份 $relative"
            $expected = $State.managed_hashes[$relative]
            if ((Get-Sha256 $sourceItem.FullName) -ne $expected) { throw "事务备份前发现受管文件已变化: $relative" }
            $backup = Join-Path $backupRoot ($relative.Replace('/', '\'))
            [IO.Directory]::CreateDirectory((Split-Path -Parent $backup)) | Out-Null
            Copy-Item -LiteralPath $sourceItem.FullName -Destination $backup -Force
            if ((Get-Sha256 $backup) -ne $expected) { throw "事务备份 SHA-256 不一致: $relative" }
            $records.Add([pscustomobject]@{ path = $relative; source = $backup; sha256 = $expected; bytes = $sourceItem.Length })
        }
        $manifestBackup = Join-Path $transactionRoot 'install.json'
        Copy-Item -LiteralPath $State.path -Destination $manifestBackup -Force
        return [pscustomobject]@{
            root = $Root; directory = $transactionRoot; backup_root = $backupRoot; manifest = $manifestBackup
            records = $records; touched = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        }
    }
    catch {
        if (Test-Path -LiteralPath $transactionRoot) { Remove-Item -LiteralPath $transactionRoot -Recurse -Force -ErrorAction SilentlyContinue }
        throw
    }
}

function Restore-Transaction {
    param([Parameter(Mandatory = $true)]$Transaction)
    $errors = [Collections.Generic.List[string]]::new()
    $recordMap = @{}
    foreach ($record in $Transaction.records) { $recordMap[$record.path] = $record }
    foreach ($relative in @($Transaction.touched | Sort-Object Length -Descending)) {
        try {
            $path = Get-ManagedPath -Root $Transaction.root -RelativePath $relative
            $record = if ($recordMap.ContainsKey($relative)) { $recordMap[$relative] } else { $null }
            if ($null -ne $record -and (Test-Path -LiteralPath $path -PathType Leaf)) {
                $item = Get-Item -LiteralPath $path -Force
                if (-not ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -and
                    -not $item.PSIsContainer -and (Get-Sha256 $item.FullName) -eq $record.sha256) { continue }
            }
            if (Test-Path -LiteralPath $path) {
                $item = Get-Item -LiteralPath $path -Force
                if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "恢复目标是重解析点: $relative" }
                if ($item.PSIsContainer) { throw "恢复目标变成目录，拒绝递归删除: $relative" }
                Remove-Item -LiteralPath $path -Force
            }
        }
        catch { $errors.Add("删除变更路径 $relative 失败: $($_.Exception.Message)") }
    }
    foreach ($record in $Transaction.records) {
        try {
            $destination = Get-ManagedPath -Root $Transaction.root -RelativePath $record.path
            if (Test-Path -LiteralPath $destination -PathType Leaf) {
                $item = Get-Item -LiteralPath $destination -Force
                if (-not ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -and
                    (Get-Sha256 $item.FullName) -eq $record.sha256) { continue }
            }
            Copy-FileAtomic -Source $record.source -Destination $destination -ExpectedHash $record.sha256 -RelativePath $record.path
        }
        catch { $errors.Add("恢复受管文件 $($record.path) 失败: $($_.Exception.Message)") }
    }
    try {
        $manifestPath = Get-ManagedPath -Root $Transaction.root -RelativePath $script:ManifestFileName
        $oldBytes = [IO.File]::ReadAllBytes($Transaction.manifest)
        $same = $false
        if (Test-Path -LiteralPath $manifestPath -PathType Leaf) {
            $same = [Linq.Enumerable]::SequenceEqual($oldBytes, [IO.File]::ReadAllBytes($manifestPath))
        }
        if (-not $same) { Copy-BytesAtomic -Bytes $oldBytes -Destination $manifestPath -RelativePath $script:ManifestFileName }
    }
    catch { $errors.Add("恢复 install.json 失败: $($_.Exception.Message)") }
    if ($errors.Count -gt 0) { throw ('事务失败且自动恢复不完整: ' + ($errors -join '; ')) }
}

function New-SnapshotPlan {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)]$State,
        [Parameter(Mandatory = $true)]$Transaction
    )
    $id = [Guid]::NewGuid().ToString('N')
    $snapshotRoot = "previous/$id"
    [void](Assert-PathAvailable -Root $Root -RelativePath $snapshotRoot)
    $stageRoot = Join-Path $Transaction.directory "snapshot-$id"
    $stageFiles = Join-Path $stageRoot 'files'
    [IO.Directory]::CreateDirectory($stageFiles) | Out-Null
    $recordMap = @{}
    foreach ($record in $Transaction.records) { $recordMap[$record.path] = $record }
    $records = [Collections.Generic.List[object]]::new()
    foreach ($relative in (Get-ActiveManagedFiles -State $State)) {
        if (-not $recordMap.ContainsKey($relative)) { throw "事务备份缺少当前受管文件: $relative" }
        $old = $recordMap[$relative]
        $snapshotRelative = "$snapshotRoot/files/$relative"
        [void](Assert-PathAvailable -Root $Root -RelativePath $snapshotRelative)
        $stageDestination = Join-Path $stageFiles ($relative.Replace('/', '\'))
        [IO.Directory]::CreateDirectory((Split-Path -Parent $stageDestination)) | Out-Null
        Copy-Item -LiteralPath $old.source -Destination $stageDestination -Force
        if ((Get-Sha256 $stageDestination) -ne $old.sha256) { throw "上一版快照暂存 SHA-256 不一致: $relative" }
        $records.Add([pscustomobject]@{
                path = $relative; snapshot_path = $snapshotRelative; source = $stageDestination
                sha256 = $old.sha256; bytes = $old.bytes
            })
    }
    $snapshotManifest = "$snapshotRoot/install.json"
    $snapshotManifestPath = Assert-PathAvailable -Root $Root -RelativePath $snapshotManifest
    $stageManifest = Join-Path $stageRoot 'install.json'
    Copy-Item -LiteralPath $Transaction.manifest -Destination $stageManifest -Force
    $manifestBytes = [IO.File]::ReadAllBytes($stageManifest)
    return [pscustomobject]@{
        id = $id; snapshot_root = $snapshotRoot; snapshot_manifest = $snapshotManifest
        snapshot_manifest_path = $snapshotManifestPath; stage_root = $stageRoot; stage_manifest = $stageManifest
        records = $records; snapshot_manifest_sha256 = Get-ByteSha256 -Bytes $manifestBytes
    }
}

function Commit-SnapshotPlan {
    param(
        [Parameter(Mandatory = $true)]$Plan,
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)]$Transaction
    )
    foreach ($record in $Plan.records) {
        $destination = Get-ManagedPath -Root $Root -RelativePath $record.snapshot_path
        Copy-FileAtomic -Source $record.source -Destination $destination -ExpectedHash $record.sha256 `
            -RelativePath $record.snapshot_path -Transaction $Transaction
    }
    Copy-BytesAtomic -Bytes ([IO.File]::ReadAllBytes($Plan.stage_manifest)) `
        -Destination (Get-ManagedPath -Root $Root -RelativePath $Plan.snapshot_manifest) `
        -RelativePath $Plan.snapshot_manifest -Transaction $Transaction
    $snapshotFiles = @($Plan.records | ForEach-Object {
            [ordered]@{ path = $_.path; snapshot_path = $_.snapshot_path; sha256 = $_.sha256; bytes = $_.bytes }
        })
    $exe = $Plan.records | Where-Object { $_.path -ieq 'blueberry.exe' }
    return [ordered]@{
        path = $exe.snapshot_path; version = ''; sha256 = $exe.sha256; snapshot_root = $Plan.snapshot_root
        snapshot_manifest = $Plan.snapshot_manifest; snapshot_manifest_sha256 = $Plan.snapshot_manifest_sha256
        files = $snapshotFiles
    }
}

function New-ManagedHashMap {
    param([Parameter(Mandatory = $true)]$State)
    $map = [Collections.Generic.Dictionary[string, string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($relative in $State.managed_files) {
        if ($relative -ne $script:ManifestFileName -and $relative.StartsWith('previous/', [StringComparison]::OrdinalIgnoreCase)) {
            $map[$relative] = $State.managed_hashes[$relative]
        }
    }
    return $map
}

function Add-SnapshotToManagedHashMap {
    param([Parameter(Mandatory = $true)]$Map, [Parameter(Mandatory = $true)]$Snapshot)
    foreach ($record in $Snapshot.files) { $Map[$record.snapshot_path] = $record.sha256 }
    $Map[$Snapshot.snapshot_manifest] = $Snapshot.snapshot_manifest_sha256
}

function Add-PackageToManagedHashMap {
    param([Parameter(Mandatory = $true)]$Map, [Parameter(Mandatory = $true)]$Package)
    foreach ($record in $Package.files) { $Map[$record.path] = $record.sha256 }
    $Map['release.json'] = $Package.release_sha256
}

function Get-ManagedFileList {
    param([Parameter(Mandatory = $true)]$Map)
    $result = [Collections.Generic.List[string]]::new()
    foreach ($key in ($Map.Keys | Sort-Object)) { $result.Add([string]$key) }
    $result.Add($script:ManifestFileName)
    return @($result.ToArray())
}

function Remove-ObsoleteActiveFiles {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)]$State,
        [Parameter(Mandatory = $true)]$Package,
        [Parameter(Mandatory = $true)]$Transaction
    )
    $newPaths = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($record in $Package.files) { [void]$newPaths.Add($record.path) }
    [void]$newPaths.Add('release.json')
    foreach ($relative in (Get-ActiveManagedFiles -State $State)) {
        if ($newPaths.Contains($relative)) { continue }
        $path = Get-ManagedPath -Root $Root -RelativePath $relative
        Add-TransactionTouched -Transaction $Transaction -RelativePath $relative
        if (Test-Path -LiteralPath $path) {
            $item = Get-Item -LiteralPath $path -Force
            if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
                throw "无法删除已废弃的受管路径: $relative"
            }
            Remove-Item -LiteralPath $path -Force
        }
    }
}

function Invoke-Install {
    param([Parameter(Mandatory = $true)][string]$Root, [Parameter(Mandatory = $true)]$Package)
    $manifestPath = Get-ManagedPath -Root $Root -RelativePath $script:ManifestFileName
    if (Test-Path -LiteralPath $manifestPath) { throw '安装根已有 Blueberry 清单；使用 Upgrade 更新已有安装' }
    $emptyAllowed = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $transaction = [pscustomobject]@{ touched = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase) }
    $copied = [Collections.Generic.List[string]]::new()
    try {
        Assert-PackageDestinations -Root $Root -Package $Package -AllowedExisting $emptyAllowed
        Copy-PackageFiles -Package $Package -Root $Root -AllowedExisting $emptyAllowed -Transaction $transaction -Copied $copied
        $managedHashes = [Collections.Generic.Dictionary[string, string]]::new([StringComparer]::OrdinalIgnoreCase)
        Add-PackageToManagedHashMap -Map $managedHashes -Package $Package
        $manifest = New-InstallManifest -Root $Root -Package $Package -Previous @() -ManagedHashes $managedHashes `
            -ManagedFiles (Get-ManagedFileList -Map $managedHashes)
        Write-Utf8JsonAtomic -Path $manifestPath -Value $manifest
        [void]$transaction.touched.Add($script:ManifestFileName)
        $state = Read-InstallManifest -Root $Root -Required
        Assert-InstalledState -Root $Root -State $state
        Write-Host "已安装 Blueberry $($Package.manifest.version) 到 $Root（unsigned）"
    }
    catch {
        foreach ($relative in @($transaction.touched | Sort-Object Length -Descending)) {
            try {
                $path = Get-ManagedPath -Root $Root -RelativePath $relative
                if (Test-Path -LiteralPath $path) {
                    $item = Get-Item -LiteralPath $path -Force
                    if (-not ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -and -not $item.PSIsContainer) {
                        Remove-Item -LiteralPath $path -Force
                    }
                }
            }
            catch { Write-Warning "安装失败后的清理未完成 $relative：$($_.Exception.Message)" }
        }
        throw
    }
}

function New-InstallManifest {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)]$Package,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][object]$Previous,
        [Parameter(Mandatory = $true)]$ManagedHashes,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][object]$ManagedFiles,
        [string]$InstalledUtc
    )
    $executable = Get-ManagedPath -Root $Root -RelativePath 'blueberry.exe'
    $exeItem = Get-RegularFileRecord -Path $executable -Label '安装清单生成前 blueberry.exe'
    $currentHash = Get-Sha256 $exeItem.FullName
    Assert-PeX64 -Path $executable | Out-Null
    $managedSet = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($relative in (Expand-Values $ManagedFiles)) { [void]$managedSet.Add((Assert-SafeRelativePath $relative)) }
    foreach ($key in $ManagedHashes.Keys) { [void]$managedSet.Add((Assert-SafeRelativePath ([string]$key))) }
    [void]$managedSet.Add('blueberry.exe'); [void]$managedSet.Add('release.json'); [void]$managedSet.Add($script:ManifestFileName)
    $orderedHashes = [ordered]@{}
    foreach ($key in ($ManagedHashes.Keys | Sort-Object)) {
        $safe = Assert-SafeRelativePath ([string]$key)
        $orderedHashes[$safe] = Assert-Sha256 ([string]$ManagedHashes[$key]) "受管文件 $safe"
    }
    $orderedHashes['blueberry.exe'] = $currentHash
    $previousArray = @($Previous | ForEach-Object {
            $copy = [ordered]@{}
            if ($_ -is [Collections.IDictionary]) {
                foreach ($key in $_.Keys) { $copy[$key] = $_[$key] }
            } else {
                foreach ($property in $_.PSObject.Properties) { $copy[$property.Name] = $property.Value }
            }
            $copy['version'] = [string]$_.version; $copy['sha256'] = [string]$_.sha256; $copy
        })
    return [ordered]@{
        schema_version = $script:InstallManifestSchemaVersion; product = 'Blueberry'; install_root = (Get-FullPath $Root)
        platform = 'windows-x64'; signed = $false; signature_status = 'unsigned'
        current = [ordered]@{ path = 'blueberry.exe'; version = [string]$Package.manifest.version; sha256 = $currentHash }
        previous = $previousArray; managed_files = @($managedSet | Sort-Object); managed_hashes = $orderedHashes
        installed_utc = if ($InstalledUtc) { $InstalledUtc } else { [DateTime]::UtcNow.ToString('o') }
        updated_utc = [DateTime]::UtcNow.ToString('o'); metadata_sha256 = ''
    }
}

function New-ManifestForPayload {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$Version,
        [Parameter(Mandatory = $true)]$Previous,
        [Parameter(Mandatory = $true)]$ManagedHashes,
        [Parameter(Mandatory = $true)][string]$InstalledUtc
    )
    $packageLike = [pscustomobject]@{ manifest = [ordered]@{ version = $Version } }
    return New-InstallManifest -Root $Root -Package $packageLike -Previous $Previous -ManagedHashes $ManagedHashes `
        -ManagedFiles (Get-ManagedFileList -Map $ManagedHashes) -InstalledUtc $InstalledUtc
}

function Invoke-Upgrade {
    param([Parameter(Mandatory = $true)][string]$Root, [Parameter(Mandatory = $true)]$Package, [Parameter(Mandatory = $true)]$State)
    Assert-InstalledState -Root $Root -State $State
    $allowed = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($relative in $State.managed_files) { [void]$allowed.Add($relative) }
    Assert-PackageDestinations -Root $Root -Package $Package -AllowedExisting $allowed
    [void](Assert-PathAvailable -Root $Root -RelativePath "previous/$([Guid]::NewGuid().ToString('N'))")
    $transaction = New-Transaction -Root $Root -State $State
    try {
        $plan = New-SnapshotPlan -Root $Root -State $State -Transaction $transaction
        $snapshot = Commit-SnapshotPlan -Plan $plan -Root $Root -Transaction $transaction
        $snapshot.version = $State.current.version
        $previous = @($State.previous + @($snapshot))
        Remove-ObsoleteActiveFiles -Root $Root -State $State -Package $Package -Transaction $transaction
        $copied = [Collections.Generic.List[string]]::new()
        Copy-PackageFiles -Package $Package -Root $Root -AllowedExisting $allowed -Transaction $transaction -Copied $copied
        $managedHashes = New-ManagedHashMap -State $State
        Add-SnapshotToManagedHashMap -Map $managedHashes -Snapshot $snapshot
        Add-PackageToManagedHashMap -Map $managedHashes -Package $Package
        $manifest = New-ManifestForPayload -Root $Root -Version ([string]$Package.manifest.version) -Previous $previous `
            -ManagedHashes $managedHashes -InstalledUtc ([string]$State.manifest.installed_utc)
        Write-Utf8JsonAtomic -Path (Get-ManagedPath -Root $Root -RelativePath $script:ManifestFileName) -Value $manifest
        $newState = Read-InstallManifest -Root $Root -Required
        Assert-InstalledState -Root $Root -State $newState
        Write-Host "已从本地包升级 Blueberry $($Package.manifest.version)（unsigned）；上一版完整快照保存在 $($snapshot.snapshot_root)"
    }
    catch {
        $failure = $_.Exception
        try { Restore-Transaction -Transaction $transaction; Write-Warning '升级失败；已恢复升级前的全部受管文件和 install.json 元数据。' }
        catch { throw "升级失败且自动恢复不完整：$($failure.Message)；$($_.Exception.Message)" }
        throw $failure
    }
    finally {
        if (Test-Path -LiteralPath $transaction.directory) { Remove-Item -LiteralPath $transaction.directory -Recurse -Force -ErrorAction SilentlyContinue }
    }
}

function Invoke-Rollback {
    param([Parameter(Mandatory = $true)][string]$Root, [Parameter(Mandatory = $true)]$State)
    Assert-InstalledState -Root $Root -State $State
    $selected = $null
    for ($index = $State.previous.Count - 1; $index -ge 0; $index--) {
        $candidate = $State.previous[$index]
        $differs = $false
        foreach ($record in $candidate.files) {
            if (-not $State.managed_hashes.ContainsKey($record.path) -or
                $State.managed_hashes[$record.path] -ne $record.sha256) {
                $differs = $true
                break
            }
        }
        if ($differs) { $selected = $candidate; break }
    }
    if ($null -eq $selected) { throw '没有内容不同的上一版完整快照，无法回滚' }
    $selectedExe = $selected.files | Where-Object { $_.path -ieq 'blueberry.exe' }
    if ($null -eq $selectedExe) { throw '上一版完整快照缺少 blueberry.exe' }
    $allowed = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($relative in $State.managed_files) { [void]$allowed.Add($relative) }
    [void](Assert-PathAvailable -Root $Root -RelativePath "previous/$([Guid]::NewGuid().ToString('N'))")
    $transaction = New-Transaction -Root $Root -State $State
    try {
        $plan = New-SnapshotPlan -Root $Root -State $State -Transaction $transaction
        $snapshot = Commit-SnapshotPlan -Plan $plan -Root $Root -Transaction $transaction
        $snapshot.version = $State.current.version
        $recordMap = @{}
        foreach ($record in $transaction.records) { $recordMap[$record.path] = $record }
        $selectedPaths = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        foreach ($record in $selected.files) {
            [void]$selectedPaths.Add($record.path)
            $sourceRecord = $recordMap[$record.snapshot_path]
            if ($null -eq $sourceRecord) { throw "事务备份缺少回滚快照源: $($record.snapshot_path)" }
            Copy-FileAtomic -Source $sourceRecord.source -Destination (Get-ManagedPath -Root $Root -RelativePath $record.path) `
                -ExpectedHash $record.sha256 -RelativePath $record.path -Transaction $transaction
        }
        foreach ($relative in (Get-ActiveManagedFiles -State $State)) {
            if ($selectedPaths.Contains($relative)) { continue }
            $path = Get-ManagedPath -Root $Root -RelativePath $relative
            Add-TransactionTouched -Transaction $transaction -RelativePath $relative
            if (Test-Path -LiteralPath $path) {
                $item = Get-Item -LiteralPath $path -Force
                if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "无法删除回滚时多出的受管路径: $relative" }
                Remove-Item -LiteralPath $path -Force
            }
        }
        $previous = @($State.previous + @($snapshot))
        $managedHashes = New-ManagedHashMap -State $State
        Add-SnapshotToManagedHashMap -Map $managedHashes -Snapshot $snapshot
        foreach ($record in $selected.files) { $managedHashes[$record.path] = $record.sha256 }
        $manifest = New-ManifestForPayload -Root $Root -Version $selected.version -Previous $previous `
            -ManagedHashes $managedHashes -InstalledUtc ([string]$State.manifest.installed_utc)
        Write-Utf8JsonAtomic -Path (Get-ManagedPath -Root $Root -RelativePath $script:ManifestFileName) -Value $manifest
        $newState = Read-InstallManifest -Root $Root -Required
        Assert-InstalledState -Root $Root -State $newState
        Write-Host "已回滚到 Blueberry $($selected.version)（unsigned）；回滚前的完整受管文件和元数据保存在 $($snapshot.snapshot_root)"
    }
    catch {
        $failure = $_.Exception
        try { Restore-Transaction -Transaction $transaction; Write-Warning '回滚失败；已恢复回滚前的全部受管文件和 install.json 元数据。' }
        catch { throw "回滚失败且自动恢复不完整：$($failure.Message)；$($_.Exception.Message)" }
        throw $failure
    }
    finally {
        if (Test-Path -LiteralPath $transaction.directory) { Remove-Item -LiteralPath $transaction.directory -Recurse -Force -ErrorAction SilentlyContinue }
    }
}

function Invoke-Uninstall {
    param([Parameter(Mandatory = $true)][string]$Root, [Parameter(Mandatory = $true)]$State)
    $registrationPath = Join-Path $Root 'registration.json'
    if ([IO.File]::Exists($registrationPath)) {
        $registration = Read-JsonHashtable $registrationPath
        if (-not (Test-SamePath ([string]$registration.root) $Root)) { throw 'Installation registration root mismatch' }
        $exePath = Join-Path $Root 'blueberry.exe'
        if ([IO.File]::Exists($exePath) -and (Get-Sha256 $exePath) -eq $State.managed_hashes['blueberry.exe']) {
            & $exePath startup disable --owned-only
            if ($LASTEXITCODE -ne 0) { throw 'Could not remove owned startup hooks; installation preserved' }
        }
        if ($registration.path_added) {
            $currentPath = [Environment]::GetEnvironmentVariable('Path', 'User')
            $remainingPath = @($currentPath -split ';' | Where-Object { $_.TrimEnd('\') -ine $Root.TrimEnd('\') }) -join ';'
            [Environment]::SetEnvironmentVariable('Path', $remainingPath, 'User')
            $env:Path = @($env:Path -split ';' | Where-Object { $_.TrimEnd('\') -ine $Root.TrimEnd('\') }) -join ';'
        }
        [IO.File]::Delete($registrationPath)
    }
    $changed = [Collections.Generic.List[string]]::new()
    $missing = [Collections.Generic.List[string]]::new()
    $deleteErrors = [Collections.Generic.List[string]]::new()
    foreach ($relative in ($State.managed_files | Where-Object { $_ -ne $script:ManifestFileName } | Sort-Object Length -Descending)) {
        $path = Get-ManagedPath -Root $Root -RelativePath $relative
        if (-not (Test-Path -LiteralPath $path)) { $missing.Add($relative); continue }
        try {
            $item = Get-RegularFileRecord -Path $path -Label "受管文件 $relative"
            if ((Get-Sha256 $item.FullName) -ne $State.managed_hashes[$relative]) { $changed.Add($relative); continue }
            Remove-Item -LiteralPath $path -Force
        }
        catch {
            $changed.Add($relative); $deleteErrors.Add("$relative：$($_.Exception.Message)")
        }
    }
    $manifestPath = Get-ManagedPath -Root $Root -RelativePath $script:ManifestFileName
    if ($changed.Count -eq 0 -and $deleteErrors.Count -eq 0 -and (Test-Path -LiteralPath $manifestPath)) {
        try { Remove-Item -LiteralPath $manifestPath -Force }
        catch { $deleteErrors.Add("install.json：$($_.Exception.Message)") }
    } elseif ($changed.Count -gt 0) {
        Write-Warning '检测到受管文件内容已改变；这些文件已保留，install.json 也已保留以便再次核对。'
    }
    foreach ($directory in @((Join-Path $Root 'previous'), (Join-Path $Root 'docs'), (Join-Path $Root 'specs'))) {
        if (Test-Path -LiteralPath $directory -PathType Container) {
            $directoryItem = Get-Item -LiteralPath $directory -Force
            if ($directoryItem.Attributes -band [IO.FileAttributes]::ReparsePoint) { continue }
            if (@(Get-ChildItem -LiteralPath $directory -Force).Count -eq 0) { Remove-Item -LiteralPath $directory -Force }
        }
    }
    if ($changed.Count -gt 0 -or $deleteErrors.Count -gt 0) {
        foreach ($relative in $changed) { Write-Host "已保留变更的受管文件: $relative" }
        foreach ($message in $deleteErrors) { Write-Warning $message }
    }
    if (Test-Path -LiteralPath $Root -PathType Container) {
        $remaining = @(Get-ChildItem -LiteralPath $Root -Force)
        if ($remaining.Count -eq 0) {
            Remove-Item -LiteralPath $Root -Force
            Write-Host '已卸载 Blueberry；用户配置目录未被触碰。'
        } else {
            Write-Host "已删除完整性校验通过的 Blueberry 文件；安装根仍包含用户或保留文件，已保留: $Root"
            foreach ($relative in $missing) { Write-Host "受管文件原已缺失: $relative" }
        }
    }
}

function Test-JsoncMarkers {
    param([Parameter(Mandatory = $true)][string]$Text)
    # Scan outside JSON strings so that URLs and text such as "//" do not
    # trigger the JSONC path, while inline comments after a value do.
    $inString = $false
    $escaped = $false
    for ($index = 0; $index -lt $Text.Length; $index++) {
        $character = $Text[$index]
        if ($inString) {
            if ($escaped) { $escaped = $false }
            elseif ($character -eq '\') { $escaped = $true }
            elseif ($character -eq '"') { $inString = $false }
            continue
        }
        if ($character -eq '"') {
            $inString = $true
            continue
        }
        if ($character -eq '/' -and $index + 1 -lt $Text.Length -and
            ($Text[$index + 1] -eq '/' -or $Text[$index + 1] -eq '*')) {
            return $true
        }
    }
    # Keep the existing trailing-comma behavior for valid JSON strings that
    # do not contain a comment token.  A false positive here is safe because
    # ApplySettings refuses to rewrite the source when marked JSONC.
    if ($Text -match ',\s*[\]}]') { return $true }
    return $false
}

function Convert-JsoncForPreview {
    param([Parameter(Mandatory = $true)][string]$Text)
    # Remove comments only outside strings.  This keeps PreviewSettings useful
    # for inline JSONC comments while ApplySettings still refuses to rewrite
    # the original source.
    $withoutComments = [Text.StringBuilder]::new()
    $inString = $false
    $escaped = $false
    $index = 0
    while ($index -lt $Text.Length) {
        $character = $Text[$index]
        if ($inString) {
            [void]$withoutComments.Append($character)
            if ($escaped) { $escaped = $false }
            elseif ($character -eq '\') { $escaped = $true }
            elseif ($character -eq '"') { $inString = $false }
            $index++
            continue
        }
        if ($character -eq '"') {
            $inString = $true
            [void]$withoutComments.Append($character)
            $index++
            continue
        }
        if ($character -eq '/' -and $index + 1 -lt $Text.Length -and $Text[$index + 1] -eq '/') {
            $index += 2
            while ($index -lt $Text.Length -and $Text[$index] -ne "`r" -and $Text[$index] -ne "`n") { $index++ }
            continue
        }
        if ($character -eq '/' -and $index + 1 -lt $Text.Length -and $Text[$index + 1] -eq '*') {
            $index += 2
            while ($index -lt $Text.Length) {
                if ($index + 1 -lt $Text.Length -and $Text[$index] -eq '*' -and $Text[$index + 1] -eq '/') {
                    $index += 2
                    break
                }
                if ($Text[$index] -eq "`r" -or $Text[$index] -eq "`n") {
                    [void]$withoutComments.Append($Text[$index])
                }
                $index++
            }
            continue
        }
        [void]$withoutComments.Append($character)
        $index++
    }

    # Remove trailing commas outside strings without corrupting string values
    # that happen to contain `,}` or `,]`.
    $withoutTrailingCommas = [Text.StringBuilder]::new()
    $clean = $withoutComments.ToString()
    $inString = $false
    $escaped = $false
    for ($index = 0; $index -lt $clean.Length; $index++) {
        $character = $clean[$index]
        if ($inString) {
            [void]$withoutTrailingCommas.Append($character)
            if ($escaped) { $escaped = $false }
            elseif ($character -eq '\') { $escaped = $true }
            elseif ($character -eq '"') { $inString = $false }
            continue
        }
        if ($character -eq '"') {
            $inString = $true
            [void]$withoutTrailingCommas.Append($character)
            continue
        }
        if ($character -eq ',') {
            $lookahead = $index + 1
            while ($lookahead -lt $clean.Length -and [char]::IsWhiteSpace($clean[$lookahead])) { $lookahead++ }
            if ($lookahead -lt $clean.Length -and ($clean[$lookahead] -eq ']' -or $clean[$lookahead] -eq '}')) { continue }
        }
        [void]$withoutTrailingCommas.Append($character)
    }
    return $withoutTrailingCommas.ToString()
}

function Get-BlueberryProfile {
    param([Parameter(Mandatory = $true)][string]$Executable, [Parameter(Mandatory = $true)][string]$Name)
    $escaped = $Executable.Replace('"', '\"')
    return [ordered]@{ guid = $script:BlueberryProfileGuid; name = $Name; commandline = "`"$escaped`" run"; hidden = $false }
}

function Write-ManualProfile {
    param([Parameter(Mandatory = $true)][string]$Executable, [Parameter(Mandatory = $true)][string]$Name)
    $profile = Get-BlueberryProfile -Executable $Executable -Name $Name
    Write-Host ''
    Write-Host '无法安全改写带注释或尾逗号的 Windows Terminal JSONC。请手动合并以下 profile（原文件未修改）：'
    Write-Host ($profile | ConvertTo-Json -Depth 10)
}

function Read-SettingsPlan {
    param([Parameter(Mandatory = $true)][string]$Path, [Parameter(Mandatory = $true)][string]$Executable, [Parameter(Mandatory = $true)][string]$Name)
    $settingsPath = Resolve-ExistingFile -Path $Path -Label 'Windows Terminal settings'
    $bytes = [IO.File]::ReadAllBytes($settingsPath)
    $raw = [Text.UTF8Encoding]::new($false, $false).GetString($bytes)
    $isJsonc = Test-JsoncMarkers $raw
    $parseText = if ($isJsonc) { Convert-JsoncForPreview $raw } else { $raw }
    try { $settings = $parseText | ConvertFrom-BlueberryInstallJson }
    catch { Write-ManualProfile -Executable $Executable -Name $Name; throw "Windows Terminal settings 无法解析；原文件未修改: $($_.Exception.Message)" }
    if (-not ($settings -is [Collections.IDictionary]) -or -not $settings.ContainsKey('profiles') -or
        -not ($settings.profiles -is [Collections.IDictionary]) -or -not $settings.profiles.ContainsKey('list')) {
        Write-ManualProfile -Executable $Executable -Name $Name
        throw 'Windows Terminal settings 缺少 profiles.list；原文件未修改'
    }
    $profiles = Convert-ToArray $settings.profiles.list
    $newProfile = Get-BlueberryProfile -Executable $Executable -Name $Name
    $newList = [Collections.Generic.List[object]]::new(); $found = $false
    foreach ($profile in $profiles) {
        if ($profile -is [Collections.IDictionary] -and $profile.ContainsKey('guid') -and
            [string]::Equals([string]$profile.guid, $script:BlueberryProfileGuid, [StringComparison]::OrdinalIgnoreCase)) {
            $updated = [ordered]@{}; foreach ($key in $profile.Keys) { $updated[$key] = $profile[$key] }
            $updated['guid'] = $script:BlueberryProfileGuid; $updated['name'] = $Name
            $updated['commandline'] = $newProfile.commandline; $updated['hidden'] = $false
            $newList.Add($updated); $found = $true
        } else { $newList.Add($profile) }
    }
    if (-not $found) { $newList.Add($newProfile) }
    $settings.profiles.list = @($newList.ToArray())
    return [pscustomobject]@{
        path = $settingsPath; original = $bytes; raw = $raw; is_jsonc = $isJsonc
        before = $parseText | ConvertFrom-BlueberryInstallJson; after = $settings; executable = $Executable; name = $Name
    }
}

function Show-SettingsPreview {
    param([Parameter(Mandatory = $true)]$Plan)
    $beforeLines = (($Plan.before | ConvertTo-Json -Depth 30) -split "`r?`n")
    $afterLines = (($Plan.after | ConvertTo-Json -Depth 30) -split "`r?`n")
    $diff = Compare-Object -ReferenceObject $beforeLines -DifferenceObject $afterLines
    Write-Host "Windows Terminal profile 预览（$($Plan.path)）："
    if ($null -eq $diff) { Write-Host '  （无变化）' }
    else {
        foreach ($line in $diff) {
            $prefix = if ($line.SideIndicator -eq '=>') { '+' } else { '-' }
            Write-Host ("  {0} {1}" -f $prefix, $line.InputObject)
        }
    }
    if ($Plan.is_jsonc) { Write-Host '  检测到 JSONC 注释或尾逗号；ApplySettings 将拒绝改写并只输出手动合并 profile。' }
}

function Invoke-PreviewSettings {
    param([Parameter(Mandatory = $true)][string]$Path, [Parameter(Mandatory = $true)][string]$Executable, [Parameter(Mandatory = $true)][string]$Name)
    $plan = Read-SettingsPlan -Path $Path -Executable $Executable -Name $Name
    Show-SettingsPreview -Plan $plan
}

function Invoke-ApplySettings {
    param([Parameter(Mandatory = $true)][string]$Path, [Parameter(Mandatory = $true)][string]$Executable, [Parameter(Mandatory = $true)][string]$Name)
    $plan = Read-SettingsPlan -Path $Path -Executable $Executable -Name $Name
    Show-SettingsPreview -Plan $plan
    if ($plan.is_jsonc) {
        Write-ManualProfile -Executable $Executable -Name $Name
        throw 'ApplySettings 拒绝改写 JSONC；原文件未修改，请手动合并上面的 profile'
    }
    $currentBytes = [IO.File]::ReadAllBytes($plan.path)
    if ((Get-ByteSha256 $currentBytes) -ne (Get-ByteSha256 $plan.original)) { throw 'Preview 后 Windows Terminal settings 已变化；未创建备份或写入，请重试' }
    $stamp = [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ')
    $backupPath = "$($plan.path).$stamp.bak"
    [IO.File]::Copy($plan.path, $backupPath, $false)
    $temporary = "$($plan.path).new-$([Guid]::NewGuid().ToString('N'))"
    try {
        $json = $plan.after | ConvertTo-Json -Depth 30
        [IO.File]::WriteAllText($temporary, "$json`n", [Text.UTF8Encoding]::new($false))
        Move-BlueberryFile $temporary $plan.path
        Write-Host "已应用 Blueberry profile；原 settings 字节备份为 $backupPath"
    }
    finally {
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue }
    }
}

if ([string]::IsNullOrWhiteSpace($InstallRoot)) { $InstallRoot = Get-DefaultInstallRoot }
$root = Assert-SafeInstallRoot -Path $InstallRoot -Create:($Action -eq 'Install')
$package = $null
try {
    switch ($Action) {
        'Install' {
            if ([string]::IsNullOrWhiteSpace($PackagePath)) { throw 'Install 需要 -PackagePath 指向本地 ZIP' }
            $package = Read-Package -Path $PackagePath
            Invoke-Install -Root $root -Package $package
        }
        'Upgrade' {
            if ([string]::IsNullOrWhiteSpace($PackagePath)) { throw 'Upgrade 需要 -PackagePath 指向本地 ZIP' }
            $state = Read-InstallManifest -Root $root -Required
            $package = Read-Package -Path $PackagePath
            Invoke-Upgrade -Root $root -Package $package -State $state
        }
        'Rollback' {
            $state = Read-InstallManifest -Root $root -Required
            Invoke-Rollback -Root $root -State $state
        }
        'Uninstall' {
            $state = Read-InstallManifest -Root $root -Required
            Invoke-Uninstall -Root $root -State $state
        }
        'PreviewSettings' {
            if ([string]::IsNullOrWhiteSpace($SettingsPath)) { throw 'PreviewSettings 需要显式 -SettingsPath；脚本不会猜测 Windows Terminal 配置位置' }
            $executable = if ($ExePath) { Resolve-ExistingFile -Path $ExePath -Label 'Blueberry exe' } else { Resolve-ExistingFile -Path (Join-Path $root 'blueberry.exe') -Label 'Blueberry exe' }
            Invoke-PreviewSettings -Path $SettingsPath -Executable $executable -Name $ProfileName
        }
        'ApplySettings' {
            if ([string]::IsNullOrWhiteSpace($SettingsPath)) { throw 'ApplySettings 需要显式 -SettingsPath；脚本不会猜测 Windows Terminal 配置位置' }
            $executable = if ($ExePath) { Resolve-ExistingFile -Path $ExePath -Label 'Blueberry exe' } else { Resolve-ExistingFile -Path (Join-Path $root 'blueberry.exe') -Label 'Blueberry exe' }
            Invoke-ApplySettings -Path $SettingsPath -Executable $executable -Name $ProfileName
        }
    }
}
finally {
    if ($null -ne $package -and (Test-Path -LiteralPath $package.root)) { Remove-Item -LiteralPath $package.root -Recurse -Force -ErrorAction SilentlyContinue }
}
