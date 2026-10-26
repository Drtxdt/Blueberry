$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot '../scripts/resource-readiness.ps1')
$directory=Join-Path ([IO.Path]::GetTempPath()) ('blueberry-readiness-'+[Guid]::NewGuid())
$null=New-Item -ItemType Directory -Path $directory
try {
    if (-not (Get-ResourceReadiness).ready) { throw 'Uninstrumented comparator rejected' }
    if ((Get-ResourceReadiness $directory).ready) { throw 'Missing adapter accepted' }
    $path=Join-Path $directory 'adapter.json'
    $adapter=@{host_mode='direct';session_commands=@{complete=$true;error=$null};system_commands=@{count=0;error=$null}}
    $adapter | ConvertTo-Json -Depth 5 | Set-Content $path
    if ((Get-ResourceReadiness $directory).ready) { throw 'Running initialization accepted' }
    $adapter.system_commands.count=1713
    $adapter | ConvertTo-Json -Depth 5 | Set-Content $path
    if (-not (Get-ResourceReadiness $directory).ready) { throw 'Published metadata rejected' }
    $adapter.session_commands.complete=$false
    $adapter | ConvertTo-Json -Depth 5 | Set-Content $path
    if ((Get-ResourceReadiness $directory).ready) { throw 'Incomplete session accepted' }
    $adapter.session_commands.complete=$true
    $adapter.system_commands.error='scan failed'
    $adapter | ConvertTo-Json -Depth 5 | Set-Content $path
    if ((Get-ResourceReadiness $directory).ready) { throw 'Failed metadata accepted' }
    '{broken' | Set-Content $path
    if ((Get-ResourceReadiness $directory).ready) { throw 'Malformed adapter accepted' }
    Write-Output 'Resource readiness: all six required states passed'
} finally {
    $resolved=[IO.Path]::GetFullPath($directory)
    $tempRoot=[IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')+'\'
    if (-not $resolved.StartsWith($tempRoot,[StringComparison]::OrdinalIgnoreCase)) { throw 'Unexpected test directory' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
