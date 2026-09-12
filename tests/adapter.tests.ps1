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
Assert-ShellsenseTrue -Condition ([object]::ReferenceEquals((Get-ShellsenseJsonOptions), (Get-ShellsenseJsonOptions))) -Message 'JSON serializer options are cached'

$editJson = '{"expectedLine":"a\uD83D\uDE00b","expectedCursor":3,"start":1,"length":2,"text":"X"}'
$decodedEditPayload = ConvertFrom-ShellsenseEditJson -Json $editJson
$decodedEdit = $null
Assert-ShellsenseTrue -Condition (Test-ShellsenseEditPayload -Payload $decodedEditPayload -CurrentLine $unicodeLine -CurrentCursor 3 -Edit ([ref]$decodedEdit)) -Message 'System.Text.Json edit payload preserves UTF-16 values'
Assert-ShellsenseEqual -Actual $decodedEdit.text -Expected 'X' -Message 'decoded edit text'
Assert-ShellsenseTrue -Condition ($null -eq (ConvertFrom-ShellsenseEditJson -Json '[1,2,3]')) -Message 'non-object edit payload is rejected'

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

# Collision discovery uses PSReadLine's static APIs so initialization does
# not need three cmdlet calls for the reserved chords. A child chord reserves
# only itself; a bare F12 reserves all three children because it owns the
# prefix. Exercise both cases and restore adapter state before the remaining
# helper tests run.
Import-Module PSReadLine -ErrorAction Stop
$savedReadLineAvailable = [bool]$script:SHELLSENSE_PSREADLINE_AVAILABLE
$savedReadLineWrapped = [bool]$script:SHELLSENSE_READLINE_WRAPPED
$savedOriginalReadLine = $script:SHELLSENSE_ORIGINAL_READLINE
$savedKeyHandlers = $script:SHELLSENSE_KEY_HANDLERS
function Reset-ShellsenseReadLineTestState {
    $script:SHELLSENSE_PSREADLINE_AVAILABLE = $false
    $script:SHELLSENSE_READLINE_WRAPPED = $false
    $script:SHELLSENSE_ORIGINAL_READLINE = $null
    $script:SHELLSENSE_KEY_HANDLERS = [ordered]@{
        buffer   = $false
        apply    = $false
        commands = $false
    }
}
$collisionChords = @('F12', 'F12,s', 'F12,a', 'F12,c')
$collisionHandler = { }
try {
    foreach ($chord in $collisionChords) {
        [Microsoft.PowerShell.PSConsoleReadLine]::RemoveKeyHandler([string[]]@($chord))
    }

    [Microsoft.PowerShell.PSConsoleReadLine]::SetKeyHandler(
        [string[]]@('F12,a'),
        $collisionHandler,
        'existing apply',
        'existing apply handler')
    Reset-ShellsenseReadLineTestState
    $collisionCapture = [IO.StringWriter]::new([Globalization.CultureInfo]::InvariantCulture)
    [Console]::SetOut($collisionCapture)
    try {
        Initialize-ShellsenseReadLine
    } finally {
        [Console]::SetOut($consoleWriter)
    }
    Assert-ShellsenseTrue -Condition ([bool]$script:SHELLSENSE_KEY_HANDLERS.buffer) -Message 'existing child does not falsely reserve F12,s'
    Assert-ShellsenseEqual -Actual ([bool]$script:SHELLSENSE_KEY_HANDLERS.apply) -Expected $false -Message 'existing F12,a remains reserved'
    Assert-ShellsenseTrue -Condition ([bool]$script:SHELLSENSE_KEY_HANDLERS.commands) -Message 'existing child does not falsely reserve F12,c'
    $childCollisionEvents = @(
        Get-ShellsenseCapturedEvents -Raw $collisionCapture.ToString() -Token ([string]$script:SHELLSENSE_TOKEN) |
            Where-Object { $_.event -eq 'error' -and $_.code -eq 'key_chord_collision' }
    )
    Assert-ShellsenseEqual -Actual $childCollisionEvents.Count -Expected 1 -Message 'one exact child collision is reported'
    Assert-ShellsenseEqual -Actual ([string]$childCollisionEvents[0].chord) -Expected 'F12,a' -Message 'exact child collision identifies the reserved chord'
    $existingApply = @([Microsoft.PowerShell.PSConsoleReadLine]::GetKeyHandlers([string[]]@('F12,a')))
    Assert-ShellsenseEqual -Actual $existingApply.Count -Expected 1 -Message 'existing F12,a binding remains installed'
    Assert-ShellsenseEqual -Actual ([string]$existingApply[0].Function) -Expected 'existing apply' -Message 'existing F12,a binding is not overwritten'

    foreach ($chord in $collisionChords) {
        [Microsoft.PowerShell.PSConsoleReadLine]::RemoveKeyHandler([string[]]@($chord))
    }
    [Microsoft.PowerShell.PSConsoleReadLine]::SetKeyHandler(
        [string[]]@('F12'),
        $collisionHandler,
        'existing parent',
        'existing parent handler')
    Reset-ShellsenseReadLineTestState
    $parentCapture = [IO.StringWriter]::new([Globalization.CultureInfo]::InvariantCulture)
    [Console]::SetOut($parentCapture)
    try {
        Initialize-ShellsenseReadLine
    } finally {
        [Console]::SetOut($consoleWriter)
    }
    foreach ($handlerName in @('buffer', 'apply', 'commands')) {
        Assert-ShellsenseEqual -Actual ([bool]$script:SHELLSENSE_KEY_HANDLERS[$handlerName]) -Expected $false -Message ('bare F12 preserves the parent binding for ' + $handlerName)
    }
    $parentCollisionEvents = @(
        Get-ShellsenseCapturedEvents -Raw $parentCapture.ToString() -Token ([string]$script:SHELLSENSE_TOKEN) |
            Where-Object { $_.event -eq 'error' -and $_.code -eq 'key_chord_collision' }
    )
    Assert-ShellsenseEqual -Actual $parentCollisionEvents.Count -Expected 3 -Message 'bare F12 reports all reserved child collisions'
    $existingParent = @([Microsoft.PowerShell.PSConsoleReadLine]::GetKeyHandlers([string[]]@('F12')))
    Assert-ShellsenseEqual -Actual $existingParent.Count -Expected 1 -Message 'existing bare F12 binding remains installed'
    Assert-ShellsenseEqual -Actual ([string]$existingParent[0].Function) -Expected 'existing parent' -Message 'existing bare F12 binding is not overwritten'
} finally {
    foreach ($chord in $collisionChords) {
        try {
            [Microsoft.PowerShell.PSConsoleReadLine]::RemoveKeyHandler([string[]]@($chord))
        } catch {
        }
    }
    $script:SHELLSENSE_PSREADLINE_AVAILABLE = $savedReadLineAvailable
    $script:SHELLSENSE_READLINE_WRAPPED = $savedReadLineWrapped
    $script:SHELLSENSE_ORIGINAL_READLINE = $savedOriginalReadLine
    $script:SHELLSENSE_KEY_HANDLERS = $savedKeyHandlers
}

# Trace is opt-in and must emit one bounded numeric stage event without
# recursively tracing the trace frame itself or copying user payload fields.
$savedTraceEnabled = [bool]$script:SHELLSENSE_TRACE_ENABLED
$savedTraceEmitting = [bool]$script:SHELLSENSE_TRACE_EMITTING
$savedTraceToken = [string]$script:SHELLSENSE_TOKEN
$traceToken = 'adapter-trace-token'
$traceCapture = [IO.StringWriter]::new([Globalization.CultureInfo]::InvariantCulture)
[Console]::SetOut($traceCapture)
try {
    $script:SHELLSENSE_TRACE_ENABLED = $true
    $script:SHELLSENSE_TRACE_EMITTING = $false
    $script:SHELLSENSE_TOKEN = $traceToken
    Send-ShellsenseEvent -Event 'buffer' -Data ([ordered]@{
        line   = 'trace payload stays out of diagnostics'
        cursor = 0
    })
} finally {
    $script:SHELLSENSE_TOKEN = $savedTraceToken
    $script:SHELLSENSE_TRACE_ENABLED = $savedTraceEnabled
    $script:SHELLSENSE_TRACE_EMITTING = $savedTraceEmitting
    [Console]::SetOut($consoleWriter)
}
$traceEvents = @(Get-ShellsenseCapturedEvents -Raw $traceCapture.ToString() -Token $traceToken)
$traceStageEvents = @($traceEvents | Where-Object { $_.event -eq 'trace' })
$traceBufferEvents = @($traceEvents | Where-Object { $_.event -eq 'buffer' })
Assert-ShellsenseEqual -Actual $traceStageEvents.Count -Expected 1 -Message 'trace does not recurse on its own serialization'
Assert-ShellsenseEqual -Actual $traceBufferEvents.Count -Expected 1 -Message 'trace leaves the original event intact'
Assert-ShellsenseEqual -Actual ([string]$traceStageEvents[0].stage) -Expected 'serialize' -Message 'trace stage is whitelisted'
Assert-ShellsenseTrue -Condition ([double]$traceStageEvents[0].duration_ms -ge 0) -Message 'trace duration is numeric'
Assert-ShellsenseEqual -Actual (@($traceStageEvents[0].PSObject.Properties.Name).Count) -Expected 3 -Message 'trace contains only event stage and duration'

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
