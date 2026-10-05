// Give unchanged upstream tests their required layout on
// an isolated desktop. Never switch the user's input desktop or keyboard layout.
using System;
using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Forms;

class EditorTestDesktop {
    [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern IntPtr CreateDesktop(string name, IntPtr device, IntPtr mode, uint flags, uint access, IntPtr security);
    [DllImport("user32.dll")] static extern bool CloseDesktop(IntPtr desktop);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr LoadKeyboardLayout(string name, uint flags);
    [DllImport("user32.dll")] static extern IntPtr GetKeyboardLayout(uint thread);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern IntPtr CreateFile(string name,uint access,uint share,IntPtr security,uint disposition,uint flags,IntPtr template);
    [DllImport("kernel32.dll")] static extern bool SetHandleInformation(IntPtr handle,uint mask,uint flags);
    [DllImport("kernel32.dll")] static extern IntPtr GetStdHandle(int kind);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll")] static extern uint WaitForSingleObject(IntPtr handle,uint timeout);
    [DllImport("kernel32.dll")] static extern bool GetExitCodeProcess(IntPtr handle,out uint code);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern bool CreateProcess(string application,StringBuilder command,IntPtr processSecurity,IntPtr threadSecurity,bool inherit,uint flags,IntPtr environment,string directory,ref Startup startup,out ProcessInfo process);
    [StructLayout(LayoutKind.Sequential,CharSet=CharSet.Unicode)] struct Startup {
        public int size; public string reserved,desktop,title;
        public uint x,y,width,height,columns,rows,fill,flags;
        public short show,reservedCount; public IntPtr reservedBytes,input,output,error;
    }
    [StructLayout(LayoutKind.Sequential)] struct ProcessInfo {public IntPtr process,thread;public uint pid,tid;}
    static string Quote(string value) {
        var result=new StringBuilder("\""); int slashes=0;
        foreach(char c in value) {
            if(c=='\\'){slashes++;continue;}
            result.Append('\\',c=='"'?slashes*2+1:slashes); slashes=0; result.Append(c);
        }
        return result.Append('\\',slashes*2).Append('"').ToString();
    }
    [STAThread] static int Main(string[] args) {
        if(args.Length==0)return 2;
        if(args[0]!="--child") {
            string isolated="Blueberry.Upstream."+Guid.NewGuid().ToString("N");
            IntPtr handle=CreateDesktop(isolated,IntPtr.Zero,IntPtr.Zero,0,0x1ff,IntPtr.Zero);
            if(handle==IntPtr.Zero){Console.Error.WriteLine(new Win32Exception());return 3;}
            try {
                var line=new StringBuilder(Quote(System.Reflection.Assembly.GetExecutingAssembly().Location)+" --child "+Quote(isolated));
                foreach(string arg in args)line.Append(' ').Append(Quote(arg));
                var startup=new Startup {size=Marshal.SizeOf(typeof(Startup)),desktop=isolated,flags=0x101,show=0,input=GetStdHandle(-10),output=GetStdHandle(-11),error=GetStdHandle(-12)};
                ProcessInfo child;
                if(!CreateProcess(null,line,IntPtr.Zero,IntPtr.Zero,true,0x00000010,IntPtr.Zero,null,ref startup,out child))throw new Win32Exception();
                CloseHandle(child.thread); WaitForSingleObject(child.process,0xffffffff);
                uint code; GetExitCodeProcess(child.process,out code); CloseHandle(child.process); return (int)code;
            } finally {CloseDesktop(handle);}
        }
        // The CLR may create apartment windows before Main. Start this process
        // on its private desktop instead of moving an initialized STA thread.
        string name=args[1];
        var commandArgs=new string[args.Length-2]; Array.Copy(args,2,commandArgs,0,commandArgs.Length);
        int result=4;
            using(var window=new Form()) {
                window.Text="Blueberry upstream test layout fixture";
                window.Width=320; window.Height=160;
                window.Shown+=(sender,eventArgs)=>{
                    try {
                        string layout=Environment.GetEnvironmentVariable("BLUEBERRY_UPSTREAM_LAYOUT") ?? "00000409";
                        if(LoadKeyboardLayout(layout,1)==IntPtr.Zero)throw new Win32Exception();
                        SetForegroundWindow(window.Handle);
                        if((GetKeyboardLayout(0).ToInt64()&0xffff)!=Convert.ToInt32(layout,16))
                            throw new InvalidOperationException("Isolated keyboard layout unavailable; tests not qualified");
                        // On an inactive desktop GetForegroundWindow can be
                        // null. Upstream then queries its current thread's HKL;
                        // its unchanged guard must independently accept it.
                        Console.WriteLine("Upstream fixture: isolated desktop, layout="+layout+"; foreground="+GetForegroundWindow());
                        var line=new StringBuilder();
                        foreach(string arg in commandArgs){if(line.Length!=0)line.Append(' ');line.Append(Quote(arg));}
                        IntPtr consoleInput=CreateFile("CONIN$",0xc0000000,3,IntPtr.Zero,3,0,IntPtr.Zero);
                        if(consoleInput==new IntPtr(-1))throw new Win32Exception();
                        if(!SetHandleInformation(consoleInput,1,1))throw new Win32Exception();
                        var startup=new Startup {size=Marshal.SizeOf(typeof(Startup)),desktop=name,flags=0x101,show=0,input=consoleInput,output=GetStdHandle(-11),error=GetStdHandle(-12)};
                        ProcessInfo child;
                        try {
                            if(!CreateProcess(null,line,IntPtr.Zero,IntPtr.Zero,true,0,IntPtr.Zero,null,ref startup,out child))throw new Win32Exception();
                        } finally {CloseHandle(consoleInput);}
                        CloseHandle(child.thread);
                        new Thread(()=>{
                            WaitForSingleObject(child.process,0xffffffff);
                            uint exit; GetExitCodeProcess(child.process,out exit); CloseHandle(child.process);
                            result=(int)exit; window.BeginInvoke(new Action(window.Close));
                        }) {IsBackground=true}.Start();
                    } catch(Exception error){Console.Error.WriteLine(error);window.Close();}
                };
                Application.Run(window);
            }
        return result;
    }
}
