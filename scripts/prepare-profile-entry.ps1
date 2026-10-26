[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Shell,
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$Destination
)
$ErrorActionPreference='Stop'
$Shell=if (Test-Path -LiteralPath $Shell -PathType Leaf) { (Resolve-Path -LiteralPath $Shell).Path } else { (Get-Command $Shell -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source }
$Executable=(Resolve-Path -LiteralPath $Executable).Path
$version=& $Shell -NoProfile -NonInteractive -Command '$PSVersionTable.PSVersion.Major'
if ($LASTEXITCODE -ne 0 -or [int]$version -lt 7) { throw 'Isolated automatic profile fixture requires portable PowerShell 7; PS5.1 PSHOME cannot be redirected by copying its EXE.' }
if (Test-Path -LiteralPath $Destination) { throw 'Use a new fixture directory; existing data is never replaced.' }
$destinationPath=[IO.Path]::GetFullPath($Destination)
$sourceHome=Split-Path -Parent $Shell
if ($destinationPath.StartsWith($sourceHome+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) { throw 'Fixture must be outside source PSHOME.' }
$null=New-Item -ItemType Directory -Path $destinationPath
Get-ChildItem -LiteralPath $sourceHome -Force | Copy-Item -Destination $destinationPath -Recurse -Force
$profilePath=Join-Path $destinationPath 'Microsoft.PowerShell_profile.ps1'
# Only the new copy is changed. The generated guards and invocation run during
# PowerShell's own automatic profile loading, with no -Command/-File bypass.
[IO.File]::WriteAllText($profilePath,'',[Text.UTF8Encoding]::new($true))
& (Join-Path $PSScriptRoot 'startup.ps1') -Action enable -Executable $Executable -ProfilePath $profilePath
$text=[IO.File]::ReadAllText($profilePath)
$text=$text.Replace('& $blueberryAutoExe run --shell $blueberryAutoShell', '& $blueberryAutoExe run --data-dir $env:BLUEBERRY_PROFILE_PROBE_DATA --shell $blueberryAutoShell')
# The outer fixture exits after its child, while the child skips this branch
# through the product's ordinary recursion guard. No installed profile changes.
$text += "`r`nif (`$env:BLUEBERRY_ACTIVE -ne '1' -and `$env:BLUEBERRY_PROFILE_PROBE_DATA) { [Environment]::Exit([int]`$LASTEXITCODE) }`r`n"
[IO.File]::WriteAllText($profilePath,$text,[Text.UTF8Encoding]::new($true))
$copiedShell=Join-Path $destinationPath (Split-Path -Leaf $Shell)
$resolvedHome=& $copiedShell -NoProfile -NonInteractive -Command '$PSHOME'
if ($LASTEXITCODE -ne 0 -or [IO.Path]::GetFullPath([string]$resolvedHome) -ine $destinationPath) { throw 'Shell did not resolve isolated PSHOME.' }
[ordered]@{
    schema=1
    source_shell=$Shell
    shell=$copiedShell
    shell_sha256=(Get-FileHash -LiteralPath $copiedShell).Hash
    executable=$Executable
    executable_sha256=(Get-FileHash -LiteralPath $Executable).Hash
    profile=$profilePath
    profile_sha256=(Get-FileHash -LiteralPath $profilePath).Hash
    managed_block='scripts/startup.ps1; data directory argument added; recursion and script guards unchanged'
    observed_entry='PowerShell automatic AllUsersCurrentHost profile'
} | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $destinationPath 'blueberry-profile-probe.json') -Encoding utf8
Write-Output $copiedShell
