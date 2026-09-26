param([string]$Snapshot)
[Console]::InputEncoding=[Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$global:BlueberryEquivalenceSnapshot=$Snapshot
Set-PSReadLineKeyHandler -Chord F12 -ScriptBlock {
    $line=''; $cursor=0
    [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line,[ref]$cursor)
    [IO.File]::WriteAllText($global:BlueberryEquivalenceSnapshot, ($cursor.ToString()+"`n"+$line), [Text.UTF8Encoding]::new($false))
    [Microsoft.PowerShell.PSConsoleReadLine]::RevertLine()
}
Set-PSReadLineKeyHandler -Chord Ctrl+z -Function Undo
Set-PSReadLineKeyHandler -Chord Ctrl+y -Function Redo
Set-PSReadLineKeyHandler -Chord Alt+2 -Function DigitArgument
Set-PSReadLineKeyHandler -Chord 'Ctrl+x,Ctrl+a' -ScriptBlock { [Microsoft.PowerShell.PSConsoleReadLine]::Insert('CHORD') }
[Console]::WriteLine('EQUIVALENCE-READY')
