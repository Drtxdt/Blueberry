[CmdletBinding()]
param([Parameter(Mandatory)][string]$Executable,[Parameter(Mandatory)][string]$Shell,
      [Parameter(Mandatory)][string]$OriginalManifest,[Parameter(Mandatory)][string]$Destination,
      [switch]$StockOutsideUserRoot)
$ErrorActionPreference='Stop'
$Executable=(Resolve-Path $Executable).Path
$Shell=(Get-Command $Shell).Source
$OriginalManifest=(Resolve-Path $OriginalManifest).Path
if (Test-Path -LiteralPath $Destination) { throw 'Use a fresh directory.' }
$root=[IO.Directory]::CreateDirectory([IO.Path]::GetFullPath($Destination)).FullName
$moduleRoot=Join-Path $root 'Modules'
$active=Join-Path $moduleRoot 'PSReadLine'
$stockRoot = if ($StockOutsideUserRoot) { Join-Path $root 'original-modules' } else { $moduleRoot }
$stock = Join-Path $stockRoot 'PSReadLine'
[IO.Directory]::CreateDirectory($stock) | Out-Null
$version=[string](Import-PowerShellDataFile -LiteralPath $OriginalManifest).ModuleVersion
Copy-Item -LiteralPath (Split-Path -LiteralPath $OriginalManifest) -Destination (Join-Path $stock $version) -Recurse
$file=Join-Path $root 'Microsoft.PowerShell_profile.ps1'
[IO.File]::WriteAllText($file,"Set-PSReadLineKeyHandler -Chord F8 -Function HistorySearchBackward`r`n")
$originalProfile=(Get-FileHash $file).Hash
$before=@{}
foreach ($name in 'LOCALAPPDATA','PSModulePath','BLUEBERRY_TEST_PSREADLINE_MODULE','BLUEBERRY_NO_HISTORY') { $before[$name]=[Environment]::GetEnvironmentVariable($name,'Process') }
function Get-LoadedEditor {
    $line=& $Shell -NoProfile -NonInteractive -Command 'Import-Module PSReadLine -ErrorAction Stop; Get-PSReadLineOption -ErrorAction Stop | Out-Null; $t=[Microsoft.PowerShell.PSConsoleReadLine]; [Console]::WriteLine((@{dll=$t.Assembly.Location;sha=(Get-FileHash $t.Assembly.Location).Hash;hooks=[bool]$t.GetProperty("EditorIntegrationVersion")} | ConvertTo-Json -Compress))'
    if ($LASTEXITCODE) { throw 'Cannot inspect fresh shell' }
    return ($line | ConvertFrom-Json)
}
function Start-EditorHolder([string]$Label) {
    $ready = Join-Path $root ($Label+'.ready')
    $done = Join-Path $root ($Label+'.release')
    $quoted = $ready.Replace("'","''")
    $script = "Import-Module PSReadLine -ErrorAction Stop; [IO.File]::WriteAllText('$quoted.tmp',[Microsoft.PowerShell.PSConsoleReadLine].Assembly.Location); [IO.File]::Move('$quoted.tmp','$quoted'); while (-not [IO.File]::Exists('$($done.Replace("'","''"))')) { Start-Sleep -Milliseconds 100 }"
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($script))
    $proc = Start-Process -FilePath $Shell -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-EncodedCommand',$encoded) -WindowStyle Hidden -PassThru
    $holders.Add(@{process=$proc;done=$done})
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while (-not [IO.File]::Exists($ready)) {
        if ($proc.HasExited -or [DateTime]::UtcNow -ge $deadline) { throw "Holder did not load $Label" }
        Start-Sleep -Milliseconds 50
    }
    return [IO.File]::ReadAllText($ready)
}
function Close-EditorHolders {
    foreach ($item in $holders) {
        [IO.File]::WriteAllText($item.done,'exit')
        if (-not $item.process.WaitForExit(5000)) { $item.process.Kill(); $item.process.WaitForExit() }
        $item.process.Dispose()
    }
    $holders.Clear()
}
$holders=[Collections.Generic.List[object]]::new()
try {
    $env:PSModulePath=$moduleRoot+';'+$stockRoot+';'+$env:PSModulePath
    $env:LOCALAPPDATA=Join-Path $root 'local'
    $env:BLUEBERRY_NO_HISTORY='1'
    $env:BLUEBERRY_TEST_PSREADLINE_MODULE=$OriginalManifest
    $old=Get-LoadedEditor
    if ($old.hooks -or -not $old.dll.StartsWith($stock+'\',[StringComparison]::OrdinalIgnoreCase)) { throw 'Stock module was not selected' }
    if ((Start-EditorHolder 'stock') -ine $old.dll) { throw 'Stock holder loaded a different DLL' }
    & $Executable startup enable --profile $file
    if ($LASTEXITCODE) { throw 'Enable failed' }
    $new=Get-LoadedEditor
    if (-not $new.hooks -or $new.sha -eq $old.sha -or -not $new.dll.StartsWith($active+'\',[StringComparison]::OrdinalIgnoreCase)) { throw 'Private editor was not selected' }
    & $Executable startup enable --profile $file
    if ($LASTEXITCODE) { throw 'Repeated enable failed' }
    if ((Start-EditorHolder 'private') -ine $new.dll) { throw 'Private holder loaded a different DLL' }
    & $Executable startup disable --profile $file
    if ($LASTEXITCODE) { throw 'Disable failed' }
    $restored=Get-LoadedEditor
    if ($restored.sha -cne $old.sha -or $restored.dll -ine $old.dll -or (Get-FileHash $file).Hash -cne $originalProfile) { throw 'Disable did not restore the original DLL and profile' }
    Close-EditorHolders
    & $Executable startup disable --profile $file
    if ($LASTEXITCODE -or (Test-Path -LiteralPath $new.dll)) { throw 'Unlocked managed DLL was not retired' }
    @{held_original_during_enable=$true;held_private_during_disable=$true;old=$old;enabled=$new;restored=$restored;profile_sha256=$originalProfile} | ConvertTo-Json | Set-Content (Join-Path $root 'receipt.json')
} finally {
    Close-EditorHolders
    foreach ($name in $before.Keys) { [Environment]::SetEnvironmentVariable($name,$before[$name],'Process') }
}
