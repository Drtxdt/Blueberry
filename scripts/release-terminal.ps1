[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$PackagePath,
    [Parameter(Mandatory=$true)][string]$OutputDirectory,
    [string]$Version = '1.24.12741.0-native4-b1a2d3f'
)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$terminalSourcePackage=$PackagePath
. (Join-Path $PSScriptRoot 'manage-install.ps1') -Action Install -FunctionsOnly
if ($Version -notmatch '^[0-9A-Za-z][0-9A-Za-z.-]+$') { throw 'Invalid component version' }
$output=[IO.Path]::GetFullPath($OutputDirectory)
Assert-NoReparseComponents $output
[IO.Directory]::CreateDirectory($output) | Out-Null
$stage=Join-Path $output ('.terminal-stage-'+[Guid]::NewGuid().ToString('N'))
$archive=Join-Path $output ('blueberry-terminal-component-v'+$Version+'-windows-x64.zip')
if (Test-Path -LiteralPath $archive) { throw 'Component artifact already exists' }
try {
    $identity=& (Join-Path $PSScriptRoot 'stage-terminal-payload.ps1') -PackagePath $terminalSourcePackage -Destination $stage
    $files=@(Get-ChildItem -LiteralPath $stage -File -Recurse -Force | Sort-Object FullName | ForEach-Object {
        @{path=(Get-BlueberryRelativePath $stage $_.FullName).Replace('\','/');bytes=$_.Length;sha256=(Get-Sha256 $_.FullName)}
    })
    $manifest=@{schema_version=1;product='Blueberry.Terminal';package_kind='terminal';version=$Version;platform='windows-x64';files=$files;terminal_dependency=$identity}
    Write-Utf8JsonAtomic -Path (Join-Path $stage 'component.json') -Value $manifest
    $temporary=$archive+'.'+[Guid]::NewGuid().ToString('N')+'.tmp.zip'
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $temporary -CompressionLevel Optimal
    Move-BlueberryFile -Source $temporary -Destination $archive
    [IO.File]::WriteAllText(($archive+'.sha256'), ((Get-Sha256 $archive)+'  '+[IO.Path]::GetFileName($archive)+"`n"),[Text.UTF8Encoding]::new($false))
    [pscustomobject]@{package=$archive;sha256=(Get-Sha256 $archive);version=$Version;kind='terminal'}
} finally {
    if ([IO.Directory]::Exists($stage) -and [IO.Path]::GetDirectoryName($stage) -eq $output) { Remove-Item -LiteralPath $stage -Recurse -Force }
}
