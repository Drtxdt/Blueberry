[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Shell,
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$OutputDirectory
)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
# PS5.1 resolves its system PSHOME even when its EXE is copied. Only a
# disposable GitHub-hosted account may use its actual per-user profile here.
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
    throw 'This fixture requires a disposable GitHub-hosted Windows runner.'
}
$Shell=if (Test-Path -LiteralPath $Shell -PathType Leaf) { (Resolve-Path -LiteralPath $Shell).Path } else { (Get-Command $Shell -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source }
$Executable=(Resolve-Path -LiteralPath $Executable).Path
$root=[IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $root) { throw 'Evidence directory must be new.' }
$null=New-Item -ItemType Directory -Path $root
$profilePath=[string](& $Shell -NoProfile -NonInteractive -Command '$PROFILE.CurrentUserCurrentHost')
if ($LASTEXITCODE -ne 0 -or -not [IO.Path]::IsPathRooted($profilePath)) { throw 'Cannot resolve automatic profile.' }
$profilePath=[IO.Path]::GetFullPath($profilePath.Trim())
$userRoot=[IO.Path]::GetFullPath($env:USERPROFILE).TrimEnd('\')+'\'
if (-not $profilePath.StartsWith($userRoot,[StringComparison]::OrdinalIgnoreCase)) { throw 'Profile is outside the disposable runner account.' }
$existed=Test-Path -LiteralPath $profilePath -PathType Leaf
if ($existed -and ((Get-Item -LiteralPath $profilePath -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Profile is a link.' }
[byte[]]$original=@()
if ($existed) { $original=@([IO.File]::ReadAllBytes($profilePath)) }
if ($existed) { [IO.File]::WriteAllBytes((Join-Path $root 'original-profile.bin'),$original) }
$prepared=Join-Path $root 'managed-profile.ps1'
[IO.File]::WriteAllText($prepared,'',[Text.UTF8Encoding]::new($true))
& (Join-Path $PSScriptRoot 'startup.ps1') -Action enable -Executable $Executable -ProfilePath $prepared
$text=[IO.File]::ReadAllText($prepared).Replace('& $blueberryAutoExe run --shell $blueberryAutoShell','& $blueberryAutoExe run --data-dir $env:BLUEBERRY_PROFILE_PROBE_DATA --shell $blueberryAutoShell')
$text += "`r`nif (`$env:BLUEBERRY_ACTIVE -ne '1' -and `$env:BLUEBERRY_PROFILE_PROBE_DATA) { [Environment]::Exit([int]`$LASTEXITCODE) }`r`n"
[IO.File]::WriteAllText($prepared,$text,[Text.UTF8Encoding]::new($true))
$expectedHash=(Get-FileHash -LiteralPath $prepared).Hash
$fixture=Join-Path $root 'blueberry-profile-probe.json'
[ordered]@{
    schema=1
    shell=$Shell
    shell_sha256=(Get-FileHash -LiteralPath $Shell).Hash
    executable=$Executable
    executable_sha256=(Get-FileHash -LiteralPath $Executable).Hash
    profile=$profilePath
    profile_sha256=$expectedHash
    managed_block='scripts/startup.ps1; isolated data argument; unchanged recursion and script guards'
    observed_entry='PowerShell automatic CurrentUserCurrentHost profile on disposable GitHub-hosted account'
} | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $fixture -Encoding utf8
$parent=Split-Path -Parent $profilePath
$null=New-Item -ItemType Directory -Path $parent -Force
$installed=$false
try {
    [IO.File]::WriteAllBytes($profilePath,[IO.File]::ReadAllBytes($prepared))
    $installed=$true
    & $Executable product-probe --shell $Shell --profile-entry --profile-fixture $fixture --iterations 2 --output (Join-Path $root 'diagnostic.json')
    if ($LASTEXITCODE -ne 0) { throw 'Automatic profile entry probe failed.' }
} finally {
    if ($installed) {
        if ((Get-FileHash -LiteralPath $profilePath).Hash -ne $expectedHash) { throw 'Profile changed during test; evidence retained without overwriting an unrelated change.' }
        if ($existed) { [IO.File]::WriteAllBytes($profilePath,$original) }
        else { Remove-Item -LiteralPath $profilePath -Force }
        $restored=if ($existed) {
            (Get-FileHash -LiteralPath $profilePath).Hash -eq (Get-FileHash -LiteralPath (Join-Path $root 'original-profile.bin')).Hash
        } else { -not (Test-Path -LiteralPath $profilePath) }
        [ordered]@{schema=1;profile=$profilePath;original_existed=$existed;restored=$restored} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'restoration.json') -Encoding utf8
        if (-not $restored) { throw 'Runner profile restoration failed.' }
    }
}
