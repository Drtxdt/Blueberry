[CmdletBinding()]
param(
    [string]$BridgePath = (Join-Path $PSScriptRoot 'bin\ShellSense.PSReadLineBridge.dll'),
    [switch]$Json
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$resolvedBridgePath = (Resolve-Path -LiteralPath $BridgePath -ErrorAction Stop).Path
if (-not [IO.File]::Exists($resolvedBridgePath)) {
    throw "Bridge DLL was not found: $resolvedBridgePath"
}

# Verification loads the precompiled assembly only. It intentionally does not
# call Add-Type, import a profile, register a handler, or mutate PSReadLine.
Import-Module PSReadLine -ErrorAction Stop
[Reflection.Assembly]::LoadFrom($resolvedBridgePath) | Out-Null

$bindError = ''
$bound = [ShellSense.Bridge.PsReadLineBridge]::TryBind([ref]$bindError)
if (-not $bound) {
    throw "PSReadLine public surface could not be bound: $bindError"
}

$bufferState = $null
$bufferError = ''
$bufferAvailable = [ShellSense.Bridge.PsReadLineBridge]::TryGetBufferState(
    [ref]$bufferState,
    [ref]$bufferError)
if (-not $bufferAvailable) {
    throw "PSReadLine buffer state could not be read: $bufferError"
}

$handlers = [ShellSense.Bridge.PsReadLineBridge]::GetKeyHandlerSnapshot()
if (-not $handlers.Available -or $handlers.Handlers.Count -eq 0) {
    throw "PSReadLine key-handler snapshot was unavailable or empty."
}

$payload = [ordered]@{
    text  = '中文'
    emoji = '😀'
}
$serialized = [ShellSense.Bridge.PsReadLineBridge]::Serialize($payload)
$document = [System.Text.Json.JsonDocument]::Parse($serialized)
try {
    if ($document.RootElement.GetProperty('text').GetString() -ne '中文' -or
        $document.RootElement.GetProperty('emoji').GetString() -ne '😀') {
        throw 'Serialized Unicode payload did not round-trip.'
    }
} finally {
    $document.Dispose()
}

$result = [pscustomobject]@{
    available             = $true
    bound                 = [bool]$bound
    buffer_available      = [bool]$bufferAvailable
    cursor_utf16          = [int]$bufferState.Cursor
    handler_count         = [int]$handlers.Handlers.Count
    ascii_json            = -not ($serialized -match '[^\u0000-\u007F]')
    serialization         = $serialized
    runtime               = [string]$PSVersionTable.PSVersion
}
if ($Json) {
    $result | ConvertTo-Json -Depth 4 -Compress
} else {
    $result
}
