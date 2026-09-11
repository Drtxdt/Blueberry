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
