[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$Probe,
    [Parameter(Mandatory)][string]$Original200,
    [Parameter(Mandatory)][string]$Original245,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [string]$Shell7 = (Get-Command pwsh.exe -ErrorAction Stop).Source,
    [string]$Shell51 = "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe",
    [int]$Samples = 30,
    [int]$StartupPairs = 10,
    [switch]$Formal,
    [switch]$Resume,
    [switch]$ContinueOnGateFailure,
    [string]$ExpectedExecutableSha256,
    [string]$ExpectedProbeSha256
)
$ErrorActionPreference='Stop'
$Executable=(Resolve-Path -LiteralPath $Executable).Path
$Probe=(Resolve-Path -LiteralPath $Probe).Path
$exeHash=(Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash
$probeHash=(Get-FileHash -LiteralPath $Probe -Algorithm SHA256).Hash
if ($Formal -and ($Samples -lt 300 -or $StartupPairs -lt 30 -or -not $ExpectedExecutableSha256 -or -not $ExpectedProbeSha256)) {
    throw 'Formal qualification requires 300 samples, 30 startup pairs, and both frozen SHA-256 values.'
}
if ($ExpectedExecutableSha256 -and $exeHash -ine $ExpectedExecutableSha256) { throw 'EXE identity mismatch' }
if ($ExpectedProbeSha256 -and $probeHash -ine $ExpectedProbeSha256) { throw 'Probe identity mismatch' }
$identity=& $Executable doctor --json | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or -not $identity.build.private_editors) { throw 'Missing private module build identity' }
if (-not $identity.build.conpty) { throw 'Missing pinned ConPTY build identity' }
if ($Formal -and ($identity.build.dirty -or $identity.build.profile -ne 'release' -or $identity.build.commit -notmatch '^[0-9a-fA-F]{40}$')) { throw 'Formal qualification requires a clean release CI candidate' }
if ($Formal -and ($env:BLUEBERRY_TEST_DISABLE_PREJIT -or $env:BLUEBERRY_TEST_FRAME_DELAY_MS -or $env:BLUEBERRY_DIRECT_TRACE)) { throw 'Formal qualification forbids diagnostic timing overrides.' }
if ($Resume) {
    if ($Formal) { throw 'Formal batches cannot resume across an interruption; retain the interrupted batch and use a new directory.' }
    $out=(Resolve-Path -LiteralPath $OutputDirectory).Path
    $manifest=Get-Content -LiteralPath (Join-Path $out 'matrix.json') -Raw | ConvertFrom-Json -AsHashtable
    if ($manifest.formal -or $manifest.executable_sha256 -ine $exeHash -or $manifest.probe_sha256 -ine $probeHash -or $manifest.samples -ne $Samples -or $manifest.startup_pairs -ne $StartupPairs) { throw 'Cannot resume: artifact identity or sampling configuration differs.' }
    $manifest['resumed_at_utc']=@($manifest.resumed_at_utc | Where-Object { $_ })+[DateTime]::UtcNow.ToString('o')
} else {
    if (Test-Path -LiteralPath $OutputDirectory) { throw 'Use a new output directory; previous evidence is never overwritten.' }
    $out=(New-Item -ItemType Directory -Path $OutputDirectory).FullName
    $manifest=[ordered]@{ formal=$Formal.IsPresent; started_at_utc=[DateTime]::UtcNow.ToString('o'); executable_sha256=$exeHash; probe_sha256=$probeHash; build=$identity.build; samples=$Samples; startup_pairs=$StartupPairs; results=@() }
}
$saved=@{}
$manifest['sampler_sha256']=(Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash
$manifest['measurement_complete']=$false
$manifest['automated_performance_passed']=$false
$manifest['gate_errors']=@()
function Record-GateFailure([string]$Message) {
    $manifest['gate_errors']+=@($Message)
    Save-State
    if (-not $ContinueOnGateFailure) { throw $Message }
}
function Assert-Statistics($Statistics, [int]$Count, [string]$Label) {
    $values=@($Statistics.samples)
    if ($values.Count -ne $Count -or $Count -lt 1) { throw "$Label sample count mismatch" }
    foreach ($value in @($values) + @($Statistics.median, $Statistics.p95)) {
        if ($null -eq $value -or $value -is [string] -or $value -is [bool] -or [double]::IsNaN([double]$value) -or [double]::IsInfinity([double]$value)) { throw "$Label nonfinite or invalid sample" }
    }
    $ordered=@($values | ForEach-Object { [double]$_ } | Sort-Object)
    $middle=[int][Math]::Floor($Count/2)
    $median=if($Count%2) { $ordered[$middle] } else { ($ordered[$middle-1]+$ordered[$middle])/2 }
    $p95=$ordered[[int][Math]::Ceiling($Count*0.95)-1]
    if ($null -eq $Statistics.median -or $null -eq $Statistics.p95 -or [Math]::Abs([double]$Statistics.median-$median) -gt 0.0001 -or [Math]::Abs([double]$Statistics.p95-$p95) -gt 0.0001) { throw "$Label statistics disagree with raw samples" }
}
function Assert-EditorIdentity($Report, [int]$Count, [string]$DllHash, [string]$Version, [string]$ShellPrefix) {
    if (@($Report.editors).Count -ne $Count -or @($Report.shell_versions).Count -ne $Count -or @($Report.psreadline_versions).Count -ne $Count) { throw 'Editor identity sample count mismatch' }
    foreach ($editor in $Report.editors) {
        if ($editor.mode -ne 'editor_hooks_v1' -or $editor.dll_sha256 -ine $DllHash -or $editor.patch -ne $identity.build.private_editors.patch -or $editor.fallback_reason) { throw 'Loaded editor identity mismatch' }
    }
    if (@($Report.psreadline_versions | Where-Object { $_ -ne $Version }).Count -gt 0 -or @($Report.shell_versions | Where-Object { $_ -notlike ($ShellPrefix + '.*') }).Count -gt 0) { throw 'Shell or editor version mismatch' }
}
foreach($name in @('BLUEBERRY_NO_HISTORY','BLUEBERRY_TEST_PSREADLINE_MODULE','BLUEBERRY_BENCH_EVIDENCE')) { $saved[$name]=[Environment]::GetEnvironmentVariable($name,'Process') }
function Save-State {
    $manifest | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath (Join-Path $out 'matrix.json') -Encoding utf8
}
function Read-ProbeReport([string]$Path, [string]$Name) {
    $report=Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
    if ($report.executable_sha256 -ine $exeHash -or $report.source_commit -ne $identity.build.commit) { throw "$Name product identity mismatch" }
    if ($report.probe_conpty.mode -ne 'pinned' -or $report.probe_conpty.sha256 -ine $identity.build.conpty.sha256 -or $report.product_conpty.sha256 -ine $identity.build.conpty.sha256) { throw "$Name ConPTY identity mismatch" }
    $report
}
function Invoke-Probe([string]$Name,[string[]]$Arguments) {
    $previous=@($manifest.results | Where-Object { $_.name -eq $Name })
    if($previous.Count -gt 0) {
        if($previous.Count -ne 1 -or $previous[0].exit_code -ne 0) { throw "$Name has a retained failure; resuming cannot erase it." }
        return (Read-ProbeReport $previous[0].report $Name)
    }
    $json=Join-Path $out ($Name+'.json')
    $log=Join-Path $out ($Name+'.log')
    $attempt=1
    while((Test-Path -LiteralPath $json) -or (Test-Path -LiteralPath $log)) {
        $attempt++
        $json=Join-Path $out ($Name+'-attempt-'+$attempt+'.json')
        $log=Join-Path $out ($Name+'-attempt-'+$attempt+'.log')
    }
    $env:BLUEBERRY_BENCH_EVIDENCE=Join-Path $out ('raw-'+$Name+'-attempt-'+$attempt)
    & $Probe @Arguments --host-executable $Executable --output $json *> $log
    $exitCode=$LASTEXITCODE
    $manifest.results+=@{name=$Name;exit_code=$exitCode;report=$json;log=$log}
    Save-State
    if($exitCode -ne 0) { throw "$Name failed; raw evidence and log retained at $out" }
    Read-ProbeReport $json $Name
}
try {
    $env:BLUEBERRY_NO_HISTORY='1'
    $env:BLUEBERRY_BENCH_EVIDENCE=Join-Path $out 'raw'
    Save-State
    $combinations=@(@('ps51-200',$Shell51,$Original200),@('ps51-245',$Shell51,$Original245),@('ps7-245',$Shell7,$Original245))
    foreach($combination in $combinations) {
        $name=$combination[0]; $shell=$combination[1]
        $env:BLUEBERRY_TEST_PSREADLINE_MODULE=(Resolve-Path -LiteralPath $combination[2]).Path
        $originalDirectory=Split-Path -LiteralPath $env:BLUEBERRY_TEST_PSREADLINE_MODULE
        $dllName=if($name -eq 'ps51-200') {'Microsoft.PowerShell.PSReadLine2.dll'} else {'Microsoft.PowerShell.PSReadLine.dll'}
        $editorVersion=if($name -eq 'ps51-200') {'2.0.0'} else {'2.4.5'}
        $shellPrefix=if($name -eq 'ps7-245') {'7'} else {'5.1'}
        $privateDll=@($identity.build.private_editors.files | Where-Object { $_.version -eq $editorVersion -and $_.path -eq $dllName })
        if ($privateDll.Count -ne 1 -or $privateDll[0].sha256 -notmatch '^[0-9a-fA-F]{64}$') { throw 'Missing private editor DLL identity' }
        $originalHash=(Get-FileHash -LiteralPath (Join-Path $originalDirectory $dllName) -Algorithm SHA256).Hash
        $startup=Invoke-Probe ($name+'-startup') @('product-probe','--shell',$shell,'--iterations',"$StartupPairs",'--host-mode','direct')
        if(@($startup.plain_editors).Count -ne $StartupPairs -or @($startup.plain_editors | Where-Object { $_.dll_sha256 -ine $originalHash }).Count -gt 0) { throw "$name original DLL identity mismatch; version labels alone are insufficient." }
        if(-not $startup.editor_eligible) { throw "$name editor identity is ineligible" }
        Assert-EditorIdentity $startup $StartupPairs $privateDll[0].sha256 $editorVersion $shellPrefix
        Assert-Statistics $startup.paired_first_input_delta $StartupPairs "$name startup"
        if($startup.paired_first_input_delta.median -gt 50) { Record-GateFailure "$name startup gate failed; see retained evidence." }
        foreach($descriptions in @($true,$false)) {
            $arguments=@('beta-probe','--shell',$shell,'--samples',"$Samples",'--transport','pipe','--host-mode','direct')
            if(-not $descriptions) { $arguments+='--no-descriptions' }
            $hot=Invoke-Probe ($name+'-hot-descriptions-'+$descriptions.ToString().ToLowerInvariant()) $arguments
            if($hot.transport_degraded) { throw "$name transport degraded" }
            $scenarioNames=@($hot.scenarios | ForEach-Object { $_.name } | Sort-Object)
            if(($scenarioNames -join ',') -ne 'cargo,fuzzy,git,js,path,root') { throw "$name scenario matrix incomplete" }
            foreach($scenario in $hot.scenarios) {
                foreach($cache in @('cache_miss','cache_hit')) {
                    Assert-EditorIdentity $scenario.$cache ([int][Math]::Ceiling($Samples/10.0)) $privateDll[0].sha256 $editorVersion $shellPrefix
                    $acceptance=$scenario.acceptance.$cache
                    if($acceptance.observed_samples -ne $Samples -or $acceptance.expected_samples -ne $Samples) { throw "$name observed samples incomplete" }
                    Assert-Statistics $acceptance.statistics $Samples "$name/$($scenario.name)/$cache"
                    if ($acceptance.status -notin @('passed','failed')) { throw "$name ineligible acceptance status" }
                    if($acceptance.statistics.p95 -gt 20) {
                        Record-GateFailure "$name descriptions=$descriptions $($scenario.name)/$cache P95 exceeds 20 ms"
                    } elseif($Formal -and $acceptance.status -ne 'passed') { throw "$name inconsistent acceptance status" }
                }
            }
        }
    }
    if((Get-FileHash $Executable).Hash -ne $exeHash -or (Get-FileHash $Probe).Hash -ne $probeHash -or (Get-FileHash -LiteralPath $PSCommandPath).Hash -ne $manifest.sampler_sha256) { throw 'Artifact changed during measurement' }
    $manifest['measurement_complete']=$true
    $manifest['automated_performance_passed']=($manifest.gate_errors.Count -eq 0 -and $Formal.IsPresent)
    $manifest['completed_at_utc']=[DateTime]::UtcNow.ToString('o')
    Save-State
    if($manifest.gate_errors.Count -gt 0) { throw "Measurement complete; $($manifest.gate_errors.Count) performance gates failed. See matrix.json." }
} catch {
    $manifest['gate_error']=$_.Exception.Message
    $manifest['stopped_at_utc']=[DateTime]::UtcNow.ToString('o')
    Save-State
    throw
} finally {
    foreach($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name,$saved[$name],'Process') }
}
