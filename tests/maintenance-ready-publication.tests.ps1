[CmdletBinding()]
param(
    [string]$SourceScript,
    [Parameter(Mandatory)][string]$OutputDirectory
)
$ErrorActionPreference='Stop'
if (-not $SourceScript) { $SourceScript=Join-Path $PSScriptRoot '../scripts/test-maintenance.ps1' }
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Use a new evidence directory.' }
$root=[IO.Path]::GetFullPath($OutputDirectory)
$null=New-Item -ItemType Directory -Path $root
$source=[IO.File]::ReadAllText((Resolve-Path -LiteralPath $SourceScript).Path)
$pattern="(?s)\[IO.File\]::WriteAllText\(\`$helper,@'\r?\n(.*?)\r?\n'@\)"
$match=[regex]::Match($source,$pattern)
if (-not $match.Success) { throw 'Maintenance helper source was not found.' }
# Hold the actual fixture's writer open so readiness publication is tested at
# the boundary that failed in CI, without depending on scheduler luck.
$helper=$match.Groups[1].Value.Replace('File.WriteAllText(', 'WriteDelayed(')
$delayed=@'
class Hold {
    static void WriteDelayed(string path,string text) {
        using(var stream=new FileStream(path,FileMode.Create,FileAccess.Write,FileShare.None)) {
            Thread.Sleep(250);
            byte[] bytes=System.Text.Encoding.UTF8.GetBytes(text);
            stream.Write(bytes,0,bytes.Length);
        }
    }
'@
$helper=$helper.Replace('class Hold {',$delayed)
$cs=Join-Path $root 'hold.cs';$exe=Join-Path $root 'hold.exe'
[IO.File]::WriteAllText($cs,$helper)
& "$env:SystemRoot/Microsoft.NET/Framework64/v4.0.30319/csc.exe" /nologo /target:exe "/out:$exe" $cs
if ($LASTEXITCODE -ne 0) { throw 'Fixture compilation failed.' }
$ready=Join-Path $root 'ready.txt'
$eventName='Local\Blueberry.ReadyProof.'+[Guid]::NewGuid().ToString('N')
$event=[Threading.EventWaitHandle]::new($false,'ManualReset',$eventName)
$process=$null
$receipt=[ordered]@{schema=1;source_sha256=(Get-FileHash -LiteralPath $SourceScript).Hash;writer_delay_ms=250;passed=$false}
try {
    $info=[Diagnostics.ProcessStartInfo]::new($exe)
    $info.UseShellExecute=$false;$info.CreateNoWindow=$true
    $info.EnvironmentVariables['BB_TEST_OPERATION']='hold'
    $info.EnvironmentVariables['BB_TEST_READY']=$ready
    $info.EnvironmentVariables['BB_TEST_EVENT']=$eventName
    $info.EnvironmentVariables['BB_TEST_BLOCK_JOB']='0'
    $process=[Diagnostics.Process]::Start($info)
    $deadline=[DateTime]::UtcNow.AddSeconds(10)
    while (-not [IO.File]::Exists($ready)) {
        if ($process.HasExited -or [DateTime]::UtcNow -ge $deadline) { throw 'Fixture did not publish readiness.' }
        [Threading.Thread]::Sleep(1)
    }
    $text=[IO.File]::ReadAllText($ready)
    if ($text -cne "0`n") { throw 'Reader observed partial readiness.' }
    if (Test-Path -LiteralPath ($ready+'.tmp')) { throw 'Published readiness retained its temporary file.' }
    $null=$event.Set()
    if (-not $process.WaitForExit(10000) -or $process.ExitCode -ne 0) { throw 'Fixture did not exit successfully.' }
    $receipt.passed=$true
} catch {
    $receipt.error=$_.Exception.Message
    throw
} finally {
    $null=$event.Set()
    if ($process) {
        if (-not $process.HasExited -and -not $process.WaitForExit(1000)) { $process.Kill();$process.WaitForExit() }
        $process.Dispose()
    }
    $event.Dispose()
    $receipt | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'receipt.json') -Encoding utf8
}
