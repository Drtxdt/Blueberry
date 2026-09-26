# Single-layer bootstrap. The assembly is built with the EXE, never compiled
# during product startup. The editing thread owns every console write.
if ($env:BLUEBERRY_NO_HISTORY -eq '1' -and $env:BLUEBERRY_TEST_PSREADLINE_MODULE) {
    Import-Module $env:BLUEBERRY_TEST_PSREADLINE_MODULE -Force -ErrorAction Stop
} else {
    Import-Module PSReadLine -ErrorAction Stop
}
if ($env:BLUEBERRY_NO_HISTORY -eq '1') {
    Set-PSReadLineOption -HistorySaveStyle SaveNothing
}
$null = [Reflection.Assembly]::Load([IO.File]::ReadAllBytes((Join-Path $PSScriptRoot 'direct-bridge.dll')))
[Blueberry.Direct.Bridge]::Initialize([Microsoft.PowerShell.PSConsoleReadLine], $env:BLUEBERRY_PIPE_NAME, $env:BLUEBERRY_TOKEN, $PSVersionTable.PSVersion.ToString(), (Get-Module PSReadLine).Version.ToString())
# Object events are queued to the current runspace. PSReadLine processes these
# on its editing thread, including while waiting for input. This is deliberately
# not an injected keyboard wakeup and cannot leak a key to an external program.
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
$env:BLUEBERRY_HOST_MODE = 'direct'
$env:BLUEBERRY_TRANSPORT_ACTUAL = 'pipe'
$env:BLUEBERRY_AUTOMATIC_MENU = [Blueberry.Direct.Bridge]::AutomaticMenu.ToString().ToLowerInvariant()
$env:BLUEBERRY_AUTOMATIC_MENU_DISABLED_REASON = [Blueberry.Direct.Bridge]::DisabledReason
if (-not [Blueberry.Direct.Bridge]::AutomaticMenu) {
    Write-Warning ('Blueberry 自动菜单已停用：' + [Blueberry.Direct.Bridge]::DisabledReason)
}
