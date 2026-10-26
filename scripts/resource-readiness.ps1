# The historical Direct adapters publish count only after the helper exits.
# A stable PID list alone can include that still-running initialization helper.
function Get-ResourceReadiness {
    param([string]$Directory)
    if (-not $Directory) { return @{ready=$true; required=$false} }
    $result=@{ready=$false; required=$true; reason='adapter_not_published'}
    try {
        $files=@(Get-ChildItem -LiteralPath $Directory -Filter adapter.json -Recurse -File -ErrorAction Stop)
        if ($files.Count -ne 1) { return $result }
        $adapter=Get-Content -LiteralPath $files[0].FullName -Raw -ErrorAction Stop | ConvertFrom-Json -ErrorAction Stop
        $result.path=$files[0].FullName
        $result.system_count=$adapter.system_commands.count
        $result.system_error=$adapter.system_commands.error
        $result.session_complete=$adapter.session_commands.complete
        $result.reason='metadata_initializing'
        if ($adapter.system_commands.error -or $adapter.session_commands.error) {
            $result.reason='metadata_failed'
        } elseif ($adapter.host_mode -eq 'direct' -and $adapter.session_commands.complete -eq $true -and $adapter.system_commands.count -gt 0) {
            $result.ready=$true
            $result.reason='metadata_published'
        }
    } catch { $result.reason='adapter_unreadable'; $result.error=$_.Exception.Message }
    return $result
}
