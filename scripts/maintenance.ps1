[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('Prepare','Worker','Status','Cancel','Check')][string]$Mode,
    [Parameter(Mandatory)][string]$InstallRoot,
    [Parameter(Mandatory)][string]$StateRoot,
    [ValidateSet('Upgrade','Rollback','Uninstall')][string]$Operation = 'Upgrade',
    [string]$Version,
    [string]$PackagePath
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
try { [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false) } catch { }
. (Join-Path $PSScriptRoot 'manage-install.ps1') -Action Upgrade -FunctionsOnly -InstallRoot $InstallRoot -PackagePath $PackagePath
$InstallRoot = Assert-SafeInstallRoot -Path $InstallRoot
$StateRoot = [IO.Path]::GetFullPath($StateRoot)
Assert-NoReparseComponents $StateRoot
$statePath = Join-Path $StateRoot 'operation.json'
$cancelPath = Join-Path $StateRoot 'cancel'
$terminal = @('completed','failed','cancelled')

function Read-Operation {
    if (-not [IO.File]::Exists($statePath)) { return $null }
    return ((Read-BlueberrySharedText $statePath) | ConvertFrom-BlueberryInstallJson)
}
function Save-Operation($State) {
    $temporary = $statePath + '.' + [Guid]::NewGuid().ToString('N') + '.tmp'
    [IO.File]::WriteAllText($temporary, (ConvertTo-BlueberryCanonicalJson $State), [Text.UTF8Encoding]::new($false))
    Move-BlueberryFile $temporary $statePath
}
function Find-Release {
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    $headers = @{ 'User-Agent'='Blueberry-Maintenance'; Accept='application/vnd.github+json' }
    $uri = 'https://api.github.com/repos/Drtxdt/Blueberry/releases/'
    if ($Version) {
        if ($Version -notmatch '^v?\d+\.\d+\.\d+(?:-[0-9A-Za-z][0-9A-Za-z.-]*)?$') { throw 'Invalid version' }
        $uri += 'tags/v' + $Version.TrimStart('v')
    } else { $uri += 'latest' }
    $release = Invoke-RestMethod -Uri $uri -Headers $headers
    if ($release.draft -or (-not $Version -and $release.prerelease)) { throw 'No published stable release is available' }
    if ($release.tag_name -notmatch '^v\d+\.\d+\.\d+(?:-[0-9A-Za-z][0-9A-Za-z.-]*)?$') { throw 'Invalid release tag' }
    return $release
}
function Write-Result($Value) { Write-Output ('BLUEBERRY_MAINTENANCE:' + (ConvertTo-BlueberryCanonicalJson $Value)) }

if ($Mode -eq 'Status') {
    $state = Read-Operation
    if ($null -eq $state) { Write-Result @{status='idle'} } else { Write-Result $state }
    return
}
if ($Mode -eq 'Cancel') {
    $state = Read-Operation
    if ($null -eq $state -or $state.status -in $terminal) { throw '没有待取消的维护操作' }
    $id = $state.id
    $gate = [Threading.Mutex]::new($false, ('Local\Blueberry.Commit.' + $id))
    try {
        try { $null = $gate.WaitOne() } catch [Threading.AbandonedMutexException] { }
        $state = Read-Operation
        if ($state.id -ne $id -or $state.status -in $terminal) { throw '维护状态已变化，请重新查看 status' }
        if ($state.status -in @('running','recovering','recovery_required')) { throw '事务已经开始提交或恢复，不能取消；失败时会执行事务回滚' }
        [IO.File]::WriteAllText($cancelPath, [string]$id)
        try {
            $event = [Threading.EventWaitHandle]::OpenExisting('Local\Blueberry.Maintenance.' + $id)
            try { $null = $event.Set() } finally { $event.Dispose() }
        } catch [Threading.WaitHandleCannotBeOpenedException] { }
        # A crashed preparation has no worker to observe the cancellation.
        $idleLock = $null
        try { $idleLock = [IO.File]::Open((Join-Path $StateRoot 'operation.lock'), 'OpenOrCreate', 'ReadWrite', 'None') } catch [IO.IOException] { }
        if ($null -ne $idleLock) {
            try { $state.status='cancelled'; $state.message='操作已取消，安装未修改'; Save-Operation $state } finally { $idleLock.Dispose() }
        }
        Write-Result @{status='cancellation_requested'; id=$id}
    } finally { $gate.ReleaseMutex(); $gate.Dispose() }
    return
}
if ($Mode -ne 'Worker') { $installed = Read-InstallManifest -Root $InstallRoot -Required }
if ($Mode -eq 'Check') {
    $release = Find-Release
    Write-Result @{status='checked'; installed=$installed.current.version; available=([string]$release.tag_name).TrimStart('v'); url=$release.html_url}
    return
}
[IO.Directory]::CreateDirectory($StateRoot) | Out-Null
$transcribing = $false
if ($Mode -eq 'Worker') {
    Start-Transcript -LiteralPath (Join-Path $StateRoot 'worker.log') -Append | Out-Null
    $transcribing = $true
}
$lock = [IO.File]::Open((Join-Path $StateRoot 'operation.lock'), 'OpenOrCreate', 'ReadWrite', 'None')
$state = $null
$ownsOperation = $false
try {
    $state = Read-Operation
    if ($Mode -eq 'Prepare') {
        if ($null -ne $state -and $state.status -notin $terminal) { throw '已有维护操作排队；请使用 blueberry maintenance status 或 cancel' }
        if ([IO.File]::Exists($cancelPath)) { [IO.File]::Delete($cancelPath) }
        $state = @{ schema=1; id=[Guid]::NewGuid().ToString('N'); status='preparing'; operation=$Operation; install_root=$InstallRoot; expected_manifest_sha256=(Get-Sha256 (Join-Path $InstallRoot 'install.json')); package=$null; package_sha256=$null; message='准备维护资源'; created_utc=[DateTime]::UtcNow.ToString('o') }
        $ownsOperation = $true
        Save-Operation $state
        if ($Operation -eq 'Upgrade') {
            $stage = Join-Path $StateRoot $state.id
            [IO.Directory]::CreateDirectory($stage) | Out-Null
            if ($PackagePath) {
                $source = Get-RegularFileRecord -Path $PackagePath -Label 'Upgrade package'
                Assert-ExternalPackageHash $source.FullName
                $zip = Join-Path $stage $source.Name
                [IO.File]::Copy($source.FullName, $zip, $false)
            } else {
                $release = Find-Release
                $name = 'blueberry-' + $release.tag_name + '-windows-x64.zip'
                $asset = @($release.assets | Where-Object name -CEQ $name)
                $checksum = @($release.assets | Where-Object name -CEQ ($name + '.sha256'))
                if ($asset.Count -ne 1 -or $checksum.Count -ne 1) { throw 'Release package or checksum missing/duplicated' }
                $zip = Join-Path $stage $name
                foreach ($pair in @(@($asset[0],$zip),@($checksum[0],($zip+'.sha256')))) {
                    $url = [uri]$pair[0].browser_download_url
                    if ($url.Scheme -ne 'https' -or $url.Host -ne 'github.com' -or -not $url.AbsolutePath.StartsWith('/Drtxdt/Blueberry/releases/download/')) { throw 'Unexpected release asset URL' }
                    Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $pair[1] | Out-Null
                }
                Assert-ExternalPackageHash $zip
            }
            $validated = Read-Package $zip
            try {
                if ($Version -and $validated.manifest.version -ne $Version.TrimStart('v')) { throw 'Requested version differs from package' }
                if (-not $PackagePath -and $validated.manifest.version -ne ([string]$release.tag_name).TrimStart('v')) { throw 'Release version differs from package' }
                $state.package = $zip
                $state.package_sha256 = Get-Sha256 $zip
                $state['target_version'] = $validated.manifest.version
            } finally {
                Assert-NoReparseComponents $validated.root
                Remove-Item -LiteralPath $validated.root -Recurse -Force
            }
        }
        $state.status = if ([IO.File]::Exists($cancelPath)) { 'cancelled' } else { 'queued' }
        $state.message = if ($state.status -eq 'cancelled') { '操作已取消，安装未修改' } else { '资源已校验，等待相关会话自然退出' }
        Save-Operation $state
        Write-Result $state
        return
    }
    if ($null -eq $state -or $state.status -in $terminal) { return }
    $ownsOperation = $true
    $recover = $state.status -in @('running','recovering','recovery_required')
    if (-not $recover -and $state.status -notin @('queued','waiting')) { throw '维护准备未完成，请取消后重试' }
    if ($state.install_root -ine $InstallRoot) { throw 'Maintenance installation identity mismatch' }
    $ownsOperation = $true
    $cancel = [Threading.EventWaitHandle]::new($false, 'ManualReset', ('Local\Blueberry.Maintenance.' + $state.id))
    try {
        $state.status = if ($recover) { 'recovering' } else { 'waiting' }; $state['worker_pid'] = $PID; Save-Operation $state
        while ($true) {
            if (-not $recover -and [IO.File]::Exists($cancelPath)) { $state.status='cancelled'; $state.message='操作已取消，安装未修改'; Save-Operation $state; return }
            $processes = @()
            foreach ($process in [Diagnostics.Process]::GetProcessesByName('blueberry')) {
                try {
                    if ($process.MainModule.FileName -ieq (Join-Path $InstallRoot 'blueberry.exe')) { $processes += $process } else { $process.Dispose() }
                } catch { $process.Dispose() }
            }
            if ($processes.Count -eq 0) { break }
            $handles = @($cancel)
            try {
                foreach ($process in @($processes | Select-Object -First 63)) {
                    try {
                        $wait = [Threading.EventWaitHandle]::new($false, 'ManualReset')
                        $wait.SafeWaitHandle = [Microsoft.Win32.SafeHandles.SafeWaitHandle]::new($process.Handle, $false)
                        $handles += $wait
                    } catch { if (-not $process.HasExited) { throw } }
                }
                if ($handles.Count -gt 1) { $null = [Threading.WaitHandle]::WaitAny([Threading.WaitHandle[]]$handles) }
            } finally {
                foreach ($wait in @($handles | Select-Object -Skip 1)) { $wait.Dispose() }
                foreach ($process in $processes) { $process.Dispose() }
            }
        }
        $transaction = Join-Path $StateRoot ('transaction-' + $state.id)
        if ($recover -and $state.operation -ne 'Uninstall') {
            $outcome = & (Join-Path $PSScriptRoot 'manage-install.ps1') -Action Recover -InstallRoot $InstallRoot -TransactionDirectory $transaction -ExpectedManifestSha256 $state.expected_manifest_sha256
            $state.status = if ($outcome -eq 'committed') { 'completed' } else { 'failed' }
            $state.message = if ($outcome -eq 'committed') { '已核验上次完整提交' } else { '中断事务已恢复原安装，可重新提交维护命令' }
            Save-Operation $state
            return
        }
        if ($recover -and $state.operation -eq 'Uninstall' -and -not [IO.File]::Exists((Join-Path $InstallRoot 'install.json'))) {
            $state.status='completed'; $state.message='上次卸载已完成'; Save-Operation $state; return
        }
        if ($state.package -and (Get-Sha256 $state.package) -ine $state.package_sha256) { throw '暂存升级包已变化；安装未修改' }
        $gate = [Threading.Mutex]::new($false, ('Local\Blueberry.Commit.' + $state.id))
        try {
            try { $null = $gate.WaitOne() } catch [Threading.AbandonedMutexException] { }
            if ([IO.File]::Exists($cancelPath)) { $state.status='cancelled'; $state.message='操作已取消，安装未修改'; Save-Operation $state; return }
            $state.status='running'; $state.message='正在提交安装事务'; Save-Operation $state
        } finally { $gate.ReleaseMutex(); $gate.Dispose() }
        $arguments = @{ Action=$state.operation; InstallRoot=$InstallRoot; ExpectedManifestSha256=$state.expected_manifest_sha256; TransactionDirectory=$transaction }
        if ($state.package) { $arguments.PackagePath = $state.package }
        & (Join-Path $PSScriptRoot 'manage-install.ps1') @arguments
        $state.status='completed'; $state.message='维护已完成'; $state['completed_utc']=[DateTime]::UtcNow.ToString('o'); Save-Operation $state
    } finally { $cancel.Dispose() }
} catch {
    if ($ownsOperation -and $null -ne $state -and $state.status -notin $terminal) {
        $state.status=if ($state.status -in @('running','recovering','recovery_required')) { 'recovery_required' } else { 'failed' }; $state.message=$_.Exception.Message; Save-Operation $state
    }
    throw
} finally {
    $lock.Dispose()
    if ($transcribing) { Stop-Transcript | Out-Null }
}
