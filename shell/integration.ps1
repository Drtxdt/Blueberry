$blueberryJsonInitialization = [Diagnostics.Stopwatch]::StartNew()
# Select the native JSON implementation, or the inbox .NET Framework compatibility reader.
if ($PSVersionTable.PSVersion.Major -lt 7) {
    if ($null -eq ('Blueberry.LegacyJson.JsonSerializer' -as [type])) {
        Add-Type -TypeDefinition ([IO.File]::ReadAllText((Join-Path $PSScriptRoot 'legacy-json.cs'))) -ReferencedAssemblies @('System.Web.Extensions', 'System.Core', [System.Management.Automation.PSObject].Assembly.Location)
    }
    $blueberryJsonNamespace = 'Blueberry.LegacyJson'
} else { $blueberryJsonNamespace = 'System.Text.Json' }
foreach ($blueberryJsonType in @('JsonSerializerOptions', 'JsonSerializer', 'JsonDocument', 'JsonElement', 'JsonValueKind')) {
    $ExecutionContext.SessionState.PSVariable.Set(('BLUEBERRY_' + $blueberryJsonType), (($blueberryJsonNamespace + '.' + $blueberryJsonType) -as [type]))
}

$script:BLUEBERRY_JSON_INITIALIZATION_MS = $blueberryJsonInitialization.Elapsed.TotalMilliseconds
$blueberryJsonInitialization.Stop()

# Blueberry PowerShell integration.
#
# This file is intended to be dot-sourced by the pwsh process owned by
# blueberry.  It deliberately uses only the PowerShell and PSReadLine APIs
# already present in that process; completion text is never evaluated here.

# Keep state in this script scope. Dot-sourcing makes the functions below
# available to the interactive session while script scope keeps the original
# host functions and the protocol token private to this adapter. Initialize all
# state names before reading them so a user's Set-StrictMode in their profile
# does not make the adapter fail during bootstrap.
$blueberryStateDefaults = [ordered]@{
    BLUEBERRY_TOKEN               = $null
    BLUEBERRY_PIPE_NAME           = $null
    BLUEBERRY_PIPE_STREAM         = $null
    BLUEBERRY_PIPE_BUFFER         = $null
    BLUEBERRY_PIPE_SEQUENCE       = [int64]0
    BLUEBERRY_PIPE_ENABLED        = $false
    BLUEBERRY_EDIT_PATH           = $null
    BLUEBERRY_REQUEST_PATH        = $null
    BLUEBERRY_KEY_PREFIX          = 'F12'
    BLUEBERRY_PUBLIC_KEYS          = $null
    BLUEBERRY_PUBLIC_KEY_STATUS    = $null
    BLUEBERRY_PUBLIC_KEY_CONFIG_ERROR = $false
    BLUEBERRY_FALLBACK_CWD        = $null
    BLUEBERRY_ORIGINAL_PROMPT     = $null
    BLUEBERRY_PROMPT_WRAPPED      = $false
    BLUEBERRY_ORIGINAL_READLINE   = $null
    BLUEBERRY_READLINE_WRAPPED    = $false
    BLUEBERRY_PSREADLINE_AVAILABLE = $false
    BLUEBERRY_KEY_HANDLERS        = [ordered]@{ buffer = $false; apply = $false; commands = $false; native = $false; paste = $false; enter = $false; shift_enter = $false }
    BLUEBERRY_CAPABILITIES_REFRESHED = $false
    BLUEBERRY_COMMAND_SNAPSHOT_ID = $null
    BLUEBERRY_COMMAND_SNAPSHOT = $null
    BLUEBERRY_COMMAND_SNAPSHOT_OFFSET = 0
    BLUEBERRY_COMMAND_ENUMERATOR  = $null
    BLUEBERRY_COMMAND_SNAPSHOT_FAILED = $false
    BLUEBERRY_COMMAND_SNAPSHOT_COMPLETE = $false
    BLUEBERRY_COMMAND_REQUEST_ID  = $null
    BLUEBERRY_ENVIRONMENT_SNAPSHOT = $null
    BLUEBERRY_JSON_OPTIONS        = $null
    BLUEBERRY_TRACE_ENABLED       = $false
    BLUEBERRY_TRACE_EMITTING      = $false
    BLUEBERRY_INITIALIZED         = $false
}
foreach ($blueberryStateName in $blueberryStateDefaults.Keys) {
    # PSVariable is part of the host API and does not auto-import
    # Microsoft.PowerShell.Utility the way Get-Variable/Set-Variable do.
    # Keeping bootstrap on this API avoids a cold-start module load before the
    # user's first prompt.
    if ($null -eq $ExecutionContext.SessionState.PSVariable.Get($blueberryStateName)) {
        $ExecutionContext.SessionState.PSVariable.Set($blueberryStateName, $blueberryStateDefaults[$blueberryStateName])
    }
}

$script:BLUEBERRY_TRACE_ENABLED = [string]::Equals(
    [Environment]::GetEnvironmentVariable('BLUEBERRY_TRACE', 'Process'),
    '1',
    [StringComparison]::Ordinal)

$blueberryTokenFromEnvironment = [Environment]::GetEnvironmentVariable('BLUEBERRY_TOKEN', 'Process')
if ([string]::IsNullOrEmpty([string]$script:BLUEBERRY_TOKEN)) {
    $script:BLUEBERRY_TOKEN = $blueberryTokenFromEnvironment
}

$blueberryPipeNameFromEnvironment = [Environment]::GetEnvironmentVariable('BLUEBERRY_PIPE_NAME', 'Process')
if ($null -ne $blueberryPipeNameFromEnvironment) {
    $script:BLUEBERRY_PIPE_NAME = $blueberryPipeNameFromEnvironment
}

$blueberryEditPathFromEnvironment = [Environment]::GetEnvironmentVariable('BLUEBERRY_EDIT_PATH', 'Process')
if ($null -ne $blueberryEditPathFromEnvironment) {
    $script:BLUEBERRY_EDIT_PATH = $blueberryEditPathFromEnvironment
}

$blueberryRequestPathFromEnvironment = [Environment]::GetEnvironmentVariable('BLUEBERRY_REQUEST_PATH', 'Process')
if ($null -ne $blueberryRequestPathFromEnvironment) {
    $script:BLUEBERRY_REQUEST_PATH = $blueberryRequestPathFromEnvironment
}

# The host chooses a protocol prefix per session. Keep the accepted set small
# so a malformed configuration can never install an arbitrary PSReadLine
# chord. The default remains the alpha F12 prefix for old launchers.
$blueberryKeyPrefixFromEnvironment = [Environment]::GetEnvironmentVariable('BLUEBERRY_KEY_PREFIX', 'Process')
$blueberryKeyPrefix = if ([string]::IsNullOrEmpty([string]$blueberryKeyPrefixFromEnvironment)) {
    [string]$script:BLUEBERRY_KEY_PREFIX
} else {
    [string]$blueberryKeyPrefixFromEnvironment
}
if ($blueberryKeyPrefix -notmatch '^(?i:F(?:[5-9]|1[0-2]))$') {
    $blueberryKeyPrefix = 'F12'
}
$script:BLUEBERRY_KEY_PREFIX = $blueberryKeyPrefix.ToUpperInvariant()

if ([string]::IsNullOrEmpty([string]$script:BLUEBERRY_REQUEST_PATH) -and
    -not [string]::IsNullOrEmpty([string]$script:BLUEBERRY_EDIT_PATH)) {
    try {
        $requestDirectory = [IO.Path]::GetDirectoryName([IO.Path]::GetFullPath([string]$script:BLUEBERRY_EDIT_PATH))
        $script:BLUEBERRY_REQUEST_PATH = [IO.Path]::Combine($requestDirectory, 'request.json')
    } catch {
        # A malformed edit path is reported when the request or edit operation
        # is used. Bootstrap remains harmless for a user's shell.
    }
}

# The token is inherited by this process only to bootstrap the OSC channel.
# Do not leave it available to commands run by the user.
try {
    [Environment]::SetEnvironmentVariable('BLUEBERRY_TOKEN', $null, 'Process')
} catch {
    # The process environment is the source used by child processes.  Do not
    # fall back to Remove-Item here: that cmdlet can cold-load a large module
    # during startup, and PowerShell reflects the direct environment change.
}

function Get-BlueberryJsonOptions {
    [CmdletBinding()]
    param()

    if ($null -eq $script:BLUEBERRY_JSON_OPTIONS) {
        $options = $script:BLUEBERRY_JsonSerializerOptions::new()
        # JavaScriptEncoder.Default escapes all non-ASCII code points and
        # control characters. This keeps OSC transport ASCII, including a
        # non-BMP surrogate pair, while leaving JSON's structural characters
        # available in their compact form.
        if ($PSVersionTable.PSVersion.Major -ge 7) { $options.Encoder = [System.Text.Encodings.Web.JavaScriptEncoder]::Default }
        $options.WriteIndented = $false
        $script:BLUEBERRY_JSON_OPTIONS = $options
    }
    return $script:BLUEBERRY_JSON_OPTIONS
}

function Send-BlueberryTrace {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [string]$Stage,

        [Parameter(Mandatory = $true)]
        [double]$DurationMs
    )

    if (-not $script:BLUEBERRY_TRACE_ENABLED -or
        $script:BLUEBERRY_TRACE_EMITTING -or
        [string]::IsNullOrEmpty([string]$script:BLUEBERRY_TOKEN)) {
        return
    }
    if ($Stage -notin @(
            'adapter_bootstrap',
            'script_source',
            'readline_init',
            'readline_host',
            'readline_module',
            'readline_resolve',
            'readline_history',
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

    $script:BLUEBERRY_TRACE_EMITTING = $true
    try {
        Send-BlueberryEvent -Event 'trace' -Data ([ordered]@{
            stage       = $Stage
            duration_ms = [double]$DurationMs
        })
    } finally {
        $script:BLUEBERRY_TRACE_EMITTING = $false
    }
}

function ConvertTo-BlueberryJson {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [AllowNull()]
        [object]$Payload
    )

    $traceTimer = $null
    if ($script:BLUEBERRY_TRACE_ENABLED) {
        $traceTimer = [Diagnostics.Stopwatch]::StartNew()
    }
    try {
        # The explicit object/type/options overload keeps PowerShell from
        # selecting a generic overload that changes dictionary shape. The
        # cached options keep the first call from repeatedly constructing an
        # encoder and ensure the transport remains ASCII.
        return [string]$script:BLUEBERRY_JsonSerializer::Serialize(
            [object]$Payload,
            [object],
            (Get-BlueberryJsonOptions))
    } finally {
        if ($null -ne $traceTimer) {
            $traceTimer.Stop()
            Send-BlueberryTrace -Stage 'serialize' -DurationMs $traceTimer.Elapsed.TotalMilliseconds
        }
    }
}

function Disable-BlueberryPipe {
    [CmdletBinding()]
    param()

    $stream = $script:BLUEBERRY_PIPE_STREAM
    $script:BLUEBERRY_PIPE_STREAM = $null
    $script:BLUEBERRY_PIPE_BUFFER = $null
    $script:BLUEBERRY_PIPE_ENABLED = $false
    if ($null -ne $stream) {
        try {
            $stream.Dispose()
        } catch {
        }
    }
}

function Initialize-BlueberryPipe {
    [CmdletBinding()]
    param()

    $name = [string]$script:BLUEBERRY_PIPE_NAME
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
        $script:BLUEBERRY_PIPE_STREAM = $stream
        # Reuse one bounded receive buffer for all host requests. The stream
        # is asynchronous because NamedPipeClientStream's synchronous timeout
        # setters are unsupported on this implementation.
        $script:BLUEBERRY_PIPE_BUFFER = [byte[]]::new(1048576)
        $script:BLUEBERRY_PIPE_ENABLED = $true
        return $true
    } catch {
        Disable-BlueberryPipe
        return $false
    } finally {
        # The name is retained only in script state; user commands do not need
        # to inherit this session-private transport locator.
        try {
            [Environment]::SetEnvironmentVariable('BLUEBERRY_PIPE_NAME', $null, 'Process')
        } catch {
        }
    }
}

function Send-BlueberryOscPayload {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [AllowNull()]
        [object]$Payload
    )

    if ([string]::IsNullOrEmpty([string]$script:BLUEBERRY_TOKEN)) {
        return $false
    }
    try {
        $frame = ConvertTo-BlueberryFrame -Token ([string]$script:BLUEBERRY_TOKEN) -Payload $Payload
        # [Console]::Out avoids adding the frame to a PowerShell pipeline and
        # therefore avoids contaminating prompt text or command output.
        [Console]::Out.Write($frame)
        [Console]::Out.Flush()
        return $true
    } catch {
        return $false
    }
}

function Send-BlueberryPipeEvent {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [System.Collections.IDictionary]$Payload
    )

    if (-not $script:BLUEBERRY_PIPE_ENABLED -or $null -eq $script:BLUEBERRY_PIPE_STREAM) {
        return $false
    }
    try {
        $sequence = [int64]$script:BLUEBERRY_PIPE_SEQUENCE + 1
        $script:BLUEBERRY_PIPE_SEQUENCE = $sequence
        $envelope = [ordered]@{
            sequence = $sequence
            payload  = $Payload
        }
        $json = ConvertTo-BlueberryJson -Payload $envelope
        $bytes = [Text.Encoding]::UTF8.GetBytes([string]::Concat($json, "`n"))
        if ($bytes.Length -gt 1048576) {
            return $false
        }
        $stream = $script:BLUEBERRY_PIPE_STREAM
        if (-not $stream.IsConnected) {
            Disable-BlueberryPipe
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
                Disable-BlueberryPipe
                return $false
            }
            $writeTask.GetAwaiter().GetResult()
        } finally {
            $cts.Dispose()
        }

        # The OSC barrier is emitted only after the complete pipe write. The
        # host consumes the matching sequence and then reads the payload, so
        # terminal cursor/output ordering remains identical to OSC mode.
        if (-not (Send-BlueberryOscPayload -Payload ([ordered]@{
                    event    = 'pipe'
                    sequence = $sequence
                }))) {
            Disable-BlueberryPipe
            return $false
        }
        return $true
    } catch {
        Disable-BlueberryPipe
        return $false
    }
}

function Read-BlueberryPipeJson {
    [CmdletBinding()]
    param()

    if (-not $script:BLUEBERRY_PIPE_ENABLED -or $null -eq $script:BLUEBERRY_PIPE_STREAM) {
        return $null
    }
    $stream = $script:BLUEBERRY_PIPE_STREAM
    try {
        if (-not $stream.IsConnected) {
            Disable-BlueberryPipe
            return $null
        }
        $bytes = $script:BLUEBERRY_PIPE_BUFFER
        if ($null -eq $bytes) {
            $bytes = [byte[]]::new(1048576)
            $script:BLUEBERRY_PIPE_BUFFER = $bytes
        }
        $offset = 0
        do {
            $remaining = $bytes.Length - $offset
            if ($remaining -le 0) {
                # A host frame is one message and is capped at the buffer
                # size; accepting more would desynchronise the next key.
                Disable-BlueberryPipe
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
                    Disable-BlueberryPipe
                    return $null
                }
                $read = $readTask.GetAwaiter().GetResult()
            } finally {
                $cts.Dispose()
            }
            if ($read -le 0) {
                Disable-BlueberryPipe
                return $null
            }
            $offset += $read
        } while (-not $stream.IsMessageComplete)

        $utf8Strict = [Text.UTF8Encoding]::new($false, $true)
        $json = $utf8Strict.GetString($bytes, 0, $offset).TrimEnd([char]13, [char]10)
        return $script:BLUEBERRY_JsonSerializer::Deserialize(
            $json,
            [object],
            (Get-BlueberryJsonOptions))
    } catch {
        Disable-BlueberryPipe
        return $null
    }
}

function Read-BlueberryPipeEnvelope {
    [CmdletBinding()]
    param()

    $rootValue = Read-BlueberryPipeJson
    if ($null -eq $rootValue -or $rootValue.ValueKind -ne $script:BLUEBERRY_JsonValueKind::Object) {
        return $null
    }
    $kindElement = $script:BLUEBERRY_JsonElement::new()
    $payloadElement = $script:BLUEBERRY_JsonElement::new()
    if (-not $rootValue.TryGetProperty('kind', [ref]$kindElement) -or
        -not $rootValue.TryGetProperty('payload', [ref]$payloadElement) -or
        $kindElement.ValueKind -ne $script:BLUEBERRY_JsonValueKind::String -or
        $payloadElement.ValueKind -ne $script:BLUEBERRY_JsonValueKind::Object) {
        Disable-BlueberryPipe
        return $null
    }
    return [pscustomobject]@{
        kind    = $kindElement.GetString()
        payload = $payloadElement.GetRawText()
    }
}

function ConvertTo-BlueberryFrame {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [string]$Token,

        [Parameter(Mandatory = $true)]
        [AllowNull()]
        [object]$Payload
    )

    if ([string]::IsNullOrEmpty($Token)) {
        throw 'Blueberry protocol token is empty.'
    }

    $json = ConvertTo-BlueberryJson -Payload $Payload
    $escape = [char]27
    $bell = [char]7
    return [string]::Concat($escape, ']7776;', $Token, ';', $json, $bell)
}

function Send-BlueberryEvent {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [ValidateNotNullOrEmpty()]
        [string]$Event,

        [AllowNull()]
        [System.Collections.IDictionary]$Data
    )

    if ([string]::IsNullOrEmpty([string]$script:BLUEBERRY_TOKEN)) {
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
    if (Send-BlueberryPipeEvent -Payload $payload) {
        return
    }
    [void](Send-BlueberryOscPayload -Payload $payload)
}

function Get-BlueberryWorkingDirectory {
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
    if ($null -ne $script:BLUEBERRY_FALLBACK_CWD) {
        $fallbacks += [string]$script:BLUEBERRY_FALLBACK_CWD
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

function Get-BlueberryEnvironmentSnapshot {
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
        # PATHEXT are still sent by Get-BlueberryPromptEndData.
    }
    return $snapshot
}

function Test-BlueberryEnvironmentChanged {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [System.Collections.IDictionary]$Snapshot
    )

    $previous = $script:BLUEBERRY_ENVIRONMENT_SNAPSHOT
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

function Get-BlueberryPromptEndData {
    [CmdletBinding()]
    param()

    $pathValue = [string]$env:PATH
    $pathExtValue = [string]$env:PATHEXT
    $environmentSnapshot = Get-BlueberryEnvironmentSnapshot
    $environmentChanged = Test-BlueberryEnvironmentChanged -Snapshot $environmentSnapshot
    $data = [ordered]@{
        cwd    = Get-BlueberryWorkingDirectory
        path   = $pathValue
        pathext = $pathExtValue
        pid    = [int]$PID
    }
    if ($environmentChanged) {
        # This is a transient protocol snapshot. It is never included in
        # trace events, persisted caches, diagnostics, or command history.
        $data.environment = $environmentSnapshot
        $script:BLUEBERRY_ENVIRONMENT_SNAPSHOT = $environmentSnapshot
    }
    return $data
}

function Test-BlueberryComplexLine {
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

function ConvertTo-BlueberryDecodedPrefix {
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

function Convert-BlueberrySafeAstValue {
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
        return Convert-BlueberrySafeAstValue -Ast $Ast.Expression -Value ([ref]$Value.Value)
    }
    if ($Ast -is [System.Management.Automation.Language.CommandParameterAst]) {
        if ($null -ne $Ast.Argument) {
            $argumentValue = $null
            if (-not (Convert-BlueberrySafeAstValue -Ast $Ast.Argument -Value ([ref]$argumentValue))) {
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

function Convert-BlueberryAstElementValue {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [System.Management.Automation.Language.Ast]$Ast,

        [Parameter(Mandatory = $true)]
        [ref]$Value
    )

    return Convert-BlueberrySafeAstValue -Ast $Ast -Value $Value
}

function Get-BlueberryAstBufferContext {
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

    if (-not (Test-BlueberryComplexLine -Line $Line)) {
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
        -not (Test-BlueberryUtf16Range -Line $Line -Start $Cursor -Length 0)) {
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
            $context.prefix = ConvertTo-BlueberryDecodedPrefix -Raw $environmentMatch.Value
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
    if (-not (Test-BlueberryUtf16Range -Line $Line -Start $currentStart -Length ($currentEnd - $currentStart))) {
        $context.suppressed = $true
        return $context
    }

    $context.prefix = ConvertTo-BlueberryDecodedPrefix -Raw $Line.Substring($currentStart, $Cursor - $currentStart)
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
    if (-not (Convert-BlueberryAstElementValue -Ast $elements[0] -Value ([ref]$commandValue))) {
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
        if (-not (Convert-BlueberryAstElementValue -Ast $elements[$index] -Value ([ref]$argumentValue))) {
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
        if (-not (Convert-BlueberryAstElementValue -Ast $elements[$currentIndex] -Value ([ref]$currentValue))) {
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

function Send-BlueberryBuffer {
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
        if (Test-BlueberryComplexLine -Line ([string]$line)) {
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
            $context = Get-BlueberryAstBufferContext `
                -Line ([string]$line) -Cursor ([int]$cursor) `
                -Ast $ast -Tokens $tokens -ParseErrors $parseErrors
            if ($null -ne $context) {
                $data.context = $context
            }
        }
        Send-BlueberryEvent -Event 'buffer' -Data $data
    } catch {
        Send-BlueberryEvent -Event 'error' -Data ([ordered]@{
            code    = 'buffer_unavailable'
            message = 'PSReadLine could not provide the current buffer.'
        })
    }
}

function Test-BlueberryUtf16Range {
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

function Test-BlueberryNonNegativeInteger {
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

function Test-BlueberryEditPayload {
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
    if (-not (Test-BlueberryNonNegativeInteger -Value $Payload.expectedCursor -Result ([ref]$expectedCursor))) {
        return $false
    }
    if (-not (Test-BlueberryNonNegativeInteger -Value $Payload.start -Result ([ref]$start))) {
        return $false
    }
    if (-not (Test-BlueberryNonNegativeInteger -Value $Payload.length -Result ([ref]$length))) {
        return $false
    }
    if ($expectedCursor -ne [int64]$CurrentCursor) {
        return $false
    }
    if (-not (Test-BlueberryUtf16Range -Line $CurrentLine -Start ([int64]$CurrentCursor) -Length 0)) {
        return $false
    }
    if (-not (Test-BlueberryUtf16Range -Line $CurrentLine -Start $start -Length $length)) {
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

function Get-BlueberryConsumedEditPath {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $directory = [IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($Path))
    $leaf = [IO.Path]::GetFileName($Path)
    if ([string]::IsNullOrEmpty($leaf)) {
        $leaf = 'blueberry-edit.json'
    }
    return [IO.Path]::Combine($directory, ('.' + $leaf + '.consumed.' + [Guid]::NewGuid().ToString('N')))
}

function Convert-BlueberryJsonElementValue {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [object]$Element
    )

    switch ($Element.ValueKind) {
        ($script:BLUEBERRY_JsonValueKind::String) {
            return $Element.GetString()
        }
        ($script:BLUEBERRY_JsonValueKind::Number) {
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
        ($script:BLUEBERRY_JsonValueKind::True) {
            return $true
        }
        ($script:BLUEBERRY_JsonValueKind::False) {
            return $false
        }
        ($script:BLUEBERRY_JsonValueKind::Null) {
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

function Test-BlueberryJsonFields {
    param([object]$Root, [string[]]$Allowed)
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($property in $Root.EnumerateObject()) {
        if ($property.Name -cnotin $Allowed -or -not $seen.Add([string]$property.Name)) { return $false }
    }
    return $true
}

function ConvertFrom-BlueberryEditJson {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$Json
    )

    # Deserialize to JsonElement first so a valid JSON array/null can be
    # rejected by the normal edit-payload validator, while malformed JSON
    # still follows Invoke-BlueberryApplyEdit's existing failure path.
    $rootValue = $script:BLUEBERRY_JsonSerializer::Deserialize(
        $Json,
        [object],
        (Get-BlueberryJsonOptions))
    if ($null -eq $rootValue -or $rootValue.ValueKind -ne $script:BLUEBERRY_JsonValueKind::Object) {
        return $null
    }
    if (-not (Test-BlueberryJsonFields $rootValue @('id','expectedLine','expectedCursor','start','length','text'))) { return $null }


    $payload = [ordered]@{}
    foreach ($propertyName in @('id', 'expectedLine', 'expectedCursor', 'start', 'length', 'text')) {
        $property = $script:BLUEBERRY_JsonElement::new()
        if ($rootValue.TryGetProperty($propertyName, [ref]$property)) {
            $payload[$propertyName] = Convert-BlueberryJsonElementValue -Element $property
        }
    }
    return [pscustomobject]$payload
}

function Get-BlueberryRequestPath {
    [CmdletBinding()]
    param()

    if (-not [string]::IsNullOrEmpty([string]$script:BLUEBERRY_REQUEST_PATH)) {
        return [string]$script:BLUEBERRY_REQUEST_PATH
    }
    if ([string]::IsNullOrEmpty([string]$script:BLUEBERRY_EDIT_PATH)) {
        return $null
    }
    try {
        $directory = [IO.Path]::GetDirectoryName([IO.Path]::GetFullPath([string]$script:BLUEBERRY_EDIT_PATH))
        return [IO.Path]::Combine($directory, 'request.json')
    } catch {
        return $null
    }
}

function ConvertFrom-BlueberryRequestJson {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$Json
    )

    $rootValue = $script:BLUEBERRY_JsonSerializer::Deserialize(
        $Json,
        [object],
        (Get-BlueberryJsonOptions))
    if ($null -eq $rootValue -or $rootValue.ValueKind -ne $script:BLUEBERRY_JsonValueKind::Object) {
        return $null
    }
    if (-not (Test-BlueberryJsonFields $rootValue @('id','kind','public_keys_json','public_keys','command','limit'))) { return $null }


    $idElement = $script:BLUEBERRY_JsonElement::new()
    $kindElement = $script:BLUEBERRY_JsonElement::new()
    if (-not $rootValue.TryGetProperty('id', [ref]$idElement) -or
        -not $rootValue.TryGetProperty('kind', [ref]$kindElement) -or
        ($idElement.ValueKind -ne $script:BLUEBERRY_JsonValueKind::String -and
            $idElement.ValueKind -ne $script:BLUEBERRY_JsonValueKind::Number) -or
        $kindElement.ValueKind -ne $script:BLUEBERRY_JsonValueKind::String) {
        return $null
    }
    $id = $null
    if ($idElement.ValueKind -eq $script:BLUEBERRY_JsonValueKind::String) {
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
    $publicKeysElement = $script:BLUEBERRY_JsonElement::new()
    if ($rootValue.TryGetProperty('public_keys_json', [ref]$publicKeysElement)) {
        if ($publicKeysElement.ValueKind -eq $script:BLUEBERRY_JsonValueKind::String) {
            $publicKeysJson = $publicKeysElement.GetString()
        } else {
            return $null
        }
    }
    $publicKeysElement = $script:BLUEBERRY_JsonElement::new()
    if ($rootValue.TryGetProperty('public_keys', [ref]$publicKeysElement)) {
        if ($publicKeysElement.ValueKind -eq $script:BLUEBERRY_JsonValueKind::Object) {
            $publicKeysJson = $publicKeysElement.GetRawText()
        } elseif ($publicKeysElement.ValueKind -eq $script:BLUEBERRY_JsonValueKind::String) {
            $publicKeysJson = $publicKeysElement.GetString()
        } else {
            return $null
        }
    }
    $commandName = ''
    $commandElement = $script:BLUEBERRY_JsonElement::new()
    if ($rootValue.TryGetProperty('command', [ref]$commandElement) -and $commandElement.ValueKind -eq $script:BLUEBERRY_JsonValueKind::String) { $commandName = $commandElement.GetString() }
    $limit = 2000
    $limitElement = $script:BLUEBERRY_JsonElement::new()
    if ($rootValue.TryGetProperty('limit', [ref]$limitElement)) {
        $parsedLimit = [int64]0
        if (-not $limitElement.TryGetInt64([ref]$parsedLimit) -or $parsedLimit -lt 1 -or $parsedLimit -gt 20000) { return $null }
        $limit = [int]$parsedLimit
    }
    return [pscustomobject]@{
        command          = $commandName
        id               = [string]$id
        kind             = [string]$kind
        public_keys_json = $publicKeysJson
        limit            = $limit
    }
}

function Read-BlueberryRequest {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [ValidateSet('native', 'command_metadata', 'history', 'commands_reset', 'commands_next')]
        [string[]]$ExpectedKind
    )

    # In pipe mode the host publishes the payload before sending the known
    # internal chord. This is the only place a request is read; no background
    # reader or speculative timeout is introduced into PSReadLine.
    $pipeEnvelope = Read-BlueberryPipeEnvelope
    if ($null -ne $pipeEnvelope) {
        if (-not [string]::Equals([string]$pipeEnvelope.kind, 'request', [StringComparison]::Ordinal)) {
            Send-BlueberryEvent -Event 'error' -Data ([ordered]@{
                code    = 'request_kind_mismatch'
                message = 'The named-pipe request envelope is not a request.'
            })
            return $null
        }
        $request = ConvertFrom-BlueberryRequestJson -Json ([string]$pipeEnvelope.payload)
        if ($null -eq $request) {
            Send-BlueberryEvent -Event 'error' -Data ([ordered]@{
                code    = 'request_rejected'
                message = 'The named-pipe request is not a valid v2 request.'
            })
            return $null
        }
        if ($ExpectedKind -notcontains [string]$request.kind) {
            Send-BlueberryEvent -Event 'error' -Data ([ordered]@{
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
    if ($script:BLUEBERRY_PIPE_ENABLED) {
        return $null
    }

    $path = Get-BlueberryRequestPath
    if ([string]::IsNullOrEmpty([string]$path) -or -not [IO.File]::Exists($path)) {
        return $null
    }

    $consumedPath = $null
    try {
        # Rename first so a writer can publish the next request while this
        # request is being processed. A consumed name also prevents replay.
        $consumedPath = Get-BlueberryConsumedEditPath -Path $path
        [IO.File]::Move($path, $consumedPath)
        $utf8Strict = [Text.UTF8Encoding]::new($false, $true)
        $json = [IO.File]::ReadAllText($consumedPath, $utf8Strict)
        $request = ConvertFrom-BlueberryRequestJson -Json $json
        if ($null -eq $request) {
            Send-BlueberryEvent -Event 'error' -Data ([ordered]@{
                code    = 'request_rejected'
                message = 'The Blueberry request is not a valid v2 request.'
            })
            return $null
        }
        if ($ExpectedKind -notcontains [string]$request.kind) {
            Send-BlueberryEvent -Event 'error' -Data ([ordered]@{
                code        = 'request_kind_mismatch'
                request_id  = [string]$request.id
                message     = ('Expected a {0} request.' -f ($ExpectedKind -join ' or '))
            })
            return $null
        }
        return $request
    } catch {
        Send-BlueberryEvent -Event 'error' -Data ([ordered]@{
            code    = 'request_failed'
            message = 'The Blueberry request could not be read.'
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

function Send-BlueberryEditResult {
    [CmdletBinding()]
    param(
        [AllowNull()]
        [string]$RequestId,

        [Parameter(Mandatory = $true)]
        [bool]$Applied
    )

    Send-BlueberryEvent -Event 'edit_result' -Data ([ordered]@{
        request_id = $RequestId
        applied    = [bool]$Applied
    })
}

function Invoke-BlueberryApplyEdit {
    [CmdletBinding()]
    param()

    $path = [string]$script:BLUEBERRY_EDIT_PATH
    $consumedPath = $null
    $payload = $null
    $replaceApplied = $false
    $replaceError = $null
    try {
        # Pipe mode publishes the edit payload before the known apply chord.
        # Read it first so no stale file can be selected while the pipe is
        # connected. A failed/disposed pipe falls through to the legacy file.
        $pipeEnvelope = Read-BlueberryPipeEnvelope
        if ($null -ne $pipeEnvelope) {
            if (-not [string]::Equals([string]$pipeEnvelope.kind, 'edit', [StringComparison]::Ordinal)) {
                Send-BlueberryEvent -Event 'error' -Data ([ordered]@{
                    code    = 'edit_kind_mismatch'
                    message = 'The named-pipe envelope is not an edit payload.'
                })
                return
            }
            $payload = ConvertFrom-BlueberryEditJson -Json ([string]$pipeEnvelope.payload)
        } elseif ($script:BLUEBERRY_PIPE_ENABLED) {
            # The host sends the pipe frame before its internal key. Do not
            # guess by reading a file if a connected pipe has no frame yet.
            return
        }

        if ($null -eq $payload) {
            if ([string]::IsNullOrEmpty($path)) {
                Send-BlueberryEvent -Event 'error' -Data ([ordered]@{
                    code    = 'edit_path_unavailable'
                    message = 'BLUEBERRY_EDIT_PATH is not configured.'
                })
                return
            }
            if (-not [IO.File]::Exists($path)) {
                Send-BlueberryEvent -Event 'error' -Data ([ordered]@{
                    code    = 'edit_payload_missing'
                    message = 'No pending edit payload is available.'
                })
                return
            }

            # Rename first so a payload written while this handler is running
            # is left for the next invocation. The consumed name prevents
            # replay.
            $consumedPath = Get-BlueberryConsumedEditPath -Path $path
            [IO.File]::Move($path, $consumedPath)

            $utf8Strict = [Text.UTF8Encoding]::new($false, $true)
            $json = [IO.File]::ReadAllText($consumedPath, $utf8Strict)
            $payload = ConvertFrom-BlueberryEditJson -Json $json
        }

        $line = $null
        $cursor = 0
        [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line, [ref]$cursor)
        $edit = $null
        if (-not (Test-BlueberryEditPayload -Payload $payload -CurrentLine ([string]$line) -CurrentCursor ([int]$cursor) -Edit ([ref]$edit))) {
            Send-BlueberryEvent -Event 'error' -Data ([ordered]@{
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
        $resultLine = [string]$line
        $resultCursor = [int]$cursor
        if ($replaceApplied) {
            # Replace deterministically leaves the cursor after the inserted
            # UTF-16 text. Report that known state without calling
            # GetBufferState again from inside PSReadLine's key handler; that
            # nested query can stall on Windows PowerShell runners.
            $resultLine = ([string]$line).Substring(0, [int]$edit.start) + `
                [string]$edit.text + `
                ([string]$line).Substring([int]$edit.start + [int]$edit.length)
            $resultCursor = [int]$edit.start + ([string]$edit.text).Length
        }
        Send-BlueberryEditResult -RequestId $editRequestId -Applied $replaceApplied
        Send-BlueberryEvent -Event 'buffer' -Data ([ordered]@{
            line   = $resultLine
            cursor = $resultCursor
        })
        if ($null -ne $replaceError) {
            Send-BlueberryEvent -Event 'error' -Data ([ordered]@{
                code        = 'edit_failed'
                request_id  = $editRequestId
                message     = 'The pending edit could not be applied.'
            })
        }
    } catch {
        Send-BlueberryEvent -Event 'error' -Data ([ordered]@{
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

function Get-BlueberryPasteDirectory {
    [CmdletBinding()]
    param()

    if ([string]::IsNullOrEmpty([string]$script:BLUEBERRY_EDIT_PATH)) {
        return $null
    }
    try {
        $editPath = [IO.Path]::GetFullPath([string]$script:BLUEBERRY_EDIT_PATH)
        $directory = [IO.Path]::GetDirectoryName($editPath)
        if ([string]::IsNullOrEmpty([string]$directory)) {
            return $null
        }
        return [IO.Path]::Combine($directory, 'paste')
    } catch {
        return $null
    }
}

function ConvertTo-BlueberryPasteText {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$Text
    )

    # PSReadLine's public Insert API stores continuation lines as LF and only
    # converts CRLF while processing the insertion. Replace stores its text
    # directly, so normalize both paths to the same LF representation first.
    return $Text.Replace("`r`n", "`n").Replace("`r", "`n")
}

function Test-BlueberryPastePayload {
    [CmdletBinding()]
    param(
        [AllowNull()]
        [object]$Payload,

        [Parameter(Mandatory = $true)]
        [ref]$Paste
    )

    $Paste.Value = $null
    if ($null -eq $Payload) {
        return $false
    }

    try {
        $properties = @($Payload.PSObject.Properties)
        if ($properties.Count -ne 2) {
            return $false
        }
        $idProperty = $Payload.PSObject.Properties['id']
        $textProperty = $Payload.PSObject.Properties['text']
        if ($null -eq $idProperty -or $null -eq $textProperty -or
            $idProperty.Value -isnot [string] -or
            $textProperty.Value -isnot [string] -or
            [string]::IsNullOrWhiteSpace([string]$idProperty.Value)) {
            return $false
        }

        $text = ConvertTo-BlueberryPasteText -Text ([string]$textProperty.Value)
        $utf8 = [Text.UTF8Encoding]::new($false, $true)
        if ($utf8.GetByteCount($text) -gt 1048576) {
            return $false
        }

        $Paste.Value = [pscustomobject]@{
            id   = [string]$idProperty.Value
            text = $text
        }
        return $true
    } catch {
        $Paste.Value = $null
        return $false
    }
}

function ConvertFrom-BlueberryPasteJson {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$Json
    )

    try {
        $rootValue = $script:BLUEBERRY_JsonSerializer::Deserialize(
            $Json,
            [object],
            (Get-BlueberryJsonOptions))
        if ($null -eq $rootValue -or
            $rootValue.ValueKind -ne $script:BLUEBERRY_JsonValueKind::Object) {
            return $null
        }

        # The host publishes exactly {id:string,text:string}. Reject unknown
        # and duplicate properties instead of letting JsonElement's last
        # property win silently.
        $seen = [System.Collections.Generic.HashSet[string]]::new(
            [StringComparer]::Ordinal)
        foreach ($property in $rootValue.EnumerateObject()) {
            if ($property.Name -cnotin @('id', 'text') -or
                -not $seen.Add([string]$property.Name)) {
                return $null
            }
        }
        if ($seen.Count -ne 2) {
            return $null
        }

        $idElement = $script:BLUEBERRY_JsonElement::new()
        $textElement = $script:BLUEBERRY_JsonElement::new()
        if (-not $rootValue.TryGetProperty('id', [ref]$idElement) -or
            -not $rootValue.TryGetProperty('text', [ref]$textElement) -or
            $idElement.ValueKind -ne $script:BLUEBERRY_JsonValueKind::String -or
            $textElement.ValueKind -ne $script:BLUEBERRY_JsonValueKind::String) {
            return $null
        }

        $id = $idElement.GetString()
        $text = $textElement.GetString()
        if ([string]::IsNullOrWhiteSpace([string]$id) -or $null -eq $text) {
            return $null
        }

        $utf8 = [Text.UTF8Encoding]::new($false, $true)
        if ($utf8.GetByteCount($text) -gt 1048576) {
            return $null
        }
        return [pscustomobject]@{
            id   = [string]$id
            text = [string]$text
        }
    } catch {
        return $null
    }
}

function Get-BlueberryPasteFiles {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [string]$Directory
    )

    try {
        if (-not [IO.Directory]::Exists($Directory)) {
            return @()
        }
        $directoryAttributes = [IO.File]::GetAttributes($Directory)
        if (($directoryAttributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            return @()
        }

        $files = [System.Collections.Generic.List[IO.FileInfo]]::new()
        $directoryInfo = [IO.DirectoryInfo]::new($Directory)
        foreach ($file in $directoryInfo.EnumerateFiles()) {
            # Host payloads are committed under a zero-padded 20-digit name;
            # temporary files, arbitrary JSON, and links are never consumed.
            if ($file.Name -cnotmatch '^[0-9]{20}\.json$') {
                continue
            }
            try {
                if (($file.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                    continue
                }
            } catch {
                continue
            }
            [void]$files.Add($file)
        }

        # Numeric-only fixed-width names have the same ordering under every
        # culture, but compare explicitly with Ordinal to keep FIFO semantics
        # independent of the user's PowerShell culture.
        $files.Sort([System.Comparison[IO.FileInfo]]{
                param($left, $right)
                return [StringComparer]::Ordinal.Compare($left.Name, $right.Name)
            })
        return $files.ToArray()
    } catch {
        return @()
    }
}

function Send-BlueberryPasteError {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [ValidateSet('paste_payload_rejected', 'paste_failed')]
        [string]$Code,

        [Parameter(Mandatory = $true)]
        [string]$Message
    )

    # Keep paste text and raw payloads out of diagnostics. The host only needs
    # a stable code/message pair to explain why safe insertion was unavailable.
    Send-BlueberryEvent -Event 'error' -Data ([ordered]@{
        code    = $Code
        message = $Message
    })
}

function Send-BlueberryPasteResult {
    [CmdletBinding()]
    param(
        [AllowNull()]
        [string]$RequestId,

        [Parameter(Mandatory = $true)]
        [bool]$Applied
    )

    Send-BlueberryEvent -Event 'paste_result' -Data ([ordered]@{
        request_id = $RequestId
        applied    = [bool]$Applied
    })
}

function Invoke-BlueberryPasteKeyHandler {
    [CmdletBinding()]
    param()

    $consumedPath = $null
    try {
        $directory = Get-BlueberryPasteDirectory
        if ([string]::IsNullOrEmpty([string]$directory)) {
            Send-BlueberryPasteError -Code 'paste_failed' `
                -Message 'The Blueberry paste directory is unavailable.'
            return
        }

        $candidateFiles = @(Get-BlueberryPasteFiles -Directory $directory)
        if ($candidateFiles.Count -eq 0) {
            Send-BlueberryPasteError -Code 'paste_failed' `
                -Message 'No pending Blueberry paste payload is available.'
            return
        }

        # Rename before opening the file. A host paste arriving during this
        # handler remains queued under its own sequence number and cannot be
        # replayed by a second invocation.
        foreach ($candidate in $candidateFiles) {
            $candidateConsumedPath = $null
            try {
                $candidateConsumedPath = Get-BlueberryConsumedEditPath -Path $candidate.FullName
                [IO.File]::Move($candidate.FullName, $candidateConsumedPath)
                $consumedPath = $candidateConsumedPath
                break
            } catch {
                if ($null -ne $candidateConsumedPath -and [IO.File]::Exists($candidateConsumedPath)) {
                    try {
                        [IO.File]::Delete($candidateConsumedPath)
                    } catch {
                    }
                }
            }
        }
        if ($null -eq $consumedPath) {
            Send-BlueberryPasteError -Code 'paste_failed' `
                -Message 'The pending Blueberry paste payload could not be consumed.'
            return
        }

        $consumedAttributes = [IO.File]::GetAttributes($consumedPath)
        if (($consumedAttributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            Send-BlueberryPasteError -Code 'paste_payload_rejected' `
                -Message 'The Blueberry paste payload is not a regular file.'
            return
        }
        $consumedInfo = [IO.FileInfo]::new($consumedPath)
        if ($consumedInfo.Length -gt 1048576) {
            Send-BlueberryPasteError -Code 'paste_payload_rejected' `
                -Message 'The Blueberry paste payload exceeds the 1 MiB limit.'
            return
        }
        $utf8Strict = [Text.UTF8Encoding]::new($false, $true)
        try {
            $json = [IO.File]::ReadAllText($consumedPath, $utf8Strict)
        } catch [Text.DecoderFallbackException] {
            Send-BlueberryPasteError -Code 'paste_payload_rejected' `
                -Message 'The Blueberry paste payload is not valid UTF-8.'
            return
        } catch {
            Send-BlueberryPasteError -Code 'paste_failed' `
                -Message 'The Blueberry paste payload could not be read.'
            return
        }
        $payload = ConvertFrom-BlueberryPasteJson -Json $json
        $paste = $null
        if ($null -eq $payload -or
            -not (Test-BlueberryPastePayload -Payload $payload -Paste ([ref]$paste))) {
            Send-BlueberryPasteError -Code 'paste_payload_rejected' `
                -Message 'The Blueberry paste payload is invalid.'
            return
        }

        $requestId = [string]$paste.id
        $applied = $false
        $applyError = $null
        try {
            $selectionStart = 0
            $selectionLength = 0
            [Microsoft.PowerShell.PSConsoleReadLine]::GetSelectionState(
                [ref]$selectionStart,
                [ref]$selectionLength)
            if ($selectionLength -gt 0) {
                [Microsoft.PowerShell.PSConsoleReadLine]::Replace(
                    [int]$selectionStart,
                    [int]$selectionLength,
                    [string]$paste.text)
            } else {
                [Microsoft.PowerShell.PSConsoleReadLine]::Insert([string]$paste.text)
            }
            $applied = $true
        } catch {
            $applyError = $_
        }
        Send-BlueberryPasteResult -RequestId $requestId -Applied $applied
        Send-BlueberryBuffer
        if ($null -ne $applyError) {
            Send-BlueberryPasteError -Code 'paste_failed' `
                -Message 'The Blueberry paste payload could not be inserted.'
        }
    } catch {
        Send-BlueberryPasteError -Code 'paste_failed' `
            -Message 'The Blueberry paste operation failed.'
    } finally {
        if ($null -ne $consumedPath) {
            try {
                [IO.File]::Delete($consumedPath)
            } catch {
            }
        }
    }
}

function Get-BlueberryLoadedCommands {
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

function Get-BlueberryEnumerator {
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

function Get-BlueberryCommandEnumerator {
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
        return Get-BlueberryEnumerator -Sequence $provided
    }

    $commandTypes = [System.Management.Automation.CommandTypes]::Alias -bor
        [System.Management.Automation.CommandTypes]::Function -bor
        [System.Management.Automation.CommandTypes]::Cmdlet
    $sequence = $ExecutionContext.InvokeCommand.GetCommands('*', $commandTypes, $true)
    return Get-BlueberryEnumerator -Sequence $sequence
}

function ConvertTo-BlueberryCommandRecord {
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

function Reset-BlueberryCommandSnapshot {
    [CmdletBinding()]
    param()

    $enumerator = $script:BLUEBERRY_COMMAND_ENUMERATOR
    if ($null -ne $enumerator -and $enumerator -is [IDisposable]) {
        try {
            $enumerator.Dispose()
        } catch {
        }
    }
    $script:BLUEBERRY_COMMAND_ENUMERATOR = $null
    $script:BLUEBERRY_COMMAND_SNAPSHOT_ID = $null
    $script:BLUEBERRY_COMMAND_SNAPSHOT = $null
    $script:BLUEBERRY_COMMAND_SNAPSHOT_OFFSET = 0
    $script:BLUEBERRY_COMMAND_SNAPSHOT_FAILED = $false
    $script:BLUEBERRY_COMMAND_SNAPSHOT_COMPLETE = $false
    $script:BLUEBERRY_COMMAND_REQUEST_ID = $null
}

function Start-BlueberryCommandSnapshot {
    [CmdletBinding()]
    param(
        [AllowNull()]
        [scriptblock]$CommandProvider,

        [AllowNull()]
        [string]$RequestId
    )

    $enumerator = Get-BlueberryCommandEnumerator -CommandProvider $CommandProvider
    $snapshotId = [Guid]::NewGuid().ToString()
    $script:BLUEBERRY_COMMAND_ENUMERATOR = $enumerator
    # Keep this legacy state name as a reference to the enumerator rather than
    # an array, so callers can tell that command discovery is truly lazy.
    $script:BLUEBERRY_COMMAND_SNAPSHOT = $enumerator
    $script:BLUEBERRY_COMMAND_SNAPSHOT_ID = $snapshotId
    $script:BLUEBERRY_COMMAND_SNAPSHOT_OFFSET = 0
    $script:BLUEBERRY_COMMAND_SNAPSHOT_FAILED = $false
    $script:BLUEBERRY_COMMAND_SNAPSHOT_COMPLETE = $false
    $script:BLUEBERRY_COMMAND_REQUEST_ID = $RequestId
}

function Get-BlueberryImportedCommands {
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
    if ($script:BLUEBERRY_TRACE_ENABLED) {
        $traceTimer = [Diagnostics.Stopwatch]::StartNew()
    }
    try {
        if ($Reset) {
            Reset-BlueberryCommandSnapshot
        }
        $snapshotId = [string]$script:BLUEBERRY_COMMAND_SNAPSHOT_ID
        if ([string]::IsNullOrEmpty($snapshotId) -or $null -eq $script:BLUEBERRY_COMMAND_ENUMERATOR) {
            Start-BlueberryCommandSnapshot -CommandProvider $CommandProvider -RequestId $RequestId
            $snapshotId = [string]$script:BLUEBERRY_COMMAND_SNAPSHOT_ID
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
                if (-not $script:BLUEBERRY_COMMAND_ENUMERATOR.MoveNext()) {
                    $complete = $true
                    break
                }
                $inspected++
                $record = ConvertTo-BlueberryCommandRecord -Command $script:BLUEBERRY_COMMAND_ENUMERATOR.Current
                if ($null -ne $record) {
                    [void]$batch.Add($record)
                }
                $script:BLUEBERRY_COMMAND_SNAPSHOT_OFFSET++
            } catch {
                $failed = $true
                break
            }
        }

        $script:BLUEBERRY_COMMAND_SNAPSHOT_COMPLETE = [bool]$complete
        $script:BLUEBERRY_COMMAND_SNAPSHOT_FAILED = [bool]$failed
        $eventData = [ordered]@{
            snapshot = $snapshotId
            # A failed/partial enumerator can never publish a complete marker.
            complete = [bool]($complete -and -not $failed)
            commands = @($batch.ToArray())
        }
        if (-not [string]::IsNullOrEmpty([string]$script:BLUEBERRY_COMMAND_REQUEST_ID)) {
            $eventData.request_id = [string]$script:BLUEBERRY_COMMAND_REQUEST_ID
            $script:BLUEBERRY_COMMAND_REQUEST_ID = $null
        }
        Send-BlueberryEvent -Event 'commands' -Data $eventData

        if ($failed) {
            Send-BlueberryEvent -Event 'error' -Data ([ordered]@{
                code    = 'commands_unavailable'
                message = 'Loaded command enumeration failed before the snapshot completed.'
            })
            Reset-BlueberryCommandSnapshot
        } elseif ($complete) {
            Reset-BlueberryCommandSnapshot
        }
    } catch {
        # Do not let an exception turn the last partial page into a complete
        # snapshot. Preserve a diagnostic and reset before a later retry.
        $snapshotId = [string]$script:BLUEBERRY_COMMAND_SNAPSHOT_ID
        if (-not [string]::IsNullOrEmpty($snapshotId)) {
            Send-BlueberryEvent -Event 'commands' -Data ([ordered]@{
                snapshot = $snapshotId
                complete = $false
                commands = @()
            })
        }
        Send-BlueberryEvent -Event 'error' -Data ([ordered]@{
            code    = 'commands_unavailable'
            message = 'Loaded command enumeration failed before the snapshot completed.'
        })
        Reset-BlueberryCommandSnapshot
    } finally {
        if ($null -ne $traceTimer) {
            $traceTimer.Stop()
            Send-BlueberryTrace -Stage 'command_snapshot' -DurationMs $traceTimer.Elapsed.TotalMilliseconds
        }
    }
}

function ConvertTo-BlueberryNativeCandidateKind {
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
            # other UI-only values that are not part of the stable Blueberry
            # CandidateKind enum. Keep them valid and show the original type
            # in the description/tooltip rather than breaking the response.
            return 'value'
        }
    }
}

function Get-BlueberryNativeCompletion {
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
                    if (-not (Test-BlueberryUtf16Range -Line $line -Start $replaceStart -Length ($replaceEnd - $replaceStart))) {
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
                                kind        = ConvertTo-BlueberryNativeCandidateKind -ResultType ([string]$match.ResultType)
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
        if (Test-BlueberryComplexLine -Line $line) {
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
            $context = Get-BlueberryAstBufferContext `
                -Line $line -Cursor $cursor -Ast $ast -Tokens $tokens -ParseErrors $parseErrors
        }
    } finally {
        # Native completers may run external commands. Their exit code is not
        # part of Blueberry's edit protocol and must not alter Prompt status.
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
    Send-BlueberryEvent -Event 'native_completion' -Data $data
}

function Get-BlueberryCurrentBufferState {
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

function Test-BlueberryConfirmedContinuation {
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

function Invoke-BlueberryEnterKeyHandler {
    [CmdletBinding()]
    param()

    $previous = $null
    try {
        $previous = Get-BlueberryCurrentBufferState
        $savedLastExitCode = $ExecutionContext.SessionState.PSVariable.GetValue('global:LASTEXITCODE')
        try {
            [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine($null, $null)
        } finally {
            $global:LASTEXITCODE = $savedLastExitCode
        }
        $current = Get-BlueberryCurrentBufferState
        if (Test-BlueberryConfirmedContinuation -Line $current.line -PreviousLine $previous.line) {
            Send-BlueberryEvent -Event 'editing' -Data ([ordered]@{ state = 'continuation' })
        }
    } catch {
        Send-BlueberryEvent -Event 'error' -Data ([ordered]@{
            code    = 'enter_handler_failed'
            message = 'The known PSReadLine Enter handler could not be invoked.'
        })
    }
}

function Invoke-BlueberryShiftEnterKeyHandler {
    [CmdletBinding()]
    param()

    $previous = $null
    try {
        $previous = Get-BlueberryCurrentBufferState
        $savedLastExitCode = $ExecutionContext.SessionState.PSVariable.GetValue('global:LASTEXITCODE')
        try {
            [Microsoft.PowerShell.PSConsoleReadLine]::AddLine($null, $null)
        } finally {
            $global:LASTEXITCODE = $savedLastExitCode
        }
        $current = Get-BlueberryCurrentBufferState
        if (Test-BlueberryConfirmedContinuation -Line $current.line -PreviousLine $previous.line -AddLine) {
            Send-BlueberryEvent -Event 'editing' -Data ([ordered]@{ state = 'continuation' })
        }
    } catch {
        Send-BlueberryEvent -Event 'error' -Data ([ordered]@{
            code    = 'shift_enter_handler_failed'
            message = 'The known PSReadLine Shift+Enter handler could not be invoked.'
        })
    }
}

function Invoke-BlueberryBufferKeyHandler {
    [CmdletBinding()]
    param()
    Send-BlueberryBuffer
}

function Invoke-BlueberryApplyKeyHandler {
    [CmdletBinding()]
    param()
    Invoke-BlueberryApplyEdit
}

function Invoke-BlueberryCommandsKeyHandler {
    [CmdletBinding()]
    param()
    $request = Read-BlueberryRequest -ExpectedKind @('commands_reset', 'commands_next')
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
                Update-BlueberryPublicKeyCapabilities -JsonOverride ([string]$publicKeysProperty.Value)
                Send-BlueberryCapabilities
            }
            Get-BlueberryImportedCommands -Reset -RequestId ([string]$request.id)
        } else {
            # In pipe mode every continuation batch has an explicit request so
            # the server and child remain in lockstep without a request file.
            Get-BlueberryImportedCommands -RequestId ([string]$request.id)
        }
    } else {
        # OSC mode historically triggered the next batch without a request
        # file. Preserve that compact path when no pipe is connected.
        if (-not $script:BLUEBERRY_PIPE_ENABLED) {
            Get-BlueberryImportedCommands
        }
    }
}

function Get-BlueberryCommandMetadata {
    [CmdletBinding()]
    param([string]$Name)
    $options = [Collections.Generic.List[object]]::new()
    $description = ''
    if ($Name -notmatch '^[\p{L}\p{N}_-]{1,128}$') { return $null }
    # Search loaded commands only. Do not use Get-Command or bind dynamic parameters.
    $types = [Management.Automation.CommandTypes]::Cmdlet -bor [Management.Automation.CommandTypes]::Function
    $commands = $ExecutionContext.InvokeCommand.GetCommands($Name, $types, $true)
    foreach ($command in $commands) {
        if ($command.Name -ine $Name) { continue }
        if ($command -is [Management.Automation.FunctionInfo]) {
            $ast = $command.ScriptBlock.Ast
            $help = $ast.GetHelpContent()
            if ($null -ne $help) { $description = [string]$help.Synopsis }
            if ($ast -is [Management.Automation.Language.FunctionDefinitionAst]) { $ast = $ast.Body }
            if ($null -ne $ast.ParamBlock) {
                foreach ($parameter in $ast.ParamBlock.Parameters) {
                    $parameterName = [string]$parameter.Name.VariablePath.UserPath
                    $switch = $false
                    $values = [Collections.Generic.List[string]]::new()
                    foreach ($attribute in $parameter.Attributes) {
                        if ($attribute.TypeName.FullName -in @('switch', 'switchparameter', 'System.Management.Automation.SwitchParameter')) { $switch = $true }
                        if ($attribute -is [Management.Automation.Language.AttributeAst] -and $attribute.TypeName.FullName -eq 'ValidateSet') {
                            foreach ($argument in $attribute.PositionalArguments) {
                                if ($argument -is [Management.Automation.Language.StringConstantExpressionAst]) { $values.Add($argument.Value) }
                            }
                        }
                    }
                    $text = ''
                    if ($null -ne $help -and $help.Parameters.ContainsKey($parameterName.ToUpperInvariant())) { $text = [string]$help.Parameters[$parameterName.ToUpperInvariant()] }
                    $options.Add(@{ names = @('-' + $parameterName); description = $text; detail = $text; value_name = $(if ($switch) { '' } else { '<' + $parameterName + '>' }); values = @($values.ToArray()); repeatable = $false })
                }
            }
        } elseif ($command -is [Management.Automation.CmdletInfo]) {
            # Type metadata is static and cannot invoke a provider's dynamicparam callback.
            $metadata = [Management.Automation.CommandMetadata]::new($command.ImplementingType)
            foreach ($parameter in $metadata.Parameters.Values) {
                $values = [Collections.Generic.List[string]]::new()
                if ($parameter.ParameterType.IsEnum) { foreach ($v in [Enum]::GetNames($parameter.ParameterType)) { $values.Add($v) } }
                $options.Add(@{ names = @('-' + $parameter.Name); description = '参数 ' + $parameter.Name; detail = [string]$parameter.ParameterType; value_name = $(if ($parameter.ParameterType -eq [Management.Automation.SwitchParameter]) { '' } else { '<' + $parameter.ParameterType.Name + '>' }); values = @($values.ToArray()); repeatable = $false })
            }
            # Read a bounded local MAML file, never Update-Help or an online URI.
            $base = [IO.Path]::GetDirectoryName($command.DLL)
            $helpName = [IO.Path]::GetFileName($command.HelpFile)
            foreach ($culture in @([Globalization.CultureInfo]::CurrentUICulture.Name, 'en-US', '')) {
                $helpPath = [IO.Path]::Combine($base, $culture, $helpName)
                if ([IO.File]::Exists($helpPath) -and ([IO.FileInfo]::new($helpPath)).Length -le 1048576) {
                    $xml = [Xml.XmlDocument]::new(); $xml.XmlResolver = $null
                    $readerSettings = [Xml.XmlReaderSettings]::new(); $readerSettings.DtdProcessing = [Xml.DtdProcessing]::Prohibit
                    $reader = [Xml.XmlReader]::Create($helpPath, $readerSettings)
                    try { $xml.Load($reader) } finally { $reader.Dispose() }
                    foreach ($node in $xml.SelectNodes("//*[local-name()='command']")) {
                        $nameNode = $node.SelectSingleNode("*[local-name()='details']/*[local-name()='name']")
                        if ($null -ne $nameNode -and $nameNode.InnerText -ieq $Name) {
                            $synopsis = $node.SelectSingleNode("*[local-name()='details']/*[local-name()='description']")
                            if ($null -ne $synopsis) { $description = $synopsis.InnerText }
                            foreach ($option in $options) {
                                foreach ($parameterNode in $node.SelectNodes("*[local-name()='parameters']/*[local-name()='parameter']")) {
                                    $n = $parameterNode.SelectSingleNode("*[local-name()='name']")
                                    $d = $parameterNode.SelectSingleNode("*[local-name()='description']")
                                    if ($null -ne $n -and $null -ne $d -and ('-' + $n.InnerText) -ieq $option.names[0]) { $option.description = $d.InnerText; $option.detail = $d.InnerText }
                                }
                            }
                        }
                    }
                    if (-not [string]::IsNullOrWhiteSpace($description)) { break }
                }
            }
        }
        return @{ description = $description; options = @($options.ToArray()); commands = @{}; complete = $false }
    }
    return $null
}

function Get-BlueberryHistorySnapshot {
    [CmdletBinding()]
    param([int]$Limit = 2000)

    $commands = [System.Collections.Generic.List[string]]::new()
    try {
        foreach ($entry in @(Microsoft.PowerShell.Core\Get-History -Count $Limit -ErrorAction SilentlyContinue)) {
            $text = [string]$entry.CommandLine
            if (-not [string]::IsNullOrWhiteSpace($text)) { [void]$commands.Add($text) }
        }
    } catch { }
    $path = ''
    $style = ''
    try {
        $option = Get-PSReadLineOption -ErrorAction Stop
        $path = [string]$option.HistorySavePath
        $style = [string]$option.HistorySaveStyle
    } catch { }
    return [ordered]@{ commands = @($commands.ToArray()); path = $path; save_style = $style }
}

function Invoke-BlueberryNativeKeyHandler {
    [CmdletBinding()]
    param()
    $request = Read-BlueberryRequest -ExpectedKind @('native', 'command_metadata', 'history')
    if ($null -ne $request) {
        if ($request.kind -eq 'history') {
            $snapshot = Get-BlueberryHistorySnapshot -Limit ([int]$request.limit)
            Send-BlueberryEvent -Event 'history' -Data ([ordered]@{
                request_id = [string]$request.id
                commands = $snapshot.commands
                path = $snapshot.path
                save_style = $snapshot.save_style
            })
        } elseif ($request.kind -eq 'command_metadata') {
            $page = $null
            $savedLastExitCode = $ExecutionContext.SessionState.PSVariable.GetValue('global:LASTEXITCODE')
            try { $page = Get-BlueberryCommandMetadata -Name $request.command } catch { }
            finally { $global:LASTEXITCODE = $savedLastExitCode }
            Send-BlueberryEvent -Event 'command_metadata' -Data @{ request_id = [string]$request.id; command = [string]$request.command; page = $page }
        } else { Get-BlueberryNativeCompletion -RequestId ([string]$request.id) }
    }
}

function Test-BlueberryInteractiveHost {
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

function Get-BlueberryKeyBinding {
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

function Get-BlueberryKeyHandlerSnapshot {
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

function ConvertTo-BlueberryPublicKeyChord {
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

function Get-BlueberryPublicKeyConfiguration {
    [CmdletBinding()]
    param(
        [AllowNull()]
        [string]$JsonOverride
    )

    $hasJsonOverride = $PSBoundParameters.ContainsKey('JsonOverride')
    $raw = if ($hasJsonOverride) {
        $JsonOverride
    } else {
        [Environment]::GetEnvironmentVariable('BLUEBERRY_PUBLIC_KEYS', 'Process')
    }

    # The Rust host validates and exports these six process values before the
    # adapter is loaded.  On that path avoid parsing JSON and allocating a
    # JsonDocument.  JsonOverride always takes the original parser so a hot
    # reload remains fully validated and cannot accidentally trust stale host
    # values.
    if (-not $hasJsonOverride) {
        $version = [Environment]::GetEnvironmentVariable('BLUEBERRY_PUBLIC_KEYS_VERSION', 'Process')
        if ([string]::Equals([string]$version, '1', [StringComparison]::Ordinal)) {
            $fastConfiguration = [ordered]@{}
            $fastComplete = $true
            foreach ($name in @('trigger', 'native', 'details', 'refresh', 'reload', 'search', 'resources', 'hub')) {
                $environmentName = 'BLUEBERRY_PUBLIC_KEY_' + $name.ToUpperInvariant()
                $value = [Environment]::GetEnvironmentVariable($environmentName, 'Process')
                if ([string]::IsNullOrWhiteSpace([string]$value)) {
                    $fastComplete = $false
                    break
                }
                $fastConfiguration[$name] = [string]$value
            }
            if ($fastComplete) {
                $script:BLUEBERRY_PUBLIC_KEYS = $fastConfiguration
                $script:BLUEBERRY_PUBLIC_KEY_STATUS = [ordered]@{}
                $script:BLUEBERRY_PUBLIC_KEY_CONFIG_ERROR = $false
                return $fastConfiguration
            }
        }
    }

    if ([string]::IsNullOrWhiteSpace([string]$raw)) {
        $script:BLUEBERRY_PUBLIC_KEYS = $null
        $script:BLUEBERRY_PUBLIC_KEY_STATUS = $null
        $script:BLUEBERRY_PUBLIC_KEY_CONFIG_ERROR = $false
        return $null
    }

    $configuration = [ordered]@{}
    $invalid = $false
    $document = $null
    try {
        $document = $script:BLUEBERRY_JsonDocument::Parse([string]$raw)
        if ($document.RootElement.ValueKind -ne $script:BLUEBERRY_JsonValueKind::Object) {
            $invalid = $true
        } else {
            foreach ($name in @('trigger', 'native', 'details', 'refresh', 'reload', 'search', 'resources', 'hub')) {
                $property = $script:BLUEBERRY_JsonElement::new()
                if (-not $document.RootElement.TryGetProperty($name, [ref]$property)) {
                    continue
                }
                if ($property.ValueKind -ne $script:BLUEBERRY_JsonValueKind::String -or
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

    $script:BLUEBERRY_PUBLIC_KEYS = $configuration
    $script:BLUEBERRY_PUBLIC_KEY_STATUS = [ordered]@{}
    $script:BLUEBERRY_PUBLIC_KEY_CONFIG_ERROR = $invalid
    return $configuration
}

function Update-BlueberryPublicKeyCapabilities {
    [CmdletBinding()]
    param(
        [AllowNull()]
        [string]$JsonOverride,

        [AllowNull()]
        [object]$KeySnapshot
    )

    if ($PSBoundParameters.ContainsKey('JsonOverride')) {
        $configuration = Get-BlueberryPublicKeyConfiguration -JsonOverride $JsonOverride
    } else {
        $configuration = Get-BlueberryPublicKeyConfiguration
    }
    if ($null -eq $configuration) {
        return
    }

    if ($null -eq $KeySnapshot) {
        $KeySnapshot = Get-BlueberryKeyHandlerSnapshot
    }

    $status = [ordered]@{}
    foreach ($name in $configuration.Keys) {
        $configuredChord = [string]$configuration[$name]
        $psReadLineChord = ConvertTo-BlueberryPublicKeyChord -Chord $configuredChord
        $bindings = @()
        if (-not [string]::IsNullOrEmpty([string]$psReadLineChord)) {
            if ([bool]$KeySnapshot.available) {
                if ($KeySnapshot.by_chord.ContainsKey($psReadLineChord)) {
                    $bindings = [object[]]@($KeySnapshot.by_chord[$psReadLineChord])
                }
            } else {
                $bindings = @(Get-BlueberryKeyBinding -Chord $psReadLineChord -Snapshot $KeySnapshot)
            }
        }

        $safe = $false
        if ($bindings.Count -eq 0) {
            $safe = $true
        } elseif ([string]::Equals($name, 'search', [StringComparison]::Ordinal) -or
            [string]::Equals($name, 'hub', [StringComparison]::Ordinal)) {
            # Search and Hub are host-owned overlays. Their configured public
            # chords deliberately take precedence while the prompt is active.
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
            $suggestion = 'Change the Blueberry public key in the host key settings and start a new session.'
            Send-BlueberryEvent -Event 'error' -Data ([ordered]@{
                code          = 'public_key_collision'
                name          = $name
                chord         = $configuredChord
                bound_function = $boundFunction
                suggestion    = $suggestion
                message       = ('Public key {0} ({1}) keeps its PSReadLine binding ({2}); Blueberry will not intercept it. {3}' -f $name, $configuredChord, $boundFunction, $suggestion)
            })
        }
    }
    $script:BLUEBERRY_PUBLIC_KEY_STATUS = $status
    if ($script:BLUEBERRY_PUBLIC_KEY_CONFIG_ERROR) {
        Send-BlueberryEvent -Event 'error' -Data ([ordered]@{
            code       = 'public_key_config_invalid'
            message    = 'BLUEBERRY_PUBLIC_KEYS must be a JSON object with string key chords.'
            suggestion = 'Remove the variable or provide trigger/native/details/refresh/reload chord strings.'
        })
    }
}

function Get-BlueberryAlternativeKeyPrefix {
    [CmdletBinding()]
    param()

    foreach ($candidate in @('F5', 'F6', 'F7', 'F8', 'F9', 'F10', 'F11', 'F12')) {
        if (-not [string]::Equals($candidate, [string]$script:BLUEBERRY_KEY_PREFIX, [StringComparison]::OrdinalIgnoreCase)) {
            return $candidate
        }
    }
    return 'F5'
}

function Register-BlueberryKeyHandler {
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

    if (-not $SkipCollisionCheck -and @(Get-BlueberryKeyBinding -Chord $Chord).Count -gt 0) {
        $collisionSuggestion = ('Choose another protocol prefix (F5-F12) in the next session, for example set BLUEBERRY_KEY_PREFIX={0}.' -f (Get-BlueberryAlternativeKeyPrefix))
        Send-BlueberryEvent -Event 'error' -Data ([ordered]@{
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
            ('Blueberry ' + $Name),
            ('Report blueberry ' + $Name + ' state.'))
        return $true
    } catch {
        Send-BlueberryEvent -Event 'error' -Data ([ordered]@{
            code    = 'key_handler_registration_failed'
            chord   = $Chord
            message = ('Reserved chord {0} could not be registered.' -f $Chord)
        })
        return $false
    }
}

function Send-BlueberryCapabilities {
    [CmdletBinding()]
    param()

    $coreReady = ([bool]$script:BLUEBERRY_PSREADLINE_AVAILABLE -and
        [bool]$script:BLUEBERRY_KEY_HANDLERS.buffer -and
        [bool]$script:BLUEBERRY_KEY_HANDLERS.apply -and
        [bool]$script:BLUEBERRY_KEY_HANDLERS.commands)
    $capabilityMap = [ordered]@{
        context           = $true
        command_position  = $true
        native_completion = [bool]$script:BLUEBERRY_KEY_HANDLERS.native
        paste_insert      = [bool]$script:BLUEBERRY_KEY_HANDLERS.paste
        edit_ack          = $true
        command_batches   = $true
        multiline         = ([bool]$script:BLUEBERRY_KEY_HANDLERS.enter -or
            [bool]$script:BLUEBERRY_KEY_HANDLERS.shift_enter)
        manual_native     = $true
        command_metadata  = [bool]$script:BLUEBERRY_KEY_HANDLERS.native
        history           = [bool]$script:BLUEBERRY_KEY_HANDLERS.native
    }
    # Keep the field absent when the host did not opt into public-key
    # arbitration, preserving the alpha protocol shape for existing launchers.
    if ($null -ne $script:BLUEBERRY_PUBLIC_KEY_STATUS) {
        $capabilityMap.public_keys = $script:BLUEBERRY_PUBLIC_KEY_STATUS
    }
    Send-BlueberryEvent -Event 'capabilities' -Data ([ordered]@{
        json_initialization_ms = [double]$script:BLUEBERRY_JSON_INITIALIZATION_MS
        protocol_version = 2
        protocol         = 2
        version          = 2
        # `ready` means the complete PSReadLine surface is usable. Consumers
        # can still use prompt/execute events when this is false, while the
        # detailed fields below explain whether a missing module or a chord
        # collision is responsible.
        ready        = $coreReady
        psreadline   = [bool]$script:BLUEBERRY_PSREADLINE_AVAILABLE
        key_handlers = $script:BLUEBERRY_KEY_HANDLERS
        edit_path    = (-not [string]::IsNullOrEmpty([string]$script:BLUEBERRY_EDIT_PATH))
        request_path = (-not [string]::IsNullOrEmpty([string](Get-BlueberryRequestPath)))
        key_prefix   = [string]$script:BLUEBERRY_KEY_PREFIX
        # Report the transport that actually accepted this event. A failed
        # named-pipe connection is reflected as OSC here, so host A/B probes
        # cannot mistake a silent fallback for a pipe run.
        transport    = if ($script:BLUEBERRY_PIPE_ENABLED) { 'pipe' } else { 'osc' }
        shell_version = [string]$PSVersionTable.PSVersion
        psreadline_version = if ($null -ne (Get-Module PSReadLine)) { [string](Get-Module PSReadLine).Version } else { $null }
        capabilities = $capabilityMap
    })
}

function Initialize-BlueberryReadLine {
    [CmdletBinding()]
    param(
        [switch]$DeferImport
    )

    $traceTimer = $null
    if ($script:BLUEBERRY_TRACE_ENABLED) {
        $traceTimer = [Diagnostics.Stopwatch]::StartNew()
    }
    try {
        $script:BLUEBERRY_PSREADLINE_AVAILABLE = $false
        $script:BLUEBERRY_KEY_HANDLERS = [ordered]@{
            buffer      = $false
            apply       = $false
            commands    = $false
            native      = $false
            paste       = $false
            enter       = $false
            shift_enter = $false
        }

    $phaseTimer = $null
    if ($script:BLUEBERRY_TRACE_ENABLED) {
        $phaseTimer = [Diagnostics.Stopwatch]::StartNew()
    }
    $interactive = Test-BlueberryInteractiveHost
    if ($null -ne $phaseTimer) {
        Send-BlueberryTrace -Stage 'readline_host' -DurationMs $phaseTimer.Elapsed.TotalMilliseconds
        $phaseTimer.Restart()
    }
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
    if ($null -ne $phaseTimer) {
        Send-BlueberryTrace -Stage 'readline_module' -DurationMs $phaseTimer.Elapsed.TotalMilliseconds
        $phaseTimer.Restart()
    }

    # InvokeCommand.GetCommand consults the current session without invoking
    # the Get-Command cmdlet and its startup dispatch work.
    $readLineCommand = $ExecutionContext.InvokeCommand.GetCommand(
        'PSConsoleHostReadLine',
        [System.Management.Automation.CommandTypes]::Function)
    if ($null -ne $phaseTimer) {
        Send-BlueberryTrace -Stage 'readline_resolve' -DurationMs $phaseTimer.Elapsed.TotalMilliseconds
        $phaseTimer.Restart()
    }
    if ($null -eq $readLineCommand) {
        return
    }

    $script:BLUEBERRY_PSREADLINE_AVAILABLE = $true

    # Probe and CI sessions can opt out of touching the user's history file.
    # The default remains PSReadLine's normal history behavior for ordinary
    # interactive shells.
    if ([string]::Equals([string]$env:BLUEBERRY_NO_HISTORY, '1', [StringComparison]::Ordinal)) {
        try {
            Set-PSReadLineOption -HistorySaveStyle SaveNothing -ErrorAction Stop | Out-Null
        } catch {
        }
    }
    if ($null -ne $phaseTimer) {
        Send-BlueberryTrace -Stage 'readline_history' -DurationMs $phaseTimer.Elapsed.TotalMilliseconds
    }

    $readLineWrapTimer = $null
    if ($script:BLUEBERRY_TRACE_ENABLED) {
        $readLineWrapTimer = [Diagnostics.Stopwatch]::StartNew()
    }
    try {
        if ($null -eq $script:BLUEBERRY_ORIGINAL_READLINE) {
            $script:BLUEBERRY_ORIGINAL_READLINE = $readLineCommand.ScriptBlock
        }
        if (-not $script:BLUEBERRY_READLINE_WRAPPED) {
            function global:PSConsoleHostReadLine {
                $savedLastExitCode = $ExecutionContext.SessionState.PSVariable.GetValue('global:LASTEXITCODE')
                try {
                    $acceptedLine = & $script:BLUEBERRY_ORIGINAL_READLINE @args
                    $savedLastExitCode = $ExecutionContext.SessionState.PSVariable.GetValue('global:LASTEXITCODE')
                    # ReadLine returned only after PSReadLine accepted the command.
                    # Continuation AddLine calls remain inside ReadLine and do not
                    # reach this marker, so the host cannot query an external
                    # program while the user is still entering a multiline line.
                    Reset-BlueberryCommandSnapshot
                    Send-BlueberryEvent -Event 'execute'
                    return $acceptedLine
                } finally {
                    # PSReadLine's original function may intentionally update the
                    # native exit code; only adapter operations are restored.
                    $global:LASTEXITCODE = $savedLastExitCode
                }
            }
            $script:BLUEBERRY_READLINE_WRAPPED = $true
        }
    } finally {
        if ($null -ne $readLineWrapTimer) {
            $readLineWrapTimer.Stop()
            Send-BlueberryTrace -Stage 'readline_wrap' -DurationMs $readLineWrapTimer.Elapsed.TotalMilliseconds
        }
    }

    # Query all bound handlers once before registering any reserved chords. A
    # user's exact chord and a bare protocol prefix parent both reserve a
    # Blueberry chord. The same immutable snapshot also supplies public-key
    # arbitration and lifecycle detection, avoiding repeated reflection and
    # cold GetKeyHandlers pipelines during startup.
    $keySnapshotTimer = $null
    if ($script:BLUEBERRY_TRACE_ENABLED) {
        $keySnapshotTimer = [Diagnostics.Stopwatch]::StartNew()
    }
    $keySnapshot = $null
    try {
        $keySnapshot = Get-BlueberryKeyHandlerSnapshot
    } finally {
        if ($null -ne $keySnapshotTimer) {
            $keySnapshotTimer.Stop()
            Send-BlueberryTrace -Stage 'key_snapshot' -DurationMs $keySnapshotTimer.Elapsed.TotalMilliseconds
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
            $knownEnter = @(Get-BlueberryKeyBinding -Chord 'Enter' -Snapshot $keySnapshot) |
                Where-Object { [string]::Equals([string]$_.Function, 'AcceptLine', [StringComparison]::OrdinalIgnoreCase) } |
                Select-Object -First 1 | ForEach-Object { $true }
            $knownShiftEnter = @(Get-BlueberryKeyBinding -Chord 'Shift+Enter' -Snapshot $keySnapshot) |
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
    if ($script:BLUEBERRY_TRACE_ENABLED) {
        $publicKeyTimer = [Diagnostics.Stopwatch]::StartNew()
    }
    try {
        Update-BlueberryPublicKeyCapabilities -KeySnapshot $keySnapshot
    } finally {
        if ($null -ne $publicKeyTimer) {
            $publicKeyTimer.Stop()
            Send-BlueberryTrace -Stage 'public_keys' -DurationMs $publicKeyTimer.Elapsed.TotalMilliseconds
        }
    }
    $reservedChords = [ordered]@{
        buffer   = ([string]$script:BLUEBERRY_KEY_PREFIX + ',s')
        apply    = ([string]$script:BLUEBERRY_KEY_PREFIX + ',a')
        commands = ([string]$script:BLUEBERRY_KEY_PREFIX + ',c')
        native   = ([string]$script:BLUEBERRY_KEY_PREFIX + ',n')
        paste    = ([string]$script:BLUEBERRY_KEY_PREFIX + ',p')
    }
    if ($knownEnter) {
        $reservedChords.enter = ([string]$script:BLUEBERRY_KEY_PREFIX + ',e')
    }
    if ($knownShiftEnter) {
        $reservedChords.shift_enter = ([string]$script:BLUEBERRY_KEY_PREFIX + ',l')
    }
    $existingByName = [ordered]@{
        buffer      = $false
        apply       = $false
        commands    = $false
        native      = $false
        paste       = $false
        enter       = $false
        shift_enter = $false
    }
    $keyHandlerEnumerationFailed = -not [bool]$keySnapshot.available
    if ($keyHandlerEnumerationFailed) {
        # Older PSReadLine builds may not expose GetKeyHandlers(bool,bool).
        # Preserve the compatibility path, but do not invoke it for a valid
        # empty result from the static API.
        foreach ($reservedName in $reservedChords.Keys) {
            $existingByName[$reservedName] = @(Get-BlueberryKeyBinding -Chord $reservedChords[$reservedName]).Count -gt 0
        }
    } else {
        # A bare prefix occupies every child chord. The case-insensitive map
        # makes both that parent check and each exact child check constant-time
        # lookups, with no 72-by-6 nested scan during initialization.
        $barePrefixBound = $snapshotByChord.ContainsKey([string]$script:BLUEBERRY_KEY_PREFIX)
        foreach ($reservedName in $reservedChords.Keys) {
            $reservedChord = [string]$reservedChords[$reservedName]
            $existingByName[$reservedName] = $barePrefixBound -or
                $snapshotByChord.ContainsKey($reservedChord)
        }
    }

    $keyRegisterTimer = $null
    if ($script:BLUEBERRY_TRACE_ENABLED) {
        $keyRegisterTimer = [Diagnostics.Stopwatch]::StartNew()
    }
    try {
        foreach ($reservedName in $reservedChords.Keys) {
            if ([bool]$existingByName[$reservedName]) {
                $collisionSuggestion = ('Choose another protocol prefix (F5-F12) in the next session, for example set BLUEBERRY_KEY_PREFIX={0}.' -f (Get-BlueberryAlternativeKeyPrefix))
                Send-BlueberryEvent -Event 'error' -Data ([ordered]@{
                    code    = 'key_chord_collision'
                    chord   = $reservedChords[$reservedName]
                    suggestion = $collisionSuggestion
                    message = ('Reserved chord {0} is already bound; it was left unchanged. {1}' -f $reservedChords[$reservedName], $collisionSuggestion)
                })
                continue
            }
            switch ($reservedName) {
                'buffer' {
                    $script:BLUEBERRY_KEY_HANDLERS.buffer = Register-BlueberryKeyHandler `
                        -Chord $reservedChords[$reservedName] -ScriptBlock ${function:Invoke-BlueberryBufferKeyHandler} `
                        -Name 'buffer' -SkipCollisionCheck
                }
                'apply' {
                    $script:BLUEBERRY_KEY_HANDLERS.apply = Register-BlueberryKeyHandler `
                        -Chord $reservedChords[$reservedName] -ScriptBlock ${function:Invoke-BlueberryApplyKeyHandler} `
                        -Name 'apply' -SkipCollisionCheck
                }
                'commands' {
                    $script:BLUEBERRY_KEY_HANDLERS.commands = Register-BlueberryKeyHandler `
                        -Chord $reservedChords[$reservedName] -ScriptBlock ${function:Invoke-BlueberryCommandsKeyHandler} `
                        -Name 'commands' -SkipCollisionCheck
                }
                'native' {
                    $script:BLUEBERRY_KEY_HANDLERS.native = Register-BlueberryKeyHandler `
                        -Chord $reservedChords[$reservedName] -ScriptBlock ${function:Invoke-BlueberryNativeKeyHandler} `
                        -Name 'native' -SkipCollisionCheck
                }
                'paste' {
                    $script:BLUEBERRY_KEY_HANDLERS.paste = Register-BlueberryKeyHandler `
                        -Chord $reservedChords[$reservedName] -ScriptBlock ${function:Invoke-BlueberryPasteKeyHandler} `
                        -Name 'paste' -SkipCollisionCheck
                }
                'enter' {
                    $script:BLUEBERRY_KEY_HANDLERS.enter = Register-BlueberryKeyHandler `
                        -Chord $reservedChords[$reservedName] -ScriptBlock ${function:Invoke-BlueberryEnterKeyHandler} `
                        -Name 'enter' -SkipCollisionCheck
                }
                'shift_enter' {
                    $script:BLUEBERRY_KEY_HANDLERS.shift_enter = Register-BlueberryKeyHandler `
                        -Chord $reservedChords[$reservedName] -ScriptBlock ${function:Invoke-BlueberryShiftEnterKeyHandler} `
                        -Name 'shift_enter' -SkipCollisionCheck
                }
            }
        }
    } finally {
        if ($null -ne $keyRegisterTimer) {
            $keyRegisterTimer.Stop()
            Send-BlueberryTrace -Stage 'key_register' -DurationMs $keyRegisterTimer.Elapsed.TotalMilliseconds
        }
    }
    } finally {
        if ($null -ne $traceTimer) {
            $traceTimer.Stop()
            Send-BlueberryTrace -Stage 'readline_init' -DurationMs $traceTimer.Elapsed.TotalMilliseconds
        }
    }
}

function Initialize-BlueberryPrompt {
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
    if ($null -eq $script:BLUEBERRY_ORIGINAL_PROMPT) {
        $script:BLUEBERRY_ORIGINAL_PROMPT = $promptCommand.ScriptBlock
    }
    if ($script:BLUEBERRY_PROMPT_WRAPPED) {
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
                $originalOutput = @(& $script:BLUEBERRY_ORIGINAL_PROMPT)
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
        if (-not $script:BLUEBERRY_PSREADLINE_AVAILABLE -and (Test-BlueberryInteractiveHost)) {
            Initialize-BlueberryReadLine
            if ($script:BLUEBERRY_PSREADLINE_AVAILABLE -and -not $script:BLUEBERRY_CAPABILITIES_REFRESHED) {
                Send-BlueberryCapabilities
                $script:BLUEBERRY_CAPABILITIES_REFRESHED = $true
            }
        }

        Send-BlueberryEvent -Event 'prompt_start' -Data ([ordered]@{
            cwd = Get-BlueberryWorkingDirectory
        })
        Send-BlueberryEvent -Event 'prompt_end' -Data (Get-BlueberryPromptEndData)

        $global:LASTEXITCODE = $promptState.lastExitCode
        if ($null -ne $promptState.error) {
            throw $promptState.error
        }
        return $promptState.output
    }
    $script:BLUEBERRY_PROMPT_WRAPPED = $true
}

# Capture the first filesystem location before a user changes to a provider
# that cannot be represented as a filesystem cwd.
if ($null -eq $script:BLUEBERRY_FALLBACK_CWD) {
    try {
        $initialLocation = $ExecutionContext.SessionState.Path.CurrentLocation
        if ($null -ne $initialLocation.Provider -and $initialLocation.Provider.Name -eq 'FileSystem') {
            $script:BLUEBERRY_FALLBACK_CWD = [string]$initialLocation.Path
        }
    } catch {
    }
}

# Dot-sourcing without a token is useful for syntax and helper tests and must
# remain a no-op for the user's normal shell.  The token-bearing invocation is
# initialized only once, so sourcing this file again cannot double-wrap hooks.
if ([string]::IsNullOrEmpty([string]$script:BLUEBERRY_TOKEN) -or $script:BLUEBERRY_INITIALIZED) {
    return
}
$script:BLUEBERRY_INITIALIZED = $true

# The ConPTY stream and Rust terminal model use UTF-8. PSReadLine.Replace
# redraws through Console.Out; a legacy code page would display emoji as ??.
# These settings apply only to the owned child process, never to profile files.
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
[Console]::InputEncoding = [Text.UTF8Encoding]::new($false)

$blueberryBootstrapTraceTimer = $null
if ($script:BLUEBERRY_TRACE_ENABLED) {
    $blueberryBootstrapTraceTimer = [Diagnostics.Stopwatch]::StartNew()
}
try {
    Initialize-BlueberryPipe | Out-Null
    Initialize-BlueberryPrompt
    Initialize-BlueberryReadLine -DeferImport
    Send-BlueberryCapabilities
} finally {
    if ($null -ne $blueberryBootstrapTraceTimer) {
        $blueberryBootstrapTraceTimer.Stop()
        Send-BlueberryTrace -Stage 'adapter_bootstrap' -DurationMs $blueberryBootstrapTraceTimer.Elapsed.TotalMilliseconds
    }
}
