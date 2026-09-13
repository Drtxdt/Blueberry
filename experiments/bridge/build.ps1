[CmdletBinding()]
param(
    [string]$OutputDirectory = (Join-Path $PSScriptRoot 'bin'),
    [switch]$Clean
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ($null -eq (Get-Command Add-Type -ErrorAction SilentlyContinue)) {
    throw 'PowerShell Add-Type is unavailable; use PowerShell 7 for the offline build.'
}

$sourcePath = Join-Path $PSScriptRoot 'ShellSense.PSReadLineBridge.cs'
if (-not [IO.File]::Exists($sourcePath)) {
    throw "Bridge source was not found: $sourcePath"
}

$outputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if ($Clean -and [IO.Directory]::Exists($outputDirectory)) {
    Remove-Item -LiteralPath $outputDirectory -Recurse -Force
}
[IO.Directory]::CreateDirectory($outputDirectory) | Out-Null

$outputPath = Join-Path $outputDirectory 'ShellSense.PSReadLineBridge.dll'
if ([IO.File]::Exists($outputPath)) {
    Remove-Item -LiteralPath $outputPath -Force
}

# PowerShell 7 ships the Roslyn compiler and its target-framework reference
# assemblies. This is a build-time compiler invocation only; the adapter never
# compiles or loads source at runtime. On .NET Core, passing implementation
# DLLs through `-ReferencedAssemblies` replaces Add-Type's implicit reference
# set and breaks compiler intrinsics such as Monitor.Enter. Let the selected
# pwsh supply its normal target-framework references, while checking that the
# two non-framework assemblies used by the bridge are present beside it.
$referenceNames = @(
    'System.Text.Encodings.Web.dll',
    'System.Text.Json.dll'
)
$references = @($referenceNames | ForEach-Object { Join-Path $PSHOME $_ })
$compilerNames = @(
    'Microsoft.CodeAnalysis.dll',
    'Microsoft.CodeAnalysis.CSharp.dll'
)
$compilerPaths = @($compilerNames | ForEach-Object { Join-Path $PSHOME $_ })
$missing = @($references + $compilerPaths | Where-Object { -not [IO.File]::Exists($_) })
if ($missing.Count -gt 0) {
    throw ('Required offline PowerShell/Roslyn file(s) are missing: ' + ($missing -join ', '))
}

Add-Type `
    -Path $sourcePath `
    -OutputAssembly $outputPath

if (-not [IO.File]::Exists($outputPath)) {
    throw "Compiler did not produce $outputPath"
}

$output = Get-Item -LiteralPath $outputPath
[pscustomobject]@{
    FullName  = $output.FullName
    Length    = $output.Length
    LastWriteTime = $output.LastWriteTime
    Compiler  = 'PowerShell 7 Add-Type (Roslyn, build time only)'
    PSHome    = $PSHOME
}
