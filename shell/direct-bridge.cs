// Built against CLR 4 at build time. No reference to a particular PSReadLine
// assembly: all editing operations use the loaded module's public APIs.
using System;
using System.Collections;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.IO.Pipes;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

namespace Blueberry.Direct {
    public static class Bridge {
        delegate void BufferState(out string line, out int cursor);
        delegate void Register(string[] keys, Action<ConsoleKeyInfo?, object> handler, string brief, string description);
        delegate void ReplaceText(int start, int length, string text, Action<ConsoleKeyInfo?, object> instigator, object argument);
        static Type api;
        static Version moduleVersion;
        static ICollection queuedInput;
        static BufferState buffer;
        static Register register;
        static ReplaceText replace;
        static Action<ConsoleKeyInfo?, object> selfInsert;
        static NamedPipeClientStream pipe;
        static readonly object writeLock = new object(), frameLock = new object();
        static readonly AutoResetEvent arrived = new AutoResetEvent(false);
        static Dictionary<string, object> pending, displayed;
        static long revision;
        static bool editing, connected, enabled, negotiated;
        static IntPtr responseTimer;
        static bool waitingForFrame;
        static readonly bool diagnostic=Environment.GetEnvironmentVariable("BLUEBERRY_DIRECT_TRACE")=="1";
        static int editorThread;
        static string token, directory;
        static string interaction = "completion";
        public static string DisabledReason { get; private set; }
        public static bool AutomaticMenu { get { return enabled && connected && negotiated; } }
        public sealed class Notifications {
            public event EventHandler FrameAvailable;
            internal void Raise() { var callback = FrameAvailable; if (callback != null) callback(this, EventArgs.Empty); }
        }
        public static readonly Notifications Events = new Notifications();
        static readonly Dictionary<string, Action<ConsoleKeyInfo?, object>> installed = new Dictionary<string, Action<ConsoleKeyInfo?, object>>(StringComparer.Ordinal);
        static readonly HashSet<Action<ConsoleKeyInfo?, object>> wrappers = new HashSet<Action<ConsoleKeyInfo?, object>>();
        static readonly List<SavedBinding> originals = new List<SavedBinding>();
        static FieldInfo singletonField, dispatchField;
        static IDictionary auditedTable;
        static HashSet<object> auditedHandlers;
        static IDictionary DispatchTable() {
            if(singletonField==null) singletonField=api.GetField("_singleton",BindingFlags.NonPublic|BindingFlags.Static);
            if(dispatchField==null) dispatchField=api.GetField("_dispatchTable",BindingFlags.NonPublic|BindingFlags.Instance);
            if(singletonField==null || dispatchField==null) throw new NotSupportedException("unverifiable PSReadLine binding layout");
            var table=dispatchField.GetValue(singletonField.GetValue(null)) as IDictionary;
            if(table==null) throw new NotSupportedException("unverifiable PSReadLine binding table");
            return table;
        }
        static void RememberBindingVersion() {
            auditedTable=DispatchTable();
            auditedHandlers=new HashSet<object>();
            foreach(object handler in auditedTable.Values) auditedHandlers.Add(handler);
        }
        static void AuditBindings() {
            var table=DispatchTable();
            // .NET Core Dictionary overwrite does not increment its mutation
            // counter. Public SetKeyHandler creates a new handler on both
            // supported versions, so verify every handler identity instead.
            // This avoids reflection/key-string allocation for owned entries.
            if(Object.ReferenceEquals(table,auditedTable) && auditedHandlers!=null && table.Count==auditedHandlers.Count) {
                bool unchanged=true;
                foreach(object handler in table.Values) if(!auditedHandlers.Contains(handler)) { unchanged=false; break; }
                if(unchanged) return;
            }
            Bindings(true); RememberBindingVersion();
        }
        class SavedBinding {
            internal string Key, Brief, Description;
            internal Action<ConsoleKeyInfo?, object> Action;
        }
        // PSReadLine's input loop compares some handlers by delegate identity.
        // Leave these alone; Begin/End clean overlays at the read-line boundary.
        static readonly HashSet<string> exempt = new HashSet<string>(new string[] {
            "DigitArgument", "CancelLine", "Abort", "CopyOrCancelLine", "Chord", "ChordFirstKey",
            "ViCommandMode", "ViInsertMode", "ViEditVisually", "StartSearch", "WhatIsKey"
        });

        static Delegate Public(string name, Type signature, params Type[] parameters) {
            MethodInfo method = api.GetMethod(name, BindingFlags.Public | BindingFlags.Static, null, parameters, null);
            if (method == null) throw new NotSupportedException("PSReadLine public API missing: " + name);
            return Delegate.CreateDelegate(signature, method);
        }
        // Public snapshots expose descriptions, not original delegates. Read only
        // the version-specific dispatch table to verify and capture the delegate.
        // Never set a private field or touch PSReadLine's private editing state.
        static List<SavedBinding> Bindings(bool audit) {
            if (moduleVersion.Major != 2) throw new NotSupportedException("unsupported PSReadLine module major version");
            IDictionary table = DispatchTable();
            var result = new List<SavedBinding>();
            FieldInfo cachedAction = null, cachedScript = null, cachedBrief = null;
            PropertyInfo cachedKeyString = null;
            FieldInfo cachedKeyField = null;
            foreach (DictionaryEntry entry in table) {
                Type keyType = entry.Key.GetType(), handlerType = entry.Value.GetType();
                if (cachedAction == null) {
                    cachedAction = handlerType.GetField("Action", BindingFlags.Public | BindingFlags.Instance);
                    cachedScript = handlerType.GetField("ScriptBlock", BindingFlags.Public | BindingFlags.Instance);
                    cachedBrief = handlerType.GetField("BriefDescription", BindingFlags.Public | BindingFlags.Instance);
                    cachedKeyString = keyType.GetProperty("KeyStr", BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance);
                    cachedKeyField = keyType.GetField("KeyStr", BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance);
                }
                FieldInfo actionField = cachedAction, scriptField = cachedScript, briefField = cachedBrief;
                var action = actionField == null ? null : actionField.GetValue(entry.Value) as Action<ConsoleKeyInfo?, object>;
                if (audit && action != null && wrappers.Contains(action)) continue;
                string key = cachedKeyString != null ? cachedKeyString.GetValue(entry.Key, null) as string : cachedKeyField == null ? null : cachedKeyField.GetValue(entry.Key) as string;
                if (String.IsNullOrEmpty(key) || action == null || scriptField == null || briefField == null)
                    throw new NotSupportedException("unverifiable key delegate");
                Action<ConsoleKeyInfo?, object> ours;
                if (audit && installed.TryGetValue(key, out ours) && action == ours) continue;
                if (audit && (scriptField.GetValue(entry.Value) != null || action.Method.DeclaringType != api || action.Target != null || (!action.Method.IsPublic && !exempt.Contains(action.Method.Name))))
                    throw new NotSupportedException("custom binding: " + key);
                string brief = briefField.GetValue(entry.Value) as string;
                result.Add(new SavedBinding { Key = key, Brief = brief, Description = "Original PSReadLine action", Action = action });
            }
            return result;
        }
        public static void Initialize(Type readLine, string pipeName, string sessionToken, string shellVersion, string loadedModuleVersion) {
            long initialization=Stopwatch.GetTimestamp(), snapshotTicks=0, registrationTicks=0;
            api = readLine; token = sessionToken; moduleVersion=Version.Parse(loadedModuleVersion);
            // Observe a pending input burst, without reading/consuming keys or
            // changing any private state. All buffer confirmation is still
            // via GetBufferState. The supported layouts are verified below.
            var singleton=api.GetField("_singleton",BindingFlags.NonPublic|BindingFlags.Static);
            var queue=api.GetField("_queuedKeys",BindingFlags.NonPublic|BindingFlags.Instance);
            if(singleton!=null && queue!=null) queuedInput=queue.GetValue(singleton.GetValue(null)) as ICollection;
            buffer = (BufferState)Public("GetBufferState", typeof(BufferState), typeof(string).MakeByRefType(), typeof(int).MakeByRefType());
            register = (Register)Public("SetKeyHandler", typeof(Register), typeof(string[]), typeof(Action<ConsoleKeyInfo?, object>), typeof(string), typeof(string));
            replace = (ReplaceText)Public("Replace", typeof(ReplaceText), typeof(int), typeof(int), typeof(string), typeof(Action<ConsoleKeyInfo?, object>), typeof(object));
            selfInsert = (Action<ConsoleKeyInfo?, object>)Public("SelfInsert", typeof(Action<ConsoleKeyInfo?, object>), typeof(ConsoleKeyInfo?), typeof(object));
            try {
                responseTimer=CreateWaitableTimerExW(IntPtr.Zero,null,2,0x1f0003);
                if(responseTimer==IntPtr.Zero) throw new NotSupportedException("high resolution response timer unavailable");
                // Use the complete public snapshot on 2.0.0 as well. Never call
                // the string[] GetKeyHandlers overload absent from that module.
                var snapshot = api.GetMethod("GetKeyHandlers", new Type[] { typeof(bool), typeof(bool) });
                if (snapshot == null) throw new NotSupportedException("missing full key snapshot");
                snapshot.Invoke(null, new object[] { true, false });
                originals.AddRange(Bindings(true));
                snapshotTicks=Stopwatch.GetTimestamp()-initialization;
                long registration=Stopwatch.GetTimestamp();
                // Validate every original before changing any binding.
                var bound = new HashSet<string>(StringComparer.Ordinal);
                foreach (SavedBinding original in originals) bound.Add(original.Key);
                foreach (SavedBinding original in originals) {
                    if (exempt.Contains(original.Action.Method.Name) || original.Brief == "ChordFirstKey") continue;
                    var saved = original;
                    Action<ConsoleKeyInfo?, object> hook = delegate(ConsoleKeyInfo? key, object arg) { Edit(saved.Action, key, arg); };
                    installed[saved.Key] = hook;
                    wrappers.Add(hook);
                    register(new string[] { saved.Key }, hook, saved.Brief, saved.Description);
                }
                var characters = new List<string>();
                for (int unit = 32; unit <= Char.MaxValue; unit++) {
                    char ch = (char)unit;
                    if (!Char.IsControl(ch) && !bound.Contains(ch.ToString())) characters.Add(ch.ToString());
                }
                Action<ConsoleKeyInfo?, object> insert = delegate(ConsoleKeyInfo? key, object arg) { Edit(selfInsert, key, arg); };
                wrappers.Add(insert);
                register(characters.ToArray(), insert, "SelfInsert", "Blueberry confirmed edit");
                Action<ConsoleKeyInfo?, object> hub = delegate(ConsoleKeyInfo? key, object arg) {
                    if (!AutomaticMenu || !editing) return;
                    Clear(); interaction = interaction == "completion" ? "hub" : "completion";
                    Query(interaction == "hub" ? "hub" : "cancel");
                };
                string hubKey = Environment.GetEnvironmentVariable("BLUEBERRY_PUBLIC_KEY_HUB") ?? "Ctrl+Alt+p";
                int lastPlus = hubKey.LastIndexOf('+');
                if (lastPlus >= 0 && hubKey.Length == lastPlus+2 && Char.IsLetter(hubKey[lastPlus+1]))
                    hubKey=hubKey.Substring(0,lastPlus+1)+Char.ToLowerInvariant(hubKey[lastPlus+1]);
                if (bound.Contains(hubKey)) throw new NotSupportedException("workbench key already bound: " + hubKey);
                wrappers.Add(hub); register(new string[] {hubKey},hub,"BlueberryWorkbench","Blueberry workbench");
                if(!bound.Contains("F1")) {
                    Action<ConsoleKeyInfo?,object> details=delegate(ConsoleKeyInfo? key,object arg){
                        if(editing && AutomaticMenu && key.HasValue && displayed!=null) {Clear(); SendKey(key.Value,interaction=="completion" ? "menu_key":"ui_key");}
                    };
                    wrappers.Add(details); register(new string[]{"F1"},details,"BlueberryDetails","Blueberry details");
                }
                // PSReadLine canonicalizes literal space to Spacebar and some
                // punctuation to named keys. Capture actual registered names.
                installed.Clear();
                foreach (var binding in Bindings(false)) if (wrappers.Contains(binding.Action)) installed[binding.Key] = binding.Action;
                RememberBindingVersion();
                enabled = true;
                registrationTicks=Stopwatch.GetTimestamp()-registration;
            } catch (Exception error) {
                Disable(error.GetBaseException().Message);
            }
            pipe = new NamedPipeClientStream(".", pipeName, PipeDirection.InOut, PipeOptions.Asynchronous);
            pipe.Connect(5000); connected = true;
            new Thread(ReadFrames) { IsBackground = true, Name = "Blueberry frame receiver" }.Start();
            Send(new Dictionary<string, object> {
                {"event", "hello"}, {"protocol", 1}, {"host_mode", "direct"}, {"transport", "pipe"},
                {"automatic_menu", enabled}, {"disabled_reason", DisabledReason},
                {"psreadline", moduleVersion.ToString()},
                {"shell_version", shellVersion},
                {"capabilities", new object[] {"revision", "frame_identity", "accept_identity", "utf16_edit"}}
            });
            long handshakeDeadline=Stopwatch.GetTimestamp()+Stopwatch.Frequency*5;
            while(connected && !negotiated && Stopwatch.GetTimestamp()<handshakeDeadline) arrived.WaitOne(10);
            if(!negotiated) Disable("direct protocol capabilities were not acknowledged");
            TraceStage("direct_binding_snapshot",snapshotTicks,originals.Count);
            TraceStage("direct_binding_registration",registrationTicks,installed.Count);
            TraceStage("direct_bridge_initialization",Stopwatch.GetTimestamp()-initialization,installed.Count);
        }
        static void Disable(string reason) {
            enabled = false; DisabledReason = reason; Clear();
            // Registration can fail part way through. Restore only bindings
            // still owned by this bridge; never overwrite a later user rebind.
            try {
                var current = new Dictionary<string, Action<ConsoleKeyInfo?, object>>(StringComparer.Ordinal);
                foreach (var binding in Bindings(false)) current[binding.Key] = binding.Action;
                foreach (SavedBinding original in originals) {
                    Action<ConsoleKeyInfo?, object> action, owned;
                    if (installed.TryGetValue(original.Key, out owned) && current.TryGetValue(original.Key, out action) && action == owned)
                        register(new string[] { original.Key }, original.Action, original.Brief, original.Description);
                }
                if (installed.Count > 0) {
                    var added = new List<string>();
                    var old = new HashSet<string>(StringComparer.Ordinal);
                    foreach (SavedBinding original in originals) old.Add(original.Key);
                    foreach (string key in installed.Keys) {
                        Action<ConsoleKeyInfo?, object> action;
                        if (!old.Contains(key) && current.TryGetValue(key, out action) && action == installed[key]) added.Add(key);
                    }
                    api.GetMethod("RemoveKeyHandler", new Type[] { typeof(string[]) }).Invoke(null, new object[] { added.ToArray() });
                }
                installed.Clear(); originals.Clear(); wrappers.Clear();
            } catch { /* The original edit action is still delegated by hooks. */ }
        }
        public static void Begin(string cwd, string historyPath) {
            long beginning=diagnostic ? Stopwatch.GetTimestamp() : 0;
            Clear(); displayed = null; interaction = "completion"; directory = cwd; editing = true; editorThread = Thread.CurrentThread.ManagedThreadId;
            revision++;
            if (enabled) {
                try { AuditBindings(); } catch (Exception error) { Disable(error.GetBaseException().Message); }
            }
            var environment = new Dictionary<string, object>(StringComparer.OrdinalIgnoreCase);
            foreach(DictionaryEntry entry in Environment.GetEnvironmentVariables()) environment[((string)entry.Key).ToUpperInvariant()] = (string)entry.Value;
            Send(new Dictionary<string, object> { {"event", "begin"}, {"revision", revision}, {"cwd",cwd}, {"history_path",historyPath}, {"environment",environment}, {"automatic_menu", AutomaticMenu}, {"disabled_reason", DisabledReason} });
            if(diagnostic) TraceStage("direct_readline_begin",Stopwatch.GetTimestamp()-beginning,1);
        }
        public static void End() {
            string line; int cursor; buffer(out line,out cursor);
            Clear(); displayed = null; editing = false; revision++;
            Send(new Dictionary<string, object> { {"event", "end"}, {"revision", revision}, {"line",line} });
        }
        static void Edit(Action<ConsoleKeyInfo?, object> original, ConsoleKeyInfo? key, object arg) {
            Clear();
            if (editing && AutomaticMenu && interaction != "completion" && key.HasValue) {
                SendKey(key.Value); return;
            }
            if(editing && AutomaticMenu && displayed!=null && key.HasValue && key.Value.Key==ConsoleKey.Escape) {
                interaction="completion"; Query("cancel"); return;
            }
            if (editing && AutomaticMenu && displayed != null && key.HasValue
                && (key.Value.Key==ConsoleKey.UpArrow || key.Value.Key==ConsoleKey.DownArrow || key.Value.Key==ConsoleKey.F1)
                && key.Value.Modifiers==0) { SendKey(key.Value,"menu_key"); return; }
            if (editing && AutomaticMenu && displayed != null && key.HasValue && key.Value.Key == ConsoleKey.Tab) {
                Accept(); return;
            }
            long delegated=diagnostic ? Stopwatch.GetTimestamp() : 0;
            original(key, arg); // Original key and argument, including numeric repeat counts.
            if(diagnostic) TraceStage("direct_original_edit",Stopwatch.GetTimestamp()-delegated,1);
            if (!editing || !AutomaticMenu) return;
            // AcceptLine/AddLine handlers return to the read-line owner. Do not
            // draw over a command about to execute.
            if (original.Method.Name == "AcceptLine" || original.Method.Name == "ValidateAndAcceptLine") { displayed = null; return; }
            Query("query");
        }
        static void SendKey(ConsoleKeyInfo key, string operation="ui_key") {
            string line; int cursor; buffer(out line, out cursor); revision++;
            Send(new Dictionary<string,object> {
                {"event",operation},{"revision",revision},{"line",line},{"cursor",cursor},
                {"key",key.Key.ToString()},{"character_unit",(long)key.KeyChar},{"modifiers",(int)key.Modifiers},
                {"frame_id",displayed == null ? -1 : Json.Long(displayed,"frame_id")},
                {"candidate_id",displayed == null ? null : Json.String(displayed,"candidate_id")}
            });
            displayed=null;
            if(key.Key==ConsoleKey.Escape) interaction="completion";
            WaitFrame();
        }
        static void Query(string operation) {
            string line; int cursor; buffer(out line, out cursor);
            long confirmed=diagnostic ? Stopwatch.GetTimestamp() : 0;
            // A high UTF-16 surrogate can arrive as one console key before its
            // low surrogate. Wait for PSReadLine to confirm the complete pair.
            if (SplitsPair(line,cursor) || Unpaired(line)) { displayed=null; return; }
            revision++; displayed = null;
            // Do not spend a 4 ms response budget for each character already
            // queued in a paste/burst. The final confirmed edit sends a query.
            // A single first key never skips initialization or its query.
            if(operation=="query" && queuedInput!=null && queuedInput.Count>0) return;
            // A queued burst still delegates each captured original action.
            // Audit once before publishing the final confirmed state instead
            // of rescanning 65k bindings for every code unit in the burst.
            long audited=diagnostic ? Stopwatch.GetTimestamp() : 0;
            try { AuditBindings(); } catch(Exception error) { Disable(error.GetBaseException().Message); return; }
            if(diagnostic) TraceStage("direct_binding_audit",Stopwatch.GetTimestamp()-audited,installed.Count);
            int width = Math.Max(1, Console.WindowWidth), rows = Math.Max(1, Console.WindowHeight);
            var query=new Dictionary<string, object> {
                {"event", operation}, {"revision", revision}, {"line", line}, {"cursor", cursor},
                {"cwd", directory}, {"width", width}, {"rows", rows}
            };
            if(diagnostic) query["confirmed_qpc"]=confirmed;
            Send(query);
            if(diagnostic) TraceStage("direct_confirmed_query_send",Stopwatch.GetTimestamp()-confirmed,line.Length);
            WaitFrame();
        }
        static void WaitFrame() {
            if(responseTimer==IntPtr.Zero || !AutomaticMenu) return;
            long started=Stopwatch.GetTimestamp(), due=-40000;
            if(!SetWaitableTimer(responseTimer,ref due,0,IntPtr.Zero,IntPtr.Zero,false)) { Disable("cannot arm response timer"); return; }
            var handles=new IntPtr[]{arrived.SafeWaitHandle.DangerousGetHandle(),responseTimer};
            waitingForFrame=true;
            try {
                do {
                    if(Refresh() && (displayed==null || !Object.Equals(displayed["incomplete"],true))) return;
                    if(Stopwatch.GetTimestamp()-started>=Stopwatch.Frequency*4/1000) return;
                    // A loading/static frame is not the complete response. Keep
                    // the remaining budget for the complete dynamic snapshot.
                    uint result=WaitForMultipleObjects(2,handles,false,100);
                    if(result==1) { Refresh(); return; }
                    if(result!=0) { Disable("response wait failed"); return; }
                } while(connected);
            } finally {
                waitingForFrame=false;
                if(diagnostic) TraceStage("direct_response_wait",Stopwatch.GetTimestamp()-started,1);
            }
        }
        static void Accept() {
            var frame = displayed; displayed = null;
            string line; int cursor; buffer(out line, out cursor);
            if (frame == null || Json.Long(frame, "revision") != revision) return;
            Send(new Dictionary<string, object> {
                {"event", "accept"}, {"revision", revision}, {"frame_id", Json.Long(frame, "frame_id")},
                {"candidate_id", Json.String(frame, "candidate_id")}, {"line", line}, {"cursor", cursor}
            });
            // Acceptance is an explicit operation. Wait for its validated edit;
            // ordinary typing never waits longer than the 4 ms query budget.
            long deadline = Stopwatch.GetTimestamp() + Stopwatch.Frequency / 2;
            while (connected && Stopwatch.GetTimestamp() < deadline) {
                if (Refresh()) return;
                arrived.WaitOne(1);
            }
        }
        public static bool Refresh() {
            if (!editing || Thread.CurrentThread.ManagedThreadId != editorThread) return false;
            if (!connected) { Clear(); return false; }
            if(enabled && !waitingForFrame) { try { AuditBindings(); } catch(Exception error) { Disable(error.GetBaseException().Message); return false; } }
            Dictionary<string, object> frame;
            lock (frameLock) { frame = pending; pending = null; }
            if (frame == null || Json.Long(frame, "revision") != revision) return false;
            string line; int cursor; buffer(out line, out cursor);
            if (Json.String(frame, "expected_line") != line || Json.Long(frame, "expected_cursor") != cursor) return false;
            Clear();
            if (Json.String(frame,"kind") == "clear") { displayed=null; interaction="completion"; return true; }
            if (Json.String(frame, "kind") == "edit") {
                int start = (int)Json.Long(frame, "start"), length = (int)Json.Long(frame, "length");
                string text = Json.String(frame, "text");
                if (start < 0 || length < 0 || start > line.Length - length || SplitsPair(line, start) || SplitsPair(line, start + length)) return false;
                replace(start, length, text, null, null); displayed = null; interaction="completion"; Query("query"); return true;
            }
            object lines;
            if (Json.String(frame, "kind") != "frame" || !frame.TryGetValue("lines", out lines) || !(lines is List<object>)) return false;
            long painted=diagnostic ? Stopwatch.GetTimestamp() : 0;
            if (!Paint((List<object>)lines, (int)Json.Long(frame, "tail_rows"), (int)Json.Long(frame, "width"))) { displayed = null; return false; }
            if(diagnostic) TraceStage("direct_console_menu_output",Stopwatch.GetTimestamp()-painted,((List<object>)lines).Count);
            interaction=Json.String(frame,"interaction") ?? "completion"; displayed = frame; return true;
        }
        static bool SplitsPair(string line, int at) {
            return at > 0 && at < line.Length && Char.IsHighSurrogate(line[at-1]) && Char.IsLowSurrogate(line[at]);
        }
        static bool Unpaired(string line) {
            for(int i=0;i<line.Length;i++) {
                if(Char.IsHighSurrogate(line[i])) { if(i+1>=line.Length || !Char.IsLowSurrogate(line[++i])) return true; }
                else if(Char.IsLowSurrogate(line[i])) return true;
            }
            return false;
        }
        static void Send(Dictionary<string, object> message) {
            if (!connected) return;
            message["token"] = token;
            byte[] bytes = Encoding.UTF8.GetBytes(Json.Encode(message) + "\n");
            if (bytes.Length > 1048576) { connected = false; return; }
            try { lock (writeLock) { pipe.Write(bytes, 0, bytes.Length); pipe.Flush(); } }
            catch { connected = false; }
        }
        static void TraceStage(string stage,long ticks,int count) {
            if(diagnostic && negotiated) Send(new Dictionary<string,object>{{"event","trace"},{"revision",revision},{"stage",stage},{"duration_us",ticks*1000000/Stopwatch.Frequency},{"count",count}});
        }
        static void ReadFrames() {
            try {
                using (var reader = new StreamReader(pipe, new UTF8Encoding(false, true), false, 4096, true)) {
                    var text = new StringBuilder();
                    while (connected) {
                        int next = reader.Read(); if (next < 0) break;
                        if (next != '\n') { if (text.Length >= 1048576) throw new InvalidDataException("oversized frame"); text.Append((char)next); continue; }
                        var frame = Json.Parse(text.ToString()) as Dictionary<string, object>; text.Clear();
                        if (frame == null || Json.String(frame, "token") != token) throw new InvalidDataException("invalid session frame");
                        if(Json.String(frame,"kind")=="hello") {
                            if(Json.Long(frame,"protocol")!=1 || Json.String(frame,"host_mode")!="direct" || Json.String(frame,"transport")!="pipe") throw new InvalidDataException("incompatible direct capabilities");
                            var capabilities=frame["capabilities"] as List<object>;
                            foreach(string capability in new string[]{"revision","frame_identity","accept_identity","utf16_edit"})
                                if(capabilities==null || !capabilities.Contains(capability)) throw new InvalidDataException("missing direct capability");
                            negotiated=true; arrived.Set(); continue;
                        }
                        if(!negotiated || (Json.String(frame,"kind")!="frame" && Json.String(frame,"kind")!="edit" && Json.String(frame,"kind")!="clear")) throw new InvalidDataException("unexpected direct frame");
                        lock (frameLock) {
                            if (pending == null || Json.Long(frame, "revision") > Json.Long(pending, "revision")
                                || (Json.Long(frame, "revision") == Json.Long(pending, "revision") && Json.String(pending, "kind") != "edit")) pending = frame;
                        }
                        arrived.Set();
                        Events.Raise(); // Queued runspace event, no terminal writes here.
                    }
                }
            } catch { }
            connected = false; arrived.Set();
            Events.Raise();
        }
        public static void Shutdown() { End(); connected = false; if (pipe != null) pipe.Dispose(); if(responseTimer!=IntPtr.Zero) { CloseHandle(responseTimer); responseTimer=IntPtr.Zero; } }

        [StructLayout(LayoutKind.Sequential)] struct Coord { public short X, Y; public Coord(int x, int y) { X=(short)x; Y=(short)y; } }
        [StructLayout(LayoutKind.Sequential)] struct Rect { public short Left, Top, Right, Bottom; }
        [StructLayout(LayoutKind.Explicit, CharSet=CharSet.Unicode)] struct Cell { [FieldOffset(0)] public char Character; [FieldOffset(2)] public ushort Attributes; }
        [StructLayout(LayoutKind.Sequential)] struct Info { public Coord Size, Cursor; public ushort Attributes; public Rect Window; public Coord Max; }
        [DllImport("kernel32.dll")] static extern IntPtr GetStdHandle(int which);
        [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
        [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern IntPtr CreateWaitableTimerExW(IntPtr security,string name,uint flags,uint access);
        [DllImport("kernel32.dll",SetLastError=true)] static extern bool SetWaitableTimer(IntPtr timer,ref long due,int period,IntPtr callback,IntPtr argument,bool resume);
        [DllImport("kernel32.dll",SetLastError=true)] static extern uint WaitForMultipleObjects(uint count,IntPtr[] handles,bool waitAll,uint milliseconds);
        [DllImport("kernel32.dll")] static extern bool GetConsoleScreenBufferInfo(IntPtr handle, out Info info);
        [DllImport("kernel32.dll", CharSet=CharSet.Unicode)] static extern bool ReadConsoleOutputW(IntPtr handle, [Out] Cell[] cells, Coord size, Coord origin, ref Rect region);
        [DllImport("kernel32.dll", CharSet=CharSet.Unicode)] static extern bool WriteConsoleOutputW(IntPtr handle, Cell[] cells, Coord size, Coord origin, ref Rect region);
        static Cell[] covered; static Rect region; static Coord coveredSize; static IntPtr output;
        static bool Paint(List<object> lines, int tailRows, int width) {
            output = GetStdHandle(-11); Info info;
            if (!GetConsoleScreenBufferInfo(output, out info)) return false;
            int top = info.Cursor.Y + Math.Max(0, tailRows) + 1;
            int height = Math.Min(lines.Count, info.Window.Bottom - top + 1);
            width = Math.Min(width, info.Size.X-1);
            if (height <= 0 || width <= 0) return false;
            // Preserve exactly the painted rectangle. Touching the last
            // column of every row would introduce soft wraps in ConPTY.
            region = new Rect { Left=0, Right=(short)(width-1), Top=(short)top, Bottom=(short)(top+height-1) };
            coveredSize = new Coord(width, height); covered = new Cell[width*height];
            if (!ReadConsoleOutputW(output, covered, coveredSize, new Coord(0,0), ref region)) { covered=null; return false; }
            try {
                for (int i=0; i<height; i++) {
                    Console.SetCursorPosition(0, top+i);
                    // Rust renderer emits sanitized text and SGR only. No line
                    // feeds: never scroll the inherited console from an overlay.
                    Console.Write(lines[i] as string); Console.Write("\x1b[0m");
                }
            } finally { Console.SetCursorPosition(info.Cursor.X, info.Cursor.Y); }
            return true;
        }
        static void Clear() {
            if (covered == null) return;
            var cells = covered; covered = null;
            try {
                Info current;
                if (GetConsoleScreenBufferInfo(output, out current) && region.Top < current.Size.Y && region.Left < current.Size.X) {
                    var clipped=region;
                    clipped.Right=(short)Math.Min(clipped.Right,current.Size.X-1);
                    clipped.Bottom=(short)Math.Min(clipped.Bottom,current.Size.Y-1);
                    WriteConsoleOutputW(output, cells, coveredSize, new Coord(0,0), ref clipped);
                }
            } catch { }
        }
    }

    // Small strict JSON codec for the bounded direct protocol. Works on CLR 4
    // and CoreCLR without loading System.Web or compiling serializer helpers.
    public static class Json {
        public static long Long(Dictionary<string, object> value, string key) { object item; return value.TryGetValue(key, out item) && item is long ? (long)item : -1; }
        public static string String(Dictionary<string, object> value, string key) { object item; return value.TryGetValue(key, out item) ? item as string : null; }
        public static string Encode(object value) {
            if (value == null) return "null";
            var text = value as string;
            if (text != null) {
                var output = new StringBuilder("\"");
                foreach (char c in text) {
                    if (c == '"' || c == '\\') { output.Append('\\'); output.Append(c); }
                    else if (c < 32 || Char.IsSurrogate(c)) output.Append("\\u" + ((int)c).ToString("x4"));
                    else output.Append(c);
                }
                return output.Append('"').ToString();
            }
            if (value is bool) return (bool)value ? "true" : "false";
            var map = value as IDictionary;
            if (map != null) { var items = new List<string>(); foreach (DictionaryEntry entry in map) items.Add(Encode(entry.Key)+":"+Encode(entry.Value)); return "{"+System.String.Join(",",items.ToArray())+"}"; }
            var sequence = value as IEnumerable;
            if (sequence != null) { var items = new List<string>(); foreach (object item in sequence) items.Add(Encode(item)); return "["+System.String.Join(",",items.ToArray())+"]"; }
            return Convert.ToString(value, System.Globalization.CultureInfo.InvariantCulture);
        }
        public static object Parse(string text) { var parser = new Parser(text); object value = parser.Value(0); parser.Space(); if (parser.At != text.Length) throw new InvalidDataException("trailing JSON"); return value; }
        class Parser {
            readonly string text; internal int At;
            internal Parser(string source) { text=source; }
            internal void Space() { while (At<text.Length && (text[At]==' ' || text[At]=='\r' || text[At]=='\n' || text[At]=='\t')) At++; }
            void Need(char ch) { Space(); if (At>=text.Length || text[At++]!=ch) throw new InvalidDataException("invalid JSON"); }
            internal object Value(int depth) {
                if (depth>32) throw new InvalidDataException("JSON depth"); Space(); if (At>=text.Length) throw new InvalidDataException("truncated JSON"); char c=text[At];
                if (c=='"') return Text();
                if (c=='{') { At++; var result=new Dictionary<string,object>(); Space(); if (At<text.Length && text[At]=='}') { At++; return result; } do { string key=Text(); Need(':'); result.Add(key,Value(depth+1)); Space(); if (At<text.Length && text[At]=='}') { At++; return result; } Need(','); Space(); } while(true); }
                if (c=='[') { At++; var result=new List<object>(); Space(); if (At<text.Length && text[At]==']') { At++; return result; } do { result.Add(Value(depth+1)); Space(); if (At<text.Length && text[At]==']') { At++; return result; } Need(','); } while(true); }
                foreach (string literal in new string[] {"true","false","null"}) if (text.Substring(At).StartsWith(literal,StringComparison.Ordinal)) { At+=literal.Length; return literal=="null" ? null : (object)(literal=="true"); }
                int start=At; if(c=='-') At++; while(At<text.Length && text[At]>='0' && text[At]<='9') At++;
                string number=text.Substring(start,At-start); long parsed;
                if (!Int64.TryParse(number,System.Globalization.NumberStyles.AllowLeadingSign,System.Globalization.CultureInfo.InvariantCulture,out parsed) || number=="-" || (number.Length>1 && number[0]=='0') || (number.Length>2 && number.StartsWith("-0",StringComparison.Ordinal))) throw new InvalidDataException("invalid integer");
                return parsed;
            }
            string Text() {
                Need('"'); var result=new StringBuilder();
                while(At<text.Length) {
                    char c=text[At++]; if(c=='"') return result.ToString(); if(c<32) throw new InvalidDataException("JSON control");
                    if(c=='\\') { if(At>=text.Length) break; c=text[At++]; switch(c) { case '"': case '\\': case '/': break; case 'n': c='\n'; break; case 'r': c='\r'; break; case 't': c='\t'; break; case 'b': c='\b'; break; case 'f': c='\f'; break; case 'u': if(At+4>text.Length) throw new InvalidDataException("JSON escape"); c=(char)Convert.ToInt32(text.Substring(At,4),16); At+=4; break; default: throw new InvalidDataException("JSON escape"); } }
                    result.Append(c);
                }
                throw new InvalidDataException("unterminated JSON string");
            }
        }
    }
}
