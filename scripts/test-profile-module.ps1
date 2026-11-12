[CmdletBinding()]
param([Parameter(Mandatory)][string]$Executable,[Parameter(Mandatory)][string]$Shell,
      [Parameter(Mandatory)][string]$OriginalManifest,[Parameter(Mandatory)][string]$Destination)
$ErrorActionPreference='Stop'
$Executable=(Resolve-Path $Executable).Path
$Shell=(Get-Command $Shell).Source
$OriginalManifest=(Resolve-Path $OriginalManifest).Path
if (Test-Path -LiteralPath $Destination) { throw 'Use a fresh directory.' }
$root=[IO.Directory]::CreateDirectory([IO.Path]::GetFullPath($Destination)).FullName
$moduleRoot=Join-Path $root 'Modules'
$active=Join-Path $moduleRoot 'PSReadLine'
[IO.Directory]::CreateDirectory($active) | Out-Null
$version=[string](Import-PowerShellDataFile -LiteralPath $OriginalManifest).ModuleVersion
Copy-Item -LiteralPath (Split-Path -LiteralPath $OriginalManifest) -Destination (Join-Path $active $version) -Recurse
$file=Join-Path $root 'Microsoft.PowerShell_profile.ps1'
[IO.File]::WriteAllText($file,"Set-PSReadLineKeyHandler -Chord F8 -Function HistorySearchBackward`r`n")
$originalProfile=(Get-FileHash $file).Hash
$before=@{}
foreach ($name in 'LOCALAPPDATA','PSModulePath','BLUEBERRY_TEST_PSREADLINE_MODULE','BLUEBERRY_NO_HISTORY') { $before[$name]=[Environment]::GetEnvironmentVariable($name,'Process') }
function Get-LoadedEditor {
    $line=& $Shell -NoProfile -NonInteractive -Command 'Import-Module PSReadLine -ErrorAction Stop; $t=[Microsoft.PowerShell.PSConsoleReadLine]; [Console]::WriteLine((@{dll=$t.Assembly.Location;sha=(Get-FileHash $t.Assembly.Location).Hash;hooks=[bool]$t.GetProperty("EditorIntegrationVersion")} | ConvertTo-Json -Compress))'
    if ($LASTEXITCODE) { throw 'Cannot inspect fresh shell' }
    return ($line | ConvertFrom-Json)
}
try {
    $env:PSModulePath=$moduleRoot+';'+$env:PSModulePath
    $env:LOCALAPPDATA=Join-Path $root 'local'
    $env:BLUEBERRY_NO_HISTORY='1'
    $env:BLUEBERRY_TEST_PSREADLINE_MODULE=$OriginalManifest
    $old=Get-LoadedEditor
    if ($old.hooks -or -not $old.dll.StartsWith($active,[StringComparison]::OrdinalIgnoreCase)) { throw 'Stock module was not selected' }
    & $Executable startup enable --profile $file
    if ($LASTEXITCODE) { throw 'Enable failed' }
    $new=Get-LoadedEditor
    if (-not $new.hooks -or $new.sha -eq $old.sha) { throw 'Private editor was not selected' }
    & $Executable startup enable --profile $file
    if ($LASTEXITCODE) { throw 'Repeated enable failed' }
    & $Executable startup disable --profile $file
    if ($LASTEXITCODE) { throw 'Disable failed' }
    $restored=Get-LoadedEditor
    if ($restored.sha -cne $old.sha -or $restored.dll -ine $old.dll -or (Get-FileHash $file).Hash -cne $originalProfile) { throw 'Disable did not restore the original DLL and profile' }
    @{old=$old;enabled=$new;restored=$restored;profile_sha256=$originalProfile} | ConvertTo-Json | Set-Content (Join-Path $root 'receipt.json')
} finally {
    foreach ($name in $before.Keys) { [Environment]::SetEnvironmentVariable($name,$before[$name],'Process') }
}
