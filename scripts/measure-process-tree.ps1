[CmdletBinding()]
param(
    [Parameter(Mandatory)][int]$RootProcessId,
    [Parameter(Mandatory)][string]$OutputPath,
    [ValidateRange(10,300)][int]$Seconds = 30
)
$ErrorActionPreference='Stop'
if (Test-Path -LiteralPath $OutputPath) { throw 'Refusing to overwrite resource evidence' }
$rootProcess=Get-Process -Id $RootProcessId
$rootStarted=$rootProcess.StartTime.ToUniversalTime().ToString('o')
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
    try { $threads=@(Get-CimInstance Win32_PerfRawData_PerfProc_Thread | Where-Object { $ids.Contains([int]$_.IDProcess) }) }
    catch { $threadError=$_.Exception.Message }
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
$samples=@(Read-TreeSample)
Start-Sleep -Seconds $Seconds
$samples+=Read-TreeSample
$elapsed=($samples[1].qpc-$samples[0].qpc)/[double][Diagnostics.Stopwatch]::Frequency
$delta=@(foreach ($current in $samples[1].processes) {
    $previous=@($samples[0].processes | Where-Object { $_.pid -eq $current.pid -and $_.started_utc -eq $current.started_utc })
    if ($previous.Count -ne 1 -or $current.error -or $previous[0].error) { continue }
    $switches=0L
    $matchedThreads=0
    foreach ($thread in $current.thread_counters) {
        $old=@($previous[0].thread_counters | Where-Object { $_.id -eq $thread.id })
        if ($old.Count -eq 1 -and $thread.context_switches -ge $old[0].context_switches) {
            $switches += $thread.context_switches-$old[0].context_switches
            $matchedThreads++
        }
    }
    @{pid=$current.pid;name=$current.name;cpu_percent_one_core=100*($current.cpu_seconds-$previous[0].cpu_seconds)/$elapsed;
      context_switches=$switches;matched_threads=$matchedThreads;
      handle_delta=$current.handles-$previous[0].handles;private_bytes_delta=$current.private_bytes-$previous[0].private_bytes}
})
$result=[ordered]@{
    schema=1;root_pid=$RootProcessId;root_started_utc=$rootStarted;elapsed_seconds=$elapsed
    sampler_sha256=(Get-FileHash -LiteralPath $PSCommandPath).Hash
    context_switch_note='Raw per-thread context-switch counters are a scheduling proxy, not an exact count of wait-handle signals. Exited/new threads remain in snapshots and are excluded from deltas.'
    samples=$samples;deltas=$delta
}
$result | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $OutputPath -Encoding utf8
if ((Get-Process -Id $RootProcessId).StartTime.ToUniversalTime().ToString('o') -ne $rootStarted) { throw 'Root process identity changed' }
