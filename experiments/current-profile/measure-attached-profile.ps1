# Exploratory PS7 AB/BA. Each profile and binary has its own recorded identity;
# this experiment is never eligible for the frozen release evidence matrix.
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Destination,[Parameter(Mandatory)][string]$Executable,
      [Parameter(Mandatory)][string]$Service,[Parameter(Mandatory)][string]$Shell,
      [switch]$CheckUserKey)
$ErrorActionPreference='Stop'
$Executable=(Resolve-Path $Executable).Path
$Service=(Resolve-Path $Service).Path
$Destination=[IO.Path]::GetFullPath($Destination)
if (Test-Path $Destination) { throw 'Use a new evidence directory' }
$homeCopy=Join-Path $Destination 'PSHome'
& "$PSScriptRoot/../../scripts/prepare-profile-entry.ps1" -Executable $Executable -Shell $Shell -Destination $homeCopy
$fixture=Join-Path $homeCopy 'blueberry-profile-probe.json'
$original=[IO.File]::ReadAllBytes($fixture)
$identity=Get-Content $fixture -Raw | ConvertFrom-Json
$profileFile=$identity.profile
$normal=[IO.File]::ReadAllBytes($profileFile)
$scriptPath=(Join-Path $PSScriptRoot 'probe-attached-profile.ps1').Replace("'","''")
$quotedService=$Service.Replace("'","''")
$keyFixture = if ($CheckUserKey) {
@'
    Set-PSReadLineKeyHandler -Chord g -BriefDescription BlueberryUserKeyProbe -ScriptBlock {
        param($key,$arg)
        [Microsoft.PowerShell.PSConsoleReadLine]::SelfInsert($key,$arg)
        [IO.File]::WriteAllText('__KEY_RECEIPT__','g')
    }
'@
}
$hook=@"
if (`$env:BLUEBERRY_PROFILE_PROBE_DATA -and `$env:BLUEBERRY_ACTIVE -ne '1') {
$keyFixture
    try { . '$scriptPath' -Service '$quotedService' -DataDirectory `$env:BLUEBERRY_PROFILE_PROBE_DATA -ErrorAction Stop }
    catch {
        [IO.File]::WriteAllText('$(($Destination -replace "'","''"))/attach-error.txt', `$_.Exception.Message)
        [Environment]::Exit(1)
    }
}
"@
$results=@()
try {
    foreach ($n in 0..3) {
        $side=@('normal','attached','attached','normal')[$n]
        $hostFile=if ($side -eq 'normal') { $Executable } else { $Service }
        if ($side -eq 'normal') {
            [IO.File]::WriteAllBytes($profileFile,$normal)
            [IO.File]::WriteAllBytes($fixture,$original)
        } else {
            $receipt = (Join-Path $Destination "key-$n.txt").Replace("'","''")
            [IO.File]::WriteAllText($profileFile,$hook.Replace('__KEY_RECEIPT__',$receipt),[Text.UTF8Encoding]::new($false))
            $identity.executable=$Service
            $identity.executable_sha256=(Get-FileHash $Service).Hash
            $identity.profile_sha256=(Get-FileHash $profileFile).Hash
            $identity.managed_block='experimental current shell with background Rust worker'
            [IO.File]::WriteAllText($fixture,($identity | ConvertTo-Json),[Text.UTF8Encoding]::new($false))
        }
        $file=Join-Path $Destination "$n-$side.json"
        & $Executable product-probe --host-executable $hostFile --shell "$homeCopy/pwsh.exe" --profile-entry --profile-fixture $fixture --iterations 1 --cache-state warm --output $file *> "$file.log"
        if ($LASTEXITCODE) { throw "Entry failed; see $file.log" }
        $report=Get-Content $file -Raw | ConvertFrom-Json
        if ($report.editor_eligible -ne $true -or $report.actual_automatic_menu -contains $false) { throw 'Degraded editor sample' }
        if ($CheckUserKey -and $side -eq 'attached') {
            if (-not (Test-Path "$Destination/key-$n.txt")) { throw 'The custom first-key binding was not observed' }
        }
        $results+=@{side=$side;overhead_ms=$report.paired_first_input_delta.median;report=$file}
    }
} finally {
    [IO.File]::WriteAllBytes($profileFile,$normal)
    [IO.File]::WriteAllBytes($fixture,$original)
    @{diagnostic_only=$true;custom_key_probe=[bool]$CheckUserKey;results=$results} | ConvertTo-Json -Depth 4 | Set-Content (Join-Path $Destination 'experiment.json')
}
