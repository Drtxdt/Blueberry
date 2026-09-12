$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Assert-ShellsenseTrue {
    param(
        [Parameter(Mandatory = $true)]
        [bool]$Condition,

        [Parameter(Mandatory = $true)]
        [string]$Message
    )
    if (-not $Condition) {
        throw ('Assertion failed: ' + $Message)
    }
}

function Assert-ShellsenseEqual {
    param(
        [AllowNull()]
        [object]$Actual,

        [AllowNull()]
        [object]$Expected,

        [Parameter(Mandatory = $true)]
        [string]$Message
    )
    if ($Actual -is [string] -or $Expected -is [string]) {
        $equal = [string]::Equals([string]$Actual, [string]$Expected, [StringComparison]::Ordinal)
    } else {
        $equal = $Actual -eq $Expected
    }
    if (-not $equal) {
        throw ('Assertion failed: {0}; expected [{1}], got [{2}]' -f $Message, $Expected, $Actual)
    }
}

$adapterPath = Join-Path $PSScriptRoot '..\shell\integration.ps1'
Assert-ShellsenseTrue -Condition ([IO.File]::Exists($adapterPath)) -Message 'adapter script exists'

# Bootstrap with a token while capturing the private OSC frame.  The adapter
# is run under -NonInteractive by the test command, so this must not import
# PSReadLine merely because its commands are discoverable.
$env:SHELLSENSE_TOKEN = 'adapter-test-token'
$env:SHELLSENSE_EDIT_PATH = Join-Path ([IO.Path]::GetTempPath()) ('shellsense-adapter-test-' + [Guid]::NewGuid().ToString('N') + '.json')
$capturedWriter = [IO.StringWriter]::new([Globalization.CultureInfo]::InvariantCulture)
$consoleWriter = [Console]::Out
[Console]::SetOut($capturedWriter)
try {
    . $adapterPath
} finally {
    [Console]::SetOut($consoleWriter)
}

Assert-ShellsenseTrue -Condition ([string]::IsNullOrEmpty([string]$env:SHELLSENSE_TOKEN)) -Message 'bootstrap token is removed from the process environment'
$capabilityFrame = $capturedWriter.ToString()
$framePrefix = [string]([char]27) + ']7776;adapter-test-token;'
Assert-ShellsenseTrue -Condition $capabilityFrame.StartsWith($framePrefix, [StringComparison]::Ordinal) -Message 'bootstrap emits an OSC frame'
Assert-ShellsenseEqual -Actual $capabilityFrame[$capabilityFrame.Length - 1] -Expected ([char]7) -Message 'bootstrap frame ends with BEL'
$capabilityJson = $capabilityFrame.Substring($framePrefix.Length, $capabilityFrame.Length - $framePrefix.Length - 1)
$capabilities = ConvertFrom-Json -InputObject $capabilityJson
Assert-ShellsenseEqual -Actual ([bool]$capabilities.ready) -Expected $false -Message 'readiness is false without the PSReadLine surface'
Assert-ShellsenseEqual -Actual ([bool]$capabilities.psreadline) -Expected $false -Message 'noninteractive bootstrap does not load PSReadLine'
Assert-ShellsenseEqual -Actual ([bool]$capabilities.key_handlers.buffer) -Expected $false -Message 'noninteractive bootstrap has no buffer key handler'

# Frame construction must JSON-escape terminal-sensitive text rather than
# writing it as raw control data.
$unsafeText = 'quote="' + "`n" + [char]27 + [char]7
$unsafePayload = [ordered]@{ event = 'buffer'; line = $unsafeText; cursor = 2 }
$unsafeFrame = ConvertTo-ShellsenseFrame -Token 'frame-token' -Payload $unsafePayload
$unsafePrefix = [string]([char]27) + ']7776;frame-token;'
Assert-ShellsenseTrue -Condition $unsafeFrame.StartsWith($unsafePrefix, [StringComparison]::Ordinal) -Message 'frame uses the supplied token'
Assert-ShellsenseEqual -Actual $unsafeFrame[$unsafeFrame.Length - 1] -Expected ([char]7) -Message 'frame ends with BEL'
$unsafeJson = $unsafeFrame.Substring($unsafePrefix.Length, $unsafeFrame.Length - $unsafePrefix.Length - 1)
$unsafeDecoded = ConvertFrom-Json -InputObject $unsafeJson
Assert-ShellsenseEqual -Actual ([string]$unsafeDecoded.line) -Expected $unsafeText -Message 'JSON round-trips unsafe text'

# UTF-16 offsets are code-unit offsets.  A range may include a complete
# surrogate pair but may not begin or end between its two code units.
$unicodeLine = 'a😀b'
Assert-ShellsenseTrue -Condition (Test-ShellsenseUtf16Range -Line $unicodeLine -Start 1 -Length 2) -Message 'complete surrogate pair is a safe range'
Assert-ShellsenseTrue -Condition (Test-ShellsenseUtf16Range -Line $unicodeLine -Start 0 -Length 1) -Message 'range before surrogate pair is safe'
Assert-ShellsenseTrue -Condition (Test-ShellsenseUtf16Range -Line $unicodeLine -Start 3 -Length 1) -Message 'range after surrogate pair is safe'
Assert-ShellsenseTrue -Condition (Test-ShellsenseUtf16Range -Line $unicodeLine -Start 4 -Length 0) -Message 'end cursor is a safe range'
Assert-ShellsenseTrue -Condition (-not (Test-ShellsenseUtf16Range -Line $unicodeLine -Start 2 -Length 0)) -Message 'cursor inside surrogate pair is rejected'
Assert-ShellsenseTrue -Condition (-not (Test-ShellsenseUtf16Range -Line $unicodeLine -Start 1 -Length 1)) -Message 'range ending inside surrogate pair is rejected'
Assert-ShellsenseTrue -Condition (-not (Test-ShellsenseUtf16Range -Line $unicodeLine -Start 5 -Length 0)) -Message 'range beyond line is rejected'

$acceptedEdit = [pscustomobject]@{
    expectedLine   = $unicodeLine
    expectedCursor = 3
    start          = 1
    length         = 2
    text           = 'X'
}
$edit = $null
Assert-ShellsenseTrue -Condition (Test-ShellsenseEditPayload -Payload $acceptedEdit -CurrentLine $unicodeLine -CurrentCursor 3 -Edit ([ref]$edit)) -Message 'matching edit payload is accepted'
Assert-ShellsenseEqual -Actual $edit.start -Expected 1 -Message 'accepted edit start'
Assert-ShellsenseEqual -Actual $edit.length -Expected 2 -Message 'accepted edit length'
Assert-ShellsenseEqual -Actual $edit.text -Expected 'X' -Message 'accepted edit text'

$mismatchEdit = [pscustomobject]@{
    expectedLine   = 'other'
    expectedCursor = 3
    start          = 1
    length         = 2
    text           = 'X'
}
Assert-ShellsenseTrue -Condition (-not (Test-ShellsenseEditPayload -Payload $mismatchEdit -CurrentLine $unicodeLine -CurrentCursor 3 -Edit ([ref]$edit))) -Message 'stale line is rejected'

$cursorMismatchEdit = [pscustomobject]@{
    expectedLine   = $unicodeLine
    expectedCursor = 2
    start          = 1
    length         = 2
    text           = 'X'
}
Assert-ShellsenseTrue -Condition (-not (Test-ShellsenseEditPayload -Payload $cursorMismatchEdit -CurrentLine $unicodeLine -CurrentCursor 3 -Edit ([ref]$edit))) -Message 'stale cursor is rejected'

$splitEdit = [pscustomobject]@{
    expectedLine   = $unicodeLine
    expectedCursor = 3
    start          = 2
    length         = 0
    text           = 'X'
}
Assert-ShellsenseTrue -Condition (-not (Test-ShellsenseEditPayload -Payload $splitEdit -CurrentLine $unicodeLine -CurrentCursor 3 -Edit ([ref]$edit))) -Message 'surrogate-splitting edit is rejected'

$badNumberEdit = [pscustomobject]@{
    expectedLine   = $unicodeLine
    expectedCursor = 3
    start          = 1.5
    length         = 2
    text           = 'X'
}
Assert-ShellsenseTrue -Condition (-not (Test-ShellsenseEditPayload -Payload $badNumberEdit -CurrentLine $unicodeLine -CurrentCursor 3 -Edit ([ref]$edit))) -Message 'fractional UTF-16 offset is rejected'

$fallbackCwd = Get-ShellsenseWorkingDirectory
Assert-ShellsenseTrue -Condition ([IO.Directory]::Exists($fallbackCwd)) -Message 'cwd is a filesystem directory'

$transportText = "中文$([char]0xD83D)$([char]0xDE00)"
$transportJson = ConvertTo-ShellsenseJson -Payload @{ line = $transportText }
Assert-ShellsenseTrue -Condition (-not ($transportJson.ToCharArray() | Where-Object { [int]$_ -gt 127 })) -Message 'JSON transport is ASCII under legacy console code pages'
Assert-ShellsenseEqual -Actual ($transportJson | ConvertFrom-Json).line -Expected $transportText -Message 'ASCII JSON preserves non-BMP text'

# Command snapshots are intentionally fed by a deterministic in-memory
# provider. This exercises the >512 path without creating functions or
# aliases in the test runspace, then proves that each OSC frame is at most one
# host-sized batch and that command-kind duplicates survive.
function Get-ShellsenseCapturedEvents {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Raw,

        [Parameter(Mandatory = $true)]
        [string]$Token
    )

    $prefix = [string]([char]27) + ']7776;' + $Token + ';'
    $events = [System.Collections.Generic.List[object]]::new()
    foreach ($segment in $Raw.Split([char]7, [StringSplitOptions]::RemoveEmptyEntries)) {
        if (-not $segment.StartsWith($prefix, [StringComparison]::Ordinal)) {
            continue
        }
        $json = $segment.Substring($prefix.Length)
        [void]$events.Add((ConvertFrom-Json -InputObject $json))
    }
    return @($events.ToArray())
}

$fixtureCommands = [System.Collections.Generic.List[object]]::new()
for ($fixtureIndex = 0; $fixtureIndex -lt 600; $fixtureIndex++) {
    [void]$fixtureCommands.Add([pscustomobject]@{
        Name        = ('shellsenseFixture{0:D4}' -f $fixtureIndex)
        CommandType = 'Function'
        Definition  = ('function body {{ {0} }}' -f $fixtureIndex)
    })
}
# All three records use one name deliberately. The root engine owns
# Alias > Function > Cmdlet precedence, so the adapter must not deduplicate
# these records while forming a snapshot.
[void]$fixtureCommands.Add([pscustomobject]@{
    Name        = 'shellsenseDuplicate'
    CommandType = 'Cmdlet'
    Definition  = 'cmdlet definition must stay omitted'
})
[void]$fixtureCommands.Add([pscustomobject]@{
    Name        = 'shellsenseDuplicate'
    CommandType = 'Function'
    Definition  = 'function definition must stay omitted'
})
[void]$fixtureCommands.Add([pscustomobject]@{
    Name        = 'shellsenseDuplicate'
    CommandType = 'Alias'
    Definition  = 'Get-Item'
})

$commandTypes = [System.Management.Automation.CommandTypes]::Alias -bor
    [System.Management.Automation.CommandTypes]::Function -bor
    [System.Management.Automation.CommandTypes]::Cmdlet
$fixtureNamesBefore = @($ExecutionContext.InvokeCommand.GetCommands('shellsenseFixture*', $commandTypes, $true))
$commandToken = 'adapter-commands-token'
$savedCommandToken = [string]$script:SHELLSENSE_TOKEN
$commandCapture = [IO.StringWriter]::new([Globalization.CultureInfo]::InvariantCulture)
$commandProvider = { $fixtureCommands }
[Console]::SetOut($commandCapture)
try {
    $script:SHELLSENSE_TOKEN = $commandToken
    $fixtureCount = $fixtureCommands.Count
    $snapshotEvents = [System.Collections.Generic.List[object]]::new()
    do {
        $beforeLength = $commandCapture.GetStringBuilder().Length
        Get-ShellsenseImportedCommands -CommandProvider $commandProvider
        $newRaw = $commandCapture.ToString().Substring($beforeLength)
        foreach ($event in @(Get-ShellsenseCapturedEvents -Raw $newRaw -Token $commandToken)) {
            [void]$snapshotEvents.Add($event)
        }
        $lastSnapshotEvent = $snapshotEvents[$snapshotEvents.Count - 1]
    } while (-not [bool]$lastSnapshotEvent.complete)

    Assert-ShellsenseTrue -Condition ($snapshotEvents.Count -gt 1) -Message 'large command snapshot is split across frames'
    $firstSnapshotId = [string]$snapshotEvents[0].snapshot
    $parsedSnapshotId = [Guid]::Empty
    Assert-ShellsenseTrue -Condition ([Guid]::TryParse($firstSnapshotId, [ref]$parsedSnapshotId)) -Message 'snapshot id is a UUID'
    $totalCommands = 0
    $duplicateRecords = [System.Collections.Generic.List[object]]::new()
    for ($eventIndex = 0; $eventIndex -lt $snapshotEvents.Count; $eventIndex++) {
        $event = $snapshotEvents[$eventIndex]
        Assert-ShellsenseEqual -Actual ([string]$event.event) -Expected 'commands' -Message 'snapshot event type'
        Assert-ShellsenseEqual -Actual ([string]$event.snapshot) -Expected $firstSnapshotId -Message 'all batches share one snapshot id'
        $batch = @($event.commands)
        Assert-ShellsenseTrue -Condition ($batch.Count -le 128) -Message 'snapshot batch is bounded at 128 entries'
        $totalCommands += $batch.Count
        foreach ($command in $batch) {
            if ([string]$command.name -eq 'shellsenseDuplicate') {
                [void]$duplicateRecords.Add($command)
            }
        }
        if ($eventIndex -lt $snapshotEvents.Count - 1) {
            Assert-ShellsenseEqual -Actual ([bool]$event.complete) -Expected $false -Message 'non-final snapshot batch is incomplete'
        } else {
            Assert-ShellsenseEqual -Actual ([bool]$event.complete) -Expected $true -Message 'final snapshot batch is complete'
        }
    }
    Assert-ShellsenseEqual -Actual $totalCommands -Expected $fixtureCount -Message 'snapshot includes every command beyond the old 512 limit'
    Assert-ShellsenseEqual -Actual $duplicateRecords.Count -Expected 3 -Message 'same-name alias/function/cmdlet records are preserved'
    $aliasRecord = $duplicateRecords | Where-Object { $_.kind -eq 'alias' }
    Assert-ShellsenseEqual -Actual ([string]$aliasRecord.definition) -Expected 'Get-Item' -Message 'alias target is included'
    $functionRecord = $duplicateRecords | Where-Object { $_.kind -eq 'function' }
    Assert-ShellsenseEqual -Actual ([string]$functionRecord.definition) -Expected '' -Message 'function definition is omitted'
    $cmdletRecord = $duplicateRecords | Where-Object { $_.kind -eq 'cmdlet' }
    Assert-ShellsenseEqual -Actual ([string]$cmdletRecord.definition) -Expected '' -Message 'cmdlet definition is omitted'

    # Completion resets the cursor. A new request must enumerate a new
    # snapshot rather than replaying the final batch or reusing its UUID.
    $secondCapture = [IO.StringWriter]::new([Globalization.CultureInfo]::InvariantCulture)
    [Console]::SetOut($secondCapture)
    Get-ShellsenseImportedCommands -CommandProvider $commandProvider
    $secondEvents = @(Get-ShellsenseCapturedEvents -Raw $secondCapture.ToString() -Token $commandToken)
    Assert-ShellsenseEqual -Actual $secondEvents.Count -Expected 1 -Message 'new snapshot starts with one batch'
    Assert-ShellsenseTrue -Condition (-not [string]::Equals([string]$secondEvents[0].snapshot, $firstSnapshotId, [StringComparison]::Ordinal)) -Message 'new snapshot receives a new UUID'
    Assert-ShellsenseEqual -Actual (@($secondEvents[0].commands).Count) -Expected 128 -Message 'new snapshot starts at batch zero'
    # Drain the second fixture snapshot before leaving the test so later
    # adapter calls start from a clean cursor as well.
    while ($null -ne $script:SHELLSENSE_COMMAND_SNAPSHOT_ID) {
        Get-ShellsenseImportedCommands -CommandProvider $commandProvider
    }
} finally {
    $script:SHELLSENSE_TOKEN = $savedCommandToken
    [Console]::SetOut($consoleWriter)
}
$fixtureNamesAfter = @($ExecutionContext.InvokeCommand.GetCommands('shellsenseFixture*', $commandTypes, $true))
Assert-ShellsenseEqual -Actual $fixtureNamesAfter.Count -Expected $fixtureNamesBefore.Count -Message 'fixture provider does not pollute loaded commands'

# Preserve the prior command status seen by a user's actual Prompt.
Remove-Variable LASTEXITCODE -Scope Global -ErrorAction SilentlyContinue
$script:SHELLSENSE_ORIGINAL_PROMPT = { 'strict prompt' }
[Console]::SetOut($capturedWriter)
try {
    Assert-ShellsenseEqual -Actual (Prompt) -Expected 'strict prompt' -Message 'Prompt works before the first native exit code exists under StrictMode'
} finally {
    [Console]::SetOut($consoleWriter)
}
$script:SHELLSENSE_ORIGINAL_PROMPT = { "$?/$global:LASTEXITCODE" }
[Console]::SetOut($capturedWriter)
try {
    $global:LASTEXITCODE = 7
    Write-Error 'Expected status fixture' -ErrorAction SilentlyContinue
    $promptStatus = Prompt
    Assert-ShellsenseEqual -Actual $promptStatus -Expected 'False/7' -Message 'Prompt sees failed command status and native exit code'
    Assert-ShellsenseEqual -Actual $global:LASTEXITCODE -Expected 7 -Message 'Prompt transport preserves LASTEXITCODE'
} finally {
    [Console]::SetOut($consoleWriter)
}

Remove-Item -LiteralPath $env:SHELLSENSE_EDIT_PATH -Force -ErrorAction SilentlyContinue
Write-Output 'PowerShell adapter tests passed.'
