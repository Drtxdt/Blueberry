// Dedicated companion process: actual GUI + real C# client, no PSReadLine keys.
using System;
using System.Collections.Generic;
using System.IO;
using Blueberry.Direct;

class NativeMenuRuntimeProbe {
    static int Main(string[] args) {
        string path=Path.GetFullPath(args[0]);
        if(File.Exists(path)) throw new IOException("Refusing to overwrite native runtime evidence");
        var result=new Dictionary<string,object> {
            {"schema",1L},{"diagnostic_only",true},{"supported",false},
            {"pixel_visibility_measured",false},{"all_history_attributes_verified",false},
            {"input_stream_read_or_modified",false},{"error",null}
        };
        try {
            Console.Write("\x1b[38;2;12;34;56;48;2;23;45;67m\x1b]8;id=own-history;https://example.invalid/native-menu\x1b\\历史 🚀 é\x1b]8;;\x1b\\\x1b[0m\r\nsecond history row\r\n$ ");
            using(var overlay=new NativeOverlay()) {
                if(!overlay.Connect()) throw new IOException(overlay.Error);
                var lines=new List<object>{"\x1b[38;2;123;45;67m候选 🚀 é", "\x1b[1msecond candidate"};
                if(!overlay.Paint(lines,0,32)) throw new IOException(overlay.Error ?? "Menu not confirmed");
                bool had;
                if(!overlay.Close(out had) || !had) throw new IOException("Confirmed frame was not eligible before close");
                if(!overlay.Paint(lines,0,32)) throw new IOException(overlay.Error ?? "Second menu not confirmed");
                overlay.Close();
                result["supported"]=true;
                result["native_buffer_submissions"]=2L;
                result["reply_transport"]="private owner-bound pipe";
            }
        } catch(Exception error) { result["error"]=error.GetBaseException().Message; }
        File.WriteAllText(path,Json.Encode(result)+"\n",new System.Text.UTF8Encoding(false));
        return (bool)result["supported"] ? 0 : 1;
    }
}
