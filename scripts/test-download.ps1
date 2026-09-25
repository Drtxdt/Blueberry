[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$PackagePath)
$ErrorActionPreference = 'Stop'
$PackagePath = [IO.Path]::GetFullPath($PackagePath)
$root = Join-Path ([IO.Path]::GetTempPath()) ('blueberry-download-test-' + [Guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($root) | Out-Null
$installer = Join-Path $PSScriptRoot '../install.ps1'
$blueberryDownloadTestState = @{ downloaded=@(); interrupt=$false; releases=@(); package=$PackagePath }
function New-TestRelease([string]$Tag, [bool]$Pre = $false, [bool]$Draft = $false) {
    $name = "blueberry-$Tag-windows-x64.zip"
    return [pscustomobject]@{ tag_name=$Tag; prerelease=$Pre; draft=$Draft; published_at='2026-09-13T00:00:00Z'; assets=@(
        [pscustomobject]@{name=$name;browser_download_url="https://github.com/Drtxdt/Blueberry/releases/download/$Tag/$name"},
        [pscustomobject]@{name="$name.sha256";browser_download_url="https://github.com/Drtxdt/Blueberry/releases/download/$Tag/$name.sha256"}
    ) }
}
function Invoke-RestMethod { param($Uri, $Headers); return $blueberryDownloadTestState.releases }
function Invoke-WebRequest {
    param($Uri, $OutFile, [switch]$UseBasicParsing)
    $blueberryDownloadTestState.downloaded += $Uri
    if ($blueberryDownloadTestState.interrupt) { [IO.File]::WriteAllText($OutFile, 'partial'); throw 'Simulated interrupted download' }
    if ($OutFile.EndsWith('.sha256')) {
        $name = [IO.Path]::GetFileName($OutFile).Replace('.sha256','')
        [IO.File]::WriteAllText($OutFile, ((Get-FileHash -LiteralPath $blueberryDownloadTestState.package).Hash + '  ' + $name))
    } else { [IO.File]::Copy($blueberryDownloadTestState.package, $OutFile, $true) }
}
try {
    $cases = @(
        @{ name='stable'; releases=@((New-TestRelease 'v0.4.0'), (New-TestRelease 'v0.5.0-beta.7' $true), (New-TestRelease 'v0.5.0' $false $true)); expected='v0.4.0' },
        @{ name='beta'; releases=@((New-TestRelease 'v0.4.0-beta.1' $true), (New-TestRelease 'v0.5.0-beta.7' $true)); expected='v0.5.0-beta.7' },
        @{ name='explicit'; releases=(New-TestRelease 'v0.5.0-beta.7' $true); expected='v0.5.0-beta.7'; version='0.5.0-beta.7' }
    )
    foreach ($case in $cases) {
        $blueberryDownloadTestState.releases = $case.releases
        $blueberryDownloadTestState.downloaded = @()
        $target = Join-Path $root $case.name
        $parameters = @{ InstallRoot=$target; ConfigRoot=(Join-Path $root 'config'); NoPrompt=$true; NoPath=$true }
        if ($case.ContainsKey('version')) { $parameters.Version = $case.version }
        & $installer @parameters | Out-Null
        if ($blueberryDownloadTestState.downloaded.Count -ne 2 -or $blueberryDownloadTestState.downloaded[0] -notlike ('*/' + $case.expected + '/*')) { throw "Wrong selected release: $($case.name)" }
        if (-not [IO.File]::Exists((Join-Path $target 'blueberry.exe'))) { throw 'Download was not installed' }
    }
    $target = Join-Path $root 'stable'
    $before = (Get-FileHash -LiteralPath (Join-Path $target 'blueberry.exe')).Hash
    foreach ($failure in @('draft','empty','interrupt')) {
        $blueberryDownloadTestState.releases = switch ($failure) {
            'draft' { New-TestRelease 'v0.5.0' $false $true }
            'empty' { @() }
            'interrupt' { New-TestRelease 'v0.5.0-beta.7' $true }
        }
        $blueberryDownloadTestState.interrupt = $failure -eq 'interrupt'
        $failed = $false
        try { & $installer -InstallRoot $target -ConfigRoot (Join-Path $root 'config') -NoPrompt -NoPath | Out-Null }
        catch { $failed = $true }
        if (-not $failed -or (Get-FileHash -LiteralPath (Join-Path $target 'blueberry.exe')).Hash -ne $before) { throw "Failure did not preserve existing install: $failure" }
    }
    Write-Output ('Release selection and interrupted download tests passed on PowerShell ' + $PSVersionTable.PSVersion)
} finally {
    $allowed = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\','/') + [IO.Path]::DirectorySeparatorChar
    if ([IO.Path]::GetFullPath($root).StartsWith($allowed, [StringComparison]::OrdinalIgnoreCase)) { Remove-Item -LiteralPath $root -Recurse -Force }
}
