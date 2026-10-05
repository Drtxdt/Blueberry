// VSTest environment setup only. No PSReadLine test source or guard is changed.
using System;
using System.Runtime.InteropServices;
using Microsoft.VisualStudio.TestPlatform.ObjectModel.InProcDataCollector;
using Microsoft.VisualStudio.TestPlatform.ObjectModel.DataCollection;
using Microsoft.VisualStudio.TestPlatform.ObjectModel.DataCollector.InProcDataCollector;

public class EditorTestLayout : InProcDataCollection {
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern IntPtr LoadKeyboardLayout(string name,uint flags);
    [DllImport("user32.dll")] static extern IntPtr GetKeyboardLayout(uint thread);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern IntPtr CreateFile(string name,uint access,uint share,IntPtr security,uint disposition,uint flags,IntPtr template);
    [DllImport("kernel32.dll")] static extern bool SetStdHandle(int kind,IntPtr handle);
    [DllImport("kernel32.dll")] static extern IntPtr GetStdHandle(int kind);
    [DllImport("kernel32.dll")] static extern bool GetConsoleMode(IntPtr handle,out uint mode);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    static IntPtr input, previousInput;
    static void Activate() {
        string layout=Environment.GetEnvironmentVariable("BLUEBERRY_UPSTREAM_LAYOUT") ?? "00000409";
        if(LoadKeyboardLayout(layout,0x101)==IntPtr.Zero || (GetKeyboardLayout(0).ToInt64()&0xffff)!=Convert.ToInt32(layout,16))
            throw new InvalidOperationException("Cannot activate required keyboard layout in isolated test host");
    }
    public void Initialize(IDataCollectionSink sink) { }
    public void TestSessionStart(TestSessionStartArgs args) {
        // VSTest redirects stdin even when its parent owns a console. PSReadLine
        // then selects the ANSI char map, invalidating upstream ConsoleKeyInfo
        // fixtures. Restore the real console handle before editor initialization.
        previousInput=GetStdHandle(-10);
        input=CreateFile("CONIN$",0xc0000000,3,IntPtr.Zero,3,0,IntPtr.Zero);
        uint mode;
        if(input==new IntPtr(-1) || !GetConsoleMode(input,out mode) || !SetStdHandle(-10,input))
            throw new InvalidOperationException("Test host requires console input");
        Activate();
    }
    public void TestCaseStart(TestCaseStartArgs args) { Activate(); }
    public void TestCaseEnd(TestCaseEndArgs args) { }
    public void TestSessionEnd(TestSessionEndArgs args) {
        if(input!=IntPtr.Zero && input!=new IntPtr(-1)) {
            SetStdHandle(-10,previousInput);
            CloseHandle(input);
            input=IntPtr.Zero;
        }
    }
}
