[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ExePath,

    [Parameter(Mandatory = $true)]
    [string]$Version,

    [string]$OutputDirectory,

    [string]$LicenseNoticesPath,

    [string]$ReleaseNotesPath,

    [string]$PreviousExePath,

    [switch]$Force
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$requiredPayload = @(
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
    'specs/builtin/remote.toml',
    'manage-install.ps1',
    'install-common.ps1',
    'install.ps1'
)

function Get-FullPath {
    param([Parameter(Mandatory = $true)][string]$Path)
    if ([string]::IsNullOrWhiteSpace($Path)) { throw '路径不能为空' }
    return [IO.Path]::GetFullPath($Path)
}

function Assert-NoReparseComponents {
    param([Parameter(Mandatory = $true)][string]$Path)
    $full = Get-FullPath $Path
    $root = [IO.Path]::GetPathRoot($full)
    if ([string]::IsNullOrWhiteSpace($root)) { throw "无法确定路径根: $Path" }
    $current = $root
    $remainder = $full.Substring($root.Length)
    foreach ($part in ($remainder.Split([char[]]@('\', '/'), [StringSplitOptions]::RemoveEmptyEntries))) {
        $current = Join-Path $current $part
        try {
            # Get-Item -Force still returns a dangling link, while Test-Path
            # alone treats that path as absent and would miss the reparse.
            $item = Get-Item -LiteralPath $current -Force -ErrorAction Stop
        }
        catch [System.Management.Automation.ItemNotFoundException] { break }
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw "路径包含符号链接、junction 或重解析点，拒绝访问: $current"
        }
    }
}

function Get-RegularFile {
    param([Parameter(Mandatory = $true)][string]$Path, [Parameter(Mandatory = $true)][string]$Label)
    Assert-NoReparseComponents -Path $Path
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw "$Label 不存在: $Path" }
    $item = Get-Item -LiteralPath $Path -Force
    if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -or $item.Length -le 0) {
        throw "$Label 必须是非空普通文件: $Path"
    }
    return $item
}

function Resolve-ReleaseFile {
    param([Parameter(Mandatory = $true)][string]$Path, [Parameter(Mandatory = $true)][string]$Label)
    return (Get-RegularFile -Path $Path -Label $Label).FullName
}

function Assert-ReleaseVersion {
    param([Parameter(Mandatory = $true)][string]$Value)
    if (-not [Text.RegularExpressions.Regex]::IsMatch(
            $Value, '^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z][0-9A-Za-z.-]*)?$')) {
        throw "版本号无效（需要例如 0.5.0-beta.1）: $Value"
    }
}

function Get-Sha256 {
    param([Parameter(Mandatory = $true)][string]$Path)
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToUpperInvariant()
}

function Get-ByteSha256 {
    param([Parameter(Mandatory = $true)][byte[]]$Bytes)
    $algorithm = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($algorithm.ComputeHash($Bytes))).Replace('-', '').ToUpperInvariant() }
    finally { $algorithm.Dispose() }
}

function Write-Utf8NoBom {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Content
    )
    $parent = Split-Path -Parent $Path
    if ($parent) {
        Assert-NoReparseComponents -Path $parent
        [IO.Directory]::CreateDirectory($parent) | Out-Null
    }
    [IO.File]::WriteAllText($Path, $Content, [Text.UTF8Encoding]::new($false))
    [void](Get-RegularFile -Path $Path -Label '生成文件')
}

function Assert-SafeRelativePath {
    param([Parameter(Mandatory = $true)][string]$RelativePath)
    if ([string]::IsNullOrWhiteSpace($RelativePath)) { throw '发布文件路径不能为空' }
    $normalized = $RelativePath.Replace('/', '\')
    if ([IO.Path]::IsPathRooted($normalized) -or $normalized.StartsWith('\', [StringComparison]::Ordinal) -or
        $normalized -match '^[A-Za-z]:') { throw "发布文件路径必须是相对路径: $RelativePath" }
    if ($normalized -match '\\{2,}') { throw "发布文件路径含有空目录段: $RelativePath" }
    $parts = $normalized.Split('\', [StringSplitOptions]::RemoveEmptyEntries)
    if ($parts | Where-Object { $_ -eq '..' -or $_ -eq '.' }) { throw "发布文件路径含有不安全目录段: $RelativePath" }
    return $normalized.Replace('\', '/')
}

function Get-MarkdownLinkTargets {
    param([Parameter(Mandatory = $true)][string]$Path)
    $item = Get-RegularFile -Path $Path -Label '性能报告'
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

function Resolve-PerformanceArtifactFiles {
    param([Parameter(Mandatory = $true)][string]$ReportPath)
    $reportItem = Get-RegularFile -Path $ReportPath -Label '性能报告'
    $prefix = 'benchmarks/v0.5'
    $paths = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $files = [Collections.Generic.List[object]]::new()
    foreach ($target in (Get-MarkdownLinkTargets -Path $reportItem.FullName)) {
        $normalized = ([string]$target).Trim().Replace('\', '/')
        if ([string]::IsNullOrWhiteSpace($normalized)) { continue }
        if ($normalized -match '^(?:[A-Za-z][A-Za-z0-9+.-]*:|//)') { continue }
        while ($normalized.StartsWith('./', [StringComparison]::Ordinal)) { $normalized = $normalized.Substring(2) }
        $inScope = $normalized -ieq $prefix -or $normalized.StartsWith("$prefix/", [StringComparison]::OrdinalIgnoreCase)
        if (-not $inScope) { continue }
        if ($normalized -ieq $prefix) { throw "性能报告链接必须指向 benchmarks/v0.5 下的 .json 或 .md 文件: $target" }
        if ($normalized.Contains('?') -or $normalized.Contains('#')) {
            throw "性能原始文件链接不能含查询或片段: $target"
        }
        $safe = Assert-SafeRelativePath $normalized
        $extension = [IO.Path]::GetExtension($safe).ToLowerInvariant()
        if ($extension -ne '.json' -and $extension -ne '.md') {
            throw "性能原始文件只能是 .json 或 .md: $target"
        }
        $packagePath = Assert-SafeRelativePath ("docs/$safe")
        if (-not $paths.Add($packagePath)) { continue }
        $source = Join-Path (Split-Path -Parent $reportItem.FullName) ($safe.Replace('/', '\'))
        $sourceItem = Get-RegularFile -Path $source -Label "性能原始文件 $packagePath"
        [void]$files.Add([pscustomobject]@{ RelativePath = $packagePath; Source = $sourceItem.FullName })
    }
    return @($files | Sort-Object RelativePath)
}

function Assert-PeX64 {
    param([Parameter(Mandatory = $true)][string]$Path)
    $item = Get-RegularFile -Path $Path -Label 'Windows x64 exe'
    if ($item.Length -lt 0x40) { throw "exe 不是有效的 PE x64 文件: $Path" }
    $bytes = [IO.File]::ReadAllBytes($item.FullName)
    if ($bytes.Length -lt 0x40 -or $bytes[0] -ne 0x4d -or $bytes[1] -ne 0x5a) { throw "exe 缺少有效的 MZ 头: $Path" }
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
    if ($machine -ne 0x8664 -or $sectionCount -le 0 -or $optionalSize -lt 2 -or
        $optionalOffset -gt ($bytes.Length - $optionalSize) -or
        [BitConverter]::ToUInt16($bytes, $optionalOffset) -ne 0x20b) {
        throw "exe 必须是 PE32+ Windows x64 (AMD64): $Path"
    }
    return $item.FullName
}

function Invoke-VersionCheck {
    param([Parameter(Mandatory = $true)][string]$Path, [Parameter(Mandatory = $true)][string]$ExpectedVersion)
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $Path
    $startInfo.WorkingDirectory = Split-Path -Parent $Path
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    [void]$startInfo.ArgumentList.Add('--version')
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    try {
        if (-not $process.Start()) { throw "无法启动打包 exe: $Path" }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(30000)) {
            try { $process.Kill($true) } catch { }
            throw "打包 exe --version 超时: $Path"
        }
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        if ($process.ExitCode -ne 0) { throw "打包 exe --version 失败 (exit $($process.ExitCode)): $stderr" }
        $escaped = [Text.RegularExpressions.Regex]::Escape($ExpectedVersion)
        if ($stdout -notmatch "(?m)(?<![0-9A-Za-z])$escaped(?![0-9A-Za-z])") {
            throw "打包 exe --version 未报告 Version $ExpectedVersion；输出为: $stdout"
        }
    }
    finally { $process.Dispose() }
}

function Copy-ToStage {
    param(
        [Parameter(Mandatory = $true)][string]$Source,
        [Parameter(Mandatory = $true)][string]$Stage,
        [Parameter(Mandatory = $true)][string]$RelativePath
    )
    $safe = Assert-SafeRelativePath $RelativePath
    $destination = Join-Path $Stage ($safe.Replace('/', '\'))
    $sourceItem = Get-RegularFile -Path $Source -Label '发布输入文件'
    $parent = Split-Path -Parent $destination
    if ($parent) { Assert-NoReparseComponents -Path $parent; [IO.Directory]::CreateDirectory($parent) | Out-Null }
    if (Test-Path -LiteralPath $destination) { throw "发布暂存目标重复: $safe" }
    Copy-Item -LiteralPath $sourceItem.FullName -Destination $destination -Force
    if ((Get-RegularFile -Path $destination -Label "暂存文件 $safe").Length -le 0) { throw "必要资产不能为空: $safe" }
}

function Resolve-OptionalInput {
    param([string]$Requested, [string[]]$Candidates, [string]$Label)
    if (-not [string]::IsNullOrWhiteSpace($Requested)) { return Resolve-ReleaseFile -Path $Requested -Label $Label }
    foreach ($candidate in $Candidates) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { return Resolve-ReleaseFile -Path $candidate -Label $Label }
    }
    return $null
}

function Assert-ExistingShaPair {
    param([Parameter(Mandatory = $true)][string]$ZipPath, [Parameter(Mandatory = $true)][string]$ShaPath)
    try { Get-Item -LiteralPath $ShaPath -Force -ErrorAction Stop | Out-Null }
    catch [System.Management.Automation.ItemNotFoundException] { return }
    if (-not (Test-Path -LiteralPath $ZipPath -PathType Leaf)) { throw "已有外部 SHA-256 但缺少同名 ZIP，拒绝覆盖: $ShaPath" }
    $shaItem = Get-RegularFile -Path $ShaPath -Label '已有外部 SHA-256'
    $lines = @([IO.File]::ReadAllLines($shaItem.FullName, [Text.UTF8Encoding]::new($false, $false)) |
        Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    if ($lines.Count -ne 1) { throw "已有外部 SHA-256 必须只有一条非空记录: $ShaPath" }
    $parts = $lines[0].Trim() -split '\s+'
    if ($parts.Count -lt 2 -or $parts[0] -notmatch '^[0-9A-Fa-f]{64}$' -or
        [IO.Path]::GetFileName($parts[1]) -ine [IO.Path]::GetFileName($ZipPath)) {
        throw "已有外部 SHA-256 格式或文件名无效: $ShaPath"
    }
    if ((Get-Sha256 $ZipPath) -ne $parts[0].ToUpperInvariant()) { throw "已有外部 SHA-256 与 ZIP 不匹配: $ZipPath" }
}

function Replace-OutputSet {
    param([Parameter(Mandatory = $true)]$Entries, [switch]$Force)
    $backupRoot = Join-Path ([IO.Path]::GetTempPath()) "blueberry-release-backup-$([Guid]::NewGuid().ToString('N'))"
    $backups = [Collections.Generic.List[object]]::new()
    $attempted = [Collections.Generic.List[object]]::new()
    $preserveBackup = $false
    try {
        foreach ($entry in $Entries) {
            Assert-NoReparseComponents -Path $entry.target
            $targetExists = Test-Path -LiteralPath $entry.target
            if ($targetExists) {
                $targetItem = Get-Item -LiteralPath $entry.target -Force
                if ($targetItem.PSIsContainer -or ($targetItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
                    throw "发布输出目标不能是目录或重解析点: $($entry.target)"
                }
                if (-not $Force) { throw "输出文件已存在；使用 -Force 才能替换: $($entry.target)" }
                [IO.Directory]::CreateDirectory($backupRoot) | Out-Null
                $backup = Join-Path $backupRoot ([IO.Path]::GetFileName($entry.target))
                Copy-Item -LiteralPath $entry.target -Destination $backup -Force
                if ((Get-Sha256 $backup) -ne (Get-Sha256 $entry.target)) { throw "输出备份 SHA-256 不一致: $($entry.target)" }
                $backups.Add([pscustomobject]@{ target = $entry.target; path = $backup })
            }
            if (-not (Test-Path -LiteralPath $entry.stage -PathType Leaf)) { throw "输出暂存文件不存在: $($entry.stage)" }
        }
        foreach ($entry in $Entries) {
            $attempted.Add($entry)
            [IO.File]::Move($entry.stage, $entry.target, $true)
        }
        foreach ($entry in $Entries) {
            if ((Get-Sha256 $entry.target) -ne $entry.sha256) { throw "发布输出替换后 SHA-256 不一致: $($entry.target)" }
        }
    }
    catch {
        $failure = $_.Exception
        $restoreErrors = [Collections.Generic.List[string]]::new()
        foreach ($entry in @($attempted | Sort-Object { $_.target.Length } -Descending)) {
            try {
                if (Test-Path -LiteralPath $entry.target) {
                    $targetItem = Get-Item -LiteralPath $entry.target -Force
                    if ($targetItem.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw '替换目标变成重解析点' }
                    Remove-Item -LiteralPath $entry.target -Force
                }
            }
            catch { $restoreErrors.Add("删除新输出 $($entry.target) 失败: $($_.Exception.Message)") }
        }
        foreach ($backup in $backups) {
            try {
                [IO.File]::Move($backup.path, $backup.target, $true)
            }
            catch { $restoreErrors.Add("恢复旧输出 $($backup.target) 失败: $($_.Exception.Message)") }
        }
        if ($restoreErrors.Count -gt 0) {
            $preserveBackup = $true
            throw "发布输出替换失败且恢复不完整：$($failure.Message)；$($restoreErrors -join '; ')；恢复备份保留于 $backupRoot"
        }
        throw "发布输出替换失败；三件套已恢复：$($failure.Message)"
    }
    finally {
        if (-not $preserveBackup -and (Test-Path -LiteralPath $backupRoot)) {
            Remove-Item -LiteralPath $backupRoot -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Assert-ReleaseVersion $Version
$sourceExe = Resolve-ReleaseFile -Path $ExePath -Label '已构建的 Windows x64 exe'
if ([IO.Path]::GetExtension($sourceExe) -ine '.exe') { throw "-ExePath 必须指向 .exe；脚本不会自行构建二进制: $sourceExe" }
Assert-PeX64 -Path $sourceExe | Out-Null

if ([string]::IsNullOrWhiteSpace($OutputDirectory)) { $OutputDirectory = Join-Path $repoRoot 'dist\beta' }
$outputRoot = Get-FullPath $OutputDirectory
if ([string]::Equals($outputRoot.TrimEnd([char[]]@('\', '/')), $repoRoot.TrimEnd([char[]]@('\', '/')), [StringComparison]::OrdinalIgnoreCase)) {
    throw 'OutputDirectory 不能是仓库根目录'
}
Assert-NoReparseComponents -Path $outputRoot
[IO.Directory]::CreateDirectory($outputRoot) | Out-Null
Assert-NoReparseComponents -Path $outputRoot

$zipName = "blueberry-v$Version-windows-x64.zip"
$zipPath = Join-Path $outputRoot $zipName
$shaPath = "$zipPath.sha256"
$externalManifestPath = Join-Path $outputRoot "blueberry-v$Version-release.json"
Assert-ExistingShaPair -ZipPath $zipPath -ShaPath $shaPath

$previousSource = $null
$previousTarget = $null
if (-not [string]::IsNullOrWhiteSpace($PreviousExePath)) {
    $previousSource = Resolve-ReleaseFile -Path $PreviousExePath -Label '上一版 exe'
    if ([IO.Path]::GetExtension($previousSource) -ine '.exe') { throw "-PreviousExePath 必须指向 .exe: $previousSource" }
    Assert-PeX64 -Path $previousSource | Out-Null
    $previousDirectory = Join-Path $outputRoot 'previous'
    Assert-NoReparseComponents -Path $previousDirectory
    [IO.Directory]::CreateDirectory($previousDirectory) | Out-Null
    Assert-NoReparseComponents -Path $previousDirectory
    $previousTarget = Join-Path $previousDirectory "blueberry-v$Version-previous.exe"
    if (Test-Path -LiteralPath $previousTarget) {
        $previousItem = Get-Item -LiteralPath $previousTarget -Force
        if ($previousItem.PSIsContainer -or ($previousItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "上一版保存目标不能是目录或重解析点: $previousTarget"
        }
        if (-not $Force) { throw "上一版保存文件已存在；使用 -Force 才能替换: $previousTarget" }
    }
}

$stage = Join-Path $outputRoot ".release-stage-$([Guid]::NewGuid().ToString('N'))"
$stagePayload = Join-Path $stage 'payload'
$stageOutput = Join-Path $stage 'output'
$manifest = $null
$performanceArtifactFiles = @()
try {
    Assert-NoReparseComponents -Path $stage
    [IO.Directory]::CreateDirectory($stagePayload) | Out-Null
    [IO.Directory]::CreateDirectory($stageOutput) | Out-Null

    $requiredFiles = @(
        @{ Source = (Join-Path $repoRoot 'README.md'); Destination = 'README.md' },
        @{ Source = (Join-Path $repoRoot 'LICENSE'); Destination = 'LICENSE' },
        @{ Source = (Join-Path $repoRoot 'config.example.toml'); Destination = 'config.example.toml' },
        @{ Source = (Join-Path $repoRoot 'docs/installation.md'); Destination = 'docs/installation.md' },
        @{ Source = (Join-Path $repoRoot 'docs/releasing.md'); Destination = 'docs/releasing.md' },
        @{ Source = (Join-Path $repoRoot 'docs/specifications.md'); Destination = 'docs/specifications.md' },
        @{ Source = (Join-Path $repoRoot 'docs/powershell-adapter.md'); Destination = 'docs/powershell-adapter.md' },
        @{ Source = (Join-Path $repoRoot 'specs/tools.toml'); Destination = 'specs/tools.toml' },
        @{ Source = (Join-Path $repoRoot 'specs/builtin/_catalog.toml'); Destination = 'specs/builtin/_catalog.toml' },
        @{ Source = (Join-Path $repoRoot 'specs/builtin/core.toml'); Destination = 'specs/builtin/core.toml' },
        @{ Source = (Join-Path $repoRoot 'specs/builtin/vcs.toml'); Destination = 'specs/builtin/vcs.toml' },
        @{ Source = (Join-Path $repoRoot 'specs/builtin/rust.toml'); Destination = 'specs/builtin/rust.toml' },
        @{ Source = (Join-Path $repoRoot 'specs/builtin/python.toml'); Destination = 'specs/builtin/python.toml' },
        @{ Source = (Join-Path $repoRoot 'specs/builtin/javascript.toml'); Destination = 'specs/builtin/javascript.toml' },
        @{ Source = (Join-Path $repoRoot 'specs/builtin/build.toml'); Destination = 'specs/builtin/build.toml' },
        @{ Source = (Join-Path $repoRoot 'specs/builtin/containers.toml'); Destination = 'specs/builtin/containers.toml' },
        @{ Source = (Join-Path $repoRoot 'specs/builtin/remote.toml'); Destination = 'specs/builtin/remote.toml' },
        @{ Source = (Join-Path $repoRoot 'scripts/manage-install.ps1'); Destination = 'manage-install.ps1' },
        @{ Source = (Join-Path $repoRoot 'scripts/install-common.ps1'); Destination = 'install-common.ps1' },
        @{ Source = (Join-Path $repoRoot 'install.ps1'); Destination = 'install.ps1' }
    )
    foreach ($required in $requiredFiles) { Copy-ToStage -Source $required.Source -Stage $stagePayload -RelativePath $required.Destination }

    $noticeCandidates = @(
        (Join-Path $repoRoot 'THIRD-PARTY-NOTICES.txt'), (Join-Path $repoRoot 'docs\third-party-notices.txt'),
        (Join-Path $repoRoot 'THIRD-PARTY-NOTICES.md'), (Join-Path $repoRoot 'docs\third-party-notices.md')
    )
    $noticeSource = Resolve-OptionalInput -Requested $LicenseNoticesPath -Candidates $noticeCandidates -Label '依赖许可证说明'
    if (-not $noticeSource) { throw '缺少完整依赖许可证原文；请先生成并通过 -LicenseNoticesPath 传入 THIRD-PARTY-NOTICES.txt' }
    Copy-ToStage -Source $noticeSource -Stage $stagePayload -RelativePath 'THIRD-PARTY-NOTICES.txt'

    $notesCandidates = @((Join-Path $repoRoot 'CHANGELOG.md'), (Join-Path $repoRoot 'docs\release-notes.md'))
    $notesSource = Resolve-OptionalInput -Requested $ReleaseNotesPath -Candidates $notesCandidates -Label '版本说明'
    if ($notesSource) {
        if (-not $ReleaseNotesPath -and [IO.Path]::GetFileName($notesSource) -eq 'CHANGELOG.md') {
            $changelog = [IO.File]::ReadAllText($notesSource)
            $section = [regex]::Match($changelog, '(?ms)^## ' + [regex]::Escape($Version) + '\s*\r?\n(.*?)(?=^## |\z)')
            if (-not $section.Success) { throw "CHANGELOG.md lacks a section for $Version" }
            Write-Utf8NoBom -Path (Join-Path $stagePayload 'RELEASE-NOTES.md') -Content ("# Blueberry $Version`n`n" + $section.Groups[1].Value.Trim() + "`n")
        } else {
            Copy-ToStage -Source $notesSource -Stage $stagePayload -RelativePath 'RELEASE-NOTES.md'
        }
    } else {
        $notes = "# Blueberry $Version`n`n这是 Windows x64 的 Blueberry Beta 本地交付包。该构建未签名（unsigned）。`n"
        Write-Utf8NoBom -Path (Join-Path $stagePayload 'RELEASE-NOTES.md') -Content $notes
    }

    Copy-ToStage -Source $sourceExe -Stage $stagePayload -RelativePath 'blueberry.exe'
    Assert-PeX64 -Path (Join-Path $stagePayload 'blueberry.exe') | Out-Null
    $versionText = "Blueberry $Version`nPlatform: windows-x64`nSignature: unsigned`n"
    Write-Utf8NoBom -Path (Join-Path $stagePayload 'VERSION.txt') -Content $versionText
    Invoke-VersionCheck -Path (Join-Path $stagePayload 'blueberry.exe') -ExpectedVersion $Version

    foreach ($required in $requiredPayload) {
        $requiredPath = Join-Path $stagePayload ($required.Replace('/', '\'))
        $requiredItem = Get-RegularFile -Path $requiredPath -Label "必要资产 $required"
        if ($requiredItem.Length -le 0) { throw "必要资产不能为空: $required" }
    }
    $manifestFiles = [Collections.Generic.List[object]]::new()
    foreach ($file in @(Get-ChildItem -LiteralPath $stagePayload -File -Recurse -Force | Sort-Object FullName)) {
        $relative = [IO.Path]::GetRelativePath($stagePayload, $file.FullName).Replace('\', '/')
        if ($file.Attributes -band [IO.FileAttributes]::ReparsePoint -or $file.Length -le 0) { throw "发布资产必须是非空普通文件: $relative" }
        $manifestFiles.Add([ordered]@{ path = $relative; sha256 = Get-Sha256 $file.FullName; bytes = $file.Length })
    }
    $manifest = [ordered]@{
        schema_version = 1; product = 'Blueberry'; name = 'Blueberry'; version = $Version
        platform = 'windows-x64'; architecture = 'x64'; executable = 'blueberry.exe'
        signed = $false; signature_status = 'unsigned'; release_notes = 'RELEASE-NOTES.md'
        license_notices = 'THIRD-PARTY-NOTICES.txt'; license_source = 'provided'
        performance_artifacts = @($performanceArtifactFiles | ForEach-Object { $_.RelativePath })
        files = @($manifestFiles.ToArray())
    }
    $manifestJson = $manifest | ConvertTo-Json -Depth 20
    Write-Utf8NoBom -Path (Join-Path $stagePayload 'release.json') -Content ($manifestJson + "`n")
    $stageZip = Join-Path $stageOutput $zipName
    Compress-Archive -Path (Join-Path $stagePayload '*') -DestinationPath $stageZip -CompressionLevel Optimal
    $stageZipHash = Get-Sha256 $stageZip
    $stageSha = Join-Path $stageOutput ([IO.Path]::GetFileName($shaPath))
    $stageManifest = Join-Path $stageOutput ([IO.Path]::GetFileName($externalManifestPath))
    Write-Utf8NoBom -Path $stageSha -Content "$stageZipHash  $zipName`n"
    Write-Utf8NoBom -Path $stageManifest -Content ($manifestJson + "`n")

    $entries = [Collections.Generic.List[object]]::new()
    $entries.Add([pscustomobject]@{ stage = $stageZip; target = $zipPath; sha256 = $stageZipHash })
    $entries.Add([pscustomobject]@{ stage = $stageSha; target = $shaPath; sha256 = Get-Sha256 $stageSha })
    $entries.Add([pscustomobject]@{ stage = $stageManifest; target = $externalManifestPath; sha256 = Get-Sha256 $stageManifest })
    if ($previousSource) {
        $stagePrevious = Join-Path $stageOutput ([IO.Path]::GetFileName($previousTarget))
        Copy-Item -LiteralPath $previousSource -Destination $stagePrevious -Force
        $previousHash = Get-Sha256 $previousSource
        if ((Get-Sha256 $stagePrevious) -ne $previousHash) { throw '上一版 exe 暂存后 SHA-256 不一致' }
        $entries.Add([pscustomobject]@{ stage = $stagePrevious; target = $previousTarget; sha256 = $previousHash })
    }
    Replace-OutputSet -Entries $entries -Force:$Force
    [pscustomobject]@{
        package = $zipPath; sha256 = $shaPath; manifest = $externalManifestPath; version = $Version
        platform = 'windows-x64'; signature_status = 'unsigned'
    }
}
finally {
    if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue }
}
