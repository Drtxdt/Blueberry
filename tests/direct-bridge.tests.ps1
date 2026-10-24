param([Parameter(Mandatory=$true)][string]$AssemblyPath)
$ErrorActionPreference = 'Stop'
$null = [Reflection.Assembly]::Load([IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $AssemblyPath)))
$contract = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'direct-diagnostics.json') -Raw | ConvertFrom-Json
foreach ($event in $contract.events.PSObject.Properties) {
    foreach ($stage in $event.Value.stages) {
        $diagnostic = [Collections.Generic.Dictionary[string,object]]::new()
        $diagnostic['event']=$event.Name; $diagnostic['stage']=$stage; $diagnostic['qpc']=[long]123
        if (-not [Blueberry.Direct.DiagnosticContract]::Valid($diagnostic)) { throw "Declared diagnostic rejected: $stage" }
        foreach ($field in $event.Value.required_integer_fields) {
            $diagnostic.Remove($field) | Out-Null
            if ([Blueberry.Direct.DiagnosticContract]::Valid($diagnostic)) { throw "Missing diagnostic field accepted: $field" }
            $diagnostic[$field]='invalid'
            if ([Blueberry.Direct.DiagnosticContract]::Valid($diagnostic)) { throw "Invalid diagnostic field accepted: $field" }
            $diagnostic[$field]=[long]123
        }
        $diagnostic['stage']='future_event'
        if ([Blueberry.Direct.DiagnosticContract]::Valid($diagnostic)) { throw 'Unknown diagnostic accepted' }
    }
}
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
# Manual fallback leaves custom handlers and ordinary character bindings intact.
Set-PSReadLineKeyHandler -Chord Spacebar -Function ForwardChar
Set-PSReadLineKeyHandler -Chord Backspace -ScriptBlock { [Microsoft.PowerShell.PSConsoleReadLine]::Insert('CUSTOM') }
$before = @(Get-PSReadLineKeyHandler)
$arguments = @([Diagnostics.Stopwatch]::GetTimestamp(), [long]0, [long]0)
$null = $bridge.GetMethod('InitializeLegacy',$flags).Invoke($null,$arguments)
$after = @(Get-PSReadLineKeyHandler)
if ($after.Count -gt $before.Count + 3) { throw 'Manual mode registered more than three shortcuts' }
if (($after | Where-Object Key -eq Spacebar).Function -ne 'ForwardChar') { throw 'Manual mode changed Spacebar' }
if (($after | Where-Object Key -eq Backspace).Function -ne 'CustomAction') { throw 'Manual mode changed custom binding' }
$bridge.GetField('connected',$flags).SetValue($null,$true)
$bridge.GetField('negotiated',$flags).SetValue($null,$true)
if (-not $bridge::ServiceAvailable -or $bridge::AutomaticMenu) { throw 'Service availability and automatic mode are coupled' }
$bridge.GetField('connected',$flags).SetValue($null,$false)
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
