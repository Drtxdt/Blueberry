[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateSet('enable','disable','status','prepare-rollback')][string]$Action,
    [Parameter(Mandatory = $true)][string]$Executable,
    [string[]]$ProfilePath,
    [string]$EditorPayload,
    [switch]$OwnedOnly
)
$ErrorActionPreference = 'Stop'
$beginMarker = '# >>> Blueberry startup >>>'
$endMarker = '# <<< Blueberry startup <<<'
$Executable = [IO.Path]::GetFullPath($Executable)
$index = if ($EditorPayload) { Join-Path $env:LOCALAPPDATA 'Blueberry/startup-profiles' }
$profileShells = @{}
$compat = $Action -eq 'prepare-rollback'
if ($compat) {
    # Only a module installed by this executable needs conversion.
    $OwnedOnly = $true
    if (-not $index -or -not [IO.Directory]::Exists($index)) { return }
    $ProfilePath = @(foreach ($file in Get-ChildItem -LiteralPath $index -File -Filter '*.json') {
        $entry = [IO.File]::ReadAllText($file.FullName) | ConvertFrom-Json
        if ($entry.exe -ieq $Executable) { $entry.profile }
    })
    if (-not $ProfilePath.Count) { return }
}
if (-not $ProfilePath) {
    $shells = @()
    $seven = Get-Command pwsh.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($seven) { $shells += $seven.Source }
    $inbox = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    if (Test-Path -LiteralPath $inbox) { $shells += $inbox }
    $ProfilePath = @($shells | ForEach-Object {
        $path = & $_ -NoLogo -NoProfile -NonInteractive -Command '[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); $PROFILE.CurrentUserCurrentHost'
        if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace([string]$path)) { throw 'Cannot resolve PowerShell profile.' }
        $resolved = ([string]$path).Trim()
        $profileShells[$resolved] = $_
        $resolved
    } | Select-Object -Unique)
    if ($index -and $Action -eq 'disable' -and [IO.Directory]::Exists($index)) {
        foreach ($record in Get-ChildItem -LiteralPath $index -Filter '*.json' -File) {
            $entry = [IO.File]::ReadAllText($record.FullName) | ConvertFrom-Json
            if ($entry.exe -ieq $Executable) { $ProfilePath += @($entry.profile) }
        }
        $ProfilePath = @($ProfilePath | Select-Object -Unique)
    }
}
if ($EditorPayload) { . (Join-Path $PSScriptRoot 'profile-module.ps1') }
$encodedExe = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($Executable))
$block = @'
# >>> Blueberry startup >>>
# Managed by blueberry startup enable / disable.
if ($env:BLUEBERRY_PROFILE_TRACE) { $blueberryAutoHookQpc = [Diagnostics.Stopwatch]::GetTimestamp() }
$blueberryAutoExe = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('__EXECUTABLE__'))
$blueberryAutoArgs = [Environment]::GetCommandLineArgs()
$blueberryAutoScript = $false
foreach ($blueberryArgument in $blueberryAutoArgs) {
    if ($blueberryArgument.Length -lt 2 -or $blueberryArgument[0] -ne '-') { continue }
    $blueberrySwitch = $blueberryArgument.Substring(1)
    foreach ($blueberryName in 'NonInteractive','Command','CommandWithArgs','EncodedCommand','EncodedArguments','File','ServerMode','SSHServerMode','SocketServerMode','NamedPipeServerMode') {
        if ($blueberryName.StartsWith($blueberrySwitch,[StringComparison]::OrdinalIgnoreCase)) { $blueberryAutoScript=$true; break }
    }
    if ($blueberryAutoScript) { break }
}
if ($Host.Name -eq 'ConsoleHost' -and -not $blueberryAutoScript -and
    $env:BLUEBERRY_ACTIVE -ne '1' -and $env:BLUEBERRY_PROFILE_PID -ne [string]$PID -and -not [Console]::IsInputRedirected -and
    -not [Console]::IsOutputRedirected -and [IO.File]::Exists($blueberryAutoExe)) {
    try {
__STARTUP_BODY__
    } catch { Write-Warning ('Blueberry: ' + $_.Exception.Message) }
}
# <<< Blueberry startup <<<
'@
$body = if ($compat) {
    '        & $blueberryAutoExe run --shell ([Diagnostics.Process]::GetCurrentProcess().MainModule.FileName)'
} else {
@'
        # Select-Object would load Microsoft.PowerShell.Utility on the first prompt.
        $blueberryEditor = @(Get-Module PSReadLine)[0]
        if (-not $blueberryEditor) { Import-Module PSReadLine -ErrorAction Stop; $blueberryEditor = @(Get-Module PSReadLine)[0] }
        if ($env:BLUEBERRY_PROFILE_TRACE) { $env:BLUEBERRY_PROFILE_EDITOR_QPC = [string][Diagnostics.Stopwatch]::GetTimestamp() }
        if (-not [Microsoft.PowerShell.PSConsoleReadLine].GetProperty('EditorIntegrationVersion')) { throw 'Expected the managed PSReadLine module. Check Get-Module PSReadLine and run blueberry startup enable.' }
        if ($env:BLUEBERRY_NO_HISTORY -eq '1') { Set-PSReadLineOption -HistorySaveStyle SaveNothing }
        if ($env:BLUEBERRY_PROFILE_TRACE) {
            $env:BLUEBERRY_PROFILE_HOOK_QPC = [string]$blueberryAutoHookQpc
            $env:BLUEBERRY_DIRECT_TRACE = '1'
        }
        $null = [Reflection.Assembly]::LoadFrom([IO.Path]::Combine([IO.Path]::GetDirectoryName([Microsoft.PowerShell.PSConsoleReadLine].Assembly.Location),'blueberry-profile-bridge.dll'))
        if ($env:BLUEBERRY_PROFILE_TRACE) { $env:BLUEBERRY_PROFILE_BRIDGE_QPC = [string][Diagnostics.Stopwatch]::GetTimestamp() }
        [Blueberry.Direct.Bridge]::AttachProfile($blueberryAutoExe,[Microsoft.PowerShell.PSConsoleReadLine],$PSVersionTable.PSVersion.ToString(),$blueberryEditor.Version.ToString())
'@
}
$block = $block.Replace('__STARTUP_BODY__',$body)
$block = $block.Replace('__EXECUTABLE__', $encodedExe).Replace("`r`n", "`n").Replace("`n", "`r`n") + "`r`n"
foreach ($path in $ProfilePath) {
    $path = [IO.Path]::GetFullPath($path)
    $mutexHash = [Security.Cryptography.SHA256]::Create()
    try { $lockName = [BitConverter]::ToString($mutexHash.ComputeHash([Text.Encoding]::UTF8.GetBytes(([IO.Path]::GetDirectoryName($path)).ToUpperInvariant()))).Replace('-','') }
    finally { $mutexHash.Dispose() }
    $mutex = [Threading.Mutex]::new($false,('Local\BlueberryStartup'+$lockName))
    $locked = $false
    $undoNewModule = $false
    try { try { $locked=$mutex.WaitOne(30000) } catch [Threading.AbandonedMutexException] { $locked=$true }
    if (-not $locked) { throw 'Another startup operation is still active' }
    $exists = [IO.File]::Exists($path)
    if ($exists -and ((Get-Item -LiteralPath $path -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Profile is a link: $path" }
    $encoding = [Text.UTF8Encoding]::new($true, $true)
    $preamble = $encoding.GetPreamble()
    $text = ''
    if ($exists) {
        $bytes = [IO.File]::ReadAllBytes($path)
        $offset = 0
        if ($bytes.Length -ge 2 -and $bytes[0] -eq 255 -and $bytes[1] -eq 254) { $encoding = [Text.Encoding]::Unicode; $offset = 2 }
        elseif ($bytes.Length -ge 2 -and $bytes[0] -eq 254 -and $bytes[1] -eq 255) { $encoding = [Text.Encoding]::BigEndianUnicode; $offset = 2 }
        elseif ($bytes.Length -ge 3 -and $bytes[0] -eq 239 -and $bytes[1] -eq 187 -and $bytes[2] -eq 191) { $offset = 3 }
        else { $encoding = [Text.UTF8Encoding]::new($false, $true) }
        try { $text = $encoding.GetString($bytes, $offset, $bytes.Length - $offset) }
        catch { $encoding = [Text.Encoding]::GetEncoding([Globalization.CultureInfo]::CurrentCulture.TextInfo.ANSICodePage); $text = $encoding.GetString($bytes) }
        $preamble = if ($offset) { $encoding.GetPreamble() } else { [byte[]]@() }
    }
    $pattern = '(?ms)(?:\r?\n# Blueberry inserted line break\r?\n)?^' + [regex]::Escape($beginMarker) + '\r?\n.*?^' + [regex]::Escape($endMarker) + '(?:\r?\n|$)'
    $matches = [regex]::Matches($text, $pattern)
    if ($matches.Count -gt 1 -or ($text.Contains($beginMarker) -or $text.Contains($endMarker)) -and $matches.Count -ne 1) { throw "Ambiguous Blueberry block; profile preserved: $path" }
    $enabled = $matches.Count -eq 1
    if ($Action -eq 'status') { [pscustomobject]@{ profile = $path; enabled = $enabled }; continue }
    if ($OwnedOnly -and $enabled -and -not $matches[0].Value.Contains("'" + $encodedExe + "'")) { continue }
    $updated = [regex]::Replace($text, $pattern, '')
    if ($Action -eq 'enable' -or ($compat -and $enabled)) {
        # Preserve the original profile byte-for-byte after the managed prefix.
        if ($enabled) { $separator = if ($matches[0].Value.StartsWith("`r`n# Blueberry inserted line break")) { "`r`n# Blueberry inserted line break`r`n" } else { '' }; $updated = $text.Substring(0, $matches[0].Index) + $separator + $block + $text.Substring($matches[0].Index + $matches[0].Length) }
        else {
            # using / param must precede executable statements in a PowerShell file.
            $tokens = $null; $parseErrors = $null
            $ast = [Management.Automation.Language.Parser]::ParseInput($text, [ref]$tokens, [ref]$parseErrors)
            $headers = @($ast.UsingStatements | ForEach-Object { $_.Extent.EndOffset })
            if ($ast.ParamBlock) { $headers += $ast.ParamBlock.Extent.EndOffset }
            $insert = 0
            $prefix = ''
            if ($headers.Count) {
                $insert = ($headers | Measure-Object -Maximum).Maximum
                # Insert at the syntax boundary, before any following statement.
                $prefix = "`r`n# Blueberry inserted line break`r`n"
            }
            $updated = $text.Substring(0, $insert) + $prefix + $block + $text.Substring($insert)
        }
    }
    $marker = if ($index) { Join-Path $index ($lockName+'.json') }
    if ($EditorPayload -and -not $compat) {
        $version = '2.4.5'
        if ($Action -eq 'enable') {
            $modulePath = $env:BLUEBERRY_TEST_PSREADLINE_MODULE
            if ($env:BLUEBERRY_NO_HISTORY -eq '1' -and $modulePath) {
                $observedVersion = (Import-PowerShellDataFile -LiteralPath $modulePath).ModuleVersion
            } else {
                $targetShell = if ((Split-Path -Leaf (Split-Path -Parent $path)) -eq 'WindowsPowerShell') { 'powershell.exe' } else { 'pwsh.exe' }
                $observedVersion = if (Get-Command $targetShell -ErrorAction SilentlyContinue) {
                    & $targetShell -NoProfile -NonInteractive -Command '(Get-Module -ListAvailable PSReadLine | Select-Object -First 1).Version.ToString()'
                }
            }
            if ($observedVersion -in '2.0','2.0.0') { $version = '2.0.0' }
        }
        if ($Action -eq 'enable') {
            [IO.Directory]::CreateDirectory($index) | Out-Null
            if (-not [IO.File]::Exists($marker)) {
                $pending = $marker+'.'+[guid]::NewGuid().ToString('N')
                [IO.File]::WriteAllText($pending,(@{exe=$Executable;profile=$path} | ConvertTo-Json))
                [IO.File]::Move($pending,$marker)
            }
        }
        $newModule = $Action -eq 'enable' -and -not [IO.File]::Exists((Join-Path (Split-Path -LiteralPath $path) '.blueberry-editor/owner.json'))
        Update-BlueberryProfileEditor $Action $path $Executable $EditorPayload $version -OwnedOnly:$OwnedOnly
        $undoNewModule = $newModule
        if ($Action -eq 'enable' -and $profileShells.ContainsKey($path)) {
            $shellPath = $profileShells[$path]
            $loaded = & $shellPath -NoProfile -NonInteractive -Command '[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); Import-Module PSReadLine -ErrorAction Stop; Get-PSReadLineOption -ErrorAction Stop | Out-Null; [Console]::WriteLine([Microsoft.PowerShell.PSConsoleReadLine].Assembly.Location)'
            $parent = Split-Path -LiteralPath $path
            $owner = [IO.File]::ReadAllText((Join-Path $parent '.blueberry-editor/owner.json')) | ConvertFrom-Json
            $dir = Join-Path $parent 'Modules/PSReadLine'
            if (-not $owner.flat) { $dir = Join-Path $dir $owner.version }
            $dll = if ($owner.version -eq '2.0.0') { 'Microsoft.PowerShell.PSReadLine2.dll' } else { 'Microsoft.PowerShell.PSReadLine.dll' }
            $expected = [IO.Path]::GetFullPath((Join-Path (Join-Path $dir $owner.payload) $dll))
            if ($LASTEXITCODE -or ([string]$loaded).Trim() -ine $expected) { throw "PowerShell module precedence selected a different editor: $loaded" }
        }
    }
    if ($updated -cne $text) {
        [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($path)) | Out-Null
        if ($exists) { [IO.File]::Copy($path, ($path + '.blueberry-' + [Guid]::NewGuid().ToString('N') + '.bak'), $false) }
        $temporary = $path + '.blueberry-' + [Guid]::NewGuid().ToString('N') + '.tmp'
        try {
            [IO.File]::WriteAllBytes($temporary, [byte[]]($preamble + $encoding.GetBytes($updated)))
            if ($exists) { [IO.File]::Replace($temporary, $path, [NullString]::Value) } else { [IO.File]::Move($temporary, $path) }
        } finally { if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) } }
        Write-Output "$Action`: $path"
    } else { Write-Output "$Action unchanged: $path" }
    if ($compat) {
        # The fallback works with either executable, including after a crash
        # before the rollback transaction replaces the installed binary.
        Update-BlueberryProfileEditor disable $path $Executable $EditorPayload '' -OwnedOnly
    }
    if ($EditorPayload -and ($compat -or $Action -eq 'disable') -and [IO.File]::Exists($marker)) {
        $entry = [IO.File]::ReadAllText($marker) | ConvertFrom-Json
        $storePath = Join-Path (Split-Path -LiteralPath $path) '.blueberry-editor'
        $deferred = [IO.Directory]::Exists($storePath) -and @(Get-ChildItem -LiteralPath $storePath -Filter 'retire-*.json').Count
        if (-not $deferred -and $entry.exe -ieq $Executable -and $entry.profile -ieq $path) { [IO.File]::Delete($marker) }
    }
    } catch {
        if ($undoNewModule) {
            Update-BlueberryProfileEditor disable $path $Executable $EditorPayload $version -OwnedOnly
        }
        throw
    } finally { if ($locked) { $mutex.ReleaseMutex() }; $mutex.Dispose() }
}
