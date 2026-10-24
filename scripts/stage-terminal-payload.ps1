[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$PackagePath,
    [Parameter(Mandatory = $true)][string]$Destination
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$pin = (Get-Content -LiteralPath (Join-Path $PSScriptRoot 'terminal-dependency.json') -Raw | ConvertFrom-Json).terminal_dependency
if ((Get-FileHash -LiteralPath $PackagePath -Algorithm SHA256).Hash -ne $pin.package_sha256) {
    throw 'Companion Terminal original artifact SHA-256 mismatch'
}
if (Test-Path -LiteralPath $Destination) { throw 'Terminal staging destination already exists' }
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [IO.Compression.ZipFile]::OpenRead($PackagePath)
$prefix = 'bin/x64/Release/WindowsTerminal/'
$seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
try {
    # Keep original build logs and PDBs in the separate release artifact;
    # the managed install contains the byte-identical portable runtime.
    $entries = @($archive.Entries | Where-Object { $_.FullName.StartsWith($prefix, [StringComparison]::Ordinal) -and -not $_.FullName.EndsWith('/') })
    foreach ($entry in $entries) {
        $relative = $entry.FullName.Substring($prefix.Length)
        if ($relative -match '(^|/)\.{1,2}(/|$)|[\\:]' -or $relative.StartsWith('/') -or
            -not $seen.Add($relative) -or (($entry.ExternalAttributes -shr 16) -band 0xF000) -eq 0xA000) {
            throw 'Unsafe companion runtime member'
        }
        if ($relative.StartsWith('settings/', [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Companion archive must not provide user settings'
        }
    }
    foreach ($required in @('WindowsTerminal.exe', 'Microsoft.Terminal.Settings.Model.dll', '.portable')) {
        if (-not $seen.Contains($required)) { throw "Missing companion runtime member: $required" }
    }
    [IO.Directory]::CreateDirectory($Destination) | Out-Null
    foreach ($entry in $entries) {
        $relative = $entry.FullName.Substring($prefix.Length)
        # Build stamp is not runtime data. The portable marker is ours and
        # nonempty so the existing managed-file transaction can own it.
        if ($relative -eq 'stamp' -or $relative -eq '.portable' -or $relative.EndsWith('.pdb', [StringComparison]::OrdinalIgnoreCase)) { continue }
        if ($entry.Length -le 0) { throw "Empty companion runtime member: $relative" }
        $target = Join-Path $Destination $relative
        [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($target)) | Out-Null
        [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $target, $false)
    }
    [IO.File]::WriteAllText((Join-Path $Destination '.portable'), "Blueberry managed portable Terminal`n", [Text.UTF8Encoding]::new($false))
    if ((Get-FileHash -LiteralPath (Join-Path $Destination 'WindowsTerminal.exe') -Algorithm SHA256).Hash -ne $pin.executable_sha256) {
        throw 'Companion Terminal executable SHA-256 mismatch'
    }
    foreach ($license in $pin.licenses.PSObject.Properties) {
        $path = Join-Path $Destination $license.Name.Substring($prefix.Length)
        if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $license.Value) {
            throw 'Companion Terminal license SHA-256 mismatch'
        }
    }
    $pin
}
finally { $archive.Dispose() }
