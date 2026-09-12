param(
    [Parameter(Mandatory)][string]$InputPath,
    [Parameter(Mandatory)][string]$OutputPath
)
$ErrorActionPreference = 'Stop'
$report = Get-Content -LiteralPath $InputPath -Raw | ConvertFrom-Json
function Update-SampleStatistics([object]$value) {
    if ($null -eq $value -or $value -is [string] -or $value -is [ValueType]) { return }
    if ($value -is [System.Collections.IList]) {
        foreach ($item in $value) { Update-SampleStatistics $item }
        return
    }
    $samplesProperty = $value.PSObject.Properties['samples']
    if ($null -ne $samplesProperty -and @($samplesProperty.Value).Count -gt 0) {
        $samples = @($samplesProperty.Value | Sort-Object)
        $count = $samples.Count
        $median = ([double]$samples[[Math]::Floor(($count - 1) / 2)] + [double]$samples[[Math]::Floor($count / 2)]) / 2
        $p95 = $samples[[int][Math]::Ceiling($count * 0.95) - 1]
        $value | Add-Member -NotePropertyName median -NotePropertyValue $median -Force
        $value | Add-Member -NotePropertyName p95 -NotePropertyValue $p95 -Force
    }
    foreach ($property in @($value.PSObject.Properties)) {
        if ($property.Name -ne 'samples') { Update-SampleStatistics $property.Value }
    }
}
Update-SampleStatistics $report
$report | Add-Member -NotePropertyName recomputed_statistics -NotePropertyValue 'Standard median; nearest-rank p95. Original measured samples unchanged.' -Force
$report | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $OutputPath -Encoding utf8NoBOM
