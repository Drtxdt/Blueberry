# Single-layer bootstrap. The assembly is built with the EXE, never compiled
# during product startup. The editing thread owns every console write.
$blueberryDirectScriptEnter = if ($env:BLUEBERRY_DIRECT_TRACE -eq '1') { [Diagnostics.Stopwatch]::GetTimestamp() } else { 0 }
if (-not $blueberryDirectModule) { $blueberryDirectModule = Get-Module PSReadLine }
if (-not $blueberryDirectModule) {
    Import-Module PSReadLine -ErrorAction Stop
    $blueberryDirectModule = Get-Module PSReadLine
}
$blueberryDirectModuleLoaded = if ($blueberryDirectScriptEnter) { [Diagnostics.Stopwatch]::GetTimestamp() } else { 0 }
$null = [Reflection.Assembly]::LoadFrom("$PSScriptRoot\direct-bridge.dll")
$blueberryDirectAssemblyLoaded = if ($blueberryDirectScriptEnter) { [Diagnostics.Stopwatch]::GetTimestamp() } else { 0 }
[Blueberry.Direct.Bridge]::Initialize([Microsoft.PowerShell.PSConsoleReadLine], $env:BLUEBERRY_PIPE_NAME, $env:BLUEBERRY_TOKEN, $PSVersionTable.PSVersion.ToString(), $blueberryDirectModule.Version.ToString())
if ($blueberryDirectScriptEnter) {
    [Blueberry.Direct.Bridge]::StartupPoint('direct_script_enter', $blueberryDirectScriptEnter)
    [Blueberry.Direct.Bridge]::StartupPoint('direct_module_loaded', $blueberryDirectModuleLoaded)
    [Blueberry.Direct.Bridge]::StartupPoint('direct_assembly_loaded', $blueberryDirectAssemblyLoaded)
}
if (-not [Blueberry.Direct.Bridge]::EditorHooks) {
    Write-Warning ('Blueberry direct 兼容模式：' + [Blueberry.Direct.Bridge]::EditorFallbackReason)
    # Compatibility only: stock PSReadLine processes this queue on its idle timer.
    $global:BlueberryDirectSubscription = Register-ObjectEvent -InputObject ([Blueberry.Direct.Bridge]::Events) -EventName FrameAvailable -SourceIdentifier Blueberry.Direct.Frame -SupportEvent -Action {
        $null = [Blueberry.Direct.Bridge]::Refresh()
    }
    $global:BlueberryDirectReadLine = $function:global:PSConsoleHostReadLine
    $global:BlueberryDirectHistoryPath = if ($env:BLUEBERRY_NO_HISTORY -eq '1') { $null } else { (Get-PSReadLineOption).HistorySavePath }
    function global:PSConsoleHostReadLine {
        [Blueberry.Direct.Bridge]::Begin($ExecutionContext.SessionState.Path.CurrentFileSystemLocation.Path, $global:BlueberryDirectHistoryPath)
        try { & $global:BlueberryDirectReadLine }
        finally { [Blueberry.Direct.Bridge]::End() }
    }
}
# Keep the originally loaded module, but do not expose the private search root
# to external PowerShell programs launched from this session.
[Blueberry.Direct.Bridge]::PublishEnvironment()
if (-not [Blueberry.Direct.Bridge]::AutomaticMenu) {
    Write-Warning ('Blueberry 自动菜单已停用：' + [Blueberry.Direct.Bridge]::DisabledReason)
}
