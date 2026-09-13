[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ShellSenseExecutable,

    [string]$Shell = 'pwsh.exe',

    [ValidateSet('Baseline', 'Bridge')]
    [string]$Mode = 'Bridge',

    [ValidateRange(1, 100)]
    [int]$Iterations = 30,

    [switch]$NoProfile,

    [string]$OutputPath = ''
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$hostPath = (Resolve-Path -LiteralPath $ShellSenseExecutable -ErrorAction Stop).Path
if (-not [IO.File]::Exists($hostPath)) {
    throw "ShellSense executable was not found: $hostPath"
}
$pwsh = (Get-Command pwsh -ErrorAction SilentlyContinue)
if ($null -eq $pwsh) {
    throw 'pwsh is required to generate the temporary adapter copy.'
}

$preparePath = Join-Path $PSScriptRoot 'prepare-ab-adapter.ps1'
$preparedJson = & $pwsh.Source -NoProfile -File $preparePath -Mode $Mode -Json
if ($LASTEXITCODE -ne 0) {
    throw "A/B adapter preparation failed with exit code $LASTEXITCODE."
}
$prepared = ($preparedJson -join '') | ConvertFrom-Json
if ([string]::IsNullOrWhiteSpace([string]$prepared.AdapterPath)) {
    throw 'A/B adapter preparation did not return an adapter path.'
}

$arguments = [System.Collections.Generic.List[string]]::new()
[void]$arguments.Add('probe')
[void]$arguments.Add('--shell')
[void]$arguments.Add($Shell)
[void]$arguments.Add('--iterations')
[void]$arguments.Add([string]$Iterations)
if (-not $NoProfile) {
    [void]$arguments.Add('--with-profile')
}
[void]$arguments.Add('--adapter-script')
[void]$arguments.Add([string]$prepared.AdapterPath)
if (-not [string]::IsNullOrWhiteSpace($OutputPath)) {
    [void]$arguments.Add('--output')
    [void]$arguments.Add([IO.Path]::GetFullPath($OutputPath))
}

$probeOutput = @(& $hostPath @arguments)
$exitCode = $LASTEXITCODE
if ($exitCode -ne 0) {
    throw "ShellSense A/B probe failed with exit code $exitCode."
}
$probeOutput

if ($Mode -eq 'Bridge') {
    $proofPath = [string]$prepared.ProofPath
    if ([string]::IsNullOrWhiteSpace($proofPath) -or -not [IO.File]::Exists($proofPath)) {
        throw 'Bridge A/B probe did not publish a binding proof; its timings are rejected as fallback.'
    }
    $proof = Get-Content -Raw -LiteralPath $proofPath | ConvertFrom-Json
    if ($proof.mode -ne 'bridge' -or -not [bool]$proof.bound -or @($proof.surface).Count -lt 5) {
        throw 'Bridge A/B probe published an invalid binding proof; its timings are rejected.'
    }
    $fallbackProofPath = $proofPath + '.fallback.json'
    if ([IO.File]::Exists($fallbackProofPath)) {
        throw 'Bridge A/B probe recorded an invocation fallback; its timings are rejected.'
    }
    Write-Verbose ("Bridge binding proof verified: {0}" -f $proofPath)
}
