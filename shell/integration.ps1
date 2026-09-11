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

function ConvertTo-ShellsenseJson {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [AllowNull()]
        [object]$Payload
    )

    # ConvertTo-Json is the one place where user supplied strings enter the
    # frame.  Its JSON escaping prevents newlines, BEL, and ESC in command
    # text from becoming terminal control sequences.
    # Keep the transport ASCII even under a legacy Windows console code page.
    # Otherwise Console.Out can replace non-BMP characters with question marks.
    return [string]($Payload | ConvertTo-Json -Compress -Depth 10 -EscapeHandling EscapeNonAscii -ErrorAction Stop)
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
        $payload = ConvertFrom-Json -InputObject $json -ErrorAction Stop

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

function Get-ShellsenseImportedCommands {
    [CmdletBinding()]
    param()

    $maxCommands = 512
    try {
        $commands = @(Get-Command -ListImported -CommandType Alias,Function,Cmdlet -ErrorAction Stop |
            Sort-Object -Property Name,CommandType |
            Select-Object -First $maxCommands)
    } catch {
        Send-ShellsenseEvent -Event 'error' -Data ([ordered]@{
            code    = 'commands_unavailable'
            message = 'Loaded command enumeration failed.'
        })
        return
    }

    $items = New-Object 'System.Collections.Generic.List[object]'
    $seen = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
    foreach ($command in $commands) {
        if ($null -eq $command -or $null -eq $command.Name) {
            continue
        }
        $name = [string]$command.Name
        if (-not $seen.Add($name)) {
            continue
        }
        $kind = ([string]$command.CommandType).ToLowerInvariant()
        $definition = ''
        if ($kind -eq 'alias' -and $null -ne $command.Definition) {
            $definition = [string]$command.Definition
        }
        [void]$items.Add([ordered]@{
            name       = $name
            kind       = $kind
            definition = $definition
        })
    }

    Send-ShellsenseEvent -Event 'commands' -Data ([ordered]@{
        commands = @($items.ToArray())
    })
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
        # The cmdlet fallback keeps this usable with older PSReadLine builds.
        # GetKeyHandlers expects an array of chord specifications. Passing
        # the comma-delimited chord as one element keeps `s` from being
        # mistaken for an independent SelfInsert binding.
        $keys = [string[]]@($Chord)
        $handlers = @([Microsoft.PowerShell.PSConsoleReadLine]::GetKeyHandlers($keys))
        if ($handlers.Count -gt 0) {
            return $handlers
        }
        # PSReadLine 2.4 does not expose custom multi-key sequences through
        # the static overload even though the overload is public. Ask the
        # cmdlet for the exact sequence when that happens; this preserves a
        # user's reserved binding instead of silently overwriting it.
        return @(Get-PSReadLineKeyHandler -Chord $Chord -ErrorAction SilentlyContinue)
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
        Set-PSReadLineKeyHandler -Chord $Chord -ScriptBlock $ScriptBlock `
            -BriefDescription ('Shellsense ' + $Name) `
            -Description ('Report shellsense ' + $Name + ' state.') -ErrorAction Stop | Out-Null
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
    # the Get-Command cmdlet (which can cold-load Microsoft.PowerShell.Utility).
    $setHandlerCommand = $ExecutionContext.InvokeCommand.GetCommand(
        'Set-PSReadLineKeyHandler',
        [System.Management.Automation.CommandTypes]::All)
    $readLineCommand = $ExecutionContext.InvokeCommand.GetCommand(
        'PSConsoleHostReadLine',
        [System.Management.Automation.CommandTypes]::Function)
    if ($null -eq $setHandlerCommand -or $null -eq $readLineCommand) {
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

    # Query every reserved chord before registering any of them. PSReadLine's
    # direct API reports the already-registered F12 prefix as a handler for
    # later chords in the same prefix, so checking after the first Set would
    # falsely report F12,a and F12,c as collisions with our own F12,s.
    $reservedChords = [ordered]@{
        buffer   = 'F12,s'
        apply    = 'F12,a'
        commands = 'F12,c'
    }
    $existingByName = [ordered]@{}
    foreach ($reservedName in $reservedChords.Keys) {
        $existingByName[$reservedName] = @(Get-ShellsenseKeyBinding -Chord $reservedChords[$reservedName])
    }

    foreach ($reservedName in $reservedChords.Keys) {
        $existing = @($existingByName[$reservedName])
        if ($existing.Count -gt 0) {
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
}

function Initialize-ShellsensePrompt {
    [CmdletBinding()]
    param()

    $promptCommand = Get-Command -Name Prompt -CommandType Function -ErrorAction SilentlyContinue
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

Initialize-ShellsensePrompt
Initialize-ShellsenseReadLine -DeferImport
Send-ShellsenseCapabilities
