[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateSet('enable','disable','status')][string]$Action,
    [Parameter(Mandatory = $true)][string]$Executable,
    [string[]]$ProfilePath,
    [switch]$OwnedOnly
)
$ErrorActionPreference = 'Stop'
$beginMarker = '# >>> Blueberry startup >>>'
$endMarker = '# <<< Blueberry startup <<<'
$Executable = [IO.Path]::GetFullPath($Executable)
if (-not $ProfilePath) {
    $shells = @()
    $seven = Get-Command pwsh.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($seven) { $shells += $seven.Source }
    $inbox = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    if (Test-Path -LiteralPath $inbox) { $shells += $inbox }
    $ProfilePath = @($shells | ForEach-Object {
        $path = & $_ -NoLogo -NoProfile -NonInteractive -Command '[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); $PROFILE.CurrentUserCurrentHost'
        if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace([string]$path)) { throw 'Cannot resolve PowerShell profile.' }
        ([string]$path).Trim()
    } | Select-Object -Unique)
}
$encodedExe = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($Executable))
$block = @'
# >>> Blueberry startup >>>
# Managed by blueberry startup enable / disable.
$blueberryAutoExe = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('__EXECUTABLE__'))
$blueberryAutoArgs = [Environment]::GetCommandLineArgs()
$blueberryAutoScript = @($blueberryAutoArgs | Where-Object {
    $argument = [string]$_
    if ($argument.StartsWith('-') -and $argument.Length -gt 1) {
        $argument = $argument.Substring(1)
        @('NonInteractive','Command','CommandWithArgs','EncodedCommand','EncodedArguments','File','ServerMode','SSHServerMode','SocketServerMode','NamedPipeServerMode') | Where-Object { $_.StartsWith($argument, [StringComparison]::OrdinalIgnoreCase) }
    }
}).Count -gt 0
if ($Host.Name -eq 'ConsoleHost' -and -not $blueberryAutoScript -and
    $env:BLUEBERRY_ACTIVE -ne '1' -and -not [Console]::IsInputRedirected -and
    -not [Console]::IsOutputRedirected -and [IO.File]::Exists($blueberryAutoExe)) {
    try {
        $blueberryAutoShell = [Diagnostics.Process]::GetCurrentProcess().MainModule.FileName
        & $blueberryAutoExe run --shell $blueberryAutoShell
    } catch { Write-Warning ('Blueberry: ' + $_.Exception.Message) }
}
# <<< Blueberry startup <<<
'@
$block = $block.Replace('__EXECUTABLE__', $encodedExe).Replace("`r`n", "`n").Replace("`n", "`r`n") + "`r`n"
foreach ($path in $ProfilePath) {
    $path = [IO.Path]::GetFullPath($path)
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
    if ($Action -eq 'enable') {
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
    if ($updated -ceq $text) { Write-Output "$Action unchanged: $path"; continue }
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($path)) | Out-Null
    if ($exists) { [IO.File]::Copy($path, ($path + '.blueberry-' + [Guid]::NewGuid().ToString('N') + '.bak'), $false) }
    $temporary = $path + '.blueberry-' + [Guid]::NewGuid().ToString('N') + '.tmp'
    try {
        [IO.File]::WriteAllBytes($temporary, [byte[]]($preamble + $encoding.GetBytes($updated)))
        if ($exists) { [IO.File]::Replace($temporary, $path, [NullString]::Value) } else { [IO.File]::Move($temporary, $path) }
    } finally { if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) } }
    Write-Output "$Action`: $path"
}
