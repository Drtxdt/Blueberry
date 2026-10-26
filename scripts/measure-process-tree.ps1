[CmdletBinding()]
param(
    [Parameter(Mandatory)][int]$RootProcessId,
    [Parameter(Mandatory)][string]$OutputPath,
    [ValidateRange(10,300)][int]$Seconds = 30,
    [switch]$IncludeThreadCounters,
    [string]$ReadyAdapterDirectory = $env:BLUEBERRY_RESOURCE_READY_ADAPTER_DIRECTORY
)
$ErrorActionPreference='Stop'
if (Test-Path -LiteralPath $OutputPath) { throw 'Refusing to overwrite resource evidence' }
$rootProcess=Get-Process -Id $RootProcessId
$rootStarted=$rootProcess.StartTime.ToUniversalTime().ToString('o')
. (Join-Path $PSScriptRoot 'resource-readiness.ps1')
function Read-TreeSample {
    $all=@(Get-CimInstance Win32_Process)
    $ids=[Collections.Generic.HashSet[int]]::new()
    $null=$ids.Add($RootProcessId)
    do {
        $changed=$false
        foreach ($item in $all) {
            if ($ids.Contains([int]$item.ParentProcessId) -and $ids.Add([int]$item.ProcessId)) { $changed=$true }
        }
    } while ($changed)
    $threads=@()
    $threadError=$null
    # The performance-counter provider took about 11 seconds even with a
    # server-side filter. Keep it in independent opt-in diagnostics, so the
    # ordinary memory/CPU series can observe the intended one-second cadence.
    if ($IncludeThreadCounters) {
        $threadFilter=($ids | ForEach-Object { "IDProcess = $_" }) -join ' OR '
        try { $threads=@(Get-CimInstance Win32_PerfRawData_PerfProc_Thread -Filter $threadFilter) }
        catch { $threadError=$_.Exception.Message }
    }
    $rows=@(foreach ($item in $all | Where-Object { $ids.Contains([int]$_.ProcessId) }) {
        try {
            $process=Get-Process -Id $item.ProcessId -ErrorAction Stop
            [ordered]@{
                pid=[int]$item.ProcessId; parent_pid=[int]$item.ParentProcessId
                name=$item.Name; path=$item.ExecutablePath
                started_utc=$process.StartTime.ToUniversalTime().ToString('o')
                cpu_seconds=$process.TotalProcessorTime.TotalSeconds
                working_set_bytes=$process.WorkingSet64; private_bytes=$process.PrivateMemorySize64
                handles=$process.HandleCount; threads=$process.Threads.Count
                thread_counters=@($threads | Where-Object { $_.IDProcess -eq $item.ProcessId } | ForEach-Object {
                    @{id=[int]$_.IDThread; context_switches=[uint64]$_.ContextSwitchesPersec; state=$_.ThreadState; wait_reason=$_.ThreadWaitReason}
                })
            }
        } catch { @{pid=[int]$item.ProcessId; error=$_.Exception.Message} }
    })
    [ordered]@{qpc=[Diagnostics.Stopwatch]::GetTimestamp();utc=[DateTime]::UtcNow.ToString('o');processes=$rows;thread_counter_error=$threadError}
}
$warmup=@()
$requiredStable=if($IncludeThreadCounters) {2} else {4}
$settling=[Diagnostics.Stopwatch]::StartNew()
$stable=0
$lastIdentity=$null
do {
    $sample=Read-TreeSample
    $readiness=Get-ResourceReadiness -Directory $ReadyAdapterDirectory
    $sample['initialization']=$readiness
    $warmup+=$sample
    $identity=@($sample.processes | ForEach-Object { "$($_.pid):$($_.started_utc)" } | Sort-Object) -join ','
    $valid=(@($sample.processes | Where-Object { $_.error }).Count -eq 0 -and
        @($sample.processes | Where-Object { $_.pid -eq $RootProcessId -and $_.started_utc -eq $rootStarted }).Count -eq 1)
    $valid=$valid -and $readiness.ready
    if ($valid -and $identity -eq $lastIdentity) { $stable++ } elseif ($valid) { $stable=1 } else { $stable=0 }
    $lastIdentity=$identity
    $root=Get-Process -Id $RootProcessId -ErrorAction SilentlyContinue
    if ($stable -ge $requiredStable -or $null -eq $root -or $root.StartTime.ToUniversalTime().ToString('o') -ne $rootStarted -or $settling.Elapsed.TotalSeconds -ge 30) { break }
    Start-Sleep -Milliseconds 1000
} while ($true)
$settled=($stable -ge $requiredStable)
$warmupElapsed=$settling.Elapsed.TotalSeconds
# Initialization snapshots remain separate from the idle interval. Never
# restart or remove a timed sample if the process tree changes later.
$samples=@($warmup[-1])
$clock=[Diagnostics.Stopwatch]::StartNew()
while($settled -and $clock.Elapsed.TotalSeconds -lt $Seconds) {
    Start-Sleep -Milliseconds ([int][Math]::Min(1000,[Math]::Max(1,($Seconds-$clock.Elapsed.TotalSeconds)*1000)))
    $samples+=Read-TreeSample
    $root=Get-Process -Id $RootProcessId -ErrorAction SilentlyContinue
    if ($null -eq $root -or $root.StartTime.ToUniversalTime().ToString('o') -ne $rootStarted) { break }
}
$elapsed=($samples[-1].qpc-$samples[0].qpc)/[double][Diagnostics.Stopwatch]::Frequency
$initial=@($samples[0].processes | ForEach-Object { "$($_.pid):$($_.started_utc)" } | Sort-Object)
$errors=@()
if (-not $settled) { $errors+='Process tree did not settle within 30 seconds before idle sampling' }
foreach($sample in $samples) {
    if (@($sample.processes | Where-Object { $_.error }).Count) { $errors+='A process could not be sampled' }
    $identities=@($sample.processes | ForEach-Object { "$($_.pid):$($_.started_utc)" } | Sort-Object)
    if (($identities -join ',') -ne ($initial -join ',')) { $errors+='Process tree changed during idle sampling; CPU for exited/new processes is incomplete' }
    if (@($sample.processes | Where-Object { $_.pid -eq $RootProcessId -and $_.started_utc -eq $rootStarted }).Count -ne 1) { $errors+='Root process identity changed or exited' }
}
if ($elapsed -lt $Seconds) { $errors+='Resource interval is incomplete' }
if ($null -eq $root -or $root.StartTime.ToUniversalTime().ToString('o') -ne $rootStarted) { $errors+='Root process identity changed or exited' }
$delta=@(foreach ($current in $samples[-1].processes) {
    $previous=@($samples[0].processes | Where-Object { $_.pid -eq $current.pid -and $_.started_utc -eq $current.started_utc })
    if ($elapsed -le 0 -or $previous.Count -ne 1 -or $current.error -or $previous[0].error) { continue }
    $switches=0L
    $matchedThreads=0
    foreach ($thread in $current.thread_counters) {
        $old=@($previous[0].thread_counters | Where-Object { $_.id -eq $thread.id })
        if ($old.Count -eq 1 -and $thread.context_switches -ge $old[0].context_switches) {
            $switches += $thread.context_switches-$old[0].context_switches
            $matchedThreads++
        }
    }
    if ($current.cpu_seconds -lt $previous[0].cpu_seconds) { $errors+='Process CPU interval is incomplete'; continue }
    @{pid=$current.pid;name=$current.name;cpu_percent_one_core=100*($current.cpu_seconds-$previous[0].cpu_seconds)/$elapsed;
      context_switches=$(if($IncludeThreadCounters) {$switches} else {$null});matched_threads=$matchedThreads;
      handle_delta=$current.handles-$previous[0].handles;private_bytes_delta=$current.private_bytes-$previous[0].private_bytes}
})
$result=[ordered]@{
    schema=2;root_pid=$RootProcessId;root_started_utc=$rootStarted;elapsed_seconds=$elapsed
    qpc_frequency=[Diagnostics.Stopwatch]::Frequency;requested_seconds=$Seconds
    measurement_complete=($errors.Count -eq 0);collection_errors=@($errors | Select-Object -Unique)
    sampler_sha256=(Get-FileHash -LiteralPath $PSCommandPath).Hash
    warmup_complete=$settled;warmup_required_stable_snapshots=$requiredStable
    warmup_elapsed_seconds=$warmupElapsed;warmup_samples=$warmup
    readiness_directory=$ReadyAdapterDirectory
    readiness_sha256=(Get-FileHash -LiteralPath (Join-Path $PSScriptRoot 'resource-readiness.ps1')).Hash
    thread_counters_measured=$IncludeThreadCounters.IsPresent
    diagnostic_only=$IncludeThreadCounters.IsPresent
    context_switch_note=$(if($IncludeThreadCounters) {'Raw per-thread context-switch counters are a scheduling proxy, not an exact count of wait-handle signals. Exited/new threads remain in snapshots and are excluded from deltas.'} else {'Thread context-switch counts are not measured. Use a separate -IncludeThreadCounters diagnostic batch.'})
    samples=$samples;deltas=$delta
}
$result | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $OutputPath -Encoding utf8
if ($errors.Count) { throw 'Resource collection invalid; raw snapshots and reasons were retained' }
