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
if ($Formal -and $identity.build.dirty) { throw 'Formal qualification requires a clean CI candidate' }
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
foreach($name in @('BLUEBERRY_NO_HISTORY','BLUEBERRY_TEST_PSREADLINE_MODULE','BLUEBERRY_BENCH_EVIDENCE')) { $saved[$name]=[Environment]::GetEnvironmentVariable($name,'Process') }
function Save-State {
    $manifest | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath (Join-Path $out 'matrix.json') -Encoding utf8
}
function Invoke-Probe([string]$Name,[string[]]$Arguments) {
    $previous=@($manifest.results | Where-Object { $_.name -eq $Name })
    if($previous.Count -gt 0) {
        if($previous.Count -ne 1 -or $previous[0].exit_code -ne 0) { throw "$Name has a retained failure; resuming cannot erase it." }
        return (Get-Content -LiteralPath $previous[0].report -Raw | ConvertFrom-Json)
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
    Get-Content -LiteralPath $json -Raw | ConvertFrom-Json
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
        $originalHash=(Get-FileHash -LiteralPath (Join-Path $originalDirectory $dllName) -Algorithm SHA256).Hash
        $startup=Invoke-Probe ($name+'-startup') @('product-probe','--shell',$shell,'--iterations',"$StartupPairs",'--host-mode','direct')
        if(@($startup.plain_editors).Count -ne $StartupPairs -or @($startup.plain_editors | Where-Object { $_.dll_sha256 -ine $originalHash }).Count -gt 0) { throw "$name original DLL identity mismatch; version labels alone are insufficient." }
        if(-not $startup.editor_eligible -or $startup.paired_first_input_delta.median -gt 50) { throw "$name startup gate failed; see retained evidence." }
        foreach($descriptions in @($true,$false)) {
            $arguments=@('beta-probe','--shell',$shell,'--samples',"$Samples",'--transport','pipe','--host-mode','direct')
            if(-not $descriptions) { $arguments+='--no-descriptions' }
            $hot=Invoke-Probe ($name+'-hot-descriptions-'+$descriptions.ToString().ToLowerInvariant()) $arguments
            $failed=@($hot.scenarios | Where-Object { $_.acceptance.cache_miss.status -ne 'passed' -or $_.acceptance.cache_hit.status -ne 'passed' })
            if($hot.transport_degraded -or $failed.Count -gt 0) { throw "$name hot gate failed; retain and attribute slow samples before proceeding." }
        }
    }
    if((Get-FileHash $Executable).Hash -ne $exeHash -or (Get-FileHash $Probe).Hash -ne $probeHash) { throw 'Artifact changed during measurement' }
    $manifest['automated_performance_passed']=$true
    $manifest['completed_at_utc']=[DateTime]::UtcNow.ToString('o')
    Save-State
} catch {
    $manifest['gate_error']=$_.Exception.Message
    $manifest['stopped_at_utc']=[DateTime]::UtcNow.ToString('o')
    Save-State
    throw
} finally {
    foreach($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name,$saved[$name],'Process') }
}
