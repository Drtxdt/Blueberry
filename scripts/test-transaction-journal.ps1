[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$scratch = Join-Path (Split-Path $PSScriptRoot) ('target/journal-tests-' + [Guid]::NewGuid().ToString('N'))
$root = Join-Path $scratch 'install'
$directory = Join-Path $scratch 'transaction'
[IO.Directory]::CreateDirectory((Join-Path $root 'previous/old')) | Out-Null
. (Join-Path $PSScriptRoot 'manage-install.ps1') -Action Upgrade -FunctionsOnly -TransactionDirectory $directory
[IO.File]::WriteAllText((Join-Path $root 'install.json'), '{}')
[IO.File]::WriteAllText((Join-Path $root 'active.txt'), 'before')
[IO.File]::WriteAllText((Join-Path $root 'previous/old/history.txt'), 'immutable')
$state = [pscustomobject]@{
    path=(Join-Path $root 'install.json')
    managed_files=@('install.json','active.txt','previous/old/history.txt')
    managed_hashes=@{'active.txt'=(Get-Sha256 (Join-Path $root 'active.txt')); 'previous/old/history.txt'=(Get-Sha256 (Join-Path $root 'previous/old/history.txt'))}
}
$transaction = Measure-InstallPhase 'backup' { New-Transaction $root $state } -Files @(Get-ActiveManagedFiles -State $state).Count
if ($transaction.records.Count -ne 1) { throw 'Historical snapshot was unnecessarily backed up' }
$headerHash = Get-Sha256 $transaction.journal
Add-TransactionTouched $transaction 'active.txt'
[IO.File]::WriteAllText((Join-Path $root 'active.txt'), 'after')
for ($i=0; $i -lt 128; $i++) { Add-TransactionTouched $transaction "new/$i.txt" }
Add-TransactionTouched $transaction 'active.txt'
if ($transaction.wal_sequence -ne 129) { throw 'Duplicate paths rewrote the journal' }
if ((Get-Sha256 $transaction.journal) -ne $headerHash) { throw 'Immutable backup manifest was rewritten' }
$rejected = $false
try { Add-TransactionTouched $transaction 'previous/old/history.txt' } catch { $rejected = $true }
if (-not $rejected) { throw 'Historical snapshot was allowed to mutate' }
$data = Read-JsonHashtable $transaction.journal
$wal = Read-TransactionLog $directory $data
if ($data.touched.Count -ne 129 -or $wal.sequence -ne 129) { throw 'Write-ahead records did not replay' }
$log = Join-Path $directory 'transaction.wal'
$before = [IO.File]::ReadAllBytes($log)
[IO.File]::AppendAllText($log, '{"partial":')
$data = Read-JsonHashtable $transaction.journal
$wal = Read-TransactionLog $directory $data
if ($wal.bytes -ne $before.Length -or (Get-Item $log).Length -ne $before.Length) { throw 'Incomplete tail was not repaired' }
Restore-Transaction $transaction
if ([IO.File]::ReadAllText((Join-Path $root 'active.txt')) -ne 'before') { throw 'Active payload did not recover' }
if ([IO.File]::ReadAllText((Join-Path $root 'previous/old/history.txt')) -ne 'immutable') { throw 'Historical snapshot changed' }
Complete-Transaction $transaction 'rolled_back'
$data = Read-JsonHashtable $transaction.journal
$wal = Read-TransactionLog $directory $data
if ($data.outcome -ne 'rolled_back' -or $wal.sequence -ne 130) { throw 'Outcome did not replay' }
# A complete corrupt record is never mistaken for an interrupted final write.
[IO.File]::AppendAllText($log, "{}`n")
$rejected = $false
try { [void](Read-TransactionLog $directory (Read-JsonHashtable $transaction.journal)) } catch { $rejected = $true }
if (-not $rejected) { throw 'Corrupt complete record accepted' }
Write-Output "Transaction v2 write-ahead/recovery tests passed; evidence: $scratch"
