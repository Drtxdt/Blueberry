# Dot-source only in an isolated measurement PSHOME. Leave user profiles and
# installed shells alone. Refuse stock assemblies rather than replacing keys.
param([Parameter(Mandatory)][string]$Service,[Parameter(Mandatory)][string]$DataDirectory,
      [ValidateSet('2.4.5','2.0.0')][string]$Editor='2.4.5')
if ($env:BLUEBERRY_NO_HISTORY -ne '1' -or -not $env:BLUEBERRY_PROFILE_PROBE_DATA -or
    $DataDirectory -ne $env:BLUEBERRY_PROFILE_PROBE_DATA) {
    throw 'This experiment only runs inside the isolated product probe.'
}
$loaded = Get-Module PSReadLine
if ($loaded -and -not [Microsoft.PowerShell.PSConsoleReadLine].GetProperty('EditorIntegrationVersion')) {
    throw 'An active stock editor cannot provide the automatic first-key hook'
}
$info=[Diagnostics.ProcessStartInfo]::new()
$info.FileName=$Service
$info.UseShellExecute=$false
$info.CreateNoWindow=$true
$info.RedirectStandardOutput=$true
$shellFile=[Diagnostics.Process]::GetCurrentProcess().MainModule.FileName
$info.Arguments=('{0} "{1}" "{2}" {3}' -f $PID,$shellFile,$DataDirectory,$Editor)
$serviceProcess=[Diagnostics.Process]::Start($info)
try {
    $response=$serviceProcess.StandardOutput.ReadLineAsync()
    if (-not $response.Wait(10000)) { throw 'Editor service did not initialize' }
    $hello=$response.Result | ConvertFrom-Json
    if (-not $hello.token -or -not $hello.manifest) { throw 'Missing service handshake' }
    $env:BLUEBERRY_ACTIVE='1'
    $env:BLUEBERRY_PIPE_NAME=$hello.pipe
    $env:BLUEBERRY_TOKEN=$hello.token
    $env:BLUEBERRY_SESSION_DIR=$hello.session
    $env:BLUEBERRY_EDITOR_DLL_SHA256=$hello.editor_sha256
    $env:BLUEBERRY_HOST_MODE='direct'
    foreach ($key in $hello.keys.PSObject.Properties) {
        [Environment]::SetEnvironmentVariable($key.Name,[string]$key.Value,'Process')
    }
    . $hello.bootstrap -EditorManifest $hello.manifest
    if (-not [Blueberry.Direct.Bridge]::AutomaticMenu) { throw 'Automatic completion unavailable' }
} catch {
    if (-not $serviceProcess.HasExited) { $serviceProcess.Kill() }
    throw
} finally { $serviceProcess.Dispose() }
