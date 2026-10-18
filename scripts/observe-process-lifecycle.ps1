#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Executable,
    [string[]]$Arguments=@(),
    [Parameter(Mandatory)][string]$OutputDirectory,
    [ValidateRange(1,3600)][int]$TimeoutSeconds=600
)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$Executable=(Resolve-Path -LiteralPath $Executable).Path
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Use a new directory; lifecycle evidence is never overwritten.' }
$output=(New-Item -ItemType Directory -Path $OutputDirectory).FullName
$prefix='Blueberry.Lifecycle.'+[Guid]::NewGuid().ToString('N')
$events=[Collections.Generic.List[object]]::new()
$subscriptions=[Collections.Generic.List[string]]::new()
$process=$null
$stdoutTask=$null
$stderrTask=$null
$receipt=[ordered]@{
    schema=1; diagnostic_only=$true; formal_timing_eligible=$false
    observer='Win32_ProcessStartTrace + Win32_ProcessStopTrace'
    observer_sha256=(Get-FileHash -LiteralPath $PSCommandPath).Hash
    event_loss_counter='unavailable; reconciled events do not prove absence of provider event loss'
    executable=$Executable; executable_sha256=(Get-FileHash -LiteralPath $Executable).Hash
    arguments=$Arguments; started_at_utc=[DateTime]::UtcNow.ToString('o')
    root_pid=$null; root_started_utc=$null; exit_code=$null
    lifecycle_reconciled=$false; processes=@(); error=$null
}
function Drain-LifecycleEvents {
    foreach ($source in $subscriptions) {
        foreach ($item in @(Get-Event -SourceIdentifier $source -ErrorAction SilentlyContinue)) {
            $native=$item.SourceEventArgs.NewEvent
            $events.Add([pscustomobject]@{
                kind=if ($source.EndsWith('.start')) {'start'} else {'stop'}
                time=[long]$native.TIME_CREATED
                pid=[int]$native.ProcessID
                parent_pid=if ($source.EndsWith('.start')) {[int]$native.ParentProcessID} else {$null}
                name=[string]$native.ProcessName
            })
            Remove-Event -EventIdentifier $item.EventIdentifier
        }
    }
}
try {
    # Subscription must succeed before launching the owned workload. On an
    # access-denied host the receipt records failure and no program is started.
    foreach ($kind in @('start','stop')) {
        $class=if ($kind -eq 'start') {'Win32_ProcessStartTrace'} else {'Win32_ProcessStopTrace'}
        $source=$prefix+'.'+$kind
        Register-CimIndicationEvent -Namespace root/cimv2 -Query "SELECT * FROM $class" -SourceIdentifier $source | Out-Null
        $subscriptions.Add($source)
    }
    $info=[Diagnostics.ProcessStartInfo]::new($Executable)
    $info.UseShellExecute=$false
    $info.CreateNoWindow=$true
    $info.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
    $info.RedirectStandardOutput=$true
    $info.RedirectStandardError=$true
    foreach ($argument in $Arguments) { $info.ArgumentList.Add($argument) }
    $process=[Diagnostics.Process]::Start($info)
    $receipt.root_pid=$process.Id
    $receipt.root_started_utc=$process.StartTime.ToUniversalTime().ToString('o')
    $stdoutTask=$process.StandardOutput.ReadToEndAsync()
    $stderrTask=$process.StandardError.ReadToEndAsync()
    $clock=[Diagnostics.Stopwatch]::StartNew()
    while (-not $process.HasExited) {
        Drain-LifecycleEvents
        if ($clock.Elapsed.TotalSeconds -ge $TimeoutSeconds) { throw 'Owned diagnostic process exceeded its timeout.' }
        Start-Sleep -Milliseconds 50
    }
    $receipt.exit_code=$process.ExitCode
    # Allow asynchronous stop notifications to reach this observer. This is
    # an explicitly separate diagnostic run, never a latency sample.
    $drain=[Diagnostics.Stopwatch]::StartNew()
    while ($drain.Elapsed.TotalSeconds -lt 2) { Drain-LifecycleEvents; Start-Sleep -Milliseconds 50 }
} catch {
    $receipt.error=$_.Exception.Message
} finally {
    if ($null -ne $process -and -not $process.HasExited) { $process.Kill($true); $process.WaitForExit() }
    Drain-LifecycleEvents
    foreach ($source in $subscriptions) { Unregister-Event -SourceIdentifier $source }
    if ($null -ne $stdoutTask) { [IO.File]::WriteAllText((Join-Path $output 'stdout.log'),$stdoutTask.GetAwaiter().GetResult()) }
    if ($null -ne $stderrTask) { [IO.File]::WriteAllText((Join-Path $output 'stderr.log'),$stderrTask.GetAwaiter().GetResult()) }
    $live=@{}
    $rows=[Collections.Generic.List[object]]::new()
    $rootObserved=$false
    foreach ($event in @($events | Sort-Object time,@{Expression={if ($_.kind -eq 'start') {0} else {1}}})) {
        if ($event.kind -eq 'start') {
            $isRoot=($event.pid -eq $receipt.root_pid -and -not $rootObserved)
            if ($isRoot -or $live.ContainsKey($event.parent_pid)) {
                $node=[ordered]@{
                    pid=$event.pid; parent_pid=$event.parent_pid; name=$event.name
                    started_utc=[DateTime]::FromFileTimeUtc($event.time).ToString('o')
                    stopped_utc=$null
                }
                $rows.Add($node)
                $live[$event.pid]=$node
                if ($isRoot) { $rootObserved=$true }
            }
        } elseif ($live.ContainsKey($event.pid)) {
            $live[$event.pid].stopped_utc=[DateTime]::FromFileTimeUtc($event.time).ToString('o')
            $live.Remove($event.pid)
        }
    }
    $receipt.processes=@($rows.ToArray())
    $receipt.lifecycle_reconciled=($rootObserved -and $live.Count -eq 0 -and -not $receipt.error)
    $receipt.completed_at_utc=[DateTime]::UtcNow.ToString('o')
    $receipt | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $output 'lifecycle.json') -Encoding utf8
    if ($null -ne $process) { $process.Dispose() }
}
if (-not $receipt.lifecycle_reconciled) { throw "Incomplete lifecycle diagnostic; inspect $output/lifecycle.json" }
Write-Output (Join-Path $output 'lifecycle.json')
