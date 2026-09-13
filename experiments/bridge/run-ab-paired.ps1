[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ShellSenseExecutable,

    [string]$Shell = 'pwsh.exe',

    [ValidateRange(30, 1000)]
    [int]$PairCount = 30,

    [switch]$NoProfile,

    [string]$OutputPath = ''
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Resolve-ExistingFile {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [string]$Description
    )

    $resolved = $null
    try {
        $resolved = (Resolve-Path -LiteralPath $Path -ErrorAction Stop).Path
    } catch {
        throw "$Description was not found: $Path"
    }
    if (-not [IO.File]::Exists($resolved)) {
        throw "$Description is not a file: $resolved"
    }
    return [IO.Path]::GetFullPath($resolved)
}

function Invoke-PrepareAdapter {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Mode,

        [Parameter(Mandatory = $true)]
        [string]$PreparePath,

        [Parameter(Mandatory = $true)]
        [string]$PwshPath
    )

    $jsonLines = & $PwshPath -NoProfile -File $PreparePath -Mode $Mode -Json
    if ($LASTEXITCODE -ne 0) {
        throw "Preparing the $Mode A/B adapter failed with exit code $LASTEXITCODE."
    }
    $json = ($jsonLines -join '')
    if ([string]::IsNullOrWhiteSpace($json)) {
        throw "Preparing the $Mode A/B adapter returned no manifest."
    }
    return $json | ConvertFrom-Json
}

function Get-ProbeDelta {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Report,

        [Parameter(Mandatory = $true)]
        [string]$Field
    )

    $metric = $Report.PSObject.Properties[$Field]
    if ($null -eq $metric) {
        throw "Probe report does not contain $Field."
    }
    $samples = @($metric.Value.samples)
    if ($samples.Count -ne 1) {
        throw "Probe --iterations 1 returned $($samples.Count) samples for $Field."
    }
    return [double]$samples[0]
}

function Get-Stats {
    param(
        [Parameter(Mandatory = $true)]
        [double[]]$Values
    )

    if ($Values.Count -eq 0) {
        return [ordered]@{ samples = @(); median = $null; p95 = $null; unit = 'ms' }
    }
    $sorted = @($Values | Sort-Object)
    $count = $sorted.Count
    if (($count % 2) -eq 1) {
        $median = [double]$sorted[[int][math]::Floor($count / 2)]
    } else {
        $median = ([double]$sorted[($count / 2) - 1] + [double]$sorted[$count / 2]) / 2.0
    }
    # Nearest-rank P95: rank = ceil(0.95 * N), one-based.
    $rank = [int][math]::Ceiling(0.95 * $count)
    $p95 = [double]$sorted[$rank - 1]
    return [ordered]@{
        samples = @($Values)
        median  = $median
        p95     = $p95
        unit    = 'ms'
    }
}

function Read-BridgeProof {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $fallbackPath = if ([string]::IsNullOrWhiteSpace($Path)) { '' } else { $Path + '.fallback.json' }
    if ([string]::IsNullOrWhiteSpace($Path) -or -not [IO.File]::Exists($Path)) {
        return [pscustomobject]@{
            valid  = $false
            reason = 'no binding proof was published; the bridge may have silently fallen back'
            data   = $null
            fallback = $null
        }
    }
    try {
        $data = Get-Content -Raw -LiteralPath $Path | ConvertFrom-Json
        $fallback = if ([IO.File]::Exists($fallbackPath)) {
            Get-Content -Raw -LiteralPath $fallbackPath | ConvertFrom-Json
        } else {
            $null
        }
        $surface = @($data.surface)
        $valid = ($data.mode -eq 'bridge' -and
            [bool]$data.bound -and
            $surface.Count -ge 5 -and
            ($surface -contains 'GetKeyHandlers') -and
            ($surface -contains 'GetBufferState') -and
            ($surface -contains 'Replace') -and
            ($surface -contains 'SetKeyHandler') -and
            ($surface -contains 'Serialize') -and
            $null -eq $fallback)
        return [pscustomobject]@{
            valid  = [bool]$valid
            reason = if ($valid) { $null } elseif ($null -ne $fallback) { 'bridge invocation fell back to the original PSReadLine call' } else { 'binding proof is incomplete or reports fallback' }
            data   = $data
            fallback = $fallback
        }
    } catch {
        return [pscustomobject]@{
            valid  = $false
            reason = "binding proof is not valid JSON: $($_.Exception.Message)"
            data   = $null
            fallback = $null
        }
    }
}

function Invoke-OneProbe {
    param(
        [Parameter(Mandatory = $true)]
        [string]$AdapterPath,

        [Parameter(Mandatory = $true)]
        [string]$Mode,

        [Parameter(Mandatory = $true)]
        [string]$HostPath
    )

    $arguments = [System.Collections.Generic.List[string]]::new()
    [void]$arguments.Add('probe')
    [void]$arguments.Add('--shell')
    [void]$arguments.Add($Shell)
    [void]$arguments.Add('--iterations')
    [void]$arguments.Add('1')
    if (-not $NoProfile) {
        [void]$arguments.Add('--with-profile')
    }
    [void]$arguments.Add('--adapter-script')
    [void]$arguments.Add($AdapterPath)

    $rawLines = @(& $HostPath @arguments)
    $exitCode = $LASTEXITCODE
    $rawText = $rawLines -join [Environment]::NewLine
    if ($exitCode -ne 0) {
        throw "$Mode probe failed with exit code $exitCode. Output: $rawText"
    }
    try {
        $report = $rawText | ConvertFrom-Json
    } catch {
        throw "$Mode probe returned invalid JSON: $($_.Exception.Message)"
    }
    return [pscustomobject]@{
        report = $report
        raw    = $rawText
    }
}

$hostPath = Resolve-ExistingFile -Path $ShellSenseExecutable -Description 'ShellSense executable'
$pwshCommand = Get-Command pwsh -ErrorAction SilentlyContinue
if ($null -eq $pwshCommand) {
    throw 'pwsh is required to generate the temporary A/B adapter copies.'
}
$preparePath = Join-Path $PSScriptRoot 'prepare-ab-adapter.ps1'
$baselineAdapter = Invoke-PrepareAdapter -Mode Baseline -PreparePath $preparePath -PwshPath $pwshCommand.Source
$bridgeAdapter = Invoke-PrepareAdapter -Mode Bridge -PreparePath $preparePath -PwshPath $pwshCommand.Source

$records = [System.Collections.Generic.List[object]]::new()
$verificationFailures = [System.Collections.Generic.List[string]]::new()

for ($pairNumber = 1; $pairNumber -le $PairCount; $pairNumber++) {
    # Odd pairs run baseline first; even pairs reverse the order to reduce
    # systematic process-start and OS-cache bias.
    $order = if (($pairNumber % 2) -eq 1) {
        @('baseline', 'bridge')
    } else {
        @('bridge', 'baseline')
    }
    $runs = @{}
    foreach ($mode in $order) {
        if ($mode -eq 'bridge') {
            $proofPath = [string]$bridgeAdapter.ProofPath
            if ([IO.File]::Exists($proofPath)) {
                Remove-Item -LiteralPath $proofPath -Force
            }
            $fallbackProofPath = $proofPath + '.fallback.json'
            if ([IO.File]::Exists($fallbackProofPath)) {
                Remove-Item -LiteralPath $fallbackProofPath -Force
            }
        }
        $adapter = if ($mode -eq 'bridge') { $bridgeAdapter } else { $baselineAdapter }
        $started = [Diagnostics.Stopwatch]::StartNew()
        $invocation = Invoke-OneProbe -AdapterPath ([string]$adapter.AdapterPath) -Mode $mode -HostPath $hostPath
        $started.Stop()

        $proof = if ($mode -eq 'bridge') {
            Read-BridgeProof -Path ([string]$bridgeAdapter.ProofPath)
        } else {
            [pscustomobject]@{ valid = $true; reason = $null; data = $null; fallback = $null }
        }
        if ($mode -eq 'bridge' -and -not $proof.valid) {
            [void]$verificationFailures.Add(("pair {0}: {1}" -f $pairNumber, $proof.reason))
        }

        $report = $invocation.report
        $runs[$mode] = [ordered]@{
            mode                      = $mode
            elapsed_wall_ms           = [double]$started.Elapsed.TotalMilliseconds
            adapter_prompt_delta_ms   = Get-ProbeDelta -Report $report -Field 'paired_adapter_delta'
            first_input_delta_ms      = Get-ProbeDelta -Report $report -Field 'paired_first_input_delta'
            adapter_prompt_samples_ms = @($report.paired_adapter_delta.samples)
            first_input_samples_ms    = @($report.paired_first_input_delta.samples)
            bridge_proof               = $proof.data
            bridge_fallback            = $proof.fallback
            probe                     = $report
            raw_probe_json             = $invocation.raw
        }
    }

    $baselineRun = $runs['baseline']
    $bridgeRun = $runs['bridge']
    $promptDifference = [double]$baselineRun.adapter_prompt_delta_ms - [double]$bridgeRun.adapter_prompt_delta_ms
    $inputDifference = [double]$baselineRun.first_input_delta_ms - [double]$bridgeRun.first_input_delta_ms
    [void]$records.Add([ordered]@{
        pair                         = $pairNumber
        order                        = @($order)
        baseline_delta_ms            = [double]$baselineRun.adapter_prompt_delta_ms
        bridge_delta_ms              = [double]$bridgeRun.adapter_prompt_delta_ms
        difference_ms                = $promptDifference
        baseline_first_input_ms      = [double]$baselineRun.first_input_delta_ms
        bridge_first_input_ms        = [double]$bridgeRun.first_input_delta_ms
        first_input_difference_ms    = $inputDifference
        baseline                    = $baselineRun
        bridge                      = $bridgeRun
    })
    Write-Verbose ("Completed pair {0}/{1} ({2} first)" -f $pairNumber, $PairCount, $order[0])
}

$promptDeltas = @($records | ForEach-Object { [double]$_.difference_ms })
$baselineDeltas = @($records | ForEach-Object { [double]$_.baseline_delta_ms })
$bridgeDeltas = @($records | ForEach-Object { [double]$_.bridge_delta_ms })
$inputDifferences = @($records | ForEach-Object { [double]$_.first_input_difference_ms })
$result = [ordered]@{
    schema             = 1
    kind               = 'bridge_ab_paired_startup'
    shell              = $Shell
    shellsense         = $hostPath
    pair_count         = $PairCount
    profile_mode       = if ($NoProfile) { 'no_profile' } else { 'with_profile' }
    probe_iterations   = 1
    metric_primary     = 'paired_first_input_delta_ms (first input echo)'
    metric_secondary   = 'paired_adapter_delta_ms (prompt marker; diagnostic only)'
    ordering            = 'odd pairs baseline then bridge; even pairs bridge then baseline'
    difference          = 'baseline_delta_ms - bridge_delta_ms'
    verified            = ($verificationFailures.Count -eq 0)
    verification_errors = @($verificationFailures)
    adapters            = [ordered]@{
        baseline = $baselineAdapter
        bridge   = $bridgeAdapter
    }
    aggregate           = [ordered]@{
        baseline_delta_ms         = Get-Stats -Values $baselineDeltas
        bridge_delta_ms           = Get-Stats -Values $bridgeDeltas
        startup_difference_ms     = Get-Stats -Values $promptDeltas
        first_input_difference_ms = Get-Stats -Values $inputDifferences
    }
    raw_pairs          = @($records)
    note               = 'Each probe call used --iterations 1; no probe output file was written during a pair. The bridge copy is counted only when its one-time binding proof exists.'
}
$json = $result | ConvertTo-Json -Depth 20
if (-not [string]::IsNullOrWhiteSpace($OutputPath)) {
    $outputFile = [IO.Path]::GetFullPath($OutputPath)
    $parent = [IO.Path]::GetDirectoryName($outputFile)
    if (-not [string]::IsNullOrWhiteSpace($parent)) {
        [IO.Directory]::CreateDirectory($parent) | Out-Null
    }
    # One write after all pairs: the timing loop never writes a report per
    # query or per child process.
    [IO.File]::WriteAllText($outputFile, $json + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
}
Write-Output $json
if ($verificationFailures.Count -gt 0) {
    throw 'Bridge A/B verification failed; the report was written but bridge timings are not valid for a performance decision.'
}
