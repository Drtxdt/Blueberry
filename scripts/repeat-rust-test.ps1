[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Target,
    [Parameter(Mandatory)][string]$Test,
    [Parameter(Mandatory)][ValidateRange(1,1000)][int]$Count
)
$ErrorActionPreference='Stop'
# Resolve the executable from this compilation, never from a wildcard that
# could select an older cached binary. Repetitions do not re-run build.rs.
$messages=@(cargo test --locked --test $Target --no-run --message-format=json | ForEach-Object { $_ | ConvertFrom-Json })
if ($LASTEXITCODE -ne 0) { throw "Cannot build $Target" }
$artifacts=@($messages | Where-Object { $_.reason -eq 'compiler-artifact' -and $_.target.name -eq $Target -and $_.executable })
if ($artifacts.Count -ne 1) { throw "Expected one executable artifact for $Target" }
$binary=$artifacts[0].executable
$hash=(Get-FileHash -LiteralPath $binary).Hash
Write-Output "Repeated test executable: $binary SHA256=$hash"
for ($sample=1; $sample -le $Count; $sample++) {
    Write-Output "Repetition $sample / $Count"
    & $binary $Test --exact --nocapture
    if ($LASTEXITCODE -ne 0) { throw "$Target/$Test repetition $sample failed; retained evidence must be inspected" }
}
if ((Get-FileHash -LiteralPath $binary).Hash -ne $hash) { throw 'Repeated test executable changed' }
