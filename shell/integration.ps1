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
    SHELLSENSE_PIPE_NAME           = $null
    SHELLSENSE_PIPE_STREAM         = $null
    SHELLSENSE_PIPE_BUFFER         = $null
    SHELLSENSE_PIPE_SEQUENCE       = [int64]0
    SHELLSENSE_PIPE_ENABLED        = $false
    SHELLSENSE_EDIT_PATH           = $null
    SHELLSENSE_REQUEST_PATH        = $null
    SHELLSENSE_KEY_PREFIX          = 'F12'
    SHELLSENSE_PUBLIC_KEYS          = $null
    SHELLSENSE_PUBLIC_KEY_STATUS    = $null
    SHELLSENSE_PUBLIC_KEY_CONFIG_ERROR = $false
    SHELLSENSE_FALLBACK_CWD        = $null
    SHELLSENSE_ORIGINAL_PROMPT     = $null
    SHELLSENSE_PROMPT_WRAPPED      = $false
    SHELLSENSE_ORIGINAL_READLINE   = $null
    SHELLSENSE_READLINE_WRAPPED    = $false
    SHELLSENSE_PSREADLINE_AVAILABLE = $false
    SHELLSENSE_KEY_HANDLERS        = [ordered]@{ buffer = $false; apply = $false; commands = $false; native = $false; enter = $false; shift_enter = $false }
    SHELLSENSE_CAPABILITIES_REFRESHED = $false
    SHELLSENSE_COMMAND_SNAPSHOT_ID = $null
    SHELLSENSE_COMMAND_SNAPSHOT = $null
    SHELLSENSE_COMMAND_SNAPSHOT_OFFSET = 0
    SHELLSENSE_COMMAND_ENUMERATOR  = $null
    SHELLSENSE_COMMAND_SNAPSHOT_FAILED = $false
    SHELLSENSE_COMMAND_SNAPSHOT_COMPLETE = $false
    SHELLSENSE_COMMAND_REQUEST_ID  = $null
    SHELLSENSE_ENVIRONMENT_SNAPSHOT = $null
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

$shellsensePipeNameFromEnvironment = [Environment]::GetEnvironmentVariable('SHELLSENSE_PIPE_NAME', 'Process')
if ($null -ne $shellsensePipeNameFromEnvironment) {
    $script:SHELLSENSE_PIPE_NAME = $shellsensePipeNameFromEnvironment
}

$shellsenseEditPathFromEnvironment = [Environment]::GetEnvironmentVariable('SHELLSENSE_EDIT_PATH', 'Process')
if ($null -ne $shellsenseEditPathFromEnvironment) {
    $script:SHELLSENSE_EDIT_PATH = $shellsenseEditPathFromEnvironment
}

$shellsenseRequestPathFromEnvironment = [Environment]::GetEnvironmentVariable('SHELLSENSE_REQUEST_PATH', 'Process')
if ($null -ne $shellsenseRequestPathFromEnvironment) {
    $script:SHELLSENSE_REQUEST_PATH = $shellsenseRequestPathFromEnvironment
}

# The host chooses a protocol prefix per session. Keep the accepted set small
# so a malformed configuration can never install an arbitrary PSReadLine
# chord. The default remains the alpha F12 prefix for old launchers.
$shellsenseKeyPrefixFromEnvironment = [Environment]::GetEnvironmentVariable('SHELLSENSE_KEY_PREFIX', 'Process')
$shellsenseKeyPrefix = if ([string]::IsNullOrEmpty([string]$shellsenseKeyPrefixFromEnvironment)) {
    [string]$script:SHELLSENSE_KEY_PREFIX
} else {
    [string]$shellsenseKeyPrefixFromEnvironment
}
if ($shellsenseKeyPrefix -notmatch '^(?i:F(?:[5-9]|1[0-2]))$') {
    $shellsenseKeyPrefix = 'F12'
}
$script:SHELLSENSE_KEY_PREFIX = $shellsenseKeyPrefix.ToUpperInvariant()

if ([string]::IsNullOrEmpty([string]$script:SHELLSENSE_REQUEST_PATH) -and
    -not [string]::IsNullOrEmpty([string]$script:SHELLSENSE_EDIT_PATH)) {
    try {
        $requestDirectory = [IO.Path]::GetDirectoryName([IO.Path]::GetFullPath([string]$script:SHELLSENSE_EDIT_PATH))
        $script:SHELLSENSE_REQUEST_PATH = [IO.Path]::Combine($requestDirectory, 'request.json')
    } catch {
        # A malformed edit path is reported when the request or edit operation
        # is used. Bootstrap remains harmless for a user's shell.
    }
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
    if ($Stage -notin @(
            'adapter_bootstrap',
            'readline_init',
            'key_snapshot',
            'key_register',
            'public_keys',
            'readline_wrap',
            'command_snapshot',
            'serialize')) {
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

function Disable-ShellsensePipe {
    [CmdletBinding()]
    param()

    $stream = $script:SHELLSENSE_PIPE_STREAM
    $script:SHELLSENSE_PIPE_STREAM = $null
    $script:SHELLSENSE_PIPE_BUFFER = $null
    $script:SHELLSENSE_PIPE_ENABLED = $false
    if ($null -ne $stream) {
        try {
            $stream.Dispose()
        } catch {
        }
    }
}

function Initialize-ShellsensePipe {
    [CmdletBinding()]
    param()

    $name = [string]$script:SHELLSENSE_PIPE_NAME
    if ([string]::IsNullOrWhiteSpace($name)) {
        return $false
    }
    try {
        # Connect(0) is deliberately nonblocking. The Rust side owns the
        # single listener and falls back to OSC if this session cannot attach.
        $stream = [System.IO.Pipes.NamedPipeClientStream]::new(
            '.',
            $name,
            [System.IO.Pipes.PipeDirection]::InOut,
            [System.IO.Pipes.PipeOptions]::Asynchronous)
        $stream.Connect(0)
        $stream.ReadMode = [System.IO.Pipes.PipeTransmissionMode]::Message
        $script:SHELLSENSE_PIPE_STREAM = $stream
        # Reuse one bounded receive buffer for all host requests. The stream
        # is asynchronous because NamedPipeClientStream's synchronous timeout
        # setters are unsupported on this implementation.
        $script:SHELLSENSE_PIPE_BUFFER = [byte[]]::new(1048576)
        $script:SHELLSENSE_PIPE_ENABLED = $true
        return $true
    } catch {
        Disable-ShellsensePipe
        return $false
    } finally {
        # The name is retained only in script state; user commands do not need
        # to inherit this session-private transport locator.
        try {
            [Environment]::SetEnvironmentVariable('SHELLSENSE_PIPE_NAME', $null, 'Process')
        } catch {
        }
    }
}

function Send-ShellsenseOscPayload {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [AllowNull()]
        [object]$Payload
    )

    if ([string]::IsNullOrEmpty([string]$script:SHELLSENSE_TOKEN)) {
        return $false
    }
    try {
        $frame = ConvertTo-ShellsenseFrame -Token ([string]$script:SHELLSENSE_TOKEN) -Payload $Payload
        # [Console]::Out avoids adding the frame to a PowerShell pipeline and
        # therefore avoids contaminating prompt text or command output.
        [Console]::Out.Write($frame)
        [Console]::Out.Flush()
        return $true
    } catch {
        return $false
    }
}

function Send-ShellsensePipeEvent {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [System.Collections.IDictionary]$Payload
    )

    if (-not $script:SHELLSENSE_PIPE_ENABLED -or $null -eq $script:SHELLSENSE_PIPE_STREAM) {
        return $false
    }
    try {
        $sequence = [int64]$script:SHELLSENSE_PIPE_SEQUENCE + 1
        $script:SHELLSENSE_PIPE_SEQUENCE = $sequence
        $envelope = [ordered]@{
            sequence = $sequence
            payload  = $Payload
        }
        $json = ConvertTo-ShellsenseJson -Payload $envelope
        $bytes = [Text.Encoding]::UTF8.GetBytes([string]::Concat($json, "`n"))
        if ($bytes.Length -gt 1048576) {
            return $false
        }
        $stream = $script:SHELLSENSE_PIPE_STREAM
        if (-not $stream.IsConnected) {
            Disable-ShellsensePipe
            return $false
        }

        # One WriteAsync call is intentional: message-mode named pipes
        # preserve that call as one message. Cancellation bounds a full or
        # disconnected endpoint without blocking PSReadLine's UI thread.
        $cts = [Threading.CancellationTokenSource]::new(50)
        try {
            $writeTask = $stream.WriteAsync($bytes, 0, $bytes.Length, $cts.Token)
            if (-not $writeTask.Wait(50)) {
                $cts.Cancel()
                try {
                    $writeTask.Wait(50) | Out-Null
                } catch {
                }
                Disable-ShellsensePipe
                return $false
            }
            $writeTask.GetAwaiter().GetResult()
        } finally {
            $cts.Dispose()
        }

        # The OSC barrier is emitted only after the complete pipe write. The
        # host consumes the matching sequence and then reads the payload, so
        # terminal cursor/output ordering remains identical to OSC mode.
        if (-not (Send-ShellsenseOscPayload -Payload ([ordered]@{
                    event    = 'pipe'
                    sequence = $sequence
                }))) {
            Disable-ShellsensePipe
            return $false
        }
        return $true
    } catch {
        Disable-ShellsensePipe
        return $false
    }
}

function Read-ShellsensePipeJson {
    [CmdletBinding()]
    param()

    if (-not $script:SHELLSENSE_PIPE_ENABLED -or $null -eq $script:SHELLSENSE_PIPE_STREAM) {
        return $null
    }
    $stream = $script:SHELLSENSE_PIPE_STREAM
    try {
        if (-not $stream.IsConnected) {
            Disable-ShellsensePipe
            return $null
        }
        $bytes = $script:SHELLSENSE_PIPE_BUFFER
        if ($null -eq $bytes) {
            $bytes = [byte[]]::new(1048576)
            $script:SHELLSENSE_PIPE_BUFFER = $bytes
        }
        $offset = 0
        do {
            $remaining = $bytes.Length - $offset
            if ($remaining -le 0) {
                # A host frame is one message and is capped at the buffer
                # size; accepting more would desynchronise the next key.
                Disable-ShellsensePipe
                return $null
            }
            $cts = [Threading.CancellationTokenSource]::new(50)
            try {
                $readTask = $stream.ReadAsync($bytes, $offset, $remaining, $cts.Token)
                if (-not $readTask.Wait(50)) {
                    $cts.Cancel()
                    try {
                        $readTask.Wait(50) | Out-Null
                    } catch {
                    }
                    Disable-ShellsensePipe
                    return $null
                }
                $read = $readTask.GetAwaiter().GetResult()
            } finally {
                $cts.Dispose()
            }
            if ($read -le 0) {
                Disable-ShellsensePipe
                return $null
            }
            $offset += $read
        } while (-not $stream.IsMessageComplete)

        $utf8Strict = [Text.UTF8Encoding]::new($false, $true)
        $json = $utf8Strict.GetString($bytes, 0, $offset).TrimEnd([char]13, [char]10)
        return [System.Text.Json.JsonSerializer]::Deserialize(
            $json,
            [object],
            (Get-ShellsenseJsonOptions))
    } catch {
        Disable-ShellsensePipe
        return $null
    }
}

function Read-ShellsensePipeEnvelope {
    [CmdletBinding()]
    param()

    $rootValue = Read-ShellsensePipeJson
    if ($null -eq $rootValue -or $rootValue.ValueKind -ne [System.Text.Json.JsonValueKind]::Object) {
        return $null
    }
    $kindElement = [System.Text.Json.JsonElement]::new()
    $payloadElement = [System.Text.Json.JsonElement]::new()
    if (-not $rootValue.TryGetProperty('kind', [ref]$kindElement) -or
        -not $rootValue.TryGetProperty('payload', [ref]$payloadElement) -or
        $kindElement.ValueKind -ne [System.Text.Json.JsonValueKind]::String -or
        $payloadElement.ValueKind -ne [System.Text.Json.JsonValueKind]::Object) {
        Disable-ShellsensePipe
        return $null
    }
    return [pscustomobject]@{
        kind    = $kindElement.GetString()
        payload = $payloadElement.GetRawText()
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

    # A successful pipe write is followed by a short OSC sequence barrier.
    # Oversized events, unavailable pipes, and failed writes are sent as the
    # original OSC event so the default transport and failure path remain
    # compatible with older hosts.
    if (Send-ShellsensePipeEvent -Payload $payload) {
        return
    }
    [void](Send-ShellsenseOscPayload -Payload $payload)
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

function Get-ShellsenseEnvironmentSnapshot {
    [CmdletBinding()]
    param()

    $snapshot = [ordered]@{}
    try {
        $processEnvironment = [Environment]::GetEnvironmentVariables('Process')
        $names = [System.Collections.Generic.List[string]]::new()
        foreach ($name in $processEnvironment.Keys) {
            if ($null -ne $name) {
                [void]$names.Add([string]$name)
            }
        }
        $names.Sort([StringComparer]::OrdinalIgnoreCase)
        foreach ($name in $names) {
            $snapshot[$name] = [string]$processEnvironment[$name]
        }
    } catch {
        # A restricted host may deny environment enumeration. PATH and
        # PATHEXT are still sent by Get-ShellsensePromptEndData.
    }
    return $snapshot
}

function Test-ShellsenseEnvironmentChanged {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [System.Collections.IDictionary]$Snapshot
    )

    $previous = $script:SHELLSENSE_ENVIRONMENT_SNAPSHOT
    if ($null -eq $previous -or $previous.Count -ne $Snapshot.Count) {
        return $true
    }
    foreach ($name in $Snapshot.Keys) {
        if (-not $previous.Contains($name) -or
            -not [string]::Equals([string]$previous[$name], [string]$Snapshot[$name], [StringComparison]::Ordinal)) {
            return $true
        }
    }
    return $false
}

function Get-ShellsensePromptEndData {
    [CmdletBinding()]
    param()

    $pathValue = [string]$env:PATH
    $pathExtValue = [string]$env:PATHEXT
    $environmentSnapshot = Get-ShellsenseEnvironmentSnapshot
    $environmentChanged = Test-ShellsenseEnvironmentChanged -Snapshot $environmentSnapshot
    $data = [ordered]@{
        cwd    = Get-ShellsenseWorkingDirectory
        path   = $pathValue
        pathext = $pathExtValue
        pid    = [int]$PID
    }
    if ($environmentChanged) {
        # This is a transient protocol snapshot. It is never included in
        # trace events, persisted caches, diagnostics, or command history.
        $data.environment = $environmentSnapshot
        $script:SHELLSENSE_ENVIRONMENT_SNAPSHOT = $environmentSnapshot
    }
    return $data
}

function Test-ShellsenseComplexLine {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$Line
    )

    # Rust's fast lexer is deliberately used for ordinary words.  Anything
    # whose meaning can depend on PowerShell parsing gets the compact context
    # below instead.  This test is only a dispatch hint; the AST validator is
    # still authoritative for every complex line.
    return [regex]::IsMatch($Line, '[\r\n''"`$(){}\[\]|;&<>#]')
}

function ConvertTo-ShellsenseDecodedPrefix {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$Raw
    )

    if ([string]::IsNullOrEmpty($Raw)) {
        return ''
    }

    $value = $Raw
    $quote = [char]0
    if ($value.Length -gt 0 -and ($value[0] -eq [char]39 -or $value[0] -eq [char]34)) {
        $quote = $value[0]
        $value = $value.Substring(1)
        if ($value.Length -gt 0 -and $value[$value.Length - 1] -eq $quote) {
            $value = $value.Substring(0, $value.Length - 1)
        }
        if ($quote -eq [char]39) {
            # A doubled single quote is PowerShell's literal quote escape.
            $value = $value.Replace("''", "'")
        }
    }

    # Decode only lexical backtick escapes.  In particular, do not expand
    # variables or invoke a subexpression while preparing a completion prefix.
    $builder = [Text.StringBuilder]::new($value.Length)
    for ($index = 0; $index -lt $value.Length; $index++) {
        $character = $value[$index]
        if ($character -ne [char]96 -or $index + 1 -ge $value.Length) {
            [void]$builder.Append($character)
            continue
        }
        $index++
        $escaped = $value[$index]
        switch ([int]$escaped) {
            48 { [void]$builder.Append([char]0) } # `0
            97 { [void]$builder.Append([char]7) } # `a
            98 { [void]$builder.Append([char]8) } # `b
            101 { [void]$builder.Append([char]27) } # `e
            102 { [void]$builder.Append([char]12) } # `f
            110 { [void]$builder.Append([char]10) } # `n
            114 { [void]$builder.Append([char]13) } # `r
            116 { [void]$builder.Append([char]9) } # `t
            118 { [void]$builder.Append([char]11) } # `v
            default { [void]$builder.Append($escaped) }
        }
    }
    return $builder.ToString()
}

function Convert-ShellsenseSafeAstValue {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [AllowNull()]
        [System.Management.Automation.Language.Ast]$Ast,

        [Parameter(Mandatory = $true)]
        [ref]$Value
    )

    $Value.Value = $null
    if ($null -eq $Ast) {
        return $false
    }

    if ($Ast -is [System.Management.Automation.Language.StringConstantExpressionAst]) {
        $Value.Value = [string]$Ast.Value
        return $true
    }
    if ($Ast -is [System.Management.Automation.Language.ExpandableStringExpressionAst]) {
        if ($null -ne $Ast.NestedExpressions -and $Ast.NestedExpressions.Count -gt 0) {
            return $false
        }
        $Value.Value = [string]$Ast.Value
        return $true
    }
    if ($Ast -is [System.Management.Automation.Language.CommandExpressionAst]) {
        return Convert-ShellsenseSafeAstValue -Ast $Ast.Expression -Value ([ref]$Value.Value)
    }
    if ($Ast -is [System.Management.Automation.Language.CommandParameterAst]) {
        if ($null -ne $Ast.Argument) {
            $argumentValue = $null
            if (-not (Convert-ShellsenseSafeAstValue -Ast $Ast.Argument -Value ([ref]$argumentValue))) {
                return $false
            }
        }
        # Keep the spelling (including --name=value and short-option forms),
        # while the safety decision above rejects variables and expressions.
        $Value.Value = [string]$Ast.Extent.Text
        return $true
    }
    return $false
}

function Convert-ShellsenseAstElementValue {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [System.Management.Automation.Language.Ast]$Ast,

        [Parameter(Mandatory = $true)]
        [ref]$Value
    )

    return Convert-ShellsenseSafeAstValue -Ast $Ast -Value $Value
}

function Get-ShellsenseAstBufferContext {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$Line,

        [Parameter(Mandatory = $true)]
        [int]$Cursor,

        [AllowNull()]
        [System.Management.Automation.Language.Ast]$Ast,

        [AllowNull()]
        [System.Management.Automation.Language.Token[]]$Tokens,

        [AllowNull()]
        [System.Management.Automation.Language.ParseError[]]$ParseErrors
    )

    if (-not (Test-ShellsenseComplexLine -Line $Line)) {
        return $null
    }

    $context = [ordered]@{
        command          = $null
        arguments        = @()
        prefix           = ''
        replace_start    = [int]$Cursor
        replace_end      = [int]$Cursor
        suppressed       = $false
        complex          = $true
        command_position = $false
    }
    if ($Cursor -lt 0 -or $Cursor -gt $Line.Length -or
        -not (Test-ShellsenseUtf16Range -Line $Line -Start $Cursor -Length 0)) {
        $context.suppressed = $true
        return $context
    }

    # The helper is also directly testable outside an interactive PSReadLine
    # instance. Production calls pass the syntax tree returned by the
    # four-argument PSReadLine GetBufferState overload.
    if ($null -eq $Ast) {
        try {
            $parsedTokens = $null
            $parsedErrors = $null
            $Ast = [System.Management.Automation.Language.Parser]::ParseInput(
                $Line,
                [ref]$parsedTokens,
                [ref]$parsedErrors)
            $Tokens = $parsedTokens
            $ParseErrors = $parsedErrors
        } catch {
            $context.suppressed = $true
            return $context
        }
    }
    if ($null -eq $Tokens) {
        $Tokens = @()
    }
    if ($null -eq $ParseErrors) {
        $ParseErrors = @()
    }

    # A comment or here-string body is opaque user text. Never infer a command
    # from an AST node outside the text under the cursor.
    foreach ($token in $Tokens) {
        if ($null -eq $token -or $null -eq $token.Extent) {
            continue
        }
        $tokenKind = [string]$token.Kind
        if ($tokenKind -notmatch '(?i)Comment|HereString') {
            continue
        }
        if ($Cursor -ge $token.Extent.StartOffset -and $Cursor -le $token.Extent.EndOffset) {
            $context.suppressed = $true
            return $context
        }
    }

    $commandAsts = [System.Collections.Generic.List[System.Management.Automation.Language.CommandAst]]::new()
    try {
        foreach ($node in $Ast.FindAll({ param($item) $item -is [System.Management.Automation.Language.CommandAst] }, $true)) {
            [void]$commandAsts.Add($node)
        }
    } catch {
        $context.suppressed = $true
        return $context
    }

    $currentCommand = $null
    $currentSpan = [int64]::MaxValue
    foreach ($commandAst in $commandAsts) {
        $start = [int]$commandAst.Extent.StartOffset
        $end = [int]$commandAst.Extent.EndOffset
        if ($Cursor -ge $start -and $Cursor -le $end) {
            $span = [int64]$end - $start
            if ($span -lt $currentSpan) {
                $currentCommand = $commandAst
                $currentSpan = $span
            }
        }
    }
    if ($null -eq $currentCommand -or $currentCommand.CommandElements.Count -eq 0) {
        # A standalone environment variable is represented by a
        # VariableExpressionAst rather than a CommandAst (for example,
        # `$env:Pa` at the prompt). Keep this namespace available to the
        # environment-name data source without evaluating the variable.
        $lineBeforeCursor = $Line.Substring(0, $Cursor)
        $environmentMatch = [regex]::Match(
            $lineBeforeCursor,
            '(?i)(?<![\p{L}\p{Nd}_])\$env:(?:[A-Za-z_][A-Za-z0-9_]*)?$')
        if ($environmentMatch.Success) {
            $context.prefix = ConvertTo-ShellsenseDecodedPrefix -Raw $environmentMatch.Value
            $context.replace_start = [int]$environmentMatch.Index
            $context.replace_end = [int]$Cursor
            # An environment variable is a value expression, not an
            # executable token. Keep command_position false so the host's
            # path/environment data source handles the `$env:` namespace.
            $context.command_position = $false
            return $context
        }
        $context.suppressed = $true
        return $context
    }

    $elements = @($currentCommand.CommandElements)
    $currentIndex = -1
    $currentStart = [int]$Cursor
    $currentEnd = [int]$Cursor
    for ($index = 0; $index -lt $elements.Count; $index++) {
        $elementStart = [int]$elements[$index].Extent.StartOffset
        $elementEnd = [int]$elements[$index].Extent.EndOffset
        $inside = $Cursor -ge $elementStart -and $Cursor -le $elementEnd
        # At the right edge of a token, whitespace starts a fresh empty token.
        if ($inside -and $Cursor -eq $elementEnd -and $Cursor -lt $Line.Length -and
            [char]::IsWhiteSpace($Line[$Cursor])) {
            $inside = $false
        }
        if ($inside) {
            $currentIndex = $index
            $currentStart = $elementStart
            $currentEnd = $elementEnd
            break
        }
        if ($Cursor -lt $elementStart) {
            $currentIndex = $index
            break
        }
    }
    if ($currentIndex -lt 0) {
        $currentIndex = $elements.Count
    }
    if ($currentStart -gt $Cursor) {
        $currentStart = $Cursor
    }
    if ($currentEnd -lt $Cursor) {
        $currentEnd = $Cursor
    }
    if (-not (Test-ShellsenseUtf16Range -Line $Line -Start $currentStart -Length ($currentEnd - $currentStart))) {
        $context.suppressed = $true
        return $context
    }

    $context.prefix = ConvertTo-ShellsenseDecodedPrefix -Raw $Line.Substring($currentStart, $Cursor - $currentStart)
    $context.replace_start = [int]$currentStart
    # Carry the full token extent. The host calculates the accepted suffix
    # from cursor..replace_end, so a completion in the middle of a token does
    # not duplicate or discard the text to the right of the cursor.
    $context.replace_end = [int]$currentEnd
    $context.command_position = ($currentIndex -eq 0)

    # `$env:NAME` is a known PowerShell namespace used by the environment
    # data source. It is safe to complete its name, while other variable
    # expressions remain suppressed because their values are not known.
    if ($context.command_position -and $elements[0] -is [System.Management.Automation.Language.VariableExpressionAst] -and
        [string]::Equals([string]$elements[0].VariablePath.DriveName, 'env', [StringComparison]::OrdinalIgnoreCase)) {
        $context.command_position = $false
        return $context
    }

    $commandValue = $null
    if (-not (Convert-ShellsenseAstElementValue -Ast $elements[0] -Value ([ref]$commandValue))) {
        $context.suppressed = $true
        $context.command_position = $false
        return $context
    }
    if (-not $context.command_position) {
        $context.command = [string]$commandValue
    }

    $arguments = [System.Collections.Generic.List[string]]::new()
    for ($index = 1; $index -lt $currentIndex; $index++) {
        $argumentValue = $null
        if (-not (Convert-ShellsenseAstElementValue -Ast $elements[$index] -Value ([ref]$argumentValue))) {
            $context.suppressed = $true
            $context.arguments = @($arguments.ToArray())
            return $context
        }
        [void]$arguments.Add([string]$argumentValue)
    }
    $context.arguments = @($arguments.ToArray())

    # A dynamic expression in the current token is as unsafe as one in a
    # preceding argument. Empty whitespace tokens are safe by construction.
    if ($currentIndex -lt $elements.Count) {
        $currentRaw = $Line.Substring($currentStart, $Cursor - $currentStart)
        if ($currentRaw.StartsWith([string][char]34, [StringComparison]::Ordinal) -and
            [string]$context.prefix -match '(?i)^\$env:(?:[A-Za-z_][A-Za-z0-9_]*)?$') {
            # A double-quoted `$env:NAME` is still a safe namespace lookup;
            # preserve the quote in the raw replacement range so path
            # insertion can close it correctly. Single-quoted text stays
            # literal and is deliberately not treated as a variable.
            $context.command_position = $false
            return $context
        }
        if ($elements[$currentIndex] -is [System.Management.Automation.Language.VariableExpressionAst] -and
            [string]::Equals([string]$elements[$currentIndex].VariablePath.DriveName, 'env', [StringComparison]::OrdinalIgnoreCase)) {
            # `$env:NAME` is a safe namespace token even when it is an
            # argument of a known command. It is a value expression, so the
            # host should use environment candidates rather than command
            # candidates; the full prefix retains the namespace for lookup.
            $context.command_position = $false
            return $context
        }
        $currentValue = $null
        if (-not (Convert-ShellsenseAstElementValue -Ast $elements[$currentIndex] -Value ([ref]$currentValue))) {
            $context.suppressed = $true
        }
    }

    # Parse errors are allowed for a still-open quoted token (the AST remains
    # a safe string), but an error elsewhere means we cannot prove the context.
    if ($ParseErrors.Count -gt 0 -and $context.suppressed) {
        $context.suppressed = $true
    }
    return $context
}

function Send-ShellsenseBuffer {
    [CmdletBinding()]
    param()

    try {
        $line = $null
        $cursor = 0
        [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line, [ref]$cursor)
        $data = [ordered]@{
            line   = [string]$line
            cursor = [int]$cursor
        }
        if (Test-ShellsenseComplexLine -Line ([string]$line)) {
            $ast = $null
            $tokens = $null
            $parseErrors = $null
            try {
                # PSReadLine keeps this syntax tree in the live editing
                # runspace. It avoids reparsing a different line while an IME
                # or a paste operation is still updating the buffer.
                $astCursor = 0
                [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState(
                    [ref]$ast,
                    [ref]$tokens,
                    [ref]$parseErrors,
                    [ref]$astCursor)
            } catch {
                # The parser fallback is only for old PSReadLine builds that
                # lack the AST overload. It still applies the same safe-value
                # checks and never evaluates user expressions.
            }
            $context = Get-ShellsenseAstBufferContext `
                -Line ([string]$line) -Cursor ([int]$cursor) `
                -Ast $ast -Tokens $tokens -ParseErrors $parseErrors
            if ($null -ne $context) {
                $data.context = $context
            }
        }
        Send-ShellsenseEvent -Event 'buffer' -Data $data
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

    $idProperty = $Payload.PSObject.Properties['id']
    if ($null -ne $idProperty) {
        if ($idProperty.Value -isnot [string] -or [string]::IsNullOrEmpty([string]$idProperty.Value)) {
            return $false
        }
    }
    $Edit.Value = [ordered]@{
        start  = [int]$start
        length = [int]$length
        text   = [string]$textProperty.Value
    }
    if ($null -ne $idProperty) {
        $Edit.Value.id = [string]$idProperty.Value
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
    foreach ($propertyName in @('id', 'expectedLine', 'expectedCursor', 'start', 'length', 'text')) {
        $property = [System.Text.Json.JsonElement]::new()
        if ($rootValue.TryGetProperty($propertyName, [ref]$property)) {
            $payload[$propertyName] = Convert-ShellsenseJsonElementValue -Element $property
        }
    }
    return [pscustomobject]$payload
}

function Get-ShellsenseRequestPath {
    [CmdletBinding()]
    param()

    if (-not [string]::IsNullOrEmpty([string]$script:SHELLSENSE_REQUEST_PATH)) {
        return [string]$script:SHELLSENSE_REQUEST_PATH
    }
    if ([string]::IsNullOrEmpty([string]$script:SHELLSENSE_EDIT_PATH)) {
        return $null
    }
    try {
        $directory = [IO.Path]::GetDirectoryName([IO.Path]::GetFullPath([string]$script:SHELLSENSE_EDIT_PATH))
        return [IO.Path]::Combine($directory, 'request.json')
    } catch {
        return $null
    }
}

function ConvertFrom-ShellsenseRequestJson {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$Json
    )

    $rootValue = [System.Text.Json.JsonSerializer]::Deserialize(
        $Json,
        [object],
        (Get-ShellsenseJsonOptions))
    if ($null -eq $rootValue -or $rootValue.ValueKind -ne [System.Text.Json.JsonValueKind]::Object) {
        return $null
    }

    $idElement = [System.Text.Json.JsonElement]::new()
    $kindElement = [System.Text.Json.JsonElement]::new()
    if (-not $rootValue.TryGetProperty('id', [ref]$idElement) -or
        -not $rootValue.TryGetProperty('kind', [ref]$kindElement) -or
        ($idElement.ValueKind -ne [System.Text.Json.JsonValueKind]::String -and
            $idElement.ValueKind -ne [System.Text.Json.JsonValueKind]::Number) -or
        $kindElement.ValueKind -ne [System.Text.Json.JsonValueKind]::String) {
        return $null
    }
    $id = $null
    if ($idElement.ValueKind -eq [System.Text.Json.JsonValueKind]::String) {
        $id = $idElement.GetString()
    } else {
        $numericId = [int64]0
        if ($idElement.TryGetInt64([ref]$numericId)) {
            $id = $numericId.ToString([Globalization.CultureInfo]::InvariantCulture)
        }
    }
    $kind = $kindElement.GetString()
    if ([string]::IsNullOrEmpty($id) -or [string]::IsNullOrEmpty($kind)) {
        return $null
    }

    # A commands_reset may carry the host's current public shortcut map. Keep
    # it as JSON text until the reset handler has consumed the request so the
    # normal native/edit request path remains allocation-light. Accept both a
    # compact string (the host's preferred form) and an object for callers that
    # construct protocol messages directly.
    $publicKeysJson = $null
    $publicKeysElement = [System.Text.Json.JsonElement]::new()
    if ($rootValue.TryGetProperty('public_keys_json', [ref]$publicKeysElement)) {
        if ($publicKeysElement.ValueKind -eq [System.Text.Json.JsonValueKind]::String) {
            $publicKeysJson = $publicKeysElement.GetString()
        } else {
            return $null
        }
    }
    $publicKeysElement = [System.Text.Json.JsonElement]::new()
    if ($rootValue.TryGetProperty('public_keys', [ref]$publicKeysElement)) {
        if ($publicKeysElement.ValueKind -eq [System.Text.Json.JsonValueKind]::Object) {
            $publicKeysJson = $publicKeysElement.GetRawText()
        } elseif ($publicKeysElement.ValueKind -eq [System.Text.Json.JsonValueKind]::String) {
            $publicKeysJson = $publicKeysElement.GetString()
        } else {
            return $null
        }
    }
    return [pscustomobject]@{
        id               = [string]$id
        kind             = [string]$kind
        public_keys_json = $publicKeysJson
    }
}

function Read-ShellsenseRequest {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [ValidateSet('native', 'commands_reset', 'commands_next')]
        [string[]]$ExpectedKind
    )

    # In pipe mode the host publishes the payload before sending the known
    # internal chord. This is the only place a request is read; no background
    # reader or speculative timeout is introduced into PSReadLine.
    $pipeEnvelope = Read-ShellsensePipeEnvelope
    if ($null -ne $pipeEnvelope) {
        if (-not [string]::Equals([string]$pipeEnvelope.kind, 'request', [StringComparison]::Ordinal)) {
            Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
                code    = 'request_kind_mismatch'
                message = 'The named-pipe request envelope is not a request.'
            })
            return $null
        }
        $request = ConvertFrom-ShellsenseRequestJson -Json ([string]$pipeEnvelope.payload)
        if ($null -eq $request) {
            Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
                code    = 'request_rejected'
                message = 'The named-pipe request is not a valid v2 request.'
            })
            return $null
        }
        if ($ExpectedKind -notcontains [string]$request.kind) {
            Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
                code        = 'request_kind_mismatch'
                request_id  = [string]$request.id
                message     = ('Expected a {0} request.' -f ($ExpectedKind -join ' or '))
            })
            return $null
        }
        return $request
    }
    # A connected pipe is authoritative for this session. If its frame was
    # not ready on the known chord, do not consume a stale file or guess at a
    # request; the host will retry/fallback after the transport error.
    if ($script:SHELLSENSE_PIPE_ENABLED) {
        return $null
    }

    $path = Get-ShellsenseRequestPath
    if ([string]::IsNullOrEmpty([string]$path) -or -not [IO.File]::Exists($path)) {
        return $null
    }

    $consumedPath = $null
    try {
        # Rename first so a writer can publish the next request while this
        # request is being processed. A consumed name also prevents replay.
        $consumedPath = Get-ShellsenseConsumedEditPath -Path $path
        [IO.File]::Move($path, $consumedPath)
        $utf8Strict = [Text.UTF8Encoding]::new($false, $true)
        $json = [IO.File]::ReadAllText($consumedPath, $utf8Strict)
        $request = ConvertFrom-ShellsenseRequestJson -Json $json
        if ($null -eq $request) {
            Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
                code    = 'request_rejected'
                message = 'The ShellSense request is not a valid v2 request.'
            })
            return $null
        }
        if ($ExpectedKind -notcontains [string]$request.kind) {
            Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
                code        = 'request_kind_mismatch'
                request_id  = [string]$request.id
                message     = ('Expected a {0} request.' -f ($ExpectedKind -join ' or '))
            })
            return $null
        }
        return $request
    } catch {
        Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
            code    = 'request_failed'
            message = 'The ShellSense request could not be read.'
        })
        return $null
    } finally {
        if ($null -ne $consumedPath) {
            try {
                [IO.File]::Delete($consumedPath)
            } catch {
            }
        }
    }
}

function Send-ShellsenseEditResult {
    [CmdletBinding()]
    param(
        [AllowNull()]
        [string]$RequestId,

        [Parameter(Mandatory = $true)]
        [bool]$Applied
    )

    Send-ShellsenseEvent -Event 'edit_result' -Data ([ordered]@{
        request_id = $RequestId
        applied    = [bool]$Applied
    })
}

function Invoke-ShellsenseApplyEdit {
    [CmdletBinding()]
    param()

    $path = [string]$script:SHELLSENSE_EDIT_PATH
    $consumedPath = $null
    $payload = $null
    $replaceApplied = $false
    $replaceError = $null
    try {
        # Pipe mode publishes the edit payload before the known apply chord.
        # Read it first so no stale file can be selected while the pipe is
        # connected. A failed/disposed pipe falls through to the legacy file.
        $pipeEnvelope = Read-ShellsensePipeEnvelope
        if ($null -ne $pipeEnvelope) {
            if (-not [string]::Equals([string]$pipeEnvelope.kind, 'edit', [StringComparison]::Ordinal)) {
                Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
                    code    = 'edit_kind_mismatch'
                    message = 'The named-pipe envelope is not an edit payload.'
                })
                return
            }
            $payload = ConvertFrom-ShellsenseEditJson -Json ([string]$pipeEnvelope.payload)
        } elseif ($script:SHELLSENSE_PIPE_ENABLED) {
            # The host sends the pipe frame before its internal key. Do not
            # guess by reading a file if a connected pipe has no frame yet.
            return
        }

        if ($null -eq $payload) {
            if ([string]::IsNullOrEmpty($path)) {
                Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
                    code    = 'edit_path_unavailable'
                    message = 'SHELLSENSE_EDIT_PATH is not configured.'
                })
                return
            }
            if (-not [IO.File]::Exists($path)) {
                Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
                    code    = 'edit_payload_missing'
                    message = 'No pending edit payload is available.'
                })
                return
            }

            # Rename first so a payload written while this handler is running
            # is left for the next invocation. The consumed name prevents
            # replay.
            $consumedPath = Get-ShellsenseConsumedEditPath -Path $path
            [IO.File]::Move($path, $consumedPath)

            $utf8Strict = [Text.UTF8Encoding]::new($false, $true)
            $json = [IO.File]::ReadAllText($consumedPath, $utf8Strict)
            $payload = ConvertFrom-ShellsenseEditJson -Json $json
        }

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
        $editRequestId = $null
        if ($edit -is [System.Collections.IDictionary] -and $edit.Contains('id')) {
            $editRequestId = [string]$edit['id']
        }

        # This is the only edit operation.  The text is passed as data to
        # Replace; it is never parsed or invoked as PowerShell.
        try {
            [Microsoft.PowerShell.PSConsoleReadLine]::Replace(
                [int]$edit.start,
                [int]$edit.length,
                [string]$edit.text)
            $replaceApplied = $true
        } catch {
            $replaceError = $_
        }
        Send-ShellsenseEditResult -RequestId $editRequestId -Applied $replaceApplied
        Send-ShellsenseBuffer
        if ($null -ne $replaceError) {
            Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
                code        = 'edit_failed'
                request_id  = $editRequestId
                message     = 'The pending edit could not be applied.'
            })
        }
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
    # Unary comma keeps the lazy IEnumerable intact when this helper is used
    # in an assignment. A normal function return would let PowerShell walk
    # every command through its pipeline before the caller can obtain the
    # enumerator.
    return ,$ExecutionContext.InvokeCommand.GetCommands('*', $commandTypes, $true)
}

function Get-ShellsenseEnumerator {
    [CmdletBinding()]
    param(
        [AllowNull()]
        [object]$Sequence
    )

    if ($null -eq $Sequence) {
        throw 'The command sequence is null.'
    }
    # CommandInvocationIntrinsics currently returns an IEnumerable whose
    # GetEnumerator implementation is not always bindable from PowerShell.
    # Reflection calls the zero-argument .NET method directly and does not
    # materialize the sequence.
    $methods = $Sequence.GetType().GetMethods([Reflection.BindingFlags]'Instance,Public,NonPublic')
    foreach ($method in $methods) {
        if ([string]::Equals($method.Name, 'GetEnumerator', [StringComparison]::Ordinal) -and
            $method.GetParameters().Count -eq 0) {
            $enumerator = $method.Invoke($Sequence, [object[]]@())
            if ($enumerator -is [System.Collections.IEnumerator]) {
                return ,$enumerator
            }
        }
    }
    # GetCommands returns a compiler-generated iterator that implements both
    # IEnumerable and IEnumerator. Always ask the IEnumerable interface for a
    # fresh cursor before considering the object itself as an already-created
    # enumerator; using the latter would observe the iterator's uninitialized
    # state and produce an empty command snapshot.
    if ($Sequence -is [System.Collections.IEnumerable]) {
        $enumerator = ([System.Collections.IEnumerable]$Sequence).GetEnumerator()
        if ($enumerator -is [System.Collections.IEnumerator]) {
            return ,$enumerator
        }
    }
    if ($Sequence -is [System.Collections.IEnumerator]) {
        return ,$Sequence
    }
    throw 'The command sequence does not expose an enumerator.'
}

function Get-ShellsenseCommandEnumerator {
    [CmdletBinding()]
    param(
        # This optional provider keeps snapshot tests deterministic without
        # creating functions or aliases in the runspace. Production uses the
        # direct CommandInvocationIntrinsics sequence below.
        [AllowNull()]
        [scriptblock]$CommandProvider
    )

    if ($null -ne $CommandProvider) {
        $provided = & $CommandProvider
        return Get-ShellsenseEnumerator -Sequence $provided
    }

    $commandTypes = [System.Management.Automation.CommandTypes]::Alias -bor
        [System.Management.Automation.CommandTypes]::Function -bor
        [System.Management.Automation.CommandTypes]::Cmdlet
    $sequence = $ExecutionContext.InvokeCommand.GetCommands('*', $commandTypes, $true)
    return Get-ShellsenseEnumerator -Sequence $sequence
}

function ConvertTo-ShellsenseCommandRecord {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [AllowNull()]
        [object]$Command
    )

    if ($null -eq $Command) {
        return $null
    }
    $nameProperty = $Command.PSObject.Properties['Name']
    $typeProperty = $Command.PSObject.Properties['CommandType']
    if ($null -eq $nameProperty -or $null -eq $typeProperty -or
        [string]::IsNullOrEmpty([string]$nameProperty.Value)) {
        return $null
    }
    $kind = ([string]$typeProperty.Value).ToLowerInvariant()
    if ($kind -ne 'alias' -and $kind -ne 'function' -and $kind -ne 'cmdlet') {
        return $null
    }
    $definition = ''
    # Function/cmdlet bodies can be large or executable text. Aliases retain
    # only their target for display and root priority.
    if ($kind -eq 'alias') {
        $definitionProperty = $Command.PSObject.Properties['Definition']
        if ($null -ne $definitionProperty -and $null -ne $definitionProperty.Value) {
            $definition = [string]$definitionProperty.Value
        }
    }
    return [ordered]@{
        name       = [string]$nameProperty.Value
        kind       = $kind
        definition = $definition
    }
}

function Reset-ShellsenseCommandSnapshot {
    [CmdletBinding()]
    param()

    $enumerator = $script:SHELLSENSE_COMMAND_ENUMERATOR
    if ($null -ne $enumerator -and $enumerator -is [IDisposable]) {
        try {
            $enumerator.Dispose()
        } catch {
        }
    }
    $script:SHELLSENSE_COMMAND_ENUMERATOR = $null
    $script:SHELLSENSE_COMMAND_SNAPSHOT_ID = $null
    $script:SHELLSENSE_COMMAND_SNAPSHOT = $null
    $script:SHELLSENSE_COMMAND_SNAPSHOT_OFFSET = 0
    $script:SHELLSENSE_COMMAND_SNAPSHOT_FAILED = $false
    $script:SHELLSENSE_COMMAND_SNAPSHOT_COMPLETE = $false
    $script:SHELLSENSE_COMMAND_REQUEST_ID = $null
}

function Start-ShellsenseCommandSnapshot {
    [CmdletBinding()]
    param(
        [AllowNull()]
        [scriptblock]$CommandProvider,

        [AllowNull()]
        [string]$RequestId
    )

    $enumerator = Get-ShellsenseCommandEnumerator -CommandProvider $CommandProvider
    $snapshotId = [Guid]::NewGuid().ToString()
    $script:SHELLSENSE_COMMAND_ENUMERATOR = $enumerator
    # Keep this legacy state name as a reference to the enumerator rather than
    # an array, so callers can tell that command discovery is truly lazy.
    $script:SHELLSENSE_COMMAND_SNAPSHOT = $enumerator
    $script:SHELLSENSE_COMMAND_SNAPSHOT_ID = $snapshotId
    $script:SHELLSENSE_COMMAND_SNAPSHOT_OFFSET = 0
    $script:SHELLSENSE_COMMAND_SNAPSHOT_FAILED = $false
    $script:SHELLSENSE_COMMAND_SNAPSHOT_COMPLETE = $false
    $script:SHELLSENSE_COMMAND_REQUEST_ID = $RequestId
}

function Get-ShellsenseImportedCommands {
    [CmdletBinding()]
    param(
        [AllowNull()]
        [scriptblock]$CommandProvider,

        [switch]$Reset,

        [AllowNull()]
        [string]$RequestId
    )

    $batchSize = 64
    $batchBudgetMs = 2.0
    $traceTimer = $null
    if ($script:SHELLSENSE_TRACE_ENABLED) {
        $traceTimer = [Diagnostics.Stopwatch]::StartNew()
    }
    try {
        if ($Reset) {
            Reset-ShellsenseCommandSnapshot
        }
        $snapshotId = [string]$script:SHELLSENSE_COMMAND_SNAPSHOT_ID
        if ([string]::IsNullOrEmpty($snapshotId) -or $null -eq $script:SHELLSENSE_COMMAND_ENUMERATOR) {
            Start-ShellsenseCommandSnapshot -CommandProvider $CommandProvider -RequestId $RequestId
            $snapshotId = [string]$script:SHELLSENSE_COMMAND_SNAPSHOT_ID
        }

        $batch = [System.Collections.Generic.List[object]]::new()
        $complete = $false
        $failed = $false
        $started = [Diagnostics.Stopwatch]::StartNew()
        $inspected = 0
        while ($inspected -lt $batchSize) {
            # Always advance once; an unusually slow runspace operation must
            # not result in an empty response that can never make progress.
            if ($inspected -gt 0 -and $started.Elapsed.TotalMilliseconds -ge $batchBudgetMs) {
                break
            }
            try {
                if (-not $script:SHELLSENSE_COMMAND_ENUMERATOR.MoveNext()) {
                    $complete = $true
                    break
                }
                $inspected++
                $record = ConvertTo-ShellsenseCommandRecord -Command $script:SHELLSENSE_COMMAND_ENUMERATOR.Current
                if ($null -ne $record) {
                    [void]$batch.Add($record)
                }
                $script:SHELLSENSE_COMMAND_SNAPSHOT_OFFSET++
            } catch {
                $failed = $true
                break
            }
        }

        $script:SHELLSENSE_COMMAND_SNAPSHOT_COMPLETE = [bool]$complete
        $script:SHELLSENSE_COMMAND_SNAPSHOT_FAILED = [bool]$failed
        $eventData = [ordered]@{
            snapshot = $snapshotId
            # A failed/partial enumerator can never publish a complete marker.
            complete = [bool]($complete -and -not $failed)
            commands = @($batch.ToArray())
        }
        if (-not [string]::IsNullOrEmpty([string]$script:SHELLSENSE_COMMAND_REQUEST_ID)) {
            $eventData.request_id = [string]$script:SHELLSENSE_COMMAND_REQUEST_ID
            $script:SHELLSENSE_COMMAND_REQUEST_ID = $null
        }
        Send-ShellsenseEvent -Event 'commands' -Data $eventData

        if ($failed) {
            Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
                code    = 'commands_unavailable'
                message = 'Loaded command enumeration failed before the snapshot completed.'
            })
            Reset-ShellsenseCommandSnapshot
        } elseif ($complete) {
            Reset-ShellsenseCommandSnapshot
        }
    } catch {
        # Do not let an exception turn the last partial page into a complete
        # snapshot. Preserve a diagnostic and reset before a later retry.
        $snapshotId = [string]$script:SHELLSENSE_COMMAND_SNAPSHOT_ID
        if (-not [string]::IsNullOrEmpty($snapshotId)) {
            Send-ShellsenseEvent -Event 'commands' -Data ([ordered]@{
                snapshot = $snapshotId
                complete = $false
                commands = @()
            })
        }
        Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
            code    = 'commands_unavailable'
            message = 'Loaded command enumeration failed before the snapshot completed.'
        })
        Reset-ShellsenseCommandSnapshot
    } finally {
        if ($null -ne $traceTimer) {
            $traceTimer.Stop()
            Send-ShellsenseTrace -Stage 'command_snapshot' -DurationMs $traceTimer.Elapsed.TotalMilliseconds
        }
    }
}

function ConvertTo-ShellsenseNativeCandidateKind {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$ResultType
    )

    switch ($ResultType) {
        'Command' { return 'command' }
        'ParameterName' { return 'option' }
        'ParameterValue' { return 'value' }
        'ProviderItem' { return 'file' }
        'ProviderContainer' { return 'directory' }
        'Alias' { return 'alias' }
        'Function' { return 'function' }
        'Cmdlet' { return 'cmdlet' }
        'Subcommand' { return 'subcommand' }
        default {
            # Native CompletionResultType has Property/Method/Keyword and
            # other UI-only values that are not part of the stable ShellSense
            # CandidateKind enum. Keep them valid and show the original type
            # in the description/tooltip rather than breaking the response.
            return 'value'
        }
    }
}

function Get-ShellsenseNativeCompletion {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$RequestId
    )

    $line = ''
    $cursor = 0
    $replaceStart = 0
    $replaceEnd = 0
    $status = 'ok'
    $candidates = [System.Collections.Generic.List[object]]::new()
    $context = $null
    $savedLastExitCode = $ExecutionContext.SessionState.PSVariable.GetValue('global:LASTEXITCODE')
    try {
        try {
            [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line, [ref]$cursor)
            $line = [string]$line
            $cursor = [int]$cursor
            $replaceStart = $cursor
            $replaceEnd = $cursor
        } catch {
            $status = 'unavailable'
        }

        if ($status -eq 'ok') {
            try {
                # This overload runs in the current PowerShell runspace and can
                # invoke the user's registered completers. It is reachable only
                # through the explicit native key, never from automatic queries.
                $native = [System.Management.Automation.CommandCompletion]::CompleteInput(
                    $line,
                    $cursor,
                    [hashtable]@{})
                if ($null -ne $native) {
                    $replaceStart = [int]$native.ReplacementIndex
                    $replaceEnd = $replaceStart + [int]$native.ReplacementLength
                    if (-not (Test-ShellsenseUtf16Range -Line $line -Start $replaceStart -Length ($replaceEnd - $replaceStart))) {
                        $status = 'error'
                        # Error responses still carry a valid UTF-16 range so
                        # the host cannot accidentally slice at a sentinel
                        # such as CompleteInput's -1/-1 no-match result.
                        $replaceStart = $cursor
                        $replaceEnd = $cursor
                    } else {
                        foreach ($match in $native.CompletionMatches) {
                            if ($null -eq $match) {
                                continue
                            }
                            $label = [string]$match.ListItemText
                            if ([string]::IsNullOrEmpty($label)) {
                                $label = [string]$match.CompletionText
                            }
                            [void]$candidates.Add([ordered]@{
                                label       = $label
                                insert_text = [string]$match.CompletionText
                                description = [string]$match.ToolTip
                                kind        = ConvertTo-ShellsenseNativeCandidateKind -ResultType ([string]$match.ResultType)
                            })
                        }
                    }
                }
            } catch {
                # User completers are allowed to fail. Keep the response
                # bounded and let the caller decide whether to fall back.
                $status = 'error'
                $candidates.Clear()
                $replaceStart = $cursor
                $replaceEnd = $cursor
            }
        }
        if (Test-ShellsenseComplexLine -Line $line) {
            $ast = $null
            $tokens = $null
            $parseErrors = $null
            try {
                $astCursor = 0
                [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState(
                    [ref]$ast,
                    [ref]$tokens,
                    [ref]$parseErrors,
                    [ref]$astCursor)
            } catch {
            }
            $context = Get-ShellsenseAstBufferContext `
                -Line $line -Cursor $cursor -Ast $ast -Tokens $tokens -ParseErrors $parseErrors
        }
    } finally {
        # Native completers may run external commands. Their exit code is not
        # part of ShellSense's edit protocol and must not alter Prompt status.
        $global:LASTEXITCODE = $savedLastExitCode
    }

    $data = [ordered]@{
        request_id   = $RequestId
        line         = [string]$line
        cursor       = [int]$cursor
        replace_start = [int]$replaceStart
        replace_end  = [int]$replaceEnd
        candidates   = @($candidates.ToArray())
        status       = $status
    }
    if ($null -ne $context) {
        $data.context = $context
    }
    Send-ShellsenseEvent -Event 'native_completion' -Data $data
}

function Get-ShellsenseCurrentBufferState {
    [CmdletBinding()]
    param()

    $line = $null
    $cursor = 0
    [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line, [ref]$cursor)
    return [pscustomobject]@{
        line   = [string]$line
        cursor = [int]$cursor
    }
}

function Test-ShellsenseConfirmedContinuation {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$Line,

        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$PreviousLine,

        [switch]$AddLine
    )

    # AddLine is an explicit PSReadLine continuation action. AcceptLine is
    # accepted here only when the resulting live buffer visibly contains the
    # continuation line; a complete command leaves this function silent and
    # lets PSConsoleHostReadLine emit execute after it returns.
    if ($AddLine) {
        return $Line.Length -gt $PreviousLine.Length -or $Line -match '[\r\n]'
    }
    return $Line -match '[\r\n]'
}

function Invoke-ShellsenseEnterKeyHandler {
    [CmdletBinding()]
    param()

    $previous = $null
    try {
        $previous = Get-ShellsenseCurrentBufferState
        $savedLastExitCode = $ExecutionContext.SessionState.PSVariable.GetValue('global:LASTEXITCODE')
        try {
            [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine($null, $null)
        } finally {
            $global:LASTEXITCODE = $savedLastExitCode
        }
        $current = Get-ShellsenseCurrentBufferState
        if (Test-ShellsenseConfirmedContinuation -Line $current.line -PreviousLine $previous.line) {
            Send-ShellsenseEvent -Event 'editing' -Data ([ordered]@{ state = 'continuation' })
        }
    } catch {
        Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
            code    = 'enter_handler_failed'
            message = 'The known PSReadLine Enter handler could not be invoked.'
        })
    }
}

function Invoke-ShellsenseShiftEnterKeyHandler {
    [CmdletBinding()]
    param()

    $previous = $null
    try {
        $previous = Get-ShellsenseCurrentBufferState
        $savedLastExitCode = $ExecutionContext.SessionState.PSVariable.GetValue('global:LASTEXITCODE')
        try {
            [Microsoft.PowerShell.PSConsoleReadLine]::AddLine($null, $null)
        } finally {
            $global:LASTEXITCODE = $savedLastExitCode
        }
        $current = Get-ShellsenseCurrentBufferState
        if (Test-ShellsenseConfirmedContinuation -Line $current.line -PreviousLine $previous.line -AddLine) {
            Send-ShellsenseEvent -Event 'editing' -Data ([ordered]@{ state = 'continuation' })
        }
    } catch {
        Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
            code    = 'shift_enter_handler_failed'
            message = 'The known PSReadLine Shift+Enter handler could not be invoked.'
        })
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
    $request = Read-ShellsenseRequest -ExpectedKind @('commands_reset', 'commands_next')
    if ($null -ne $request) {
        if ([string]::Equals([string]$request.kind, 'commands_reset', [StringComparison]::Ordinal)) {
            # Host key reloads are serialized with the command reset request.
            # A supplied public map is authoritative for this session, so
            # re-run arbitration before the reset batch and publish the new
            # capability state before the host consumes refreshed commands.
            # Requests from older hosts omit this field and retain the
            # previous behaviour.
            $publicKeysProperty = $request.PSObject.Properties['public_keys_json']
            if ($null -ne $publicKeysProperty -and $null -ne $publicKeysProperty.Value) {
                Update-ShellsensePublicKeyCapabilities -JsonOverride ([string]$publicKeysProperty.Value)
                Send-ShellsenseCapabilities
            }
            Get-ShellsenseImportedCommands -Reset -RequestId ([string]$request.id)
        } else {
            # In pipe mode every continuation batch has an explicit request so
            # the server and child remain in lockstep without a request file.
            Get-ShellsenseImportedCommands -RequestId ([string]$request.id)
        }
    } else {
        # OSC mode historically triggered the next batch without a request
        # file. Preserve that compact path when no pipe is connected.
        if (-not $script:SHELLSENSE_PIPE_ENABLED) {
            Get-ShellsenseImportedCommands
        }
    }
}

function Invoke-ShellsenseNativeKeyHandler {
    [CmdletBinding()]
    param()
    $request = Read-ShellsenseRequest -ExpectedKind 'native'
    if ($null -ne $request) {
        Get-ShellsenseNativeCompletion -RequestId ([string]$request.id)
    }
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
        [string]$Chord,

        [AllowNull()]
        [object]$Snapshot
    )

    if ($null -ne $Snapshot -and [bool]$Snapshot.available) {
        if ($Snapshot.by_chord.ContainsKey($Chord)) {
            # Snapshot buckets are immutable object[] values.  Do not call
            # List.ToArray here: the fast snapshot path intentionally avoids
            # allocating a generic list for every chord.
            return @($Snapshot.by_chord[$Chord])
        }
        return @()
    }

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

function Get-ShellsenseKeyHandlerSnapshot {
    [CmdletBinding()]
    param()

    try {
        # PSReadLine's all-handler overload is the only reflection/cmdlet
        # lookup needed for normal startup. Build a case-insensitive chord map
        # once so public-key arbitration, lifecycle detection, and reserved
        # collision checks share the same immutable view.
        $handlers = [object[]]@(
            [Microsoft.PowerShell.PSConsoleReadLine]::GetKeyHandlers($true, $false))
        # A PowerShell hashtable is cheaper to access from the adapter than a
        # generic dictionary and keeps the lookup case-insensitive.  Each
        # value is an object[] bucket so all consumers share one read-only
        # view without constructing a List for every chord.
        $byChord = [System.Collections.Hashtable]::new(
            [StringComparer]::OrdinalIgnoreCase)
        foreach ($handler in $handlers) {
            $chord = [string]$handler.Key
            if ([string]::IsNullOrEmpty($chord)) {
                continue
            }
            if (-not $byChord.ContainsKey($chord)) {
                [void]$byChord.Add($chord, [object[]]@($handler))
                continue
            }
            # Duplicate chord entries are uncommon, so copying only that
            # bucket keeps the hot lookup path simple while avoiding a
            # per-chord mutable collection and its method dispatch.
            $bucket = [object[]]@($byChord[$chord])
            [void]($byChord[$chord] = [object[]]($bucket + [object]$handler))
        }
        return [pscustomobject]@{
            available = $true
            handlers  = $handlers
            by_chord  = $byChord
        }
    } catch {
        return [pscustomobject]@{
            available = $false
            handlers  = [object[]]@()
            by_chord  = [System.Collections.Hashtable]::new(
                [StringComparer]::OrdinalIgnoreCase)
        }
    }
}

function ConvertTo-ShellsensePublicKeyChord {
    [CmdletBinding()]
    param(
        [AllowNull()]
        [string]$Chord
    )

    if ([string]::IsNullOrWhiteSpace($Chord)) {
        return $null
    }
    $value = $Chord.Trim()
    # The Rust host accepts compact names such as Ctrl+Space while PSReadLine
    # spells the ConsoleKey value Spacebar. Keep the public configuration
    # readable and normalize only this known alias.
    switch -Regex ($value) {
        '^(?i:CtrlAltSpace)$' { return 'Ctrl+Alt+Spacebar' }
        '^(?i:CtrlSpace)$' { return 'Ctrl+Spacebar' }
    }
    return [regex]::Replace($value, '(?i)(^|\+)Space($|\+)', '$1Spacebar$2')
}

function Get-ShellsensePublicKeyConfiguration {
    [CmdletBinding()]
    param(
        [AllowNull()]
        [string]$JsonOverride
    )

    $hasJsonOverride = $PSBoundParameters.ContainsKey('JsonOverride')
    $raw = if ($hasJsonOverride) {
        $JsonOverride
    } else {
        [Environment]::GetEnvironmentVariable('SHELLSENSE_PUBLIC_KEYS', 'Process')
    }

    # The Rust host validates and exports these six process values before the
    # adapter is loaded.  On that path avoid parsing JSON and allocating a
    # JsonDocument.  JsonOverride always takes the original parser so a hot
    # reload remains fully validated and cannot accidentally trust stale host
    # values.
    if (-not $hasJsonOverride) {
        $version = [Environment]::GetEnvironmentVariable('SHELLSENSE_PUBLIC_KEYS_VERSION', 'Process')
        if ([string]::Equals([string]$version, '1', [StringComparison]::Ordinal)) {
            $fastConfiguration = [ordered]@{}
            $fastComplete = $true
            foreach ($name in @('trigger', 'native', 'details', 'refresh', 'reload')) {
                $environmentName = 'SHELLSENSE_PUBLIC_KEY_' + $name.ToUpperInvariant()
                $value = [Environment]::GetEnvironmentVariable($environmentName, 'Process')
                if ([string]::IsNullOrWhiteSpace([string]$value)) {
                    $fastComplete = $false
                    break
                }
                $fastConfiguration[$name] = [string]$value
            }
            if ($fastComplete) {
                $script:SHELLSENSE_PUBLIC_KEYS = $fastConfiguration
                $script:SHELLSENSE_PUBLIC_KEY_STATUS = [ordered]@{}
                $script:SHELLSENSE_PUBLIC_KEY_CONFIG_ERROR = $false
                return $fastConfiguration
            }
        }
    }

    if ([string]::IsNullOrWhiteSpace([string]$raw)) {
        $script:SHELLSENSE_PUBLIC_KEYS = $null
        $script:SHELLSENSE_PUBLIC_KEY_STATUS = $null
        $script:SHELLSENSE_PUBLIC_KEY_CONFIG_ERROR = $false
        return $null
    }

    $configuration = [ordered]@{}
    $invalid = $false
    $document = $null
    try {
        $document = [System.Text.Json.JsonDocument]::Parse([string]$raw)
        if ($document.RootElement.ValueKind -ne [System.Text.Json.JsonValueKind]::Object) {
            $invalid = $true
        } else {
            foreach ($name in @('trigger', 'native', 'details', 'refresh', 'reload')) {
                $property = [System.Text.Json.JsonElement]::new()
                if (-not $document.RootElement.TryGetProperty($name, [ref]$property)) {
                    continue
                }
                if ($property.ValueKind -ne [System.Text.Json.JsonValueKind]::String -or
                    [string]::IsNullOrWhiteSpace([string]$property.GetString())) {
                    $invalid = $true
                    continue
                }
                $configuration[$name] = [string]$property.GetString()
            }
        }
    } catch {
        $invalid = $true
    } finally {
        if ($null -ne $document) {
            $document.Dispose()
        }
    }

    $script:SHELLSENSE_PUBLIC_KEYS = $configuration
    $script:SHELLSENSE_PUBLIC_KEY_STATUS = [ordered]@{}
    $script:SHELLSENSE_PUBLIC_KEY_CONFIG_ERROR = $invalid
    return $configuration
}

function Update-ShellsensePublicKeyCapabilities {
    [CmdletBinding()]
    param(
        [AllowNull()]
        [string]$JsonOverride,

        [AllowNull()]
        [object]$KeySnapshot
    )

    if ($PSBoundParameters.ContainsKey('JsonOverride')) {
        $configuration = Get-ShellsensePublicKeyConfiguration -JsonOverride $JsonOverride
    } else {
        $configuration = Get-ShellsensePublicKeyConfiguration
    }
    if ($null -eq $configuration) {
        return
    }

    if ($null -eq $KeySnapshot) {
        $KeySnapshot = Get-ShellsenseKeyHandlerSnapshot
    }

    $status = [ordered]@{}
    foreach ($name in $configuration.Keys) {
        $configuredChord = [string]$configuration[$name]
        $psReadLineChord = ConvertTo-ShellsensePublicKeyChord -Chord $configuredChord
        $bindings = @()
        if (-not [string]::IsNullOrEmpty([string]$psReadLineChord)) {
            $bindings = @(Get-ShellsenseKeyBinding -Chord $psReadLineChord -Snapshot $KeySnapshot)
        }

        $safe = $false
        if ($bindings.Count -eq 0) {
            $safe = $true
        } elseif ([string]::Equals($name, 'trigger', [StringComparison]::Ordinal) -or
            [string]::Equals($name, 'details', [StringComparison]::Ordinal)) {
            # MenuComplete/Complete and ShowCommandHelp are the standard
            # actions that the host may supersede while its menu is active. A
            # custom ScriptBlock or any mixed binding remains user-owned.
            $safe = $true
            foreach ($binding in $bindings) {
                $standard = [string]::Equals($name, 'trigger', [StringComparison]::Ordinal) -and
                    ([string]::Equals([string]$binding.Function, 'MenuComplete', [StringComparison]::OrdinalIgnoreCase) -or
                    [string]::Equals([string]$binding.Function, 'Complete', [StringComparison]::OrdinalIgnoreCase))
                if ([string]::Equals($name, 'details', [StringComparison]::Ordinal)) {
                    $standard = [string]::Equals([string]$binding.Function, 'ShowCommandHelp', [StringComparison]::OrdinalIgnoreCase)
                }
                if (-not $standard) {
                    $safe = $false
                    break
                }
            }
        }
        $status[$name] = [bool]$safe
        if (-not $safe) {
            $boundFunction = ''
            if ($bindings.Count -gt 0) {
                $boundFunction = [string]$bindings[0].Function
            }
            $suggestion = 'Change the ShellSense public key in the host key settings and start a new session.'
            Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
                code          = 'public_key_collision'
                name          = $name
                chord         = $configuredChord
                bound_function = $boundFunction
                suggestion    = $suggestion
                message       = ('Public key {0} ({1}) keeps its PSReadLine binding ({2}); ShellSense will not intercept it. {3}' -f $name, $configuredChord, $boundFunction, $suggestion)
            })
        }
    }
    $script:SHELLSENSE_PUBLIC_KEY_STATUS = $status
    if ($script:SHELLSENSE_PUBLIC_KEY_CONFIG_ERROR) {
        Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
            code       = 'public_key_config_invalid'
            message    = 'SHELLSENSE_PUBLIC_KEYS must be a JSON object with string key chords.'
            suggestion = 'Remove the variable or provide trigger/native/details/refresh/reload chord strings.'
        })
    }
}

function Get-ShellsenseAlternativeKeyPrefix {
    [CmdletBinding()]
    param()

    foreach ($candidate in @('F5', 'F6', 'F7', 'F8', 'F9', 'F10', 'F11', 'F12')) {
        if (-not [string]::Equals($candidate, [string]$script:SHELLSENSE_KEY_PREFIX, [StringComparison]::OrdinalIgnoreCase)) {
            return $candidate
        }
    }
    return 'F5'
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
        $collisionSuggestion = ('Choose another protocol prefix (F5-F12) in the next session, for example set SHELLSENSE_KEY_PREFIX={0}.' -f (Get-ShellsenseAlternativeKeyPrefix))
        Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
            code    = 'key_chord_collision'
            chord   = $Chord
            suggestion = $collisionSuggestion
            message = ('Reserved chord {0} is already bound; it was left unchanged. {1}' -f $Chord, $collisionSuggestion)
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

    $coreReady = ([bool]$script:SHELLSENSE_PSREADLINE_AVAILABLE -and
        [bool]$script:SHELLSENSE_KEY_HANDLERS.buffer -and
        [bool]$script:SHELLSENSE_KEY_HANDLERS.apply -and
        [bool]$script:SHELLSENSE_KEY_HANDLERS.commands)
    $capabilityMap = [ordered]@{
        context           = $true
        command_position  = $true
        native_completion = [bool]$script:SHELLSENSE_KEY_HANDLERS.native
        edit_ack          = $true
        command_batches   = $true
        multiline         = ([bool]$script:SHELLSENSE_KEY_HANDLERS.enter -or
            [bool]$script:SHELLSENSE_KEY_HANDLERS.shift_enter)
        manual_native     = $true
    }
    # Keep the field absent when the host did not opt into public-key
    # arbitration, preserving the alpha protocol shape for existing launchers.
    if ($null -ne $script:SHELLSENSE_PUBLIC_KEY_STATUS) {
        $capabilityMap.public_keys = $script:SHELLSENSE_PUBLIC_KEY_STATUS
    }
    Send-ShellsenseEvent -Event 'capabilities' -Data ([ordered]@{
        protocol_version = 2
        protocol         = 2
        version          = 2
        # `ready` means the complete PSReadLine surface is usable. Consumers
        # can still use prompt/execute events when this is false, while the
        # detailed fields below explain whether a missing module or a chord
        # collision is responsible.
        ready        = $coreReady
        psreadline   = [bool]$script:SHELLSENSE_PSREADLINE_AVAILABLE
        key_handlers = $script:SHELLSENSE_KEY_HANDLERS
        edit_path    = (-not [string]::IsNullOrEmpty([string]$script:SHELLSENSE_EDIT_PATH))
        request_path = (-not [string]::IsNullOrEmpty([string](Get-ShellsenseRequestPath)))
        key_prefix   = [string]$script:SHELLSENSE_KEY_PREFIX
        # Report the transport that actually accepted this event. A failed
        # named-pipe connection is reflected as OSC here, so host A/B probes
        # cannot mistake a silent fallback for a pipe run.
        transport    = if ($script:SHELLSENSE_PIPE_ENABLED) { 'pipe' } else { 'osc' }
        capabilities = $capabilityMap
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
            buffer      = $false
            apply       = $false
            commands    = $false
            native      = $false
            enter       = $false
            shift_enter = $false
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

    $readLineWrapTimer = $null
    if ($script:SHELLSENSE_TRACE_ENABLED) {
        $readLineWrapTimer = [Diagnostics.Stopwatch]::StartNew()
    }
    try {
        if ($null -eq $script:SHELLSENSE_ORIGINAL_READLINE) {
            $script:SHELLSENSE_ORIGINAL_READLINE = $readLineCommand.ScriptBlock
        }
        if (-not $script:SHELLSENSE_READLINE_WRAPPED) {
            function global:PSConsoleHostReadLine {
                $savedLastExitCode = $ExecutionContext.SessionState.PSVariable.GetValue('global:LASTEXITCODE')
                try {
                    $acceptedLine = & $script:SHELLSENSE_ORIGINAL_READLINE @args
                    # ReadLine returned only after PSReadLine accepted the command.
                    # Continuation AddLine calls remain inside ReadLine and do not
                    # reach this marker, so the host cannot query an external
                    # program while the user is still entering a multiline line.
                    Reset-ShellsenseCommandSnapshot
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
    } finally {
        if ($null -ne $readLineWrapTimer) {
            $readLineWrapTimer.Stop()
            Send-ShellsenseTrace -Stage 'readline_wrap' -DurationMs $readLineWrapTimer.Elapsed.TotalMilliseconds
        }
    }

    # Query all bound handlers once before registering any reserved chords. A
    # user's exact chord and a bare protocol prefix parent both reserve a
    # ShellSense chord. The same immutable snapshot also supplies public-key
    # arbitration and lifecycle detection, avoiding repeated reflection and
    # cold GetKeyHandlers pipelines during startup.
    $keySnapshotTimer = $null
    if ($script:SHELLSENSE_TRACE_ENABLED) {
        $keySnapshotTimer = [Diagnostics.Stopwatch]::StartNew()
    }
    $keySnapshot = $null
    try {
        $keySnapshot = Get-ShellsenseKeyHandlerSnapshot
    } finally {
        if ($null -ne $keySnapshotTimer) {
            $keySnapshotTimer.Stop()
            Send-ShellsenseTrace -Stage 'key_snapshot' -DurationMs $keySnapshotTimer.Elapsed.TotalMilliseconds
        }
    }

    $knownEnter = $false
    $knownShiftEnter = $false
    if ([bool]$keySnapshot.available) {
        # The snapshot map already groups handlers by chord. Inspect only the
        # two lifecycle buckets instead of scanning every handler for each
        # key, while retaining all PSReadLine fallback calls below.
        $snapshotByChord = $keySnapshot.by_chord
        if ($snapshotByChord.ContainsKey('Enter')) {
            foreach ($handler in [object[]]@($snapshotByChord['Enter'])) {
                if ([string]::Equals([string]$handler.Function, 'AcceptLine', [StringComparison]::OrdinalIgnoreCase)) {
                    $knownEnter = $true
                    break
                }
            }
        }
        if ($snapshotByChord.ContainsKey('Shift+Enter')) {
            foreach ($handler in [object[]]@($snapshotByChord['Shift+Enter'])) {
                if ([string]::Equals([string]$handler.Function, 'AddLine', [StringComparison]::OrdinalIgnoreCase)) {
                    $knownShiftEnter = $true
                    break
                }
            }
        }
    } else {
        try {
            $knownEnter = @([Microsoft.PowerShell.PSConsoleReadLine]::GetKeyHandlers([string[]]@('Enter'))) |
                Where-Object { [string]::Equals([string]$_.Function, 'AcceptLine', [StringComparison]::OrdinalIgnoreCase) } |
                Select-Object -First 1 | ForEach-Object { $true }
            $knownShiftEnter = @([Microsoft.PowerShell.PSConsoleReadLine]::GetKeyHandlers([string[]]@('Shift+Enter'))) |
                Where-Object { [string]::Equals([string]$_.Function, 'AddLine', [StringComparison]::OrdinalIgnoreCase) } |
                Select-Object -First 1 | ForEach-Object { $true }
        } catch {
            # Older PSReadLine builds simply leave the optional lifecycle
            # chords unavailable; the original Enter bindings remain untouched.
        }
    }

    # Public host shortcuts are configured outside the protocol chord. Inspect
    # their current PSReadLine owners before the host starts intercepting them;
    # this also leaves an explicit diagnostic for a user ScriptBlock binding.
    $publicKeyTimer = $null
    if ($script:SHELLSENSE_TRACE_ENABLED) {
        $publicKeyTimer = [Diagnostics.Stopwatch]::StartNew()
    }
    try {
        Update-ShellsensePublicKeyCapabilities -KeySnapshot $keySnapshot
    } finally {
        if ($null -ne $publicKeyTimer) {
            $publicKeyTimer.Stop()
            Send-ShellsenseTrace -Stage 'public_keys' -DurationMs $publicKeyTimer.Elapsed.TotalMilliseconds
        }
    }
    $reservedChords = [ordered]@{
        buffer   = ([string]$script:SHELLSENSE_KEY_PREFIX + ',s')
        apply    = ([string]$script:SHELLSENSE_KEY_PREFIX + ',a')
        commands = ([string]$script:SHELLSENSE_KEY_PREFIX + ',c')
        native   = ([string]$script:SHELLSENSE_KEY_PREFIX + ',n')
    }
    if ($knownEnter) {
        $reservedChords.enter = ([string]$script:SHELLSENSE_KEY_PREFIX + ',e')
    }
    if ($knownShiftEnter) {
        $reservedChords.shift_enter = ([string]$script:SHELLSENSE_KEY_PREFIX + ',l')
    }
    $existingByName = [ordered]@{
        buffer      = $false
        apply       = $false
        commands    = $false
        native      = $false
        enter       = $false
        shift_enter = $false
    }
    $keyHandlerEnumerationFailed = -not [bool]$keySnapshot.available
    if ($keyHandlerEnumerationFailed) {
        # Older PSReadLine builds may not expose GetKeyHandlers(bool,bool).
        # Preserve the compatibility path, but do not invoke it for a valid
        # empty result from the static API.
        foreach ($reservedName in $reservedChords.Keys) {
            $existingByName[$reservedName] = @(Get-ShellsenseKeyBinding -Chord $reservedChords[$reservedName]).Count -gt 0
        }
    } else {
        # A bare prefix occupies every child chord. The case-insensitive map
        # makes both that parent check and each exact child check constant-time
        # lookups, with no 72-by-6 nested scan during initialization.
        $barePrefixBound = $snapshotByChord.ContainsKey([string]$script:SHELLSENSE_KEY_PREFIX)
        foreach ($reservedName in $reservedChords.Keys) {
            $reservedChord = [string]$reservedChords[$reservedName]
            $existingByName[$reservedName] = $barePrefixBound -or
                $snapshotByChord.ContainsKey($reservedChord)
        }
    }

    $keyRegisterTimer = $null
    if ($script:SHELLSENSE_TRACE_ENABLED) {
        $keyRegisterTimer = [Diagnostics.Stopwatch]::StartNew()
    }
    try {
        foreach ($reservedName in $reservedChords.Keys) {
            if ([bool]$existingByName[$reservedName]) {
                $collisionSuggestion = ('Choose another protocol prefix (F5-F12) in the next session, for example set SHELLSENSE_KEY_PREFIX={0}.' -f (Get-ShellsenseAlternativeKeyPrefix))
                Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
                    code    = 'key_chord_collision'
                    chord   = $reservedChords[$reservedName]
                    suggestion = $collisionSuggestion
                    message = ('Reserved chord {0} is already bound; it was left unchanged. {1}' -f $reservedChords[$reservedName], $collisionSuggestion)
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
                'native' {
                    $script:SHELLSENSE_KEY_HANDLERS.native = Register-ShellsenseKeyHandler `
                        -Chord $reservedChords[$reservedName] -ScriptBlock ${function:Invoke-ShellsenseNativeKeyHandler} `
                        -Name 'native' -SkipCollisionCheck
                }
                'enter' {
                    $script:SHELLSENSE_KEY_HANDLERS.enter = Register-ShellsenseKeyHandler `
                        -Chord $reservedChords[$reservedName] -ScriptBlock ${function:Invoke-ShellsenseEnterKeyHandler} `
                        -Name 'enter' -SkipCollisionCheck
                }
                'shift_enter' {
                    $script:SHELLSENSE_KEY_HANDLERS.shift_enter = Register-ShellsenseKeyHandler `
                        -Chord $reservedChords[$reservedName] -ScriptBlock ${function:Invoke-ShellsenseShiftEnterKeyHandler} `
                        -Name 'shift_enter' -SkipCollisionCheck
                }
            }
        }
    } finally {
        if ($null -ne $keyRegisterTimer) {
            $keyRegisterTimer.Stop()
            Send-ShellsenseTrace -Stage 'key_register' -DurationMs $keyRegisterTimer.Elapsed.TotalMilliseconds
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
    Initialize-ShellsensePipe | Out-Null
    Initialize-ShellsensePrompt
    Initialize-ShellsenseReadLine -DeferImport
    Send-ShellsenseCapabilities
} finally {
    if ($null -ne $shellsenseBootstrapTraceTimer) {
        $shellsenseBootstrapTraceTimer.Stop()
        Send-ShellsenseTrace -Stage 'adapter_bootstrap' -DurationMs $shellsenseBootstrapTraceTimer.Elapsed.TotalMilliseconds
    }
}
