# Read-only accessibility capability probe for this process's own console.
# The coloured marker is diagnostic output, not a product/menu implementation.
[CmdletBinding()]
param([Parameter(Mandatory)][string]$OutputPath)
$ErrorActionPreference='Stop'
if (Test-Path -LiteralPath $OutputPath) { throw 'Refusing to overwrite console capability evidence' }
$report=[ordered]@{schema=2;diagnostic_only=$true;source='own_console_uia';shell=$PSVersionTable.PSVersion.ToString();supported=$false}
try {
    [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
    $escape=[char]27
    $marker='BB_RGB_DIAG_'+[Guid]::NewGuid().ToString('N')+[char]0x4E2D+[char]::ConvertFromUtf32(0x1F680)+'e'+[char]0x301
    [Console]::Write("${escape}[38;2;123;45;67m${escape}[48;2;23;89;145m$marker${escape}[0m`r`n")
    Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes,WindowsBase
    $references=@([Windows.Automation.AutomationElement].Assembly.Location,[Windows.Automation.AutomationPattern].Assembly.Location,[System.Windows.Rect].Assembly.Location)
    if ($PSVersionTable.PSEdition -eq 'Core') { $references+=@(Get-ChildItem -LiteralPath (Join-Path $PSHOME 'ref') -Filter '*.dll' | ForEach-Object FullName) }
    Add-Type -ReferencedAssemblies $references -TypeDefinition @'
using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading;
using System.Windows.Automation;
using System.Windows.Automation.Text;
public static class BBConsoleAttributeProbe {
    [DllImport("kernel32.dll")] public static extern IntPtr GetConsoleWindow();
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
    public sealed class Glyph { public string RawText; public double[] Bounds; public int Foreground=-1, Background=-1; }
    public sealed class Result { public bool Completed; public string Source, Text, Error; public int Foreground=-1, Background=-1; public Glyph[] Glyphs; public double Milliseconds; }
    public static Result Read(IntPtr window, string marker, bool terminal) {
        var result=new Result { Source=terminal ? "foreground_terminal_uia":"own_console_uia" };
        var thread=new Thread(delegate() {
            var timing=Stopwatch.StartNew();
            try {
                AutomationElement element;
                if(terminal) {
                    uint pid; GetWindowThreadProcessId(GetForegroundWindow(),out pid);
                    using(var process=Process.GetProcessById((int)pid))
                        if(!String.Equals(process.ProcessName,"WindowsTerminal",StringComparison.OrdinalIgnoreCase))
                            throw new InvalidOperationException("Foreground window is not Windows Terminal");
                    element=AutomationElement.FocusedElement;
                    if(element.Current.ProcessId!=(int)pid) throw new InvalidOperationException("Focused provider belongs to another process");
                } else element=AutomationElement.FromHandle(window);
                var pattern=(TextPattern)element.GetCurrentPattern(TextPattern.Pattern);
                // No unrelated text is returned: only our unique marker range.
                var range=pattern.DocumentRange.FindText(marker,false,false);
                if(range==null) throw new InvalidOperationException("Own marker not available through text pattern");
                result.Text=range.GetText(-1);
                object fg=range.GetAttributeValue(TextPattern.ForegroundColorAttribute);
                object bg=range.GetAttributeValue(TextPattern.BackgroundColorAttribute);
                if(fg is int) result.Foreground=(int)fg;
                if(bg is int) result.Background=(int)bg;
                // Character ranges expose grapheme geometry independently of
                // UTF-16 string length. Keep raw text: some Terminal versions
                // include the cell at the otherwise exclusive end endpoint.
                var glyphs=new System.Collections.Generic.List<Glyph>();
                var position=range.Clone();
                position.MoveEndpointByRange(TextPatternRangeEndpoint.End,position,TextPatternRangeEndpoint.Start);
                for(int count=0;count<marker.Length+1;count++) {
                    if(position.CompareEndpoints(TextPatternRangeEndpoint.Start,range,TextPatternRangeEndpoint.End)>=0) break;
                    var glyph=position.Clone(); glyph.ExpandToEnclosingUnit(TextUnit.Character);
                    var rectangles=glyph.GetBoundingRectangles();
                    var bounds=new double[rectangles.Length*4];
                    for(int i=0;i<rectangles.Length;i++) { bounds[i*4]=rectangles[i].Left; bounds[i*4+1]=rectangles[i].Top; bounds[i*4+2]=rectangles[i].Width; bounds[i*4+3]=rectangles[i].Height; }
                    fg=glyph.GetAttributeValue(TextPattern.ForegroundColorAttribute);
                    bg=glyph.GetAttributeValue(TextPattern.BackgroundColorAttribute);
                    glyphs.Add(new Glyph { RawText=glyph.GetText(-1), Bounds=bounds, Foreground=fg is int ? (int)fg : -1, Background=bg is int ? (int)bg : -1 });
                    if(glyph.CompareEndpoints(TextPatternRangeEndpoint.End,position,TextPatternRangeEndpoint.Start)<=0) throw new InvalidOperationException("Character range did not advance");
                    position.MoveEndpointByRange(TextPatternRangeEndpoint.Start,glyph,TextPatternRangeEndpoint.End);
                    position.MoveEndpointByRange(TextPatternRangeEndpoint.End,position,TextPatternRangeEndpoint.Start);
                }
                result.Glyphs=glyphs.ToArray();
            } catch(Exception error) { result.Error=error.Message; }
            result.Milliseconds=timing.Elapsed.TotalMilliseconds;
            result.Completed=true;
        });
        thread.IsBackground=true; thread.SetApartmentState(ApartmentState.MTA); thread.Start();
        if(!thread.Join(2000)) return new Result { Source=result.Source, Error="UIA capability call exceeded 2 seconds" };
        return result;
    }
}
'@
    $window=[BBConsoleAttributeProbe]::GetConsoleWindow()
    $report.console_window=$window.ToInt64()
    if ($window -eq [IntPtr]::Zero) { throw 'No native console window handle' }
    $probe=[BBConsoleAttributeProbe]::Read($window,$marker,$false)
    $report.native_probe=$probe
    if (-not $probe.Completed -or $probe.Error) {
        $probe=[BBConsoleAttributeProbe]::Read($window,$marker,$true)
        $report.terminal_probe=$probe
    }
    $report.source=$probe.Source
    $report.text=$probe.Text
    $report.foreground=$probe.Foreground
    $report.background=$probe.Background
    if ($probe.Error) { $report.error=$probe.Error }
    $report.expected_foreground=123+45*256+67*65536
    $report.expected_background=23+89*256+145*65536
    $report.range_text_exact=($report.text -ceq $marker)
    $report.marker_text_preserved=($null -ne $report.text -and $report.text.StartsWith($marker,[StringComparison]::Ordinal))
    $report.rgb_supported=($report.foreground -eq $report.expected_foreground -and $report.background -eq $report.expected_background)
    # This is a colour/own-marker capability result, not proof of a complete
    # viewport snapshot or a resize restoration. Keep exact-range status separate.
    $report.supported=($probe.Completed -and -not $probe.Error -and $report.marker_text_preserved -and $report.rgb_supported)
    if ($report.supported) {
        $wrapMarker='BB_WRAP_DIAG_'+[Guid]::NewGuid().ToString('N')+('w'*([Console]::WindowWidth+8))+[char]0x4E2D+[char]::ConvertFromUtf32(0x1F680)+'e'+[char]0x301+'_END'
        [Console]::Write("${escape}[38;2;123;45;67m${escape}[48;2;23;89;145m$wrapMarker${escape}[0m`r`n")
        $report.wrap_probe=[BBConsoleAttributeProbe]::Read($window,$wrapMarker,($probe.Source -eq 'foreground_terminal_uia'))
        $report.wrap_marker=$wrapMarker
        $report.console_columns=[Console]::WindowWidth
    }
} catch {
    $report.error=$_.Exception.Message
    $report.error_type=$_.Exception.GetType().FullName
}
$temporary=$OutputPath+'.tmp'
[IO.File]::WriteAllText($temporary,($report|ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
[IO.File]::Move($temporary,$OutputPath)
if (-not $report.supported) { exit 1 }
