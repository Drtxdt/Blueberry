# Metadata only. Never change the interactive runspace or import a module.
$ErrorActionPreference = 'Stop'
$PSModuleAutoLoadingPreference = 'None'
try {
    $before = @(Microsoft.PowerShell.Core\Get-Module).Count
    $roots = @([IO.Path]::Combine($PSHOME, 'Modules'))
    $windowsModules = [IO.Path]::Combine($env:windir, 'System32\WindowsPowerShell\v1.0\Modules')
    foreach ($path in ($env:PSModulePath -split ';')) {
        if ($path.TrimEnd('\') -ieq $windowsModules.TrimEnd('\') -and $roots -notcontains $path) { $roots += $path }
    }
    foreach ($root in $roots) {
        if (-not [IO.Directory]::Exists($root)) { continue }
        foreach ($module in @(Microsoft.PowerShell.Core\Get-Module -ListAvailable -Name ([IO.Path]::Combine($root, '*')))) {
            if ($module.CompatiblePSEditions.Count -gt 0 -and $module.CompatiblePSEditions -notcontains $PSEdition) { continue }
            foreach ($command in $module.ExportedCommands.Values) {
                $kind = $command.CommandType.ToString()
                if ($kind -notin @('Alias', 'Function', 'Cmdlet')) { continue }
                $name = [BitConverter]::ToString([Text.Encoding]::UTF8.GetBytes($command.Name)).Replace('-', '')
                [Console]::Out.WriteLine("C`t$name`t$kind")
            }
        }
    }
    if (@(Microsoft.PowerShell.Core\Get-Module).Count -ne $before) { throw 'Metadata discovery imported a module' }
    [Console]::Out.WriteLine('COMPLETE')
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
