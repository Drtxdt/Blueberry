// Dedicated companion process: actual GUI + real C# client, no PSReadLine keys.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using Blueberry.Direct;

class NativeMenuRuntimeProbe {
    [DllImport("kernel32.dll")] static extern IntPtr GetStdHandle(int kind);
    [DllImport("kernel32.dll")] static extern bool GetConsoleMode(IntPtr handle, out uint mode);
    [DllImport("kernel32.dll")] static extern bool SetConsoleMode(IntPtr handle, uint mode);
    static int Main(string[] args) {
        string path=Path.GetFullPath(args[0]);
        if(File.Exists(path)) throw new IOException("Refusing to overwrite native runtime evidence");
        var result=new Dictionary<string,object> {
            {"schema",1L},{"diagnostic_only",true},{"supported",false},
            {"pixel_visibility_measured",false},{"all_history_attributes_verified",false},
            {"input_stream_read_or_modified",false},{"error",null}
        };
        IntPtr output=GetStdHandle(-11); uint originalMode=0; bool changed=false;
        try {
            if(!GetConsoleMode(output,out originalMode) || !SetConsoleMode(output,originalMode|4))
                throw new IOException("Cannot enable VT in dedicated probe console");
            changed=true;
            Console.Write("\x1b[38;2;12;34;56;48;2;23;45;67m\x1b]8;id=own-history;https://example.invalid/native-menu\x1b\\历史 🚀 é\x1b]8;;\x1b\\\x1b[0m\r\nsecond history row\r\n$ ");
            var overlay=new NativeOverlay();
            using(overlay) {
                long started=Stopwatch.GetTimestamp();
                if(!overlay.Connect()) throw new IOException(overlay.Error);
                result["handshake_us"]=(Stopwatch.GetTimestamp()-started)*1000000/Stopwatch.Frequency;
                var lines=new List<object>{"\x1b[38;2;123;45;67m候选 🚀 é", "\x1b[1msecond candidate"};
                started=Stopwatch.GetTimestamp();
                if(!overlay.Paint(lines,0,32)) throw new IOException(overlay.Error ?? "Menu not confirmed");
                result["first_submit_to_native_ack_us"]=(Stopwatch.GetTimestamp()-started)*1000000/Stopwatch.Frequency;
                bool had;
                started=Stopwatch.GetTimestamp();
                if(!overlay.Close(out had) || !had) throw new IOException("Confirmed frame was not eligible before close");
                result["first_close_to_native_ack_us"]=(Stopwatch.GetTimestamp()-started)*1000000/Stopwatch.Frequency;
                started=Stopwatch.GetTimestamp();
                if(!overlay.Paint(lines,0,32)) throw new IOException(overlay.Error ?? "Second menu not confirmed");
                result["second_submit_to_native_ack_us"]=(Stopwatch.GetTimestamp()-started)*1000000/Stopwatch.Frequency;
                overlay.Close();
                result["supported"]=true;
                result["native_buffer_submissions"]=2L;
                result["reply_transport"]="private owner-bound pipe";
            }
            result["reader_stopped"]=overlay.ReaderStopped;
            if(!overlay.ReaderStopped) throw new IOException("Native acknowledgement reader survived disposal");
        } catch(Exception error) { result["error"]=error.GetBaseException().Message; }
        finally {
            bool restored=changed && SetConsoleMode(output,originalMode);
            result["output_mode_restored"]=restored;
            if(!restored) { result["supported"]=false; if(result["error"]==null) result["error"]="Probe console mode restoration failed"; }
        }
        File.WriteAllText(path,Json.Encode(result)+"\n",new System.Text.UTF8Encoding(false));
        return (bool)result["supported"] ? 0 : 1;
    }
}
