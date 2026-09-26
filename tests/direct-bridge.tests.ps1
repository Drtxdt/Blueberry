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
# The public API canonicalizes a literal space as Spacebar. Exercise actual
# bulk registration against a pre-existing verified built-in binding, rather
# than testing just the string filter.
Set-PSReadLineKeyHandler -Chord Spacebar -Function ForwardChar
$bound = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
foreach ($binding in Get-PSReadLineKeyHandler) { $null = $bound.Add($binding.Key) }
$registerType = $bridge.GetField('register',$flags).FieldType
$registerMethod = [Microsoft.PowerShell.PSConsoleReadLine].GetMethod('SetKeyHandler',[type[]]@([string[]],[Action[Nullable[ConsoleKeyInfo],object]],[string],[string]))
$bridge.GetField('register',$flags).SetValue($null,[Delegate]::CreateDelegate($registerType,$registerMethod))
$null = $bridge.GetMethod('RegisterCharacters',$flags).Invoke($null,(,$bound))
if ((Get-PSReadLineKeyHandler | Where-Object Key -eq Spacebar).Function -ne 'ForwardChar') { throw 'Character registration overwrote original Spacebar action' }
$null = $bridge.GetMethod('RememberBindingVersion',$flags).Invoke($null,@())
Set-PSReadLineKeyHandler -Chord Backspace -ScriptBlock { [Microsoft.PowerShell.PSConsoleReadLine]::Insert('CUSTOM') }
$rejected = $false
try { $null = $bridge.GetMethod('AuditBindings',$flags).Invoke($null,@()) } catch { $rejected = $_.Exception.ToString().Contains('custom binding: Backspace') }
if (-not $rejected) { throw 'Equal-count custom binding replacement escaped identity audit' }
if ((Get-PSReadLineKeyHandler | Where-Object Key -eq Backspace).Function -ne 'CustomAction') { throw 'Audit changed custom binding' }
# Fault injection at the wire boundary: a late frame must not replace a newer
# candidate, and workbench/control responses must survive ordinary coalescing.
function New-Frame([long]$Revision,[long]$Id,[string]$Kind='frame',[string]$Interaction='completion') {
    $frame=[Collections.Generic.Dictionary[string,object]]::new()
    $frame['revision']=$Revision; $frame['frame_id']=$Id
    $frame['kind']=$Kind; $frame['interaction']=$Interaction
    return $frame
}
$queue=$bridge.GetMethod('QueueIncomingFrame',$flags)
$newer=New-Frame 8 12
if (-not $queue.Invoke($null,@($newer,[long]1))) { throw 'Newest frame rejected' }
foreach ($stale in @((New-Frame 8 11),(New-Frame 7 99),(New-Frame 8 12))) {
    if ($queue.Invoke($null,@($stale,[long]2))) { throw 'Stale, duplicate or reordered frame accepted' }
}
$form=New-Frame 9 13 'frame' 'form'
$edit=New-Frame 9 13 'edit' 'form'
$null=$queue.Invoke($null,@($form,[long]3))
$null=$queue.Invoke($null,@($edit,[long]4))
$null=$queue.Invoke($null,@((New-Frame 10 14),[long]5))
$reliable=$bridge.GetField('reliableFrames',$flags).GetValue($null)
if ($reliable.Count -ne 2 -or $reliable.Dequeue()['interaction'] -ne 'form' -or $reliable.Dequeue()['kind'] -ne 'edit') { throw 'Workbench/control order lost to menu coalescing' }
if ($bridge.GetField('pending',$flags).GetValue($null)['frame_id'] -ne 14) { throw 'Newest ordinary frame missing' }
# A matching buffer/revision alone does not authorize a replacement.
$authorized=$bridge.GetMethod('EditAuthorized',$flags)
$bridge.GetField('revision',$flags).SetValue($null,[long]9)
$edit['candidate_id']='shown'
if ($authorized.Invoke($null,(,$edit))) { throw 'Unsolicited replacement authorized' }
$shown=New-Frame 9 13; $shown['candidate_id']='shown'
$bridge.GetField('acceptingFrame',$flags).SetValue($null,$shown)
if (-not $authorized.Invoke($null,(,$edit))) { throw 'Displayed candidate acceptance rejected' }
$edit['candidate_id']='wrong'
if ($authorized.Invoke($null,(,$edit))) { throw 'Wrong candidate authorized' }
$bridge.GetField('acceptingFrame',$flags).SetValue($null,$null)
$bridge.GetField('expectedUiEditRevision',$flags).SetValue($null,[long]9)
if ($authorized.Invoke($null,(,$edit))) { throw 'Candidate edit confused with form confirmation' }
$null=$edit.Remove('frame_id')
if (-not $authorized.Invoke($null,(,$edit))) { throw 'Pending form edit rejected' }
$edit['revision']=[long]8
if ($authorized.Invoke($null,(,$edit))) { throw 'Old form revision authorized' }
Write-Output 'Compiled direct bridge codec: passed'
