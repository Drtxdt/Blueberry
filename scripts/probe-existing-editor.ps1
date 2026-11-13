# Run in a disposable shell. A profile defining any PSReadLine binding can
# load the stock assembly before the Blueberry startup block runs.
[CmdletBinding()]
param([Parameter(Mandatory)][string]$StockManifest,
      [Parameter(Mandatory)][string]$CandidateManifest)
$ErrorActionPreference = 'Stop'
Import-Module $StockManifest -ErrorAction Stop
Set-PSReadLineKeyHandler -Chord F8 -Function HistorySearchBackward
$kind = [Microsoft.PowerShell.PSConsoleReadLine]
$original = $kind.Assembly.Location
$originalBinding = (Get-PSReadLineKeyHandler -Bound | Where-Object Key -eq F8).Function
$timer = [Diagnostics.Stopwatch]::StartNew()
$failure = $null
try { Import-Module $CandidateManifest -ErrorAction Stop } catch { $failure = $_.Exception.Message }
$timer.Stop()
$active = [Microsoft.PowerShell.PSConsoleReadLine]
$binding = (Get-PSReadLineKeyHandler -Bound | Where-Object Key -eq F8).Function
[ordered]@{ shell=$PSVersionTable.PSVersion.ToString(); original=$original; active=$active.Assembly.Location
    existing_hook_compatible=($null -ne $active.GetProperty('EditorIntegrationVersion'))
    binding_retained=([bool]$originalBinding -and $binding -eq $originalBinding)
    import_error=$failure; attempted_import_ms=$timer.Elapsed.TotalMilliseconds
    diagnostic_only=$true } | ConvertTo-Json
