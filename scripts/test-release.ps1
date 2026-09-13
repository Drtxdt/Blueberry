[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [Alias('ExePath')]
    [string]$BlueberryExecutable,

    [string]$PreviousExePath,

    [switch]$KeepTemp
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$scriptRoot = Split-Path -Parent $PSScriptRoot
$releaseScript = Join-Path $PSScriptRoot 'release.ps1'
$manageScript = Join-Path $PSScriptRoot 'manage-install.ps1'
$tempRoot = Join-Path ([IO.Path]::GetTempPath()) "blueberry-release-test-$([Guid]::NewGuid().ToString('N'))"
$outputOne = Join-Path $tempRoot 'release-one'
$outputTwo = Join-Path $tempRoot 'release-two'
$installRoot = Join-Path $tempRoot 'install-root'
$configPath = Join-Path $tempRoot 'user-config.toml'
$licenseNoticesPath = Join-Path $tempRoot 'THIRD-PARTY-NOTICES.txt'
$settingsPath = Join-Path $tempRoot 'settings.json'
$jsoncPath = Join-Path $tempRoot 'settings-jsonc.json'
$jsoncInlinePath = Join-Path $tempRoot 'settings-jsonc-inline.json'
$preservedBackupRoot = $null

function Assert-Test {
    param([Parameter(Mandatory = $true)][bool]$Condition, [Parameter(Mandatory = $true)][string]$Message)
    if (-not $Condition) { throw "发布脚本自测失败: $Message" }
}

function Write-TestBytes {
    param([Parameter(Mandatory = $true)][string]$Path, [Parameter(Mandatory = $true)][string]$Text)
    [IO.Directory]::CreateDirectory((Split-Path -Parent $Path)) | Out-Null
    [IO.File]::WriteAllBytes($Path, [Text.Encoding]::UTF8.GetBytes($Text))
}

function Get-FileHashUpper {
    param([Parameter(Mandatory = $true)][string]$Path)
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToUpperInvariant()
}

function Invoke-TestScript {
    param([Parameter(Mandatory = $true)][string]$Path, [Parameter(Mandatory = $true)][hashtable]$Parameters)
    & $Path @Parameters | Out-Host
    if (-not $?) { throw "脚本执行失败: $Path" }
}

function Expect-TestFailure {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][hashtable]$Parameters,
        [Parameter(Mandatory = $true)][string]$MessagePattern
    )
    $failed = $false
    $errorText = ''
    $oldErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Stop'
        $output = @(& $Path @Parameters 2>&1)
        foreach ($item in $output) {
            if ($item -is [Management.Automation.ErrorRecord]) {
                $errorText += ($item | Out-String)
            }
        }
    }
    catch {
        $errorText += ($_ | Out-String)
    }
    finally { $ErrorActionPreference = $oldErrorActionPreference }
    $failed = $errorText -match $MessagePattern
    if (-not $failed -and $errorText) { Write-Host "预期拒绝的实际错误: $errorText" }
    Assert-Test $failed "应拒绝：$MessagePattern"
}

function Capture-TestFailure {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][hashtable]$Parameters
    )
    $errorText = ''
    $oldErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Stop'
        [void](& $Path @Parameters 2>&1)
        throw "脚本本应失败但成功返回: $Path"
    }
    catch {
        $errorText += ($_ | Out-String)
    }
    finally { $ErrorActionPreference = $oldErrorActionPreference }
    return $errorText
}

function Get-ExecutableVersion {
    param([Parameter(Mandatory = $true)][string]$Path)
    $resolved = (Get-Item -LiteralPath $Path -Force).FullName
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $resolved
    $info.WorkingDirectory = Split-Path -Parent $resolved
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    [void]$info.ArgumentList.Add('--version')
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $info
    try {
        if (-not $process.Start()) { throw "无法启动真实 Blueberry exe: $resolved" }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(30000)) {
            try { $process.Kill($true) } catch { }
            throw "真实 Blueberry exe --version 超时: $resolved"
        }
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        if ($process.ExitCode -ne 0) { throw "真实 Blueberry exe --version 失败: $stderr" }
        $match = [Text.RegularExpressions.Regex]::Match(
            $stdout, '(?m)(?<![0-9A-Za-z])([0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z][0-9A-Za-z.-]*)?)(?![0-9A-Za-z])')
        if (-not $match.Success) { throw "无法从真实 exe --version 输出识别版本: $stdout" }
        return $match.Groups[1].Value
    }
    finally { $process.Dispose() }
}

function Assert-ManifestFilesIntact {
    param([Parameter(Mandatory = $true)][string]$Root)
    $manifest = Get-Content -LiteralPath (Join-Path $Root 'install.json') -Raw | ConvertFrom-Json
    foreach ($property in $manifest.managed_hashes.psobject.Properties) {
        $path = Join-Path $Root ([string]$property.Name).Replace('/', '\')
        Assert-Test (Test-Path -LiteralPath $path -PathType Leaf) "受管文件存在: $($property.Name)"
        Assert-Test ((Get-FileHashUpper $path) -eq ([string]$property.Value).ToUpperInvariant()) "受管文件哈希正确: $($property.Name)"
    }
}

function Assert-PerformanceArtifactsPackaged {
    param([Parameter(Mandatory = $true)][string]$PackagePath)
    $zip = [IO.Compression.ZipFile]::OpenRead($PackagePath)
    try {
        $manifestEntry = $zip.GetEntry('release.json')
        Assert-Test ($null -ne $manifestEntry) 'ZIP 包含 release.json'
        $reader = [IO.StreamReader]::new($manifestEntry.Open(), [Text.Encoding]::UTF8, $true)
        try { $manifest = $reader.ReadToEnd() | ConvertFrom-Json }
        finally { $reader.Dispose() }
        $performanceProperty = @($manifest.PSObject.Properties | Where-Object { $_.Name -eq 'performance_artifacts' })
        Assert-Test ($performanceProperty.Count -eq 1) 'release.json 声明 performance_artifacts'
        $declared = @($manifest.performance_artifacts | ForEach-Object { [string]$_ })
        $records = @{}
        foreach ($record in @($manifest.files)) { $records[[string]$record.path] = $record }
        $declaredSet = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        foreach ($path in $declared) {
            $extension = [IO.Path]::GetExtension($path).ToLowerInvariant()
            Assert-Test ($path.StartsWith('docs/benchmarks/v0.5/', [StringComparison]::OrdinalIgnoreCase) -and
                ($extension -eq '.json' -or $extension -eq '.md')) "性能原始文件路径受限: $path"
            Assert-Test $declaredSet.Add($path) "性能原始文件无重复声明: $path"
            $entry = $zip.GetEntry($path)
            Assert-Test ($null -ne $entry -and -not $entry.FullName.EndsWith('/')) "ZIP 包含性能原始文件: $path"
            Assert-Test $records.ContainsKey($path) "性能原始文件具有清单记录: $path"
            Assert-Test ([int64]$records[$path].bytes -eq $entry.Length) "性能原始文件长度沿用清单: $path"
            $stream = $entry.Open()
            $algorithm = [Security.Cryptography.SHA256]::Create()
            try { $hash = ([BitConverter]::ToString($algorithm.ComputeHash($stream))).Replace('-', '').ToUpperInvariant() }
            finally { $algorithm.Dispose(); $stream.Dispose() }
            Assert-Test ($hash -eq ([string]$records[$path].sha256).ToUpperInvariant()) "性能原始文件哈希沿用清单: $path"
        }
        Assert-Test ($declared.Count -eq 0) 'Public package excludes historical benchmark artifacts'

    }
    finally { $zip.Dispose() }
}

try {
    [IO.Directory]::CreateDirectory($tempRoot) | Out-Null
    $currentExe = (Get-Item -LiteralPath $BlueberryExecutable -Force).FullName
    $hasPrevious = -not [string]::IsNullOrWhiteSpace($PreviousExePath)
    if (-not $hasPrevious) {
        $baselineCandidates = @(
            (Join-Path $scriptRoot 'artifacts\blueberry-v0.2-baseline.exe'),
            (Join-Path $scriptRoot 'artifacts\blueberry-v0.4-frozen.exe')
        )
        foreach ($candidate in $baselineCandidates) {
            if (Test-Path -LiteralPath $candidate -PathType Leaf) {
                $PreviousExePath = $candidate
                $hasPrevious = $true
                break
            }
        }
    }
    $firstExe = if (-not $hasPrevious) { $currentExe } else { (Get-Item -LiteralPath $PreviousExePath -Force).FullName }
    $firstVersion = Get-ExecutableVersion -Path $firstExe
    $currentVersion = Get-ExecutableVersion -Path $currentExe
    Write-TestBytes -Path $configPath -Text "[completion]`nmax_results = 7`n"
    Write-TestBytes -Path $licenseNoticesPath -Text "Test dependency license notice.`n"
    $configBytesBefore = [IO.File]::ReadAllBytes($configPath)
    $firstReleaseNotesPath = $null
    if (-not $hasPrevious) {
        # Make the first package observably different when the caller supplies
        # only one real executable, so rollback still exercises a payload
        # change without manufacturing a fake product binary.
        $firstReleaseNotesPath = Join-Path $tempRoot 'first-release-notes.md'
        Write-TestBytes -Path $firstReleaseNotesPath -Text "Self-test first package.`n"
    }

    $releaseOneParameters = @{
        ExePath = $firstExe; Version = $firstVersion; OutputDirectory = $outputOne; LicenseNoticesPath = $licenseNoticesPath
    }
    if ($firstReleaseNotesPath) { $releaseOneParameters['ReleaseNotesPath'] = $firstReleaseNotesPath }
    Invoke-TestScript -Path $releaseScript -Parameters $releaseOneParameters
    $packageOne = Join-Path $outputOne "blueberry-v$firstVersion-windows-x64.zip"
    Assert-Test (Test-Path -LiteralPath $packageOne -PathType Leaf) '第一版 ZIP 已生成'
    Assert-Test (Test-Path -LiteralPath "$packageOne.sha256" -PathType Leaf) '第一版外部 SHA-256 已生成'
    Assert-PerformanceArtifactsPackaged -PackagePath $packageOne

    $releaseTwoParameters = @{
        ExePath = $currentExe; Version = $currentVersion; OutputDirectory = $outputTwo; LicenseNoticesPath = $licenseNoticesPath
    }
    if ($hasPrevious) { $releaseTwoParameters['PreviousExePath'] = $firstExe }
    Invoke-TestScript -Path $releaseScript -Parameters $releaseTwoParameters
    $packageTwo = Join-Path $outputTwo "blueberry-v$currentVersion-windows-x64.zip"
    Assert-Test (Test-Path -LiteralPath $packageTwo -PathType Leaf) '第二版 ZIP 已生成'
    Assert-PerformanceArtifactsPackaged -PackagePath $packageTwo

    Invoke-TestScript -Path $manageScript -Parameters @{ Action = 'Install'; PackagePath = $packageOne; InstallRoot = $installRoot }
    Assert-Test ((Get-Content -LiteralPath (Join-Path $installRoot 'VERSION.txt') -Raw) -match [regex]::Escape($firstVersion)) '安装版本正确'
    Assert-Test ([Linq.Enumerable]::SequenceEqual($configBytesBefore, [IO.File]::ReadAllBytes($configPath))) '安装未修改用户配置'
    Write-TestBytes -Path (Join-Path $installRoot 'user-data.txt') -Text 'keep this user file'

    $manifestBeforeFailure = [IO.File]::ReadAllBytes((Join-Path $installRoot 'install.json'))
    $exeBeforeFailure = Get-FileHashUpper (Join-Path $installRoot 'blueberry.exe')
    $lock = [IO.File]::Open((Join-Path $installRoot 'blueberry.exe'), [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        Expect-TestFailure -Path $manageScript -Parameters @{ Action = 'Upgrade'; PackagePath = $packageTwo; InstallRoot = $installRoot } -MessagePattern '升级失败|恢复|占用|access|used'
    }
    finally { $lock.Dispose() }
    Assert-Test ($exeBeforeFailure -eq (Get-FileHashUpper (Join-Path $installRoot 'blueberry.exe'))) '失败升级恢复当前 exe'
    Assert-Test ([Linq.Enumerable]::SequenceEqual($manifestBeforeFailure, [IO.File]::ReadAllBytes((Join-Path $installRoot 'install.json')))) '失败升级恢复 install.json 元数据'

    Invoke-TestScript -Path $manageScript -Parameters @{ Action = 'Upgrade'; PackagePath = $packageTwo; InstallRoot = $installRoot }
    $upgradedManifest = Get-Content -LiteralPath (Join-Path $installRoot 'install.json') -Raw | ConvertFrom-Json
    Assert-Test ([int]$upgradedManifest.previous.Count -ge 1) '升级记录上一版快照'
    Assert-Test (@($upgradedManifest.previous[0].files).Count -ge 1) '上一版快照包含整套受管文件'
    Assert-Test (-not [string]::IsNullOrWhiteSpace([string]$upgradedManifest.previous[0].snapshot_manifest)) '上一版快照包含元数据'
    Assert-ManifestFilesIntact -Root $installRoot

    Invoke-TestScript -Path $manageScript -Parameters @{ Action = 'Rollback'; InstallRoot = $installRoot }
    Assert-Test ((Get-Content -LiteralPath (Join-Path $installRoot 'VERSION.txt') -Raw) -match [regex]::Escape($firstVersion)) '回滚 VERSION 正确'
    $rollbackManifest = Get-Content -LiteralPath (Join-Path $installRoot 'install.json') -Raw | ConvertFrom-Json
    Assert-Test ([string]$rollbackManifest.current.version -eq $firstVersion) '回滚清单 current 元数据正确'
    Assert-Test ((Get-FileHashUpper (Join-Path $installRoot 'blueberry.exe')) -eq (Get-FileHashUpper $firstExe)) '回滚验证当前 exe'
    Assert-ManifestFilesIntact -Root $installRoot

    $metadataBefore = [IO.File]::ReadAllBytes((Join-Path $installRoot 'install.json'))
    $tamperedManifest = Get-Content -LiteralPath (Join-Path $installRoot 'install.json') -Raw | ConvertFrom-Json
    $tamperedManifest.updated_utc = 'tampered'
    $tamperedManifest | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath (Join-Path $installRoot 'install.json') -Encoding utf8NoBOM
    Expect-TestFailure -Path $manageScript -Parameters @{ Action = 'Rollback'; InstallRoot = $installRoot } -MessagePattern '元数据|完整性|manifest|清单'
    [IO.File]::WriteAllBytes((Join-Path $installRoot 'install.json'), $metadataBefore)

    $settings = [ordered]@{
        '$schema' = 'https://aka.ms/terminal-documentation'
        profiles = [ordered]@{
            defaults = [ordered]@{ colorScheme = 'Campbell' }
            list = @([ordered]@{ guid = '{OTHER}'; name = 'PowerShell'; commandline = 'pwsh.exe' })
        }
        schemes = @([ordered]@{ name = 'Campbell'; background = '#0C0C0C' })
    }
    $settings | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $settingsPath -Encoding utf8NoBOM
    Invoke-TestScript -Path $manageScript -Parameters @{ Action = 'PreviewSettings'; SettingsPath = $settingsPath; InstallRoot = $installRoot }
    Invoke-TestScript -Path $manageScript -Parameters @{ Action = 'ApplySettings'; SettingsPath = $settingsPath; InstallRoot = $installRoot }
    $updatedSettings = Get-Content -LiteralPath $settingsPath -Raw | ConvertFrom-Json
    Assert-Test (@($updatedSettings.profiles.list | Where-Object { $_.guid -eq '{28E40931-78EB-4F61-8932-DAB5CB555972}' }).Count -eq 1) '应用 Blueberry profile'
    Assert-Test (@($updatedSettings.profiles.list | Where-Object { $_.guid -eq '{OTHER}' }).Count -eq 1) '保留其他 profile'
    Assert-Test (@(Get-ChildItem -LiteralPath $tempRoot -Filter 'settings.json.*.bak').Count -ge 1) 'Apply 创建原字节备份'

    $jsoncText = @'
{
  // Windows Terminal JSONC comment
  "profiles": {
    "list": [
      { "guid": "{OTHER}", "name": "PowerShell", "commandline": "pwsh.exe" },
    ],
  },
}
'@
    Set-Content -LiteralPath $jsoncPath -Value $jsoncText -Encoding utf8NoBOM
    $jsoncBefore = [IO.File]::ReadAllBytes($jsoncPath)
    Expect-TestFailure -Path $manageScript -Parameters @{ Action = 'ApplySettings'; SettingsPath = $jsoncPath; InstallRoot = $installRoot } -MessagePattern 'JSONC|手动合并'
    Assert-Test ([Linq.Enumerable]::SequenceEqual($jsoncBefore, [IO.File]::ReadAllBytes($jsoncPath))) 'JSONC 原文件未修改'

    $jsoncInlineText = @'
{
  "profiles": {
    "list": [
      { "guid": "{OTHER}", "name": "PowerShell", "commandline": "pwsh.exe" } // 行尾 JSONC 注释
    ]
  },
  "schemes": {}
}
'@
    Set-Content -LiteralPath $jsoncInlinePath -Value $jsoncInlineText -Encoding utf8NoBOM
    $jsoncInlineBefore = [IO.File]::ReadAllBytes($jsoncInlinePath)
    Expect-TestFailure -Path $manageScript -Parameters @{ Action = 'ApplySettings'; SettingsPath = $jsoncInlinePath; InstallRoot = $installRoot } -MessagePattern 'JSONC|手动合并'
    Assert-Test ([Linq.Enumerable]::SequenceEqual($jsoncInlineBefore, [IO.File]::ReadAllBytes($jsoncInlinePath))) 'JSONC 行尾注释原文件未修改'

    $danglingReleaseRoot = Join-Path $tempRoot 'release-dangling'
    $danglingPreviousTarget = Join-Path $tempRoot 'release-dangling-target-does-not-exist'
    [IO.Directory]::CreateDirectory($danglingReleaseRoot) | Out-Null
    [IO.Directory]::CreateDirectory($danglingPreviousTarget) | Out-Null
    New-Item -ItemType Junction -Path (Join-Path $danglingReleaseRoot 'previous') -Target $danglingPreviousTarget | Out-Null
    Remove-Item -LiteralPath $danglingPreviousTarget -Force
    Expect-TestFailure -Path $releaseScript -Parameters @{
        ExePath = $currentExe; Version = $currentVersion; OutputDirectory = $danglingReleaseRoot; PreviousExePath = $firstExe
    } -MessagePattern '重解析|junction|符号链接'
    $danglingPreviousItem = Get-Item -LiteralPath (Join-Path $danglingReleaseRoot 'previous') -Force -ErrorAction Stop
    Assert-Test (($danglingPreviousItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) '悬空 previous junction 仍未被替换'
    Assert-Test (-not (Test-Path -LiteralPath $danglingPreviousTarget)) '悬空 junction 目标未被发布脚本创建'

    $releaseZipBeforeFailure = Get-FileHashUpper -Path $packageTwo
    $releaseShaBeforeFailure = Get-FileHashUpper -Path "$packageTwo.sha256"
    $releaseManifestBeforeFailure = Get-FileHashUpper -Path (Join-Path $outputTwo "blueberry-v$currentVersion-release.json")
    $releaseLock = [IO.File]::Open($packageTwo, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        $releaseFailure = Capture-TestFailure -Path $releaseScript -Parameters @{
            ExePath = $currentExe; Version = $currentVersion; OutputDirectory = $outputTwo; LicenseNoticesPath = $licenseNoticesPath; Force = $true
        }
        Assert-Test ($releaseFailure -match '恢复备份保留于') '恢复失败错误包含保留备份位置'
        $backupMatch = [Text.RegularExpressions.Regex]::Match($releaseFailure, '恢复备份保留于\s+(.+)')
        Assert-Test $backupMatch.Success '恢复失败错误可解析备份目录'
        $preservedBackupRoot = $backupMatch.Groups[1].Value.Trim()
        Assert-Test (Test-Path -LiteralPath $preservedBackupRoot -PathType Container) '恢复失败保留备份目录'
        Assert-Test (Test-Path -LiteralPath (Join-Path $preservedBackupRoot ([IO.Path]::GetFileName($packageTwo))) -PathType Leaf) '恢复失败保留旧 ZIP 备份'
    }
    finally { $releaseLock.Dispose() }
    Assert-Test ($releaseZipBeforeFailure -eq (Get-FileHashUpper -Path $packageTwo)) '发布失败后 ZIP 未被混写'
    Assert-Test ($releaseShaBeforeFailure -eq (Get-FileHashUpper -Path "$packageTwo.sha256")) '发布失败后外部 SHA 未被混写'
    Assert-Test ($releaseManifestBeforeFailure -eq (Get-FileHashUpper -Path (Join-Path $outputTwo "blueberry-v$currentVersion-release.json"))) '发布失败后 release manifest 未被混写'

    $junctionParent = Join-Path $tempRoot 'junction-parent'
    $junctionTarget = Join-Path $tempRoot 'junction-target'
    [IO.Directory]::CreateDirectory($junctionParent) | Out-Null
    [IO.Directory]::CreateDirectory($junctionTarget) | Out-Null
    Write-TestBytes -Path (Join-Path $junctionTarget 'keep.txt') -Text 'outside'
    New-Item -ItemType Junction -Path (Join-Path $junctionParent 'inner') -Target $junctionTarget | Out-Null
    Expect-TestFailure -Path $manageScript -Parameters @{ Action = 'Install'; PackagePath = $packageTwo; InstallRoot = (Join-Path $junctionParent 'inner' 'child') } -MessagePattern '重解析|junction|符号链接'
    Assert-Test (Test-Path -LiteralPath (Join-Path $junctionTarget 'keep.txt') -PathType Leaf) 'junction 外部文件未被触碰'

    $readmePath = Join-Path $installRoot 'README.md'
    $readmeBytesBeforeChange = [IO.File]::ReadAllBytes($readmePath)
    Write-TestBytes -Path $readmePath -Text 'user changed managed file'
    Invoke-TestScript -Path $manageScript -Parameters @{ Action = 'Uninstall'; InstallRoot = $installRoot }
    Assert-Test (Test-Path -LiteralPath (Join-Path $installRoot 'README.md') -PathType Leaf) '卸载保留哈希已变化的受管文件'
    Assert-Test (Test-Path -LiteralPath (Join-Path $installRoot 'install.json') -PathType Leaf) '卸载保留清单以报告变化'
    [IO.File]::WriteAllBytes($readmePath, $readmeBytesBeforeChange)
    Invoke-TestScript -Path $manageScript -Parameters @{ Action = 'Uninstall'; InstallRoot = $installRoot }
    Assert-Test (Test-Path -LiteralPath (Join-Path $installRoot 'user-data.txt') -PathType Leaf) '卸载保留安装根用户文件'
    Assert-Test (-not (Test-Path -LiteralPath (Join-Path $installRoot 'install.json'))) '干净重试删除安装清单'
    Write-Host '变更文件保留与干净重试检查完成；自测临时安装根: ' $installRoot
    Assert-Test ([Linq.Enumerable]::SequenceEqual($configBytesBefore, [IO.File]::ReadAllBytes($configPath))) '卸载未修改用户配置'
    Write-Host "发布准备自测通过；临时目录: $tempRoot"
}
finally {
    if (-not $KeepTemp -and $preservedBackupRoot -and (Test-Path -LiteralPath $preservedBackupRoot)) {
        Remove-Item -LiteralPath $preservedBackupRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
    if (-not $KeepTemp -and (Test-Path -LiteralPath $tempRoot)) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force
    } elseif (Test-Path -LiteralPath $tempRoot) {
        Write-Host "已保留自测临时目录: $tempRoot"
    }
}
