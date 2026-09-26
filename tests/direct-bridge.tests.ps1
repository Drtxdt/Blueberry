param([Parameter(Mandatory=$true)][string]$AssemblyPath)
$ErrorActionPreference = 'Stop'
$null = [Reflection.Assembly]::Load([IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $AssemblyPath)))
function Assert-Rejected([string]$Text) {
    $rejected = $false
    try { $null = [Blueberry.Direct.Json]::Parse($Text) } catch { $rejected = $true }
    if (-not $rejected) { throw "Invalid protocol JSON accepted: $Text" }
}
$value = [Collections.Generic.Dictionary[string,object]]::new()
$value.Add('text','中文😀👩‍💻' + "`n" + '"\')
$value.Add('revision',[long]9223372036854775807)
$value.Add('enabled',$true)
$roundtrip = [Blueberry.Direct.Json]::Parse([Blueberry.Direct.Json]::Encode($value))
if ($roundtrip['text'] -cne $value['text'] -or $roundtrip['revision'] -ne $value['revision'] -or $roundtrip['enabled'] -ne $true) { throw 'JSON roundtrip changed Unicode or revision' }
foreach ($invalid in @('{"id":1,"id":2}', '01', '-01', '1 trailing', '{"id":}', '"\q"', '[1,]', '9223372036854775808')) { Assert-Rejected $invalid }
Assert-Rejected (('[' * 34) + '0' + (']' * 34))
# Dictionary overwrites on .NET Core can keep _version unchanged. The audit
# must still detect a custom handler replacing an existing key at equal count.
$modulePath = $env:BLUEBERRY_TEST_PSREADLINE_MODULE
if (-not $modulePath) { $modulePath = 'PSReadLine' }
Import-Module $modulePath
$bridge = [Blueberry.Direct.Bridge]
$flags = [Reflection.BindingFlags]'NonPublic,Static'
$bridge.GetField('api',$flags).SetValue($null,[Microsoft.PowerShell.PSConsoleReadLine])
$bridge.GetField('moduleVersion',$flags).SetValue($null,(Get-Module PSReadLine).Version)
$null = $bridge.GetMethod('RememberBindingVersion',$flags).Invoke($null,@())
Set-PSReadLineKeyHandler -Chord Backspace -ScriptBlock { [Microsoft.PowerShell.PSConsoleReadLine]::Insert('CUSTOM') }
$rejected = $false
try { $null = $bridge.GetMethod('AuditBindings',$flags).Invoke($null,@()) } catch { $rejected = $_.Exception.ToString().Contains('custom binding: Backspace') }
if (-not $rejected) { throw 'Equal-count custom binding replacement escaped identity audit' }
if ((Get-PSReadLineKeyHandler | Where-Object Key -eq Backspace).Function -ne 'CustomAction') { throw 'Audit changed custom binding' }
Write-Output 'Compiled direct bridge codec: passed'
