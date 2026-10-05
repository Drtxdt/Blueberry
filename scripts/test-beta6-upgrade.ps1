[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Beta6Package,
    [Parameter(Mandatory)][string]$CandidatePackage,
    [Parameter(Mandatory)][string]$OutputDirectory
)
$ErrorActionPreference='Stop'
if(Test-Path -LiteralPath $OutputDirectory){throw 'Use a new evidence directory'}
$scratch=[IO.Path]::GetFullPath($OutputDirectory)
$null=New-Item -ItemType Directory $scratch
$root=Join-Path $scratch 'install'
$config=Join-Path $scratch 'config'
$payload=Join-Path $scratch 'original-payload'
if((Get-FileHash -LiteralPath $Beta6Package -Algorithm SHA256).Hash -ne '4D40AE29FEA9B92820DCB72373FE1AFD6538F682EC58B72A337FC24A3B3B40BA') {
 throw 'Fixture requires the exact public v0.5.0-beta.6 Windows x64 package'
}
Expand-Archive -LiteralPath $Beta6Package -DestinationPath $payload
$legacy=Join-Path $payload 'install.ps1'
$oldPackage=(Resolve-Path -LiteralPath $Beta6Package).Path
$newPackage=(Resolve-Path -LiteralPath $CandidatePackage).Path
$ps51="$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe"
function Install-Package($Package) {
 & $ps51 -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $legacy -PackagePath $Package -InstallRoot $root -ConfigRoot $config -NoPrompt -NoPath
 if($LASTEXITCODE -ne 0){throw 'Legacy public install entry failed'}
}
Install-Package $oldPackage
$exe=Join-Path $root 'blueberry.exe'
$oldHash=(Get-FileHash $exe).Hash
$originalManifest=Get-Content (Join-Path $root 'install.json') -Raw | ConvertFrom-Json
$initial=(& $exe --version | Out-String).Trim()
if($initial -notmatch '0.5.0-beta.6'){throw 'Wrong original version'}
[IO.File]::WriteAllText((Join-Path $root 'user-data.txt'),'retain me')
Install-Package $newPackage
$current=(& $exe --version | Out-String).Trim()
if($current -ne 'blueberry 0.5.0'){throw "Unexpected candidate: $current"}
& $exe rollback
if($LASTEXITCODE -ne 0){throw 'Rollback queue failed'}
$deadline=[DateTime]::UtcNow.AddSeconds(90)
while([DateTime]::UtcNow -lt $deadline){
 try {if((Get-FileHash $exe).Hash -eq $oldHash){break}}catch{}
 Start-Sleep -Milliseconds 100
}
if((Get-FileHash $exe).Hash -ne $oldHash){throw 'Did not restore exact beta.6 executable'}
$stateRoot=(Get-ChildItem $scratch -Directory -Filter '.blueberry-maintenance-*').FullName
$deadline=[DateTime]::UtcNow.AddSeconds(90)
$done=$false
while([DateTime]::UtcNow -lt $deadline){
 try {
  $state=Get-Content (Join-Path $stateRoot 'operation.json') -Raw | ConvertFrom-Json
  if($state.status -eq 'completed'){
   $lock=[IO.File]::Open((Join-Path $stateRoot 'operation.lock'),'OpenOrCreate','ReadWrite','None')
   $lock.Dispose();$done=$true;break
  }
 }catch{}
 Start-Sleep -Milliseconds 100
}
if(-not $done){throw 'Rollback did not finish transaction'}
foreach($record in $originalManifest.managed_hashes.PSObject.Properties){
 if((Get-FileHash -LiteralPath (Join-Path $root $record.Name)).Hash -ne $record.Value){throw "Rollback changed managed file: $($record.Name)"}
}
Install-Package $newPackage
& $exe uninstall
if($LASTEXITCODE -ne 0){throw 'Uninstall queue failed'}
$deadline=[DateTime]::UtcNow.AddSeconds(90)
while((Test-Path $exe) -and [DateTime]::UtcNow -lt $deadline){Start-Sleep -Milliseconds 100}
if(Test-Path $exe){throw 'Self-uninstall did not complete'}
if((Get-Content (Join-Path $root 'user-data.txt') -Raw) -ne 'retain me'){throw 'User data lost'}
@{scratch=$scratch;original_version=$initial;candidate_version=$current;original_executable_sha256=$oldHash;original_package_sha256=(Get-FileHash $oldPackage).Hash;candidate_package_sha256=(Get-FileHash $newPackage).Hash;passed=$true} | ConvertTo-Json | Set-Content (Join-Path $scratch 'result.json')
Write-Output "Public beta.6 migration passed; retained evidence: $scratch"
