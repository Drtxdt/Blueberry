# Shared by the installer on Windows PowerShell 5.1 and PowerShell 7.
function Convert-BlueberryJsonNode {
    param($Node)
    switch ([string]$Node.ValueKind) {
        'Object' {
            $result = [Collections.Generic.Dictionary[string,object]]::new([StringComparer]::Ordinal)
            foreach ($p in $Node.EnumerateObject()) {
                if ($result.ContainsKey($p.Name)) { throw 'Duplicate JSON property' }
                $result.Add($p.Name, (Convert-BlueberryJsonNode $p.Value))
            }
            return $result
        }
        'Array' { $items = @($Node.EnumerateArray() | ForEach-Object { Convert-BlueberryJsonNode $_ }); return ,$items }
        'String' { return $Node.GetString() }
        'Number' { $number = [long]0; if ($Node.TryGetInt64([ref]$number)) { return $number }; return $Node.GetDecimal() }
        'True' { return $true }
        'False' { return $false }
        'Null' { return $null }
        default { throw 'Invalid JSON value' }
    }
}
function ConvertFrom-BlueberryInstallJson {
    [CmdletBinding()]
    param([Parameter(ValueFromPipeline=$true)][string]$Json)
    process {
        if ($PSVersionTable.PSVersion.Major -ge 7) {
            $document = [System.Text.Json.JsonDocument]::Parse($Json)
            try { return Convert-BlueberryJsonNode $document.RootElement } finally { $document.Dispose() }
        }
        Add-Type -AssemblyName System.Web.Extensions
        $reader = [Web.Script.Serialization.JavaScriptSerializer]::new()
        $reader.MaxJsonLength = 16777216
        return ,$reader.DeserializeObject($Json)
    }
}
function ConvertTo-BlueberryCanonicalJson {
    param([AllowNull()]$Value)
    if ($null -eq $Value) { return 'null' }
    if ($Value -is [string]) {
        $escaped = [regex]::Replace($Value, '["\\\x00-\x1f\x7f-\uffff]', {
            param($m)
            return '\u' + ([int][char]$m.Value).ToString('x4')
        })
        return '"' + $escaped + '"'
    }
    if ($Value -is [bool]) { return ([string]$Value).ToLowerInvariant() }
    if ($Value -is [pscustomobject]) {
        $properties = @{}
        foreach ($property in $Value.PSObject.Properties) { $properties[$property.Name] = $property.Value }
        return ConvertTo-BlueberryCanonicalJson $properties
    }
    if ($Value -is [Collections.IDictionary]) {
        $keys = [string[]]@($Value.Keys); [Array]::Sort($keys, [StringComparer]::Ordinal)
        $pairs = @($keys | ForEach-Object { (ConvertTo-BlueberryCanonicalJson ([string]$_)) + ':' + (ConvertTo-BlueberryCanonicalJson $Value[$_]) })
        return '{' + ($pairs -join ',') + '}'
    }
    if ($Value -is [Collections.IEnumerable]) {
        $items = @($Value | ForEach-Object { ConvertTo-BlueberryCanonicalJson $_ })
        return '[' + ($items -join ',') + ']'
    }
    if ($Value -is [IFormattable]) {
        try { return $Value.ToString($null, [Globalization.CultureInfo]::InvariantCulture) } catch { throw ('Cannot serialize metadata type ' + $Value.GetType().FullName + ': ' + $Value) }
    }
    throw 'Unsupported installation metadata type'
}
function Move-BlueberryFile {
    param([string]$Source, [string]$Destination)
    if ([IO.File]::Exists($Destination)) { [IO.File]::Replace($Source, $Destination, [NullString]::Value) }
    else { [IO.File]::Move($Source, $Destination) }
}
function Get-BlueberryRelativePath {
    param([string]$Root, [string]$Path)
    $rootPath = [IO.Path]::GetFullPath($Root).TrimEnd('\','/') + [IO.Path]::DirectorySeparatorChar
    $fullPath = [IO.Path]::GetFullPath($Path)
    if (-not $fullPath.StartsWith($rootPath, [StringComparison]::OrdinalIgnoreCase)) { throw 'File outside installation stage' }
    return $fullPath.Substring($rootPath.Length)
}
