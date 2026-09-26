param([string]$Log, [int]$Mode)
Import-Module PSReadLine
Set-PSReadLineOption -HistorySaveStyle SaveNothing
Add-Type -TypeDefinition @'
using System;
using System.Diagnostics;
using System.IO;
public static class OutputFenceExperiment {
    public static string Log;
    public static int Mode, Sequence;
    public static void Draw() {
        string marker = "BB-FRAME-" + (++Sequence).ToString("D4");
        int x=Console.CursorLeft,y=Console.CursorTop;
        long start=Stopwatch.GetTimestamp();
        if (Mode==0) {
            Console.SetCursorPosition(0,12); Console.Write(marker);
            Console.SetCursorPosition(x,y);
        } else {
            string text="\x1b[13;1H"+marker+"\x1b["+(y+1)+";"+(x+1)+"H";
            // Diagnostic experiment only. OSC 1337 actions flush legacy
            // ConPTY's render buffer; this unknown vendor action is ignored.
            if (Mode==2) text+="\x1b]1337;BlueberryFrame\x07";
            Console.Write(text);
        }
        long end=Stopwatch.GetTimestamp();
        File.AppendAllText(Log, Sequence+","+start+","+end+","+Stopwatch.Frequency+"\n");
    }
}
'@
[OutputFenceExperiment]::Log=$Log
[OutputFenceExperiment]::Mode=$Mode
Set-PSReadLineKeyHandler -Chord F12 -ScriptBlock { [OutputFenceExperiment]::Draw() }
[Console]::WriteLine('OUTPUT-EXPERIMENT-READY')
