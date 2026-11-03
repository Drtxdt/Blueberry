[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Shell,
    [Parameter(Mandatory)][string]$Baseline,
    [Parameter(Mandatory)][string]$Candidate,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [ValidateRange(30,1000)][int]$Pairs=30
)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
# The reused entry fixture enforces the disposable-runner boundary again.
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
    throw 'Formal automatic-profile comparison requires a disposable GitHub-hosted Windows runner.'
}
$root=[IO.Path]::GetFullPath($OutputDirectory)
if(Test-Path -LiteralPath $root) { throw 'Evidence directory must be new.' }
$null=New-Item -ItemType Directory -Path $root
$executables=@{ baseline=(Resolve-Path $Baseline).Path; candidate=(Resolve-Path $Candidate).Path }
$hashes=@{}
foreach($side in @('baseline','candidate')) { $hashes[$side]=(Get-FileHash $executables[$side]).Hash.ToLowerInvariant() }
$outcomes=@()
$summaries=@()
function Get-Stats([double[]]$Values) {
    $sorted=@($Values | Sort-Object)
    return @{ samples=$Values; median=($sorted[[int][Math]::Floor(($sorted.Count-1)/2)]+$sorted[[int][Math]::Floor($sorted.Count/2)])/2; p95=$sorted[[int][Math]::Ceiling($sorted.Count*0.95)-1]; unit='ms' }
}
foreach($batch in 1..2) {
    foreach($cache in @('cold','warm')) {
        $valid=@()
        $attempt=0
        while($valid.Count -lt $Pairs) {
            $attempt++
            if($attempt -gt $Pairs*2) { throw 'Too many invalid pairs; raw evidence retained.' }
            # Reverse the order both between adjacent pairs and between batches.
            $order=if(($attempt+$batch)%2 -eq 0) { @('baseline','candidate') } else { @('candidate','baseline') }
            $reports=@{}
            $pairRoot=Join-Path $root "batch-$batch-$cache-attempt-$attempt"
            $null=New-Item -ItemType Directory -Path $pairRoot
            $failure=$null
            $invoked=@()
            try {
                foreach($side in $order) {
                    if((Get-FileHash $executables[$side]).Hash.ToLowerInvariant() -ne $hashes[$side]) { throw 'Executable changed during comparison' }
                    $directory=Join-Path $pairRoot $side
                    $invoked+=$side
                    & "$PSScriptRoot/test-ci-profile-entry.ps1" -Shell $Shell -Executable $executables[$side] -ProbeExecutable $executables.candidate -OutputDirectory $directory -Pairs 1 -CacheState $cache *> (Join-Path $pairRoot "$side.log")
                    $report=Get-Content (Join-Path $directory 'diagnostic.json') -Raw | ConvertFrom-Json
                    $restoration=Get-Content (Join-Path $directory 'restoration.json') -Raw | ConvertFrom-Json
                    if($report.diagnostic_only -ne $false -or $report.build -ne 'release' -or $report.trace -ne 'disabled' -or
                        $report.actual_entry -ne 'isolated_automatic_profile' -or $report.local_cache_state -ne $cache -or
                        $report.local_appdata_isolated -ne $true -or $report.editor_eligible -ne $true -or
                        $report.executable_sha256 -ne $hashes[$side] -or $restoration.restored -ne $true -or
                        @($report.paired_first_input_delta.samples).Count -ne 1) { throw 'Ineligible timing sample' }
                    $reports[$side]=$report
                }
                foreach($field in @('shell_versions','psreadline_versions','actual_transports','actual_host_modes')) {
                    if(($reports.baseline.$field | ConvertTo-Json -Compress) -ne ($reports.candidate.$field | ConvertTo-Json -Compress)) { throw "Paired identity differs: $field" }
                }
            } catch {
                $failure=$_.Exception.Message
            }
            $outcome=@{ batch=$batch; cache=$cache; attempt=$attempt; order=$order; valid=($null -eq $failure); error=$failure; evidence=$pairRoot }
            $outcomes+=$outcome
            ConvertTo-Json -InputObject @($outcomes) -Depth 8 | Set-Content (Join-Path $root 'attempts.json') -Encoding utf8
            if($failure) {
                # Never continue after a profile restoration failure.
                foreach($side in $invoked) {
                    $restorationPath=Join-Path $pairRoot "$side/restoration.json"
                    if(-not (Test-Path $restorationPath) -or (Get-Content $restorationPath -Raw | ConvertFrom-Json).restored -ne $true) { throw 'Profile restoration unverified; comparison stopped' }
                }
                continue
            }
            $valid+=,$reports
        }
        $metrics=@{}
        foreach($field in @('candidate_first_prompt','candidate_first_input','first_key_echo','first_visible_static_candidate','complete_static_candidates','first_visible_dynamic_candidate','complete_dynamic_candidates','paired_first_input_delta')) {
            $before=@($valid | ForEach-Object { [double]$_.baseline.$field.samples[0] })
            $after=@($valid | ForEach-Object { [double]$_.candidate.$field.samples[0] })
            $deltas=for($i=0; $i -lt $Pairs; $i++) { $after[$i]-$before[$i] }
            $metrics[$field]=@{ baseline=(Get-Stats $before); candidate=(Get-Stats $after); candidate_minus_baseline=(Get-Stats $deltas) }
        }
        $summaries+=@{ batch=$batch; cache=$cache; valid_pairs=$valid.Count; invalid_pairs=($attempt-$valid.Count); metrics=$metrics; startup_target_p50_ms=50; startup_target_met=($metrics.paired_first_input_delta.candidate.median -le 50) }
        @{ schema=1; formal=$true; executable_sha256=$hashes; observer_sha256=$hashes.candidate; os_cache_cleared=$false; batches=$summaries } | ConvertTo-Json -Depth 15 | Set-Content (Join-Path $root 'summary.json') -Encoding utf8
    }
}
