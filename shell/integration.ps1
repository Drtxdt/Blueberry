# Shellsense PowerShell integration.
#
# This file is intended to be dot-sourced by the pwsh process owned by
# shellsense.  It deliberately uses only the PowerShell and PSReadLine APIs
# already present in that process; completion text is never evaluated here.

# Keep state in this script scope. Dot-sourcing makes the functions below
# available to the interactive session while script scope keeps the original
# host functions and the protocol token private to this adapter. Initialize all
# state names before reading them so a user's Set-StrictMode in their profile
# does not make the adapter fail during bootstrap.
$shellsenseStateDefaults = [ordered]@{
    SHELLSENSE_TOKEN               = $null
    SHELLSENSE_EDIT_PATH           = $null
    SHELLSENSE_FALLBACK_CWD        = $null
    SHELLSENSE_ORIGINAL_PROMPT     = $null
    SHELLSENSE_PROMPT_WRAPPED      = $false
    SHELLSENSE_ORIGINAL_READLINE   = $null
    SHELLSENSE_READLINE_WRAPPED    = $false
    SHELLSENSE_PSREADLINE_AVAILABLE = $false
    SHELLSENSE_KEY_HANDLERS        = [ordered]@{ buffer = $false; apply = $false; commands = $false }
    SHELLSENSE_CAPABILITIES_REFRESHED = $false
    SHELLSENSE_COMMAND_SNAPSHOT_ID = $null
    SHELLSENSE_COMMAND_SNAPSHOT = $null
    SHELLSENSE_COMMAND_SNAPSHOT_OFFSET = 0
    SHELLSENSE_JSON_OPTIONS        = $null
    SHELLSENSE_TRACE_ENABLED       = $false
    SHELLSENSE_TRACE_EMITTING      = $false
    SHELLSENSE_INITIALIZED         = $false
}
foreach ($shellsenseStateName in $shellsenseStateDefaults.Keys) {
    # PSVariable is part of the host API and does not auto-import
    # Microsoft.PowerShell.Utility the way Get-Variable/Set-Variable do.
    # Keeping bootstrap on this API avoids a cold-start module load before the
    # user's first prompt.
    if ($null -eq $ExecutionContext.SessionState.PSVariable.Get($shellsenseStateName)) {
        $ExecutionContext.SessionState.PSVariable.Set($shellsenseStateName, $shellsenseStateDefaults[$shellsenseStateName])
    }
}

$script:SHELLSENSE_TRACE_ENABLED = [string]::Equals(
    [Environment]::GetEnvironmentVariable('SHELLSENSE_TRACE', 'Process'),
    '1',
    [StringComparison]::Ordinal)

$shellsenseTokenFromEnvironment = [Environment]::GetEnvironmentVariable('SHELLSENSE_TOKEN', 'Process')
if ([string]::IsNullOrEmpty([string]$script:SHELLSENSE_TOKEN)) {
    $script:SHELLSENSE_TOKEN = $shellsenseTokenFromEnvironment
}

$shellsenseEditPathFromEnvironment = [Environment]::GetEnvironmentVariable('SHELLSENSE_EDIT_PATH', 'Process')
if ($null -ne $shellsenseEditPathFromEnvironment) {
    $script:SHELLSENSE_EDIT_PATH = $shellsenseEditPathFromEnvironment
}

# The token is inherited by this process only to bootstrap the OSC channel.
# Do not leave it available to commands run by the user.
try {
    [Environment]::SetEnvironmentVariable('SHELLSENSE_TOKEN', $null, 'Process')
} catch {
    # The process environment is the source used by child processes.  Do not
    # fall back to Remove-Item here: that cmdlet can cold-load a large module
    # during startup, and PowerShell reflects the direct environment change.
}

function Get-ShellsenseJsonOptions {
    [CmdletBinding()]
    param()

    if ($null -eq $script:SHELLSENSE_JSON_OPTIONS) {
        $options = [System.Text.Json.JsonSerializerOptions]::new()
        # JavaScriptEncoder.Default escapes all non-ASCII code points and
        # control characters. This keeps OSC transport ASCII, including a
        # non-BMP surrogate pair, while leaving JSON's structural characters
        # available in their compact form.
        $options.Encoder = [System.Text.Encodings.Web.JavaScriptEncoder]::Default
        $options.WriteIndented = $false
        $script:SHELLSENSE_JSON_OPTIONS = $options
    }
    return $script:SHELLSENSE_JSON_OPTIONS
}

function Send-ShellsenseTrace {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [string]$Stage,

        [Parameter(Mandatory = $true)]
        [double]$DurationMs
    )

    if (-not $script:SHELLSENSE_TRACE_ENABLED -or
        $script:SHELLSENSE_TRACE_EMITTING -or
        [string]::IsNullOrEmpty([string]$script:SHELLSENSE_TOKEN)) {
        return
    }
    if ($Stage -notin @('adapter_bootstrap', 'readline_init', 'command_snapshot', 'serialize')) {
        return
    }
    if ([double]::IsNaN($DurationMs) -or [double]::IsInfinity($DurationMs)) {
        return
    }

    $script:SHELLSENSE_TRACE_EMITTING = $true
    try {
        Send-ShellsenseEvent -Event 'trace' -Data ([ordered]@{
            stage       = $Stage
            duration_ms = [double]$DurationMs
        })
    } finally {
        $script:SHELLSENSE_TRACE_EMITTING = $false
    }
}

function ConvertTo-ShellsenseJson {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [AllowNull()]
        [object]$Payload
    )

    $traceTimer = $null
    if ($script:SHELLSENSE_TRACE_ENABLED) {
        $traceTimer = [Diagnostics.Stopwatch]::StartNew()
    }
    try {
        # The explicit object/type/options overload keeps PowerShell from
        # selecting a generic overload that changes dictionary shape. The
        # cached options keep the first call from repeatedly constructing an
        # encoder and ensure the transport remains ASCII.
        return [string][System.Text.Json.JsonSerializer]::Serialize(
            [object]$Payload,
            [object],
            (Get-ShellsenseJsonOptions))
    } finally {
        if ($null -ne $traceTimer) {
            $traceTimer.Stop()
            Send-ShellsenseTrace -Stage 'serialize' -DurationMs $traceTimer.Elapsed.TotalMilliseconds
        }
    }
}

function ConvertTo-ShellsenseFrame {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [string]$Token,

        [Parameter(Mandatory = $true)]
        [AllowNull()]
        [object]$Payload
    )

    if ([string]::IsNullOrEmpty($Token)) {
        throw 'Shellsense protocol token is empty.'
    }

    $json = ConvertTo-ShellsenseJson -Payload $Payload
    $escape = [char]27
    $bell = [char]7
    return [string]::Concat($escape, ']7776;', $Token, ';', $json, $bell)
}

function Send-ShellsenseEvent {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [ValidateNotNullOrEmpty()]
        [string]$Event,

        [AllowNull()]
        [System.Collections.IDictionary]$Data
    )

    if ([string]::IsNullOrEmpty([string]$script:SHELLSENSE_TOKEN)) {
        return
    }

    $payload = [ordered]@{ event = $Event }
    if ($null -ne $Data) {
        foreach ($key in $Data.Keys) {
            if ([string]$key -eq 'event') {
                continue
            }
            $payload[[string]$key] = $Data[$key]
        }
    }

    try {
        $frame = ConvertTo-ShellsenseFrame -Token ([string]$script:SHELLSENSE_TOKEN) -Payload $payload
        # [Console]::Out avoids adding the frame to a PowerShell pipeline (and
        # therefore avoids contaminating prompt text or command output).
        [Console]::Out.Write($frame)
        [Console]::Out.Flush()
    } catch {
        # A diagnostic channel failure must not break the user's shell.  In
        # particular, do not write the exception to the terminal: that would
        # be indistinguishable from command output to the host.
    }
}

function Get-ShellsenseWorkingDirectory {
    [CmdletBinding()]
    param()

    try {
        $location = $ExecutionContext.SessionState.Path.CurrentLocation
        if ($null -ne $location.Provider -and $location.Provider.Name -eq 'FileSystem') {
            if (-not [string]::IsNullOrEmpty([string]$location.Path)) {
                return [string]$location.Path
            }
        }
    } catch {
    }

    # A provider such as Registry:, Cert:, or Variable: has no filesystem
    # path.  Prefer stable, useful filesystem locations in that case.
    $fallbacks = @()
    if ($null -ne $script:SHELLSENSE_FALLBACK_CWD) {
        $fallbacks += [string]$script:SHELLSENSE_FALLBACK_CWD
    }
    try {
        $userProfile = [Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)
        if (-not [string]::IsNullOrEmpty($userProfile)) {
            $fallbacks += $userProfile
        }
    } catch {
    }
    if (-not [string]::IsNullOrEmpty([string]$env:USERPROFILE)) {
        $fallbacks += [string]$env:USERPROFILE
    }
    try {
        if (-not [string]::IsNullOrEmpty([string][Environment]::CurrentDirectory)) {
            $fallbacks += [string][Environment]::CurrentDirectory
        }
    } catch {
    }
    if (-not [string]::IsNullOrEmpty([string]$env:SystemDrive)) {
        $fallbacks += ([string]$env:SystemDrive + '\')
    }

    foreach ($candidate in $fallbacks) {
        if ([string]::IsNullOrEmpty([string]$candidate)) {
            continue
        }
        try {
            if ([IO.Directory]::Exists([string]$candidate)) {
                return [string]$candidate
            }
        } catch {
        }
    }

    # This final value is still a filesystem shaped path on supported Windows
    # hosts.  It is only reached if the process has an unusual environment.
    return [string][Environment]::CurrentDirectory
}

function Get-ShellsensePromptEndData {
    [CmdletBinding()]
    param()

    $pathValue = [string]$env:PATH
    $pathExtValue = [string]$env:PATHEXT
    return [ordered]@{
        cwd    = Get-ShellsenseWorkingDirectory
        path   = $pathValue
        pathext = $pathExtValue
        pid    = [int]$PID
    }
}

function Send-ShellsenseBuffer {
    [CmdletBinding()]
    param()

    try {
        $line = $null
        $cursor = 0
        [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line, [ref]$cursor)
        Send-ShellsenseEvent -Event 'buffer' -Data ([ordered]@{
            line   = [string]$line
            cursor = [int]$cursor
        })
    } catch {
        Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
            code    = 'buffer_unavailable'
            message = 'PSReadLine could not provide the current buffer.'
        })
    }
}

function Test-ShellsenseUtf16Range {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$Line,

        [Parameter(Mandatory = $true)]
        [int64]$Start,

        [Parameter(Mandatory = $true)]
        [int64]$Length
    )

    if ($Start -lt 0 -or $Length -lt 0) {
        return $false
    }
    if ($Start -gt [int64]$Line.Length -or $Length -gt ([int64]$Line.Length - $Start)) {
        return $false
    }

    $end = $Start + $Length
    # Never cut between the two UTF-16 code units of a surrogate pair.
    if ($Start -gt 0 -and $Start -lt $Line.Length) {
        if ([char]::IsHighSurrogate($Line[[int]($Start - 1)]) -and
            [char]::IsLowSurrogate($Line[[int]$Start])) {
            return $false
        }
    }
    if ($end -gt 0 -and $end -lt $Line.Length) {
        if ([char]::IsHighSurrogate($Line[[int]($end - 1)]) -and
            [char]::IsLowSurrogate($Line[[int]$end])) {
            return $false
        }
    }
    return $true
}

function Test-ShellsenseNonNegativeInteger {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [AllowNull()]
        [object]$Value,

        [Parameter(Mandatory = $true)]
        [ref]$Result
    )

    if ($null -eq $Value -or $Value -is [bool]) {
        return $false
    }

    $parsed = [int64]0
    $text = [string]$Value
    if ($text -notmatch '^[0-9]+$') {
        return $false
    }
    if (-not [int64]::TryParse(
            $text,
            [Globalization.NumberStyles]::Integer,
            [Globalization.CultureInfo]::InvariantCulture,
            [ref]$parsed)) {
        return $false
    }
    $Result.Value = $parsed
    return $true
}

function Test-ShellsenseEditPayload {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [AllowNull()]
        [object]$Payload,

        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$CurrentLine,

        [Parameter(Mandatory = $true)]
        [int]$CurrentCursor,

        [Parameter(Mandatory = $true)]
        [ref]$Edit
    )

    $Edit.Value = $null
    if ($null -eq $Payload -or $Payload -is [array] -or $Payload -is [string]) {
        return $false
    }

    $properties = @($Payload.PSObject.Properties.Name)
    foreach ($required in @('expectedLine', 'expectedCursor', 'start', 'length', 'text')) {
        if ($properties -notcontains $required) {
            return $false
        }
    }

    $expectedLineProperty = $Payload.PSObject.Properties['expectedLine']
    $textProperty = $Payload.PSObject.Properties['text']
    if ($null -eq $expectedLineProperty -or $null -eq $textProperty) {
        return $false
    }
    if ($expectedLineProperty.Value -isnot [string] -or $textProperty.Value -isnot [string]) {
        return $false
    }
    if (-not [string]::Equals([string]$expectedLineProperty.Value, $CurrentLine, [StringComparison]::Ordinal)) {
        return $false
    }

    $expectedCursor = [int64]0
    $start = [int64]0
    $length = [int64]0
    if (-not (Test-ShellsenseNonNegativeInteger -Value $Payload.expectedCursor -Result ([ref]$expectedCursor))) {
        return $false
    }
    if (-not (Test-ShellsenseNonNegativeInteger -Value $Payload.start -Result ([ref]$start))) {
        return $false
    }
    if (-not (Test-ShellsenseNonNegativeInteger -Value $Payload.length -Result ([ref]$length))) {
        return $false
    }
    if ($expectedCursor -ne [int64]$CurrentCursor) {
        return $false
    }
    if (-not (Test-ShellsenseUtf16Range -Line $CurrentLine -Start ([int64]$CurrentCursor) -Length 0)) {
        return $false
    }
    if (-not (Test-ShellsenseUtf16Range -Line $CurrentLine -Start $start -Length $length)) {
        return $false
    }

    $Edit.Value = [ordered]@{
        start  = [int]$start
        length = [int]$length
        text   = [string]$textProperty.Value
    }
    return $true
}

function Get-ShellsenseConsumedEditPath {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $directory = [IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($Path))
    $leaf = [IO.Path]::GetFileName($Path)
    if ([string]::IsNullOrEmpty($leaf)) {
        $leaf = 'shellsense-edit.json'
    }
    return [IO.Path]::Combine($directory, ('.' + $leaf + '.consumed.' + [Guid]::NewGuid().ToString('N')))
}

function Convert-ShellsenseJsonElementValue {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [System.Text.Json.JsonElement]$Element
    )

    switch ($Element.ValueKind) {
        ([System.Text.Json.JsonValueKind]::String) {
            return $Element.GetString()
        }
        ([System.Text.Json.JsonValueKind]::Number) {
            $integer = [int64]0
            if ($Element.TryGetInt64([ref]$integer)) {
                return $integer
            }
            $decimal = [decimal]0
            if ($Element.TryGetDecimal([ref]$decimal)) {
                return $decimal
            }
            $double = [double]0
            if ($Element.TryGetDouble([ref]$double)) {
                return $double
            }
            return $Element.ToString()
        }
        ([System.Text.Json.JsonValueKind]::True) {
            return $true
        }
        ([System.Text.Json.JsonValueKind]::False) {
            return $false
        }
        ([System.Text.Json.JsonValueKind]::Null) {
            return $null
        }
        default {
            # Keep arrays/objects as JsonElement values. The edit validator
            # rejects them for every scalar field instead of coercing nested
            # data into executable or ambiguous text.
            return $Element
        }
    }
}

function ConvertFrom-ShellsenseEditJson {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$Json
    )

    # Deserialize to JsonElement first so a valid JSON array/null can be
    # rejected by the normal edit-payload validator, while malformed JSON
    # still follows Invoke-ShellsenseApplyEdit's existing failure path.
    $rootValue = [System.Text.Json.JsonSerializer]::Deserialize(
        $Json,
        [object],
        (Get-ShellsenseJsonOptions))
    if ($null -eq $rootValue -or $rootValue.ValueKind -ne [System.Text.Json.JsonValueKind]::Object) {
        return $null
    }

    $payload = [ordered]@{}
    foreach ($propertyName in @('expectedLine', 'expectedCursor', 'start', 'length', 'text')) {
        $property = [System.Text.Json.JsonElement]::new()
        if ($rootValue.TryGetProperty($propertyName, [ref]$property)) {
            $payload[$propertyName] = Convert-ShellsenseJsonElementValue -Element $property
        }
    }
    return [pscustomobject]$payload
}

function Invoke-ShellsenseApplyEdit {
    [CmdletBinding()]
    param()

    $path = [string]$script:SHELLSENSE_EDIT_PATH
    if ([string]::IsNullOrEmpty($path)) {
        Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
            code    = 'edit_path_unavailable'
            message = 'SHELLSENSE_EDIT_PATH is not configured.'
        })
        return
    }

    $consumedPath = $null
    try {
        if (-not [IO.File]::Exists($path)) {
            Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
                code    = 'edit_payload_missing'
                message = 'No pending edit payload is available.'
            })
            return
        }

        # Rename first so a payload written while this handler is running is
        # left for the next invocation.  The consumed name prevents replay.
        $consumedPath = Get-ShellsenseConsumedEditPath -Path $path
        [IO.File]::Move($path, $consumedPath)

        $utf8Strict = [Text.UTF8Encoding]::new($false, $true)
        $json = [IO.File]::ReadAllText($consumedPath, $utf8Strict)
        $payload = ConvertFrom-ShellsenseEditJson -Json $json

        $line = $null
        $cursor = 0
        [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line, [ref]$cursor)
        $edit = $null
        if (-not (Test-ShellsenseEditPayload -Payload $payload -CurrentLine ([string]$line) -CurrentCursor ([int]$cursor) -Edit ([ref]$edit))) {
            Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
                code    = 'edit_payload_rejected'
                message = 'The edit payload does not match the current UTF-16 buffer.'
            })
            return
        }

        # This is the only edit operation.  The text is passed as data to
        # Replace; it is never parsed or invoked as PowerShell.
        [Microsoft.PowerShell.PSConsoleReadLine]::Replace(
            [int]$edit.start,
            [int]$edit.length,
            [string]$edit.text)
        Send-ShellsenseBuffer
    } catch {
        Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
            code    = 'edit_failed'
            message = 'The pending edit could not be applied.'
        })
    } finally {
        if ($null -ne $consumedPath) {
            try {
                [IO.File]::Delete($consumedPath)
            } catch {
            }
        }
    }
}

function Get-ShellsenseLoadedCommands {
    [CmdletBinding()]
    param()

    # CommandInvocationIntrinsics searches the current runspace only. Unlike
    # Get-Command, it does not resolve commands from module manifests or load
    # an available module as a side effect of a wildcard search.
    $commandTypes = [System.Management.Automation.CommandTypes]::Alias -bor
        [System.Management.Automation.CommandTypes]::Function -bor
        [System.Management.Automation.CommandTypes]::Cmdlet
    return @($ExecutionContext.InvokeCommand.GetCommands('*', $commandTypes, $true))
}

function Get-ShellsenseImportedCommands {
    [CmdletBinding()]
    param(
        # This optional provider keeps the snapshot protocol deterministic in
        # adapter tests without creating functions or aliases in the runspace.
        # The key handler calls the default runspace provider below.
        [AllowNull()]
        [scriptblock]$CommandProvider
    )

    $batchSize = 128
    $traceTimer = $null
    if ($script:SHELLSENSE_TRACE_ENABLED) {
        $traceTimer = [Diagnostics.Stopwatch]::StartNew()
    }
    try {
        $snapshotId = [string]$script:SHELLSENSE_COMMAND_SNAPSHOT_ID
        if ([string]::IsNullOrEmpty($snapshotId)) {
            $commands = if ($null -ne $CommandProvider) {
                @(& $CommandProvider)
            } else {
                @(Get-ShellsenseLoadedCommands)
            }

            $items = [System.Collections.Generic.List[object]]::new()
            foreach ($command in $commands) {
                if ($null -eq $command -or $null -eq $command.Name) {
                    continue
                }

                $kind = ([string]$command.CommandType).ToLowerInvariant()
                if ($kind -ne 'alias' -and $kind -ne 'function' -and $kind -ne 'cmdlet') {
                    continue
                }
                $definition = ''
                # Function and cmdlet definitions can be large or executable
                # text. Aliases retain only their target for root priority and
                # display; returned text is never evaluated by the host.
                if ($kind -eq 'alias' -and $null -ne $command.Definition) {
                    $definition = [string]$command.Definition
                }
                [void]$items.Add([ordered]@{
                    name       = [string]$command.Name
                    kind       = $kind
                    definition = $definition
                })
            }

            $script:SHELLSENSE_COMMAND_SNAPSHOT_ID = [Guid]::NewGuid().ToString()
            $script:SHELLSENSE_COMMAND_SNAPSHOT = [object[]]$items.ToArray()
            $script:SHELLSENSE_COMMAND_SNAPSHOT_OFFSET = 0
            $snapshotId = [string]$script:SHELLSENSE_COMMAND_SNAPSHOT_ID
        }

        $snapshotCommands = @($script:SHELLSENSE_COMMAND_SNAPSHOT)
        $offset = [int]$script:SHELLSENSE_COMMAND_SNAPSHOT_OFFSET
        if ($offset -lt 0 -or $offset -gt $snapshotCommands.Count) {
            throw 'Command snapshot offset is invalid.'
        }

        $end = [Math]::Min($offset + $batchSize, $snapshotCommands.Count)
        $batch = [System.Collections.Generic.List[object]]::new()
        for ($index = $offset; $index -lt $end; $index++) {
            [void]$batch.Add($snapshotCommands[$index])
        }
        $complete = $end -ge $snapshotCommands.Count
        $script:SHELLSENSE_COMMAND_SNAPSHOT_OFFSET = $end

        Send-ShellsenseEvent -Event 'commands' -Data ([ordered]@{
            snapshot = $snapshotId
            complete = [bool]$complete
            commands = @($batch.ToArray())
        })

        # A completed request is also the reset point. The next F12,c starts a
        # fresh UUID and snapshot, while an incomplete request keeps this
        # batch cursor for the host's next chord call.
        if ($complete) {
            $script:SHELLSENSE_COMMAND_SNAPSHOT_ID = $null
            $script:SHELLSENSE_COMMAND_SNAPSHOT = $null
            $script:SHELLSENSE_COMMAND_SNAPSHOT_OFFSET = 0
        }
    } catch {
        $script:SHELLSENSE_COMMAND_SNAPSHOT_ID = $null
        $script:SHELLSENSE_COMMAND_SNAPSHOT = $null
        $script:SHELLSENSE_COMMAND_SNAPSHOT_OFFSET = 0
        Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
            code    = 'commands_unavailable'
            message = 'Loaded command enumeration failed.'
        })
    } finally {
        if ($null -ne $traceTimer) {
            $traceTimer.Stop()
            Send-ShellsenseTrace -Stage 'command_snapshot' -DurationMs $traceTimer.Elapsed.TotalMilliseconds
        }
    }
}

function Invoke-ShellsenseBufferKeyHandler {
    [CmdletBinding()]
    param()
    Send-ShellsenseBuffer
}

function Invoke-ShellsenseApplyKeyHandler {
    [CmdletBinding()]
    param()
    Invoke-ShellsenseApplyEdit
}

function Invoke-ShellsenseCommandsKeyHandler {
    [CmdletBinding()]
    param()
    Get-ShellsenseImportedCommands
}

function Test-ShellsenseInteractiveHost {
    [CmdletBinding()]
    param()

    # -NonInteractive is explicit and reliable for tests and redirected jobs.
    # Windows Terminal/ConPTY launches omit it, even when the adapter's stdin
    # is represented as a redirected stream by the host process.
    try {
        $arguments = @([Environment]::GetCommandLineArgs())
        foreach ($argument in $arguments) {
            if ([string]::Equals([string]$argument, '-NonInteractive', [StringComparison]::OrdinalIgnoreCase) -or
                [string]::Equals([string]$argument, '-noni', [StringComparison]::OrdinalIgnoreCase)) {
                return $false
            }
        }
    } catch {
    }
    try {
        return ($Host.Name -eq 'ConsoleHost')
    } catch {
        return $false
    }
}

function Get-ShellsenseKeyBinding {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [string]$Chord
    )

    try {
        # The public PSConsoleReadLine API returns the handlers for each key
        # in a chord without invoking the Get-PSReadLineKeyHandler cmdlet.
        # The cmdlet fallback keeps this usable with older PSReadLine builds,
        # but is used only when the static API itself is unavailable.
        # GetKeyHandlers expects an array of chord specifications. Passing
        # the comma-delimited chord as one element keeps `s` from being
        # mistaken for an independent SelfInsert binding.
        $keys = [string[]]@($Chord)
        return @([Microsoft.PowerShell.PSConsoleReadLine]::GetKeyHandlers($keys))
    } catch {
        try {
            return @(Get-PSReadLineKeyHandler -Chord $Chord -ErrorAction SilentlyContinue)
        } catch {
            return @()
        }
    }
}

function Register-ShellsenseKeyHandler {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [string]$Chord,

        [Parameter(Mandatory = $true)]
        [scriptblock]$ScriptBlock,

        [Parameter(Mandatory = $true)]
        [string]$Name,

        [switch]$SkipCollisionCheck
    )

    if (-not $SkipCollisionCheck -and @(Get-ShellsenseKeyBinding -Chord $Chord).Count -gt 0) {
        Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
            code    = 'key_chord_collision'
            chord   = $Chord
            message = ('Reserved chord {0} is already bound; it was left unchanged.' -f $Chord)
        })
        return $false
    }

    try {
        [Microsoft.PowerShell.PSConsoleReadLine]::SetKeyHandler(
            [string[]]@($Chord),
            $ScriptBlock,
            ('Shellsense ' + $Name),
            ('Report shellsense ' + $Name + ' state.'))
        return $true
    } catch {
        Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
            code    = 'key_handler_registration_failed'
            chord   = $Chord
            message = ('Reserved chord {0} could not be registered.' -f $Chord)
        })
        return $false
    }
}

function Send-ShellsenseCapabilities {
    [CmdletBinding()]
    param()

    Send-ShellsenseEvent -Event 'capabilities' -Data ([ordered]@{
        # `ready` means the complete PSReadLine surface is usable. Consumers
        # can still use prompt/execute events when this is false, while the
        # detailed fields below explain whether a missing module or a chord
        # collision is responsible.
        ready        = ([bool]$script:SHELLSENSE_PSREADLINE_AVAILABLE -and
            [bool]$script:SHELLSENSE_KEY_HANDLERS.buffer -and
            [bool]$script:SHELLSENSE_KEY_HANDLERS.apply -and
            [bool]$script:SHELLSENSE_KEY_HANDLERS.commands)
        psreadline   = [bool]$script:SHELLSENSE_PSREADLINE_AVAILABLE
        key_handlers = $script:SHELLSENSE_KEY_HANDLERS
        edit_path    = (-not [string]::IsNullOrEmpty([string]$script:SHELLSENSE_EDIT_PATH))
    })
}

function Initialize-ShellsenseReadLine {
    [CmdletBinding()]
    param(
        [switch]$DeferImport
    )

    $traceTimer = $null
    if ($script:SHELLSENSE_TRACE_ENABLED) {
        $traceTimer = [Diagnostics.Stopwatch]::StartNew()
    }
    try {
        $script:SHELLSENSE_PSREADLINE_AVAILABLE = $false
        $script:SHELLSENSE_KEY_HANDLERS = [ordered]@{
            buffer   = $false
            apply    = $false
            commands = $false
        }

    $interactive = Test-ShellsenseInteractiveHost
    $readLineModule = Get-Module -Name PSReadLine -ErrorAction SilentlyContinue
    # Looking up a PSReadLine command can trigger module auto-loading. Avoid
    # that lookup entirely for a noninteractive process unless a caller has
    # deliberately loaded PSReadLine already (for example, a controlled test).
    if ($null -eq $readLineModule -and (-not $interactive -or $DeferImport)) {
        return
    }
    if ($null -eq $readLineModule -and $interactive) {
        try {
            Import-Module PSReadLine -ErrorAction Stop | Out-Null
        } catch {
        }
    }

    # InvokeCommand.GetCommand consults the current session without invoking
    # the Get-Command cmdlet and its startup dispatch work.
    $readLineCommand = $ExecutionContext.InvokeCommand.GetCommand(
        'PSConsoleHostReadLine',
        [System.Management.Automation.CommandTypes]::Function)
    if ($null -eq $readLineCommand) {
        return
    }

    $script:SHELLSENSE_PSREADLINE_AVAILABLE = $true

    # Probe and CI sessions can opt out of touching the user's history file.
    # The default remains PSReadLine's normal history behavior for ordinary
    # interactive shells.
    if ([string]::Equals([string]$env:SHELLSENSE_NO_HISTORY, '1', [StringComparison]::Ordinal)) {
        try {
            Set-PSReadLineOption -HistorySaveStyle SaveNothing -ErrorAction Stop | Out-Null
        } catch {
        }
    }

    if ($null -eq $script:SHELLSENSE_ORIGINAL_READLINE) {
        $script:SHELLSENSE_ORIGINAL_READLINE = $readLineCommand.ScriptBlock
    }
    if (-not $script:SHELLSENSE_READLINE_WRAPPED) {
        function global:PSConsoleHostReadLine {
            $savedLastExitCode = $ExecutionContext.SessionState.PSVariable.GetValue('global:LASTEXITCODE')
            try {
                $acceptedLine = & $script:SHELLSENSE_ORIGINAL_READLINE @args
                Send-ShellsenseEvent -Event 'execute'
                return $acceptedLine
            } finally {
                # PSReadLine's original function may intentionally update the
                # native exit code; only adapter operations are restored.
                $global:LASTEXITCODE = $savedLastExitCode
            }
        }
    $script:SHELLSENSE_READLINE_WRAPPED = $true
    }

    # Query all bound handlers once before registering any of the reserved
    # chords. A user's exact chord and a bare F12 parent both reserve a
    # shellsense chord. Sibling chords under F12 remain available when only a
    # different child is already bound.
    $reservedChords = [ordered]@{
        buffer   = 'F12,s'
        apply    = 'F12,a'
        commands = 'F12,c'
    }
    $existingByName = [ordered]@{
        buffer   = $false
        apply    = $false
        commands = $false
    }
    $allKeyHandlers = $null
    $keyHandlerEnumerationFailed = $false
    try {
        $allKeyHandlers = @([Microsoft.PowerShell.PSConsoleReadLine]::GetKeyHandlers($true, $false))
    } catch {
        $keyHandlerEnumerationFailed = $true
    }
    if ($keyHandlerEnumerationFailed) {
        # Older PSReadLine builds may not expose GetKeyHandlers(bool,bool).
        # Preserve the compatibility path, but do not invoke it for a valid
        # empty result from the static API.
        foreach ($reservedName in $reservedChords.Keys) {
            $existingByName[$reservedName] = @(Get-ShellsenseKeyBinding -Chord $reservedChords[$reservedName]).Count -gt 0
        }
    } else {
        foreach ($handler in $allKeyHandlers) {
            $boundKey = [string]$handler.Key
            if ([string]::IsNullOrEmpty($boundKey)) {
                continue
            }
            foreach ($reservedName in $reservedChords.Keys) {
                if ([string]::Equals($boundKey, [string]$reservedChords[$reservedName], [StringComparison]::OrdinalIgnoreCase) -or
                    [string]::Equals($boundKey, 'F12', [StringComparison]::OrdinalIgnoreCase)) {
                    $existingByName[$reservedName] = $true
                }
            }
        }
    }

        foreach ($reservedName in $reservedChords.Keys) {
            if ([bool]$existingByName[$reservedName]) {
                Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
                    code    = 'key_chord_collision'
                    chord   = $reservedChords[$reservedName]
                    message = ('Reserved chord {0} is already bound; it was left unchanged.' -f $reservedChords[$reservedName])
                })
                continue
            }
            switch ($reservedName) {
                'buffer' {
                    $script:SHELLSENSE_KEY_HANDLERS.buffer = Register-ShellsenseKeyHandler `
                        -Chord $reservedChords[$reservedName] -ScriptBlock ${function:Invoke-ShellsenseBufferKeyHandler} `
                        -Name 'buffer' -SkipCollisionCheck
                }
                'apply' {
                    $script:SHELLSENSE_KEY_HANDLERS.apply = Register-ShellsenseKeyHandler `
                        -Chord $reservedChords[$reservedName] -ScriptBlock ${function:Invoke-ShellsenseApplyKeyHandler} `
                        -Name 'apply' -SkipCollisionCheck
                }
                'commands' {
                    $script:SHELLSENSE_KEY_HANDLERS.commands = Register-ShellsenseKeyHandler `
                        -Chord $reservedChords[$reservedName] -ScriptBlock ${function:Invoke-ShellsenseCommandsKeyHandler} `
                        -Name 'commands' -SkipCollisionCheck
                }
            }
        }
    } finally {
        if ($null -ne $traceTimer) {
            $traceTimer.Stop()
            Send-ShellsenseTrace -Stage 'readline_init' -DurationMs $traceTimer.Elapsed.TotalMilliseconds
        }
    }
}

function Initialize-ShellsensePrompt {
    [CmdletBinding()]
    param()

    # InvokeCommand resolves the already-loaded function without invoking the
    # Get-Command cmdlet and its startup dispatch work.
    $promptCommand = $ExecutionContext.InvokeCommand.GetCommand(
        'Prompt',
        [System.Management.Automation.CommandTypes]::Function)
    if ($null -eq $promptCommand) {
        return
    }
    if ($null -eq $script:SHELLSENSE_ORIGINAL_PROMPT) {
        $script:SHELLSENSE_ORIGINAL_PROMPT = $promptCommand.ScriptBlock
    }
    if ($script:SHELLSENSE_PROMPT_WRAPPED) {
        return
    }

    function global:Prompt {
        # The original invocation must be the first executable operation in
        # the wrapper.  PowerShell carries the caller's $? into that nested
        # scriptblock; assignments or helper calls before it would turn a
        # failed command into a successful status for a user's Prompt.
        $promptState = & {
            param($savedLastExitCode)
            try {
                $originalOutput = @(& $script:SHELLSENSE_ORIGINAL_PROMPT)
                [pscustomobject]@{
                    output       = $originalOutput
                    error        = $null
                    lastExitCode = $savedLastExitCode
                }
            } catch {
                [pscustomobject]@{
                    output       = @()
                    error        = $_
                    lastExitCode = $savedLastExitCode
                }
            }
        } ($ExecutionContext.SessionState.PSVariable.GetValue('global:LASTEXITCODE'))

        # ConsoleHost normally auto-loads PSReadLine immediately before the
        # first prompt. Defer our import/registration until this point when it
        # was absent during bootstrap; this keeps the common launch path from
        # paying the module import cost twice while still handling hosts that
        # do not perform that automatic load.
        if (-not $script:SHELLSENSE_PSREADLINE_AVAILABLE -and (Test-ShellsenseInteractiveHost)) {
            Initialize-ShellsenseReadLine
            if ($script:SHELLSENSE_PSREADLINE_AVAILABLE -and -not $script:SHELLSENSE_CAPABILITIES_REFRESHED) {
                Send-ShellsenseCapabilities
                $script:SHELLSENSE_CAPABILITIES_REFRESHED = $true
            }
        }

        Send-ShellsenseEvent -Event 'prompt_start' -Data ([ordered]@{
            cwd = Get-ShellsenseWorkingDirectory
        })
        Send-ShellsenseEvent -Event 'prompt_end' -Data (Get-ShellsensePromptEndData)

        $global:LASTEXITCODE = $promptState.lastExitCode
        if ($null -ne $promptState.error) {
            throw $promptState.error
        }
        return $promptState.output
    }
    $script:SHELLSENSE_PROMPT_WRAPPED = $true
}

# Capture the first filesystem location before a user changes to a provider
# that cannot be represented as a filesystem cwd.
if ($null -eq $script:SHELLSENSE_FALLBACK_CWD) {
    try {
        $initialLocation = $ExecutionContext.SessionState.Path.CurrentLocation
        if ($null -ne $initialLocation.Provider -and $initialLocation.Provider.Name -eq 'FileSystem') {
            $script:SHELLSENSE_FALLBACK_CWD = [string]$initialLocation.Path
        }
    } catch {
    }
}

# Dot-sourcing without a token is useful for syntax and helper tests and must
# remain a no-op for the user's normal shell.  The token-bearing invocation is
# initialized only once, so sourcing this file again cannot double-wrap hooks.
if ([string]::IsNullOrEmpty([string]$script:SHELLSENSE_TOKEN) -or $script:SHELLSENSE_INITIALIZED) {
    return
}
$script:SHELLSENSE_INITIALIZED = $true

# The ConPTY stream and Rust terminal model use UTF-8. PSReadLine.Replace
# redraws through Console.Out; a legacy code page would display emoji as ??.
# These settings apply only to the owned child process, never to profile files.
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
[Console]::InputEncoding = [Text.UTF8Encoding]::new($false)

$shellsenseBootstrapTraceTimer = $null
if ($script:SHELLSENSE_TRACE_ENABLED) {
    $shellsenseBootstrapTraceTimer = [Diagnostics.Stopwatch]::StartNew()
}
try {
    Initialize-ShellsensePrompt
    Initialize-ShellsenseReadLine -DeferImport
    Send-ShellsenseCapabilities
} finally {
    if ($null -ne $shellsenseBootstrapTraceTimer) {
        $shellsenseBootstrapTraceTimer.Stop()
        Send-ShellsenseTrace -Stage 'adapter_bootstrap' -DurationMs $shellsenseBootstrapTraceTimer.Elapsed.TotalMilliseconds
    }
}
