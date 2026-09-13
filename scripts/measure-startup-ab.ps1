[CmdletBinding()]
param(
    # When omitted, look for the release executable in the repository and then
    # for a blueberry application on PATH.  Formal measurements never fall
    # back to a debug build; probe reports are required to say build=release.
    [string]$BlueberryExecutable = '',

    [string]$Shell = 'pwsh.exe',

    # These are passed through to `probe`.  An omitted adapter means that the
    # executable's embedded adapter is the side being measured.
    [string]$BaselineAdapter = '',

    [string]$CandidateAdapter = '',

    [ValidateScript({ $_ -ge 30 })]
    [int]$PairCount = 30,

    [Parameter(Mandatory = $true)]
    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$scriptSchema = 1
$scriptKind = 'powershell_adapter_ab_paired_startup'
$probeTimeout = [TimeSpan]::FromMinutes(2)

function Resolve-ExistingFile {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [string]$Description
    )

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

function Resolve-Blueberry {
    param(
        [string]$RequestedPath
    )

    if (-not [string]::IsNullOrWhiteSpace($RequestedPath)) {
        return Resolve-ExistingFile -Path $RequestedPath -Description 'Blueberry executable'
    }

    $localCandidates = @(
        (Join-Path $PSScriptRoot '..\target\release\blueberry.exe')
    )
    foreach ($candidate in $localCandidates) {
        if ([IO.File]::Exists($candidate)) {
            return Resolve-ExistingFile -Path $candidate -Description 'Blueberry executable'
        }
    }

    foreach ($commandName in @('blueberry.exe', 'blueberry')) {
        $command = Get-Command $commandName -CommandType Application -ErrorAction SilentlyContinue
        if ($null -ne $command -and -not [string]::IsNullOrWhiteSpace([string]$command.Source)) {
            return Resolve-ExistingFile -Path ([string]$command.Source) -Description 'Blueberry executable'
        }
    }

    throw 'Blueberry executable was not supplied and no target/release or PATH executable was found.'
}

function Resolve-OptionalAdapter {
    param(
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [string]$Description
    )

    if ([string]::IsNullOrWhiteSpace($Path)) {
        return $null
    }
    return Resolve-ExistingFile -Path $Path -Description $Description
}

function Resolve-ShellForProbe {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    $looksLikePath = [IO.Path]::IsPathRooted($Name) -or
        $Name.Contains([IO.Path]::DirectorySeparatorChar) -or
        $Name.Contains([IO.Path]::AltDirectorySeparatorChar) -or
        $Name.StartsWith('.')
    if ($looksLikePath) {
        return Resolve-ExistingFile -Path $Name -Description 'Shell executable'
    }

    $command = Get-Command $Name -CommandType Application -ErrorAction SilentlyContinue
    if ($null -eq $command -or [string]::IsNullOrWhiteSpace([string]$command.Source)) {
        throw "Shell executable was not found: $Name"
    }
    return Resolve-ExistingFile -Path ([string]$command.Source) -Description 'Shell executable'
}

function Resolve-AbsolutePath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $candidate = if ([IO.Path]::IsPathRooted($Path)) {
        $Path
    } else {
        Join-Path (Get-Location).ProviderPath $Path
    }
    return [IO.Path]::GetFullPath($candidate)
}

function Get-PropertyValue {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Object,

        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) {
        return $null
    }
    return $property.Value
}

function Get-Sha256 {
    param(
        [string]$Path
    )

    if ([string]::IsNullOrWhiteSpace($Path)) {
        return $null
    }
    $hash = Get-FileHash -LiteralPath $Path -Algorithm SHA256
    return ([string]$hash.Hash).ToUpperInvariant()
}

function Normalize-Hash {
    param(
        [object]$Value
    )

    if ($null -eq $Value -or [string]::IsNullOrWhiteSpace([string]$Value)) {
        return $null
    }
    return ([string]$Value).ToUpperInvariant()
}

function Assert-InputHashes {
    param(
        [Parameter(Mandatory = $true)][string]$ExecutablePath,
        [Parameter(Mandatory = $true)][string]$ExpectedExecutableSha256,
        [string]$BaselineAdapterPath,
        [string]$ExpectedBaselineAdapterSha256,
        [string]$CandidateAdapterPath,
        [string]$ExpectedCandidateAdapterSha256
    )

    $currentExecutableSha256 = Normalize-Hash (Get-Sha256 -Path $ExecutablePath)
    $expectedExecutableSha256 = Normalize-Hash $ExpectedExecutableSha256
    if ($currentExecutableSha256 -ine $expectedExecutableSha256) {
        throw "Blueberry executable changed during this run; expected SHA-256 $ExpectedExecutableSha256 but found $currentExecutableSha256."
    }
    $currentBaselineSha256 = Normalize-Hash (Get-Sha256 -Path $BaselineAdapterPath)
    $expectedBaselineSha256 = Normalize-Hash $ExpectedBaselineAdapterSha256
    if ($currentBaselineSha256 -ine $expectedBaselineSha256) {
        throw "Baseline adapter changed during this run; expected SHA-256 $ExpectedBaselineAdapterSha256 but found $currentBaselineSha256."
    }
    $currentCandidateSha256 = Normalize-Hash (Get-Sha256 -Path $CandidateAdapterPath)
    $expectedCandidateSha256 = Normalize-Hash $ExpectedCandidateAdapterSha256
    if ($currentCandidateSha256 -ine $expectedCandidateSha256) {
        throw "Candidate adapter changed during this run; expected SHA-256 $ExpectedCandidateAdapterSha256 but found $currentCandidateSha256."
    }
}

function Read-JsonFile {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [string]$Description
    )

    if (-not [IO.File]::Exists($Path)) {
        throw "$Description was not written: $Path"
    }
    $raw = [IO.File]::ReadAllText($Path, [Text.UTF8Encoding]::new($false))
    if ([string]::IsNullOrWhiteSpace($raw)) {
        throw "$Description is empty: $Path"
    }
    try {
        $value = $raw | ConvertFrom-Json -Depth 100
    } catch {
        throw "$Description is not valid UTF-8 JSON: $($_.Exception.Message)"
    }
    return [pscustomobject]@{
        value = $value
        raw   = $raw
    }
}

function Get-MetricMedian {
    param(
        [Parameter(Mandatory = $true)]
        [object]$ProbeReport,

        [Parameter(Mandatory = $true)]
        [string]$Mode
    )

    $metric = Get-PropertyValue -Object $ProbeReport -Name 'paired_first_input_delta'
    if ($null -eq $metric) {
        throw "$Mode probe report does not contain paired_first_input_delta."
    }
    $medianValue = Get-PropertyValue -Object $metric -Name 'median'
    if ($null -eq $medianValue -or [string]::IsNullOrWhiteSpace([string]$medianValue)) {
        throw "$Mode probe report has no paired_first_input_delta.median."
    }
    try {
        $median = [double]$medianValue
    } catch {
        throw "$Mode paired_first_input_delta.median is not numeric: $medianValue"
    }
    if ([double]::IsNaN($median) -or [double]::IsInfinity($median)) {
        throw "$Mode paired_first_input_delta.median is not finite: $medianValue"
    }
    return $median
}

function Validate-ProbeReport {
    param(
        [Parameter(Mandatory = $true)]
        [object]$ProbeReport,

        [Parameter(Mandatory = $true)]
        [string]$Mode,

        [Parameter(Mandatory = $true)]
        [string]$ExpectedAdapter
    )

    $build = Get-PropertyValue -Object $ProbeReport -Name 'build'
    $profileMode = Get-PropertyValue -Object $ProbeReport -Name 'profile_mode'
    $noProfile = Get-PropertyValue -Object $ProbeReport -Name 'no_profile'
    $iterations = Get-PropertyValue -Object $ProbeReport -Name 'iterations'
    $adapterSource = [string](Get-PropertyValue -Object $ProbeReport -Name 'adapter_source')
    if ($build -ne 'release') {
        throw "$Mode probe must report build=release; got '$build'."
    }
    if ($profileMode -ne 'with_profile' -or $noProfile -ne $false) {
        throw "$Mode probe must report profile_mode=with_profile and no_profile=false."
    }
    if ($iterations -ne 1) {
        throw "$Mode probe must report iterations=1; got '$iterations'."
    }
    if ($adapterSource -ine $ExpectedAdapter) {
        throw "$Mode probe adapter_source does not match the requested adapter; expected '$ExpectedAdapter' but got '$adapterSource'."
    }
}

function Get-Stats {
    param(
        [Parameter(Mandatory = $true)]
        [object[]]$Values
    )

    $numbers = @($Values | ForEach-Object { [double]$_ })
    if ($numbers.Count -eq 0) {
        return [ordered]@{
            samples = @()
            median  = $null
            p95     = $null
            unit    = 'ms'
        }
    }

    $sorted = @($numbers | Sort-Object)
    $count = $sorted.Count
    if (($count % 2) -eq 1) {
        $median = [double]$sorted[[int][math]::Floor($count / 2)]
    } else {
        # Standard median: average the two middle values for an even count.
        $median = ([double]$sorted[($count / 2) - 1] + [double]$sorted[$count / 2]) / 2.0
    }

    # Nearest-rank P95: ceil(0.95 * N), with a one-based rank.
    $rank = [int][math]::Ceiling(0.95 * $count)
    $p95 = [double]$sorted[$rank - 1]
    return [ordered]@{
        samples = @($numbers)
        median  = $median
        p95     = $p95
        unit    = 'ms'
    }
}

function Invoke-ProbeOnce {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ExecutablePath,

        [Parameter(Mandatory = $true)]
        [string]$ShellName,

        [Parameter(Mandatory = $true)]
        [string]$Mode,

        [string]$AdapterPath,

        [Parameter(Mandatory = $true)]
        [string]$ProbeOutputPath
    )

    $arguments = [System.Collections.Generic.List[string]]::new()
    [void]$arguments.Add('probe')
    [void]$arguments.Add('--shell')
    [void]$arguments.Add($ShellName)
    [void]$arguments.Add('--with-profile')
    [void]$arguments.Add('--iterations')
    [void]$arguments.Add('1')
    if (-not [string]::IsNullOrWhiteSpace($AdapterPath)) {
        [void]$arguments.Add('--adapter-script')
        [void]$arguments.Add($AdapterPath)
    }
    [void]$arguments.Add('--output')
    [void]$arguments.Add($ProbeOutputPath)

    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $ExecutablePath
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
    $startInfo.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
    # A caller's diagnostic environment must not silently change a formal
    # sample.  The probe itself also disables history in its child shell.
    $startInfo.Environment['BLUEBERRY_TRACE'] = '0'
    $startInfo.Environment['BLUEBERRY_PROBE_TOKEN'] = ''
    foreach ($argument in $arguments) {
        [void]$startInfo.ArgumentList.Add($argument)
    }

    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    try {
        if (-not $process.Start()) {
            throw "$Mode probe process did not start."
        }
    } catch {
        throw "Unable to start $Mode probe: $($_.Exception.Message)"
    }

    # Read both redirected streams asynchronously so a verbose or diagnostic
    # child cannot block while its other pipe is full.
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $finished = $process.WaitForExit([int][math]::Ceiling($probeTimeout.TotalMilliseconds))
    if (-not $finished) {
        try {
            $process.Kill($true)
        } catch {
            Write-Verbose ("Unable to terminate timed-out {0} probe: {1}" -f $Mode, $_.Exception.Message)
        }
        $process.WaitForExit()
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        $process.Dispose()
        throw "$Mode probe timed out after $($probeTimeout.TotalSeconds) seconds."
    }
    $stdout = $stdoutTask.GetAwaiter().GetResult()
    $stderr = $stderrTask.GetAwaiter().GetResult()
    $exitCode = $process.ExitCode
    $process.Dispose()

    if ($exitCode -ne 0) {
        $detail = ($stderr + $stdout).Trim()
        if ([string]::IsNullOrWhiteSpace($detail)) {
            $detail = 'the probe produced no diagnostic output'
        }
        throw "$Mode probe failed with exit code ${exitCode}: $detail"
    }

    $parsed = Read-JsonFile -Path $ProbeOutputPath -Description "$Mode probe output"
    $expectedAdapter = if ([string]::IsNullOrWhiteSpace($AdapterPath)) { 'embedded' } else { $AdapterPath }
    Validate-ProbeReport -ProbeReport $parsed.value -Mode $Mode -ExpectedAdapter $expectedAdapter
    $median = Get-MetricMedian -ProbeReport $parsed.value -Mode $Mode
    return [ordered]@{
        mode                              = $Mode
        adapter                           = if ([string]::IsNullOrWhiteSpace($AdapterPath)) { 'embedded' } else { $AdapterPath }
        output_path                       = $ProbeOutputPath
        exit_code                         = $exitCode
        elapsed_wall_ms                   = $null
        paired_first_input_delta_median_ms = $median
        probe                             = $parsed.value
        raw_probe_json                    = $parsed.raw
        stdout                            = $stdout
        stderr                            = $stderr
    }
}

function Write-Checkpoint {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Report,

        [Parameter(Mandatory = $true)]
        [string]$Destination
    )

    $json = $Report | ConvertTo-Json -Depth 100
    $destinationDirectory = [IO.Path]::GetDirectoryName($Destination)
    $temporaryPath = Join-Path $destinationDirectory ('.{0}.{1}.checkpoint.tmp' -f [IO.Path]::GetFileName($Destination), [guid]::NewGuid().ToString('N'))
    # Write beside the destination, then rename it, so a failed write cannot
    # destroy the previous complete checkpoint.
    [IO.File]::WriteAllText($temporaryPath, $json + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
    [IO.File]::Move($temporaryPath, $Destination, $true)
}

function Write-FailureArtifact {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Destination,

        [Parameter(Mandatory = $true)]
        [string]$TemporaryDirectory,

        [Parameter(Mandatory = $true)]
        [int]$Pair,

        [Parameter(Mandatory = $true)]
        [object[]]$Attempts,

        [Parameter(Mandatory = $true)]
        [string]$Message
    )

    $failurePath = "$Destination.failed.json"
    $payload = [ordered]@{
        schema              = $scriptSchema
        kind                = $scriptKind
        complete            = $false
        failed_pair         = $Pair
        error               = $Message
        temporary_directory = $TemporaryDirectory
        attempts            = @($Attempts)
        note                = 'The main output remains the last completed checkpoint. Raw per-probe files are retained in temporary_directory.'
    }
    try {
        $json = $payload | ConvertTo-Json -Depth 100
        [IO.File]::WriteAllText($failurePath, $json + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
    } catch {
        Write-Verbose ("Unable to write failure artifact {0}: {1}" -f $failurePath, $_.Exception.Message)
    }
}

function New-Report {
    param(
        [Parameter(Mandatory = $true)]
        [object[]]$Samples,

        [Parameter(Mandatory = $true)]
        [int]$RequestedPairs,

        [Parameter(Mandatory = $true)]
        [string]$ExecutablePath,

        [Parameter(Mandatory = $true)]
        [string]$ExecutableSha256,

        [Parameter(Mandatory = $true)]
        [string]$ShellName,

        [string]$BaselineAdapterPath,

        [string]$BaselineAdapterSha256,

        [string]$CandidateAdapterPath,

        [string]$CandidateAdapterSha256,

        [Parameter(Mandatory = $true)]
        [string]$TemporaryDirectory
    )

    $baselineValues = @($Samples | ForEach-Object { [double]$_.baseline_metric_ms })
    $candidateValues = @($Samples | ForEach-Object { [double]$_.candidate_metric_ms })
    $gainValues = @($Samples | ForEach-Object { [double]$_.gain_ms })
    $isComplete = ($Samples.Count -eq $RequestedPairs)
    return [ordered]@{
        schema             = $scriptSchema
        kind               = $scriptKind
        complete           = $isComplete
        checkpoint_complete = $true
        requested_pair_count = $RequestedPairs
        completed_pair_count = $Samples.Count
        blueberry_executable = $ExecutablePath
        shell              = $ShellName
        build              = 'release'
        no_profile         = $false
        sha256             = [ordered]@{
            blueberry_executable = $ExecutableSha256
            baseline_adapter      = $BaselineAdapterSha256
            candidate_adapter     = $CandidateAdapterSha256
        }
        profile_mode       = 'with_profile'
        trace              = 'disabled; no trace option is passed'
        probe_iterations   = 1
        probe_timeout_seconds = [int]$probeTimeout.TotalSeconds
        adapters            = [ordered]@{
            baseline = if ([string]::IsNullOrWhiteSpace($BaselineAdapterPath)) { 'embedded' } else { $BaselineAdapterPath }
            candidate = if ([string]::IsNullOrWhiteSpace($CandidateAdapterPath)) { 'embedded' } else { $CandidateAdapterPath }
        }
        metric_primary     = 'paired_first_input_delta.median'
        difference         = 'baseline - candidate; positive gain means the candidate is faster (ms)'
        ordering           = 'odd pairs baseline then candidate; even pairs candidate then baseline'
        method             = 'Each side invokes the same executable with probe --with-profile --iterations 1 and an independent --output JSON path. Adapter files are passed through to probe only; omitted adapters use the executable embedded adapter. Input SHA-256 values are rechecked before and after every side. A hung probe is terminated after the recorded timeout. The script does not edit profiles or settings, does not pass trace, and does not run project scripts.'
        temporary_directory = $TemporaryDirectory
        primary_metric     = [ordered]@{
            baseline = Get-Stats -Values $baselineValues
            candidate = Get-Stats -Values $candidateValues
            gain = Get-Stats -Values $gainValues
        }
        # samples contains one completed pair per element.  Each side retains
        # the complete parsed probe report, exact JSON, and child output.
        samples            = @($Samples)
        updated_utc        = [DateTime]::UtcNow.ToString('o')
    }
}

if ($PairCount -lt 30) {
    throw 'PairCount must be at least 30.'
}
if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    throw 'OutputPath is required and cannot be empty.'
}
if ([string]::IsNullOrWhiteSpace($Shell)) {
    throw 'Shell cannot be empty.'
}

$hostPath = Resolve-Blueberry -RequestedPath $BlueberryExecutable
$shellPath = Resolve-ShellForProbe -Name $Shell
$baselineAdapterPath = Resolve-OptionalAdapter -Path $BaselineAdapter -Description 'BaselineAdapter'
$candidateAdapterPath = Resolve-OptionalAdapter -Path $CandidateAdapter -Description 'CandidateAdapter'
$hostSha256 = Get-Sha256 -Path $hostPath
$baselineAdapterSha256 = Get-Sha256 -Path $baselineAdapterPath
$candidateAdapterSha256 = Get-Sha256 -Path $candidateAdapterPath
$outputFile = Resolve-AbsolutePath -Path $OutputPath
$outputDirectory = [IO.Path]::GetDirectoryName($outputFile)
if ([string]::IsNullOrWhiteSpace($outputDirectory)) {
    throw "Unable to determine the output directory for $outputFile"
}
[IO.Directory]::CreateDirectory($outputDirectory) | Out-Null
if ([IO.Directory]::Exists($outputFile)) {
    throw "OutputPath is a directory: $outputFile"
}

# Keep every independent probe JSON in a run directory.  It is intentionally
# not removed on success or failure: the final report embeds each raw result,
# while the files make a failed child or malformed report auditable.
$temporaryDirectory = Join-Path ([IO.Path]::GetTempPath()) ('blueberry-startup-ab-' + [guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($temporaryDirectory) | Out-Null

$records = [System.Collections.Generic.List[object]]::new()
$existingRaw = $null
if ([IO.File]::Exists($outputFile)) {
    $existing = Read-JsonFile -Path $outputFile -Description 'Existing startup A/B checkpoint'
    $existingRaw = $existing.raw
    $existingKind = Get-PropertyValue -Object $existing.value -Name 'kind'
    $existingSchema = Get-PropertyValue -Object $existing.value -Name 'schema'
    if ($existingKind -ne $scriptKind -or [int]$existingSchema -ne $scriptSchema) {
        throw "OutputPath already contains a different report; refusing to overwrite it: $outputFile"
    }

    $existingShell = [string](Get-PropertyValue -Object $existing.value -Name 'shell')
    $existingExecutable = [string](Get-PropertyValue -Object $existing.value -Name 'blueberry_executable')
    $existingRequested = [int](Get-PropertyValue -Object $existing.value -Name 'requested_pair_count')
    $existingAdapters = Get-PropertyValue -Object $existing.value -Name 'adapters'
    $existingBaseline = [string](Get-PropertyValue -Object $existingAdapters -Name 'baseline')
    $existingCandidate = [string](Get-PropertyValue -Object $existingAdapters -Name 'candidate')
    $existingHashes = Get-PropertyValue -Object $existing.value -Name 'sha256'
    if ($null -eq $existingHashes) {
        throw 'Existing startup A/B checkpoint has no input hashes; use a new OutputPath.'
    }
    $existingHostSha256 = Normalize-Hash (Get-PropertyValue -Object $existingHashes -Name 'blueberry_executable')
    $existingBaselineSha256 = Normalize-Hash (Get-PropertyValue -Object $existingHashes -Name 'baseline_adapter')
    $existingCandidateSha256 = Normalize-Hash (Get-PropertyValue -Object $existingHashes -Name 'candidate_adapter')
    $expectedBaseline = if ($null -eq $baselineAdapterPath) { 'embedded' } else { $baselineAdapterPath }
    $expectedCandidate = if ($null -eq $candidateAdapterPath) { 'embedded' } else { $candidateAdapterPath }
    if ($existingShell -ine $shellPath -or
        $existingExecutable -ine $hostPath -or
        $existingRequested -ne $PairCount -or
        $existingBaseline -ine $expectedBaseline -or
        $existingCandidate -ine $expectedCandidate -or
        $existingHostSha256 -ine $hostSha256 -or
        $existingBaselineSha256 -ine $baselineAdapterSha256 -or
        $existingCandidateSha256 -ine $candidateAdapterSha256) {
        throw 'Existing startup A/B checkpoint parameters do not match this invocation; use a new OutputPath.'
    }

    $existingSamples = @(Get-PropertyValue -Object $existing.value -Name 'samples')
    if ($existingSamples.Count -gt $PairCount) {
        throw 'Existing startup A/B checkpoint contains more samples than PairCount.'
    }
    foreach ($sample in $existingSamples) {
        $sampleComplete = Get-PropertyValue -Object $sample -Name 'complete'
        if ($sampleComplete -ne $true) {
            throw 'Existing startup A/B checkpoint contains an incomplete pair; refusing to discard it.'
        }
        [void]$records.Add($sample)
    }

    $existingComplete = Get-PropertyValue -Object $existing.value -Name 'complete'
    if ($existingComplete -eq $true -and $records.Count -eq $PairCount) {
        # A completed report is already the requested measurement.  Preserve
        # its exact bytes and avoid silently measuring a second data set.
        Write-Output $existingRaw
        return
    }
}

$firstPair = $records.Count + 1
$activePair = $firstPair
$attempts = [System.Collections.Generic.List[object]]::new()
try {
    for ($pairNumber = $firstPair; $pairNumber -le $PairCount; $pairNumber++) {
        $activePair = $pairNumber
        Assert-InputHashes `
            -ExecutablePath $hostPath `
            -ExpectedExecutableSha256 $hostSha256 `
            -BaselineAdapterPath $baselineAdapterPath `
            -ExpectedBaselineAdapterSha256 $baselineAdapterSha256 `
            -CandidateAdapterPath $candidateAdapterPath `
            -ExpectedCandidateAdapterSha256 $candidateAdapterSha256
        # Reverse order on alternating pairs to reduce process-start and OS
        # cache ordering bias while keeping the same executable and arguments.
        $order = if (($pairNumber % 2) -eq 1) {
            @('baseline', 'candidate')
        } else {
            @('candidate', 'baseline')
        }
        $runs = @{}
        $attempts = [System.Collections.Generic.List[object]]::new()

        foreach ($mode in $order) {
            $adapterPath = if ($mode -eq 'baseline') { $baselineAdapterPath } else { $candidateAdapterPath }
            Assert-InputHashes `
                -ExecutablePath $hostPath `
                -ExpectedExecutableSha256 $hostSha256 `
                -BaselineAdapterPath $baselineAdapterPath `
                -ExpectedBaselineAdapterSha256 $baselineAdapterSha256 `
                -CandidateAdapterPath $candidateAdapterPath `
                -ExpectedCandidateAdapterSha256 $candidateAdapterSha256
            $runOutputPath = Join-Path $temporaryDirectory ('pair-{0:D4}-{1}-{2}.json' -f $pairNumber, $mode, [guid]::NewGuid().ToString('N'))
            [void]$attempts.Add([ordered]@{
                pair        = $pairNumber
                mode        = $mode
                adapter     = if ($null -eq $adapterPath) { 'embedded' } else { $adapterPath }
                output_path = $runOutputPath
            })

            $started = [Diagnostics.Stopwatch]::StartNew()
            $run = Invoke-ProbeOnce `
                -ExecutablePath $hostPath `
                -ShellName $shellPath `
                -Mode $mode `
                -AdapterPath ([string]$adapterPath) `
                -ProbeOutputPath $runOutputPath
            $started.Stop()
            Assert-InputHashes `
                -ExecutablePath $hostPath `
                -ExpectedExecutableSha256 $hostSha256 `
                -BaselineAdapterPath $baselineAdapterPath `
                -ExpectedBaselineAdapterSha256 $baselineAdapterSha256 `
                -CandidateAdapterPath $candidateAdapterPath `
                -ExpectedCandidateAdapterSha256 $candidateAdapterSha256
            $run['elapsed_wall_ms'] = [double]$started.Elapsed.TotalMilliseconds
            $attemptRecord = $attempts[$attempts.Count - 1]
            $attemptRecord['complete'] = $true
            $attemptRecord['result'] = $run
            $runs[$mode] = $run
        }

        $baselineRun = $runs['baseline']
        $candidateRun = $runs['candidate']
        $baselineMetric = [double]$baselineRun['paired_first_input_delta_median_ms']
        $candidateMetric = [double]$candidateRun['paired_first_input_delta_median_ms']
        $gain = $baselineMetric - $candidateMetric
        # No outlier filtering or sign-based removal: negative gains and slow
        # samples are part of the evidence and remain in samples[].
        $pairRecord = [ordered]@{
            pair               = $pairNumber
            order              = @($order)
            complete            = $true
            baseline_metric_ms = $baselineMetric
            candidate_metric_ms = $candidateMetric
            gain_ms            = $gain
            baseline           = $baselineRun
            candidate          = $candidateRun
        }
        [void]$records.Add($pairRecord)

        $checkpoint = New-Report `
            -Samples @($records) `
            -RequestedPairs $PairCount `
            -ExecutablePath $hostPath `
            -ExecutableSha256 $hostSha256 `
            -ShellName $shellPath `
            -BaselineAdapterPath $baselineAdapterPath `
            -BaselineAdapterSha256 $baselineAdapterSha256 `
            -CandidateAdapterPath $candidateAdapterPath `
            -CandidateAdapterSha256 $candidateAdapterSha256 `
            -TemporaryDirectory $temporaryDirectory
        Write-Checkpoint -Report $checkpoint -Destination $outputFile
        Write-Verbose ("Completed startup A/B pair {0}/{1} ({2} first); checkpoint saved to {3}" -f $pairNumber, $PairCount, $order[0], $outputFile)
    }
} catch {
    $attemptList = @($attempts)
    Write-FailureArtifact `
        -Destination $outputFile `
        -TemporaryDirectory $temporaryDirectory `
        -Pair $activePair `
        -Attempts $attemptList `
        -Message $_.Exception.Message
    # The checkpoint was written only after both sides of a pair succeeded;
    # therefore the previous samples remain intact.  Re-throw to make a
    # failed measurement visible to CI or the calling shell.
    throw
}

$final = Read-JsonFile -Path $outputFile -Description 'Final startup A/B checkpoint'
Write-Output $final.raw
