// Real local pipes, no console input or user terminal/profile.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.IO.Pipes;
using System.Text;
using System.Reflection;
using System.Threading;
using Blueberry.Direct;

class NativeOverlayClientTests {
    sealed class Terminal : TextWriter {
        NamedPipeClientStream pipe;
        internal bool Reject;
        internal long LastFrame;
        bool active;
        public override Encoding Encoding { get { return Encoding.UTF8; } }
        internal void Reply(string text) {
            byte[] bytes=Encoding.ASCII.GetBytes(text+"\n"); pipe.Write(bytes,0,bytes.Length); pipe.Flush();
        }
        public override void Write(string value) {
            if(!value.StartsWith("\x1b]7775;",StringComparison.Ordinal)) throw new Exception("Unexpected terminal output");
            string[] parts=value.Substring(7).TrimEnd('\x07').Split(';');
            if(parts[0]=="channel") {
                pipe=new NamedPipeClientStream(".","Blueberry.TerminalOverlay."+parts[2],PipeDirection.InOut);
                pipe.Connect(1000); Reply("ready;3;"+parts[2]);
            } else if(parts[0]=="native-frame") {
                LastFrame=Int64.Parse(parts[2]); active=!Reject; Reply("frame;"+LastFrame+";"+(Reject ? "rejected" : "presented"));
            } else if(parts[0]=="native-close") {
                bool restored=active && Int64.Parse(parts[2])==LastFrame;
                if(restored) { active=false; Reply("retired;"+parts[2]); }
                Reply("closed;"+parts[2]+";"+(restored ? "restored" : "absent"));
            }
            else throw new Exception("Unexpected operation");
        }
        internal void Disconnect() { if(pipe!=null) { pipe.Dispose(); pipe=null; } }
        internal void RetireWithoutNotification() { active=false; }
        protected override void Dispose(bool disposing) { Disconnect(); base.Dispose(disposing); }
    }
    static void Check(bool condition,string message) { if(!condition) throw new Exception(message); }
    static void BridgeEof(NativeOverlay overlay) {
        string name="blueberry-test-eof-"+Guid.NewGuid().ToString("N");
        using(var server=new NamedPipeServerStream(name,PipeDirection.InOut,1,PipeTransmissionMode.Byte,PipeOptions.Asynchronous))
        using(var client=new NamedPipeClientStream(".",name,PipeDirection.InOut)) {
            var waiting=server.BeginWaitForConnection(null,null);
            client.Connect(1000); server.EndWaitForConnection(waiting);
            const BindingFlags flags=BindingFlags.Static|BindingFlags.NonPublic;
            Type bridge=typeof(Bridge);
            bridge.GetField("nativeOverlay",flags).SetValue(null,overlay);
            bridge.GetField("pipe",flags).SetValue(null,client);
            bridge.GetField("connected",flags).SetValue(null,true);
            var worker=new Thread(delegate() { bridge.GetMethod("ReadFrames",flags).Invoke(null,null); }) { IsBackground=true };
            worker.Start();
            server.Dispose();
            Check(worker.Join(3000),"Bridge EOF worker did not terminate");
            Check(overlay.ReaderStopped,"Bridge EOF left native channel alive without editor refresh");
            bridge.GetField("nativeOverlay",flags).SetValue(null,null);
            bridge.GetField("pipe",flags).SetValue(null,null);
        }
    }
    public static int Main() {
        var original=Console.Out;
        try {
            var timer=Stopwatch.StartNew();
            var unconnected=new NativeOverlay();
            unconnected.Dispose();
            Check(unconnected.ReaderStopped,"Unconnected reader survived disposal");
            Check(timer.ElapsedMilliseconds<2500,"Unconnected reader did not cancel promptly");
            using(var terminal=new Terminal()) {
                Console.SetOut(terminal);
                using(var overlay=new NativeOverlay()) {
                    Check(overlay.Connect(),"Handshake failed: "+overlay.Error);
                    var lines=new List<object>{"中文🚀é \x1b[38;2;1;2;3mRGB"};
                    Check(overlay.Paint(lines,0,20),"Valid acknowledgement not observed");
                    bool had;
                    Check(overlay.Close(out had) && had,"Visible frame lost its pre-key eligibility");
                    Check(!overlay.Close(out had) && !had,"Duplicate close retained a generation");
                    Check(overlay.Paint(lines,0,20),"Frame before external retirement was not confirmed");
                    terminal.RetireWithoutNotification();
                    Check(!overlay.Close(out had) && had,"Missing retirement notification authorized an absent frame");
                    terminal.Reject=true;
                    Check(!overlay.Paint(lines,0,20),"Rejected frame became visible");
                    Check(!overlay.Close(out had) && had,"Rejected frame became eligible");
                    terminal.Reject=false;
                    terminal.Reply("frame;1;presented"); // Stale ACK cannot qualify the next frame.
                    Check(overlay.Paint(lines,0,20),"Fresh frame was lost after a stale acknowledgement");
                    terminal.Reply("retired;"+terminal.LastFrame);
                    System.Threading.Thread.Sleep(25);
                    Check(!overlay.Close(out had) && had,"Retired frame remained eligible");
                    terminal.Disconnect();
                    System.Threading.Thread.Sleep(25);
                    Check(!overlay.Paint(lines,0,20),"Disconnected channel accepted a frame");
                    overlay.Dispose();
                    Check(overlay.ReaderStopped,"Connected reader survived disposal");
                }
            }
            using(var terminal=new Terminal()) {
                Console.SetOut(terminal);
                using(var overlay=new NativeOverlay()) {
                    Check(overlay.Connect(),"Bridge EOF setup handshake failed");
                    Check(overlay.Paint(new List<object>{"menu"},0,20),"Bridge EOF setup frame failed");
                    BridgeEof(overlay);
                }
            }
            Console.SetOut(original);
            Console.WriteLine("Native overlay client: real pipe, submit/reject, stale ACK, retirement, EOF and cancellation passed");
            return 0;
        } catch(Exception error) { Console.SetOut(original); Console.Error.WriteLine(error); return 1; }
    }
}
