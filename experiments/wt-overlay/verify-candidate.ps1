param([Parameter(Mandatory=$true)][string]$CandidateData, [ValidateSet('direct','nested')][string]$HostMode='direct', [ValidateSet('pipe','osc')][string]$Transport='pipe', [string]$ExpectedPSReadLine='2.4.5')
$ErrorActionPreference='Stop'
if(-not $env:BLUEBERRY_SESSION_DIR) { throw 'Run this check inside the dedicated candidate window' }
$actual=[IO.Path]::GetFullPath($env:BLUEBERRY_SESSION_DIR)
$expected=[IO.Path]::GetFullPath($CandidateData).TrimEnd('\')+'\'
if(-not $actual.StartsWith($expected,[StringComparison]::OrdinalIgnoreCase)) { throw 'This is not the fixed candidate session' }
$adapter=Get-Content -LiteralPath (Join-Path $actual 'adapter.json') -Raw | ConvertFrom-Json
$privateEditorInvalid=$HostMode -eq 'direct' -and ($adapter.editor_mode -ne 'editor_hooks_v1' -or $adapter.editor_fallback_reason)
if($adapter.host_mode -ne $HostMode -or $adapter.transport -ne $Transport -or $adapter.psreadline_version -ne $ExpectedPSReadLine -or
   $adapter.terminal_restore -ne 'native_overlay_v4' -or -not $adapter.automatic_menu -or $privateEditorInvalid) {
    $adapter | Format-List
    throw 'Candidate host, transport, editor integration or native overlay differs from its fixed contract'
}
$adapter | Select-Object shell_version,psreadline_version,host_mode,terminal_restore,editor_mode,editor_patch,automatic_menu,disabled_reason | Format-List
