// Internal companion-Terminal channel. No key records or public CLI additions.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.IO.Pipes;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using Microsoft.Win32.SafeHandles;

namespace Blueberry.Direct {
    internal sealed class NativeOverlay : IDisposable {
        readonly string owner = Guid.NewGuid().ToString("N");
        readonly object state = new object();
        readonly AutoResetEvent changed = new AutoResetEvent(false);
        readonly NamedPipeServerStream pipe;
        readonly Thread reader;
        bool ready, stopped, visible, resolved;
        long generation, expected;
        long closing;
        bool closeResolved, closeRestored;
        int disposed;
        public string Error { get; private set; }
        internal bool ReaderStopped { get { return !reader.IsAlive; } }

        [StructLayout(LayoutKind.Sequential)] struct SecurityAttributes {
            public int Length; public IntPtr Descriptor; public int Inherit;
        }
        [DllImport("advapi32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        static extern bool ConvertStringSecurityDescriptorToSecurityDescriptorW(string text, uint revision, out IntPtr descriptor, IntPtr size);
        [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr memory);
        [DllImport("kernel32.dll")] static extern uint GetCurrentProcessId();
        [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        static extern SafePipeHandle CreateNamedPipeW(string name, uint access, uint mode, uint instances,
            uint outputSize, uint inputSize, uint timeout, ref SecurityAttributes attributes);

        internal NativeOverlay() {
            IntPtr descriptor;
            string sid;
            using(var identity=System.Security.Principal.WindowsIdentity.GetCurrent()) sid=identity.User.Value;
            if (!ConvertStringSecurityDescriptorToSecurityDescriptorW("D:P(A;;GA;;;"+sid+")", 1, out descriptor, IntPtr.Zero))
                throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
            SafePipeHandle handle = null;
            try {
                var attributes = new SecurityAttributes { Length=Marshal.SizeOf(typeof(SecurityAttributes)), Descriptor=descriptor };
                handle = CreateNamedPipeW("\\\\.\\pipe\\Blueberry.TerminalOverlay."+owner,
                    0x40080003, 8, 1, 4096, 4096, 0, ref attributes);
                if (handle.IsInvalid) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
                pipe = new NamedPipeServerStream(PipeDirection.InOut, true, false, handle);
                handle = null;
            } finally { if(handle!=null) handle.Dispose(); LocalFree(descriptor); }
            reader = new Thread(Read) { IsBackground=true, Name="Blueberry Terminal acknowledgements" };
            reader.Start();
        }

        internal bool Connect() {
            Console.Write("\x1b]7775;channel;"+GetCurrentProcessId()+";"+owner+"\x07");
            long deadline=Stopwatch.GetTimestamp()+Stopwatch.Frequency/2;
            while(Stopwatch.GetTimestamp()<deadline) {
                lock(state) { if(ready) return true; if(stopped) return false; }
                changed.WaitOne(10);
            }
            Error="Companion Terminal handshake timed out";
            return false;
        }

        void Read() {
            try {
                pipe.WaitForConnection();
                using(var input = new StreamReader(pipe,new UTF8Encoding(false,true),false,1024,true)) {
                    var text=new StringBuilder();
                    while(true) {
                        int next=input.Read(); if(next<0) throw new EndOfStreamException("Companion Terminal disconnected");
                        if(next!='\n') { if(next<32 || next>126 || text.Length>=192) throw new InvalidDataException("Invalid Terminal acknowledgement"); text.Append((char)next); continue; }
                        string value=text.ToString(); text.Clear();
                        lock(state) {
                            if(!ready) { if(value!="ready;4;"+owner) throw new InvalidDataException("Terminal channel version or owner mismatch"); ready=true; }
                            else {
                                var parts=value.Split(';'); long id;
                                if(parts.Length<2 || !Int64.TryParse(parts[1],out id) || id<=0) throw new InvalidDataException("Invalid Terminal generation");
                                if(parts[0]=="frame" && parts.Length==3 && (parts[2]=="presented" || parts[2]=="rejected")) {
                                    if(id==expected) { visible=parts[2]=="presented"; resolved=true; }
                                } else if(parts[0]=="retired" && parts.Length==2) { if(id==expected) visible=false; }
                                else if(parts[0]=="closed" && parts.Length==3 && (parts[2]=="restored" || parts[2]=="absent")) {
                                    if(id==closing) { closeResolved=true; closeRestored=parts[2]=="restored"; }
                                }
                                else throw new InvalidDataException("Invalid Terminal result");
                            }
                        }
                        changed.Set();
                    }
                }
            } catch(Exception error) {
                lock(state) { if(!stopped) Error=error.GetBaseException().Message; stopped=true; visible=false; }
                changed.Set();
            }
        }

        internal bool Paint(List<object> lines, int top, int width) {
            var text=new StringBuilder();
            for(int i=0;i<lines.Count;i++) { if(i>0) text.Append('\n'); text.Append(lines[i] as string).Append("\x1b[0m"); }
            if(text.Length==0 || text.Length>65536) { Error="Native menu frame exceeds bounded payload"; return false; }
            const string digits="0123456789abcdef";
            var hex=new char[text.Length*4];
            for(int i=0;i<text.Length;i++) { int c=text[i]; for(int n=0;n<4;n++) hex[i*4+n]=digits[(c>>(12-4*n))&15]; }
            long id;
            lock(state) { if(!ready || stopped) return false; id=++generation; expected=id; visible=false; resolved=false; closing=0; }
            Console.Write("\x1b]7775;native-frame;"+owner+";"+id+";"+top+";0;"+lines.Count+";"+width+";"+new string(hex)+"\x07");
            long deadline=Stopwatch.GetTimestamp()+Stopwatch.Frequency/4;
            while(Stopwatch.GetTimestamp()<deadline) {
                lock(state) { if(stopped || expected!=id) return false; if(resolved) return visible; }
                changed.WaitOne(5);
            }
            Close(); Error="Companion Terminal did not confirm menu submission";
            return false;
        }

        // A frame remains eligible only if it was still present immediately
        // before the editor's own synchronous pre-key restoration.
        internal bool Close(out bool hadFrame) {
            long id; bool wasVisible;
            lock(state) { id=expected; expected=0; wasVisible=visible && !stopped; visible=false; hadFrame=id!=0; closing=id; closeResolved=false; closeRestored=false; }
            if(id==0) return false;
            Console.Write("\x1b]7775;native-close;"+owner+";"+id+"\x07");
            long deadline=Stopwatch.GetTimestamp()+Stopwatch.Frequency/4;
            while(Stopwatch.GetTimestamp()<deadline) {
                lock(state) { if(stopped || closing!=id) return false; if(closeResolved) return wasVisible && closeRestored; }
                changed.WaitOne(5);
            }
            Error="Companion Terminal did not confirm frame retirement";
            return false;
        }
        internal void Close() { bool ignored; Close(out ignored); }
        public void Dispose() {
            if(Interlocked.Exchange(ref disposed,1)!=0) return;
            lock(state) { stopped=true; visible=false; }
            pipe.Dispose();
            if(reader!=Thread.CurrentThread && reader.Join(2000)) changed.Dispose();
            else if(reader!=Thread.CurrentThread) Error="Terminal acknowledgement reader did not stop";
            // A still-unwinding reader owns its event until process exit.
        }
    }
}
