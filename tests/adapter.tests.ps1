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
$env:SHELLSENSE_REQUEST_PATH = $null
$env:SHELLSENSE_KEY_PREFIX = $null
$env:SHELLSENSE_PUBLIC_KEYS = $null
$env:SHELLSENSE_PUBLIC_KEYS_VERSION = $null
foreach ($publicKeyName in @('TRIGGER', 'NATIVE', 'DETAILS', 'REFRESH', 'RELOAD')) {
    [Environment]::SetEnvironmentVariable(
        ('SHELLSENSE_PUBLIC_KEY_' + $publicKeyName),
        $null,
        'Process')
}
$env:SHELLSENSE_NO_HISTORY = '1'
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
Assert-ShellsenseEqual -Actual ([bool]$capabilities.key_handlers.paste) -Expected $false -Message 'noninteractive bootstrap has no paste key handler'
Assert-ShellsenseEqual -Actual ([int]$capabilities.protocol_version) -Expected 2 -Message 'bootstrap advertises protocol v2'
Assert-ShellsenseEqual -Actual ([int]$capabilities.protocol) -Expected 2 -Message 'legacy protocol version alias is retained'
Assert-ShellsenseEqual -Actual ([string]$capabilities.key_prefix) -Expected 'F12' -Message 'default protocol prefix is F12'
Assert-ShellsenseEqual -Actual ([bool]$capabilities.capabilities.manual_native) -Expected $true -Message 'native completion is manual-only'
Assert-ShellsenseEqual -Actual ([bool]$capabilities.capabilities.command_position) -Expected $true -Message 'context command-position capability is advertised'
Assert-ShellsenseEqual -Actual ([bool]$capabilities.capabilities.paste_insert) -Expected $false -Message 'noninteractive bootstrap does not advertise paste insertion'
Assert-ShellsenseTrue -Condition ([string]$script:SHELLSENSE_REQUEST_PATH -match '(?i)[\\/]request\.json$') -Message 'request path defaults beside edit path'

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

$savedEnvironmentSnapshot = $script:SHELLSENSE_ENVIRONMENT_SNAPSHOT
try {
    $script:SHELLSENSE_ENVIRONMENT_SNAPSHOT = $null
    $firstPromptEnd = Get-ShellsensePromptEndData
    $secondPromptEnd = Get-ShellsensePromptEndData
    Assert-ShellsenseTrue -Condition $firstPromptEnd.Contains('environment') -Message 'first prompt end carries the environment snapshot'
    Assert-ShellsenseTrue -Condition ($firstPromptEnd.environment.Count -gt 0) -Message 'environment snapshot contains process names'
    Assert-ShellsenseTrue -Condition ($firstPromptEnd.Contains('path') -and $firstPromptEnd.Contains('pathext') -and $firstPromptEnd.Contains('pid')) -Message 'prompt end retains legacy process fields'
    Assert-ShellsenseTrue -Condition (-not $secondPromptEnd.Contains('environment')) -Message 'unchanged environment is not resent'
} finally {
    $script:SHELLSENSE_ENVIRONMENT_SNAPSHOT = $savedEnvironmentSnapshot
}

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

# Bracketed paste payloads use their own strict, bounded protocol object. The
# adapter normalizes all newline spellings before passing the text to the
# PSReadLine editing API, while rejecting coercible JSON values and unknown
# fields.
$pastePayload = ConvertFrom-ShellsensePasteJson -Json '{"id":"paste-1","text":"first\nsecond\rthird\r\nfourth"}'
$normalizedPaste = $null
Assert-ShellsenseTrue -Condition (Test-ShellsensePastePayload -Payload $pastePayload -Paste ([ref]$normalizedPaste)) -Message 'valid paste payload is accepted'
Assert-ShellsenseEqual -Actual ([string]$normalizedPaste.id) -Expected 'paste-1' -Message 'paste request id is preserved'
Assert-ShellsenseEqual -Actual ([string]$normalizedPaste.text) -Expected "first`nsecond`nthird`nfourth" -Message 'paste newlines normalize to LF'
Assert-ShellsenseTrue -Condition ($null -eq (ConvertFrom-ShellsensePasteJson -Json '[{"id":"paste-1","text":"x"}]')) -Message 'paste arrays are rejected'
Assert-ShellsenseTrue -Condition ($null -eq (ConvertFrom-ShellsensePasteJson -Json '{"id":42,"text":"x"}')) -Message 'numeric paste ids are rejected'
Assert-ShellsenseTrue -Condition ($null -eq (ConvertFrom-ShellsensePasteJson -Json '{"id":"paste-1","text":42}')) -Message 'numeric paste text is rejected'
Assert-ShellsenseTrue -Condition ($null -eq (ConvertFrom-ShellsensePasteJson -Json '{"id":"paste-1","text":"x","extra":true}')) -Message 'unknown paste fields are rejected'
Assert-ShellsenseTrue -Condition ($null -eq (ConvertFrom-ShellsensePasteJson -Json '{"id":"   ","text":"x"}')) -Message 'blank paste ids are rejected'
Assert-ShellsenseTrue -Condition ($null -eq (ConvertFrom-ShellsensePasteJson -Json '{"id":"paste-1","text":"x","text":"y"}')) -Message 'duplicate paste fields are rejected'

# Complex context comes from the live PSReadLine AST in production and from
# the same parser fallback in this isolated test.  Only safe preceding
# arguments are returned, and the replacement range stops at the cursor so
# text to the right of an in-word cursor remains untouched.
Assert-ShellsenseTrue -Condition ($null -eq (Get-ShellsenseAstBufferContext `
        -Line 'git sw' -Cursor 6 -Ast $null -Tokens $null -ParseErrors $null)) -Message 'simple lines omit compact context'
$quotedContext = Get-ShellsenseAstBufferContext `
    -Line "git switch 'fea" -Cursor 15 -Ast $null -Tokens $null -ParseErrors $null
Assert-ShellsenseEqual -Actual ([string]$quotedContext.command) -Expected 'git' -Message 'quoted context resolves command'
Assert-ShellsenseEqual -Actual ([string]$quotedContext.arguments[0]) -Expected 'switch' -Message 'preceding argument is retained'
Assert-ShellsenseEqual -Actual ([string]$quotedContext.prefix) -Expected 'fea' -Message 'quoted prefix is decoded'
Assert-ShellsenseEqual -Actual ([int]$quotedContext.replace_start) -Expected 11 -Message 'quoted replacement starts at token extent'
Assert-ShellsenseEqual -Actual ([int]$quotedContext.replace_end) -Expected 15 -Message 'replacement ends at the UTF-16 cursor'
Assert-ShellsenseEqual -Actual ([bool]$quotedContext.suppressed) -Expected $false -Message 'open safe quote remains completable'
Assert-ShellsenseEqual -Actual ([bool]$quotedContext.complex) -Expected $true -Message 'complex context marks its protocol shape'

$middleLine = "git switch 'fea-tail"
$middleCursor = $middleLine.IndexOf('fea', [StringComparison]::Ordinal) + 2
$middleContext = Get-ShellsenseAstBufferContext `
    -Line $middleLine -Cursor $middleCursor -Ast $null -Tokens $null -ParseErrors $null
Assert-ShellsenseEqual -Actual ([string]$middleContext.prefix) -Expected 'fe' -Message 'cursor-in-word prefix is limited to text before cursor'
Assert-ShellsenseEqual -Actual ([int]$middleContext.replace_start) -Expected 11 -Message 'cursor-in-word start is exact'
Assert-ShellsenseEqual -Actual ([int]$middleContext.replace_end) -Expected $middleLine.Length -Message 'cursor-in-word suffix is preserved'

$commentContext = Get-ShellsenseAstBufferContext `
    -Line 'git switch # comment' -Cursor 20 -Ast $null -Tokens $null -ParseErrors $null
Assert-ShellsenseEqual -Actual ([bool]$commentContext.suppressed) -Expected $true -Message 'comment text suppresses context'
$expressionContext = Get-ShellsenseAstBufferContext `
    -Line 'git switch $(Get-Date)' -Cursor 22 -Ast $null -Tokens $null -ParseErrors $null
Assert-ShellsenseEqual -Actual ([bool]$expressionContext.suppressed) -Expected $true -Message 'unknown expression suppresses context'

$environmentContext = Get-ShellsenseAstBufferContext `
    -Line '$env:Pa' -Cursor 7 -Ast $null -Tokens $null -ParseErrors $null
Assert-ShellsenseEqual -Actual ([string]$environmentContext.prefix) -Expected '$env:Pa' -Message 'environment namespace stays in prefix'
Assert-ShellsenseEqual -Actual ([int]$environmentContext.replace_start) -Expected 0 -Message 'standalone environment prefix starts at variable'
Assert-ShellsenseEqual -Actual ([int]$environmentContext.replace_end) -Expected 7 -Message 'environment range uses UTF-16 cursor'
Assert-ShellsenseEqual -Actual ([bool]$environmentContext.command_position) -Expected $false -Message 'environment variable is a value expression'
Assert-ShellsenseEqual -Actual ([bool]$environmentContext.suppressed) -Expected $false -Message 'environment names are safe to complete'
$quotedEnvironmentLine = [string]::Concat('Write-Output ', [char]34, '$env:Pa')
$quotedEnvironmentContext = Get-ShellsenseAstBufferContext `
    -Line $quotedEnvironmentLine -Cursor $quotedEnvironmentLine.Length -Ast $null -Tokens $null -ParseErrors $null
Assert-ShellsenseEqual -Actual ([string]$quotedEnvironmentContext.prefix) -Expected '$env:Pa' -Message 'double-quoted environment prefix is decoded'
Assert-ShellsenseEqual -Actual ([bool]$quotedEnvironmentContext.suppressed) -Expected $false -Message 'double-quoted environment names remain safe'

Assert-ShellsenseEqual -Actual (Test-ShellsenseConfirmedContinuation -Line 'Get-Item' -PreviousLine 'Get-Item') -Expected $false -Message 'complete Enter does not report continuation'
Assert-ShellsenseEqual -Actual (Test-ShellsenseConfirmedContinuation -Line "Get-Item`r`n" -PreviousLine 'Get-Item') -Expected $true -Message 'confirmed multiline Enter reports continuation'
Assert-ShellsenseEqual -Actual (Test-ShellsenseConfirmedContinuation -Line 'Get-Item ' -PreviousLine 'Get-Item' -AddLine) -Expected $true -Message 'known AddLine reports a changed buffer'
Assert-ShellsenseEqual -Actual (ConvertTo-ShellsenseNativeCandidateKind -ResultType 'ParameterName') -Expected 'option' -Message 'native parameter kind maps to protocol option'
Assert-ShellsenseEqual -Actual (ConvertTo-ShellsenseNativeCandidateKind -ResultType 'ProviderContainer') -Expected 'directory' -Message 'native provider container maps to directory'
Assert-ShellsenseEqual -Actual (ConvertTo-ShellsenseNativeCandidateKind -ResultType 'Method') -Expected 'value' -Message 'unknown native kind remains valid'

$nativeRequest = ConvertFrom-ShellsenseRequestJson -Json '{"id":"native-1","kind":"native"}'
Assert-ShellsenseEqual -Actual ([string]$nativeRequest.id) -Expected 'native-1' -Message 'native request id is parsed'
Assert-ShellsenseEqual -Actual ([string]$nativeRequest.kind) -Expected 'native' -Message 'native request kind is parsed'
$numericRequest = ConvertFrom-ShellsenseRequestJson -Json '{"id":42,"kind":"native"}'
Assert-ShellsenseEqual -Actual ([string]$numericRequest.id) -Expected '42' -Message 'numeric request ids remain interoperable'
$publicObjectRequest = ConvertFrom-ShellsenseRequestJson -Json '{"id":"keys-1","kind":"commands_reset","public_keys":{"trigger":"Ctrl+Space","native":"Ctrl+Alt+Space"}}'
Assert-ShellsenseEqual -Actual ([string]$publicObjectRequest.public_keys_json) -Expected '{"trigger":"Ctrl+Space","native":"Ctrl+Alt+Space"}' -Message 'commands reset accepts an object public-key map'
$publicStringRequest = ConvertFrom-ShellsenseRequestJson -Json '{"id":"keys-2","kind":"commands_reset","public_keys_json":"{\"details\":\"F1\"}"}'
Assert-ShellsenseEqual -Actual ([string]$publicStringRequest.public_keys_json) -Expected '{"details":"F1"}' -Message 'commands reset accepts a serialized public-key map'
Assert-ShellsenseTrue -Condition ($null -eq (ConvertFrom-ShellsenseRequestJson -Json '{"id":"native-1"}')) -Message 'request without kind is rejected'
Assert-ShellsenseTrue -Condition ($null -eq (ConvertFrom-ShellsenseRequestJson -Json '[{"id":"native-1","kind":"native"}]')) -Message 'request arrays are rejected'

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
# only itself; a bare F12 reserves all children because it owns the
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
        buffer      = $false
        apply       = $false
        commands    = $false
        native      = $false
        paste       = $false
        enter       = $false
        shift_enter = $false
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
    # The wrapper must preserve the original ReadLine function's deliberate
    # native exit-code update, while suppressing only adapter side effects.
    $exitCodeBeforeReadLineTest = $ExecutionContext.SessionState.PSVariable.GetValue('global:LASTEXITCODE')
    $readLineBeforeExitCodeTest = $script:SHELLSENSE_ORIGINAL_READLINE
    try {
        $script:SHELLSENSE_ORIGINAL_READLINE = { $global:LASTEXITCODE = 23; 'Write-Output test' }
        $global:LASTEXITCODE = 7
        [Console]::SetOut($collisionCapture)
        $acceptedByWrapper = PSConsoleHostReadLine
        Assert-ShellsenseEqual -Actual $acceptedByWrapper -Expected 'Write-Output test' -Message 'ReadLine wrapper keeps accepted input'
        Assert-ShellsenseEqual -Actual $global:LASTEXITCODE -Expected 23 -Message 'ReadLine wrapper keeps original function exit-code update'
    } finally {
        [Console]::SetOut($consoleWriter)
        $script:SHELLSENSE_ORIGINAL_READLINE = $readLineBeforeExitCodeTest
        $global:LASTEXITCODE = $exitCodeBeforeReadLineTest
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
    $childSnapshot = Get-ShellsenseKeyHandlerSnapshot
    Assert-ShellsenseTrue -Condition ($childSnapshot.by_chord -is [System.Collections.Hashtable]) -Message 'key snapshot uses a case-insensitive hashtable'
    Assert-ShellsenseTrue -Condition ($childSnapshot.by_chord['f12,a'] -is [array]) -Message 'key snapshot chord bucket uses an object array'
    $mappedChild = @(Get-ShellsenseKeyBinding -Chord 'F12,A' -Snapshot $childSnapshot)
    Assert-ShellsenseEqual -Actual $mappedChild.Count -Expected 1 -Message 'case-insensitive chord lookup finds exact child binding'
    Assert-ShellsenseEqual -Actual ([string]$mappedChild[0].Function) -Expected 'existing apply' -Message 'mapped child lookup preserves binding metadata'

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
    Assert-ShellsenseEqual -Actual ([bool]$script:SHELLSENSE_KEY_HANDLERS.paste) -Expected $false -Message 'bare F12 preserves the parent binding for paste'
    $parentCollisionEvents = @(
        Get-ShellsenseCapturedEvents -Raw $parentCapture.ToString() -Token ([string]$script:SHELLSENSE_TOKEN) |
            Where-Object { $_.event -eq 'error' -and $_.code -eq 'key_chord_collision' }
    )
    Assert-ShellsenseEqual -Actual $parentCollisionEvents.Count -Expected 7 -Message 'bare F12 reports all reserved child and lifecycle collisions'
    Assert-ShellsenseTrue -Condition ([string]$parentCollisionEvents[0].suggestion -match 'SHELLSENSE_KEY_PREFIX=F(?:5|6|7|8|9|10|11)\.' -and
        [string]$parentCollisionEvents[0].suggestion -notmatch 'SHELLSENSE_KEY_PREFIX=F12') -Message 'collision suggests a different configurable protocol prefix'
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

# Public host shortcuts are optional. When configured, the standard
# PSReadLine MenuComplete binding is safe to supersede, while F1's built-in
# ShowCommandHelp and any user ScriptBlock remain owned by the user.
$publicKeyValues = [ordered]@{
    trigger = 'Ctrl+Space'
    native  = 'Ctrl+Alt+Space'
    details = 'F1'
    refresh = 'Ctrl+Alt+C'
    reload  = 'Ctrl+Alt+R'
}
$env:SHELLSENSE_PUBLIC_KEYS = '{"trigger":"Ctrl+Space","native":"Ctrl+Alt+Space","details":"F1","refresh":"Ctrl+Alt+C","reload":"Ctrl+Alt+R"}'
$env:SHELLSENSE_PUBLIC_KEYS_VERSION = '1'
foreach ($publicKeyName in $publicKeyValues.Keys) {
    [Environment]::SetEnvironmentVariable(
        ('SHELLSENSE_PUBLIC_KEY_' + $publicKeyName.ToUpperInvariant()),
        [string]$publicKeyValues[$publicKeyName],
        'Process')
}
$fastPublicConfiguration = Get-ShellsensePublicKeyConfiguration
Assert-ShellsenseEqual -Actual ([string]$fastPublicConfiguration.trigger) -Expected 'Ctrl+Space' -Message 'validated public-key environment fast path is used'
Assert-ShellsenseEqual -Actual $fastPublicConfiguration.Count -Expected 5 -Message 'validated public-key fast path exposes all keys'
$env:SHELLSENSE_PUBLIC_KEYS = '{"trigger":"F7","native":"Ctrl+Alt+Space","details":"F1","refresh":"Ctrl+Alt+C","reload":"Ctrl+Alt+R"}'
$env:SHELLSENSE_PUBLIC_KEY_RELOAD = $null
$fallbackPublicConfiguration = Get-ShellsensePublicKeyConfiguration
Assert-ShellsenseEqual -Actual ([string]$fallbackPublicConfiguration.trigger) -Expected 'F7' -Message 'missing validated key falls back to the original JSON'
$env:SHELLSENSE_PUBLIC_KEYS = '{"trigger":"Ctrl+Space","native":"Ctrl+Alt+Space","details":"F1","refresh":"Ctrl+Alt+C","reload":"Ctrl+Alt+R"}'
[Environment]::SetEnvironmentVariable('SHELLSENSE_PUBLIC_KEY_RELOAD', $publicKeyValues.reload, 'Process')
Reset-ShellsenseReadLineTestState
$publicCapture = [IO.StringWriter]::new([Globalization.CultureInfo]::InvariantCulture)
[Console]::SetOut($publicCapture)
try {
    Initialize-ShellsenseReadLine
    Send-ShellsenseCapabilities
} finally {
    [Console]::SetOut($consoleWriter)
}
$publicEvents = @(Get-ShellsenseCapturedEvents -Raw $publicCapture.ToString() -Token ([string]$script:SHELLSENSE_TOKEN))
$publicCapabilities = @($publicEvents | Where-Object { $_.event -eq 'capabilities' } | Select-Object -Last 1)
Assert-ShellsenseEqual -Actual $publicCapabilities.Count -Expected 1 -Message 'public-key capabilities are advertised when configured'
Assert-ShellsenseEqual -Actual ([bool]$publicCapabilities[0].capabilities.public_keys.trigger) -Expected $true -Message 'standard trigger binding is safe to supersede'
Assert-ShellsenseEqual -Actual ([bool]$publicCapabilities[0].capabilities.public_keys.native) -Expected $true -Message 'unbound native shortcut is available'
Assert-ShellsenseEqual -Actual ([bool]$publicCapabilities[0].capabilities.public_keys.details) -Expected $true -Message 'standard F1 help binding can coexist with menu details'
Assert-ShellsenseEqual -Actual ([bool]$publicCapabilities[0].capabilities.public_keys.refresh) -Expected $true -Message 'unbound refresh shortcut is available'
Assert-ShellsenseEqual -Actual ([bool]$publicCapabilities[0].capabilities.public_keys.reload) -Expected $true -Message 'unbound reload shortcut is available'

$customTriggerHandler = { }
[Microsoft.PowerShell.PSConsoleReadLine]::RemoveKeyHandler([string[]]@('Ctrl+Spacebar'))
[Microsoft.PowerShell.PSConsoleReadLine]::SetKeyHandler(
    [string[]]@('Ctrl+Spacebar'),
    $customTriggerHandler,
    'custom trigger test handler',
    'custom trigger test handler')
Reset-ShellsenseReadLineTestState
$customPublicCapture = [IO.StringWriter]::new([Globalization.CultureInfo]::InvariantCulture)
[Console]::SetOut($customPublicCapture)
try {
    Initialize-ShellsenseReadLine
    Send-ShellsenseCapabilities
} finally {
    [Console]::SetOut($consoleWriter)
}
$customPublicEvents = @(Get-ShellsenseCapturedEvents -Raw $customPublicCapture.ToString() -Token ([string]$script:SHELLSENSE_TOKEN))
$customPublicCapabilities = @($customPublicEvents | Where-Object { $_.event -eq 'capabilities' } | Select-Object -Last 1)
Assert-ShellsenseEqual -Actual ([bool]$customPublicCapabilities[0].capabilities.public_keys.trigger) -Expected $false -Message 'custom trigger binding is reported as a conflict'
$customTriggerBindings = @([Microsoft.PowerShell.PSConsoleReadLine]::GetKeyHandlers([string[]]@('Ctrl+Spacebar')))
Assert-ShellsenseEqual -Actual ([string]$customTriggerBindings[0].Function) -Expected 'custom trigger test handler' -Message 'custom trigger binding remains installed'
$env:SHELLSENSE_PUBLIC_KEYS = $null
$env:SHELLSENSE_PUBLIC_KEYS_VERSION = $null
foreach ($publicKeyName in @('TRIGGER', 'NATIVE', 'DETAILS', 'REFRESH', 'RELOAD')) {
    [Environment]::SetEnvironmentVariable(
        ('SHELLSENSE_PUBLIC_KEY_' + $publicKeyName),
        $null,
        'Process')
}
Get-ShellsensePublicKeyConfiguration | Out-Null

# A custom Enter binding must remain untouched.  The lifecycle override is
# only safe when PSReadLine still reports its built-in AcceptLine handler.
$customEnterHandler = { }
[Microsoft.PowerShell.PSConsoleReadLine]::RemoveKeyHandler([string[]]@('Enter'))
[Microsoft.PowerShell.PSConsoleReadLine]::SetKeyHandler(
    [string[]]@('Enter'),
    $customEnterHandler,
    'custom Enter test handler',
    'custom Enter test handler')
Reset-ShellsenseReadLineTestState
$customEnterCapture = [IO.StringWriter]::new([Globalization.CultureInfo]::InvariantCulture)
[Console]::SetOut($customEnterCapture)
try {
    Initialize-ShellsenseReadLine
} finally {
    [Console]::SetOut($consoleWriter)
}
Assert-ShellsenseEqual -Actual ([bool]$script:SHELLSENSE_KEY_HANDLERS.enter) -Expected $false -Message 'custom Enter leaves lifecycle override unavailable'
$customEnterBindings = @([Microsoft.PowerShell.PSConsoleReadLine]::GetKeyHandlers([string[]]@('Enter')))
Assert-ShellsenseEqual -Actual $customEnterBindings.Count -Expected 1 -Message 'custom Enter binding remains installed'
Assert-ShellsenseEqual -Actual ([string]$customEnterBindings[0].Function) -Expected 'custom Enter test handler' -Message 'custom Enter function is preserved'

# Native completion is reachable only from its manual request key.  A
# redirected/noninteractive runspace cannot provide a real PSReadLine buffer;
# it must still return a diagnostic status and a safe zero-width UTF-16 range.
$savedNativeToken = [string]$script:SHELLSENSE_TOKEN
$nativeToken = 'adapter-native-token'
$nativeCapture = [IO.StringWriter]::new([Globalization.CultureInfo]::InvariantCulture)
[Console]::SetOut($nativeCapture)
try {
    $script:SHELLSENSE_TOKEN = $nativeToken
    Get-ShellsenseNativeCompletion -RequestId 'native-error-1'
} finally {
    $script:SHELLSENSE_TOKEN = $savedNativeToken
    [Console]::SetOut($consoleWriter)
}
$nativeEvents = @(Get-ShellsenseCapturedEvents -Raw $nativeCapture.ToString() -Token $nativeToken)
Assert-ShellsenseEqual -Actual $nativeEvents.Count -Expected 1 -Message 'native failure still returns one protocol frame'
Assert-ShellsenseEqual -Actual ([string]$nativeEvents[0].event) -Expected 'native_completion' -Message 'native failure event type'
Assert-ShellsenseEqual -Actual ([string]$nativeEvents[0].request_id) -Expected 'native-error-1' -Message 'native failure request id'
Assert-ShellsenseTrue -Condition ([string]$nativeEvents[0].status -in @('error', 'unavailable')) -Message 'native failure returns a diagnostic status'
Assert-ShellsenseEqual -Actual ([int]$nativeEvents[0].replace_start) -Expected 0 -Message 'native failure has a safe replacement start'
Assert-ShellsenseEqual -Actual ([int]$nativeEvents[0].replace_end) -Expected 0 -Message 'native failure has a safe replacement end'
Assert-ShellsenseEqual -Actual (@($nativeEvents[0].candidates).Count) -Expected 0 -Message 'native failure has no candidates'

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
$commandProvider = { return ,$fixtureCommands }
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
        Assert-ShellsenseTrue -Condition ($batch.Count -le 64) -Message 'snapshot batch is bounded at 64 entries'
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
    Assert-ShellsenseTrue -Condition ((@($secondEvents[0].commands).Count -gt 0) -and (@($secondEvents[0].commands).Count -le 64)) -Message 'new snapshot starts with a bounded non-empty batch'
    # Drain the second fixture snapshot before leaving the test so later
    # adapter calls start from a clean cursor as well.
    while ($null -ne $script:SHELLSENSE_COMMAND_SNAPSHOT_ID) {
        Get-ShellsenseImportedCommands -CommandProvider $commandProvider
    }

    # A commands_reset request must discard the old enumerator and publish a
    # new snapshot id, with the request id attached to its first batch.
    $seedCapture = [IO.StringWriter]::new([Globalization.CultureInfo]::InvariantCulture)
    [Console]::SetOut($seedCapture)
    Get-ShellsenseImportedCommands -CommandProvider $commandProvider
    [Console]::SetOut($consoleWriter)
    $seedEvents = @(Get-ShellsenseCapturedEvents -Raw $seedCapture.ToString() -Token $commandToken)
    Assert-ShellsenseEqual -Actual $seedEvents.Count -Expected 1 -Message 'reset test seeds one command batch'
    $seedSnapshotId = [string]$seedEvents[0].snapshot

    $resetRequestPath = Get-ShellsenseRequestPath
    [IO.File]::WriteAllText(
        $resetRequestPath,
        '{"id":"commands-reset-1","kind":"commands_reset"}',
        [Text.UTF8Encoding]::new($false))
    $resetCapture = [IO.StringWriter]::new([Globalization.CultureInfo]::InvariantCulture)
    [Console]::SetOut($resetCapture)
    try {
        Invoke-ShellsenseCommandsKeyHandler
    } finally {
        [Console]::SetOut($consoleWriter)
        Remove-Item -LiteralPath $resetRequestPath -Force -ErrorAction SilentlyContinue
    }
    $resetEvents = @(Get-ShellsenseCapturedEvents -Raw $resetCapture.ToString() -Token $commandToken)
    Assert-ShellsenseEqual -Actual $resetEvents.Count -Expected 1 -Message 'commands_reset returns one fresh batch'
    Assert-ShellsenseTrue -Condition (-not [string]::Equals([string]$resetEvents[0].snapshot, $seedSnapshotId, [StringComparison]::Ordinal)) -Message 'commands_reset replaces the previous snapshot id'
    Assert-ShellsenseEqual -Actual ([string]$resetEvents[0].request_id) -Expected 'commands-reset-1' -Message 'commands_reset request id is acknowledged on the batch'

    # A host reload can carry its new public shortcut map in the same reset
    # request. Start from the custom Ctrl+Space collision installed above,
    # then switch trigger to an unbound chord and require a fresh capability
    # frame before the replacement command batch.
    $env:SHELLSENSE_PUBLIC_KEYS = '{"trigger":"Ctrl+Space"}'
    Update-ShellsensePublicKeyCapabilities
    Assert-ShellsenseEqual -Actual ([bool]$script:SHELLSENSE_PUBLIC_KEY_STATUS.trigger) -Expected $false -Message 'pre-reset public trigger is still colliding'
    $env:SHELLSENSE_PUBLIC_KEYS = $null
    $rebindRequestPath = Get-ShellsenseRequestPath
    [IO.File]::WriteAllText(
        $rebindRequestPath,
        '{"id":"commands-reset-keys","kind":"commands_reset","public_keys_json":"{\"trigger\":\"Ctrl+Alt+Space\"}"}',
        [Text.UTF8Encoding]::new($false))
    $rebindCapture = [IO.StringWriter]::new([Globalization.CultureInfo]::InvariantCulture)
    [Console]::SetOut($rebindCapture)
    try {
        Invoke-ShellsenseCommandsKeyHandler
    } finally {
        [Console]::SetOut($consoleWriter)
        Remove-Item -LiteralPath $rebindRequestPath -Force -ErrorAction SilentlyContinue
    }
    $rebindEvents = @(Get-ShellsenseCapturedEvents -Raw $rebindCapture.ToString() -Token $commandToken)
    $rebindCapabilities = @($rebindEvents | Where-Object { $_.event -eq 'capabilities' })
    Assert-ShellsenseEqual -Actual $rebindCapabilities.Count -Expected 1 -Message 'public-key reset emits fresh capabilities'
    Assert-ShellsenseEqual -Actual ([bool]$rebindCapabilities[0].capabilities.public_keys.trigger) -Expected $true -Message 'public-key reset re-arbitrates the new trigger chord'
    $script:SHELLSENSE_PUBLIC_KEY_STATUS = $null
    Reset-ShellsenseCommandSnapshot
} finally {
    $script:SHELLSENSE_TOKEN = $savedCommandToken
    [Console]::SetOut($consoleWriter)
}
$fixtureNamesAfter = @($ExecutionContext.InvokeCommand.GetCommands('shellsenseFixture*', $commandTypes, $true))
Assert-ShellsenseEqual -Actual $fixtureNamesAfter.Count -Expected $fixtureNamesBefore.Count -Message 'fixture provider does not pollute loaded commands'

# Production command discovery must hand the lazy GetCommands sequence to the
# enumerator without first routing it through a PowerShell pipeline.  A direct
# assignment therefore remains a non-array IEnumerable.
$loadedSequence = Get-ShellsenseLoadedCommands
Assert-ShellsenseTrue -Condition ($loadedSequence -is [System.Collections.IEnumerable] -and $loadedSequence -isnot [array]) -Message 'loaded command sequence remains lazy'
$loadedEnumerator = Get-ShellsenseEnumerator -Sequence $loadedSequence
Assert-ShellsenseTrue -Condition ($loadedEnumerator -is [System.Collections.IEnumerator]) -Message 'lazy sequence exposes an enumerator'
$realGiRecord = $null
while ($loadedEnumerator.MoveNext()) {
    if ([string]::Equals([string]$loadedEnumerator.Current.Name, 'gi', [StringComparison]::OrdinalIgnoreCase)) {
        $realGiRecord = ConvertTo-ShellsenseCommandRecord -Command $loadedEnumerator.Current
        break
    }
}
Assert-ShellsenseTrue -Condition ($null -ne $realGiRecord) -Message 'real same-runspace command enumeration includes gi alias'
Assert-ShellsenseEqual -Actual ([string]$realGiRecord.kind) -Expected 'alias' -Message 'real gi record retains alias kind'
Assert-ShellsenseEqual -Actual ([string]$realGiRecord.definition) -Expected 'Get-Item' -Message 'real gi record retains alias target'
Reset-ShellsenseCommandSnapshot

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

function Test-ShellsenseMetadataFixture {
    <#
    .SYNOPSIS
    查看元数据测试说明。
    .PARAMETER Mode
    选择测试模式。
    #>
    param([ValidateSet('fast','slow')][string]$Mode, [switch]$Force)
    dynamicparam { throw 'must not evaluate dynamic parameters' }
    process { throw 'must not execute function' }
}
$metadata = Get-ShellsenseCommandMetadata -Name Test-ShellsenseMetadataFixture
Assert-ShellsenseEqual -Actual ([string]$metadata.description).Trim() -Expected '查看元数据测试说明。' -Message 'metadata reads comment help without execution'
Assert-ShellsenseEqual -Actual $metadata.options.Count -Expected 2 -Message 'metadata uses static parameters only'
$modeMetadata = @($metadata.options | Where-Object { $_.names[0] -eq '-Mode' })[0]
Assert-ShellsenseEqual -Actual ($modeMetadata.values -join ',') -Expected 'fast,slow' -Message 'static ValidateSet values are data'
Assert-ShellsenseEqual -Actual (Get-ShellsenseCommandMetadata -Name 'x;evil') -Expected $null -Message 'metadata rejects expressions'
Remove-Item Function:Test-ShellsenseMetadataFixture
Write-Output 'PowerShell adapter tests passed.'
