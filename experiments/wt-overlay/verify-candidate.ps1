param([Parameter(Mandatory=$true)][string]$CandidateData)
$ErrorActionPreference='Stop'
if(-not $env:BLUEBERRY_SESSION_DIR) { throw 'Run this check inside the dedicated candidate window' }
$actual=[IO.Path]::GetFullPath($env:BLUEBERRY_SESSION_DIR)
$expected=[IO.Path]::GetFullPath($CandidateData).TrimEnd('\')+'\'
if(-not $actual.StartsWith($expected,[StringComparison]::OrdinalIgnoreCase)) { throw 'This is not the fixed candidate session' }
$adapter=Get-Content -LiteralPath (Join-Path $actual 'adapter.json') -Raw | ConvertFrom-Json
if($adapter.host_mode -ne 'direct' -or $adapter.terminal_restore -ne 'native_overlay_v4' -or
   -not $adapter.automatic_menu -or $adapter.editor_mode -ne 'editor_hooks_v1' -or $adapter.editor_fallback_reason) {
    $adapter | Format-List
    throw 'Native candidate requires the private editor integration and native overlay; compatibility mode is not accepted'
}
$adapter | Select-Object shell_version,psreadline_version,host_mode,terminal_restore,editor_mode,editor_patch,automatic_menu,disabled_reason | Format-List
