// Built against CLR 4 at build time. No reference to a particular PSReadLine
// assembly: all editing operations use the loaded module's public APIs.
using System;
using System.Collections;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.IO.Pipes;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Security.Cryptography;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

namespace Blueberry.Direct {
    public static class Bridge {
        delegate void BufferState(out string line, out int cursor);
        delegate void Register(string[] keys, Action<ConsoleKeyInfo?, object> handler, string brief, string description);
        delegate void ReplaceText(int start, int length, string text, Action<ConsoleKeyInfo?, object> instigator, object argument);
        static Type api;
        static int startupPreparation, startupPreparationReported, startupPreparationCount;
        static long startupPreparationStarted, startupPreparationFinished;
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
        static readonly Queue<Dictionary<string, object>> reliableFrames = new Queue<Dictionary<string, object>>();
        static Dictionary<string, object> acceptingFrame;
        static long expectedUiEditRevision = -1, receivedFrameRevision = -1, receivedFrameId = -1;
        static long revision;
        static bool editing, enabled;
        static volatile bool connected, negotiated;
        static IntPtr responseTimer;
        static bool waitingForFrame;
        static IDisposable editorRegistration;
        static Action requestEditorRefresh;
        static Func<long[]> editorTracePoints;
        static string hubChord, refreshChord;
        public static bool EditorHooks { get; private set; }
        static string editorHash, editorFallbackReason;
        public static string EditorFallbackReason { get { return editorFallbackReason; } }
        static long receivedQpc;
        static readonly bool diagnostic=Environment.GetEnvironmentVariable("BLUEBERRY_DIRECT_TRACE")=="1";
        static readonly int testFrameDelay = TestFrameDelay();
        static int TestFrameDelay() {
            int delay;
            return Environment.GetEnvironmentVariable("BLUEBERRY_NO_HISTORY")=="1"
                && Int32.TryParse(Environment.GetEnvironmentVariable("BLUEBERRY_TEST_FRAME_DELAY_MS"),out delay)
                && delay>=0 && delay<=100 ? delay : 0;
        }
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
            var hooks = api.GetProperty("EditorIntegrationVersion", BindingFlags.Public | BindingFlags.Static);
            EditorHooks = hooks != null && (int)hooks.GetValue(null, null) == 1;
            try { using(var hash=SHA256.Create()) editorHash=BitConverter.ToString(hash.ComputeHash(File.ReadAllBytes(api.Assembly.Location))).Replace("-", "").ToLowerInvariant(); }
            catch { EditorHooks=false; editorFallbackReason="cannot verify loaded editor assembly"; }
            if(EditorHooks && !String.Equals(editorHash,Environment.GetEnvironmentVariable("BLUEBERRY_EDITOR_DLL_SHA256"),StringComparison.OrdinalIgnoreCase)) {
                EditorHooks=false; editorFallbackReason="private editor identity mismatch";
            } else if(!EditorHooks && editorFallbackReason==null) editorFallbackReason="loaded PSReadLine has no editor integration v1";
            if(EditorHooks) StartBackgroundJit();
            buffer = (BufferState)Public("GetBufferState", typeof(BufferState), typeof(string).MakeByRefType(), typeof(int).MakeByRefType());
            replace = (ReplaceText)Public("Replace", typeof(ReplaceText), typeof(int), typeof(int), typeof(string), typeof(Action<ConsoleKeyInfo?, object>), typeof(object));
            try {
                if (EditorHooks) {
                    hubChord = Environment.GetEnvironmentVariable("BLUEBERRY_PUBLIC_KEY_HUB") ?? "Ctrl+Alt+p";
                    refreshChord = Environment.GetEnvironmentVariable("BLUEBERRY_PUBLIC_KEY_REFRESH") ?? "Ctrl+Alt+c";
                    requestEditorRefresh = (Action)Public("RequestEditorRefresh", typeof(Action));
                    if(diagnostic) editorTracePoints=(Func<long[]>)Public("GetEditorTracePoints",typeof(Func<long[]>));
                    editorRegistration = (IDisposable)api.GetMethod("RegisterEditorIntegration").Invoke(null, new object[] {
                        1, (Action<string,string>)Begin, (Action)Clear,
                        (Func<ConsoleKeyInfo?,object,bool,bool>)EditorKey,
                        (Action)delegate { if (editing && AutomaticMenu) Query("query"); },
                        (Action)delegate { Refresh(); }, (Action)End, (Action<string>)Disable
                    });
                    enabled = true;
                } else {
                    InitializeLegacy(initialization, out snapshotTicks, out registrationTicks);
                }
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
                {"editor_mode", EditorHooks ? "editor_hooks_v1" : "legacy"},
                {"editor_patch", EditorHooks ? (string)api.GetProperty("EditorIntegrationPatch").GetValue(null, null) : null},
                {"editor_dll_sha256", editorHash}, {"editor_fallback_reason", editorFallbackReason},
                {"shell_version", shellVersion},
                {"capabilities", new object[] {"revision", "frame_identity", "accept_identity", "utf16_edit"}}
            });
            long handshakeDeadline=Stopwatch.GetTimestamp()+Stopwatch.Frequency*5;
            while(connected && !negotiated && Stopwatch.GetTimestamp()<handshakeDeadline) arrived.WaitOne(10);
            if(!negotiated) Disable("direct protocol capabilities were not acknowledged");
            ReportStartupPreparation();
            TraceStage("direct_binding_snapshot",snapshotTicks,originals.Count);
            TraceStage("direct_binding_registration",registrationTicks,installed.Count);
            TraceStage("direct_bridge_initialization",Stopwatch.GetTimestamp()-initialization,installed.Count);
        }
        static void InitializeLegacy(long initialization, out long snapshotTicks, out long registrationTicks) {
            // Observe a pending input burst, without reading/consuming keys or
            // changing any private state. All buffer confirmation is still
            // via GetBufferState. The supported layouts are verified below.
            var singleton=api.GetField("_singleton",BindingFlags.NonPublic|BindingFlags.Static);
            var queue=api.GetField("_queuedKeys",BindingFlags.NonPublic|BindingFlags.Instance);
            if(!EditorHooks && singleton!=null && queue!=null) queuedInput=queue.GetValue(singleton.GetValue(null)) as ICollection;
            register = (Register)Public("SetKeyHandler", typeof(Register), typeof(string[]), typeof(Action<ConsoleKeyInfo?, object>), typeof(string), typeof(string));
            selfInsert = (Action<ConsoleKeyInfo?, object>)Public("SelfInsert", typeof(Action<ConsoleKeyInfo?, object>), typeof(ConsoleKeyInfo?), typeof(object));
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
                RegisterCharacters(bound);
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
        }
        public static void PublishEnvironment() {
            string root=Environment.GetEnvironmentVariable("BLUEBERRY_EDITOR_MODULE_ROOT");
            if(!String.IsNullOrEmpty(root)) {
                var paths=new List<string>();
                foreach(string path in (Environment.GetEnvironmentVariable("PSModulePath") ?? "").Split(';'))
                    if(!String.Equals(path,root,StringComparison.OrdinalIgnoreCase)) paths.Add(path);
                Environment.SetEnvironmentVariable("PSModulePath",String.Join(";",paths.ToArray()));
            }
            Environment.SetEnvironmentVariable("BLUEBERRY_HOST_MODE","direct");
            Environment.SetEnvironmentVariable("BLUEBERRY_TRANSPORT_ACTUAL","pipe");
            Environment.SetEnvironmentVariable("BLUEBERRY_AUTOMATIC_MENU",AutomaticMenu ? "true" : "false");
            Environment.SetEnvironmentVariable("BLUEBERRY_AUTOMATIC_MENU_DISABLED_REASON",DisabledReason);
        }
        public static void PrepareStartup() {
            if(Environment.GetEnvironmentVariable("BLUEBERRY_NO_HISTORY")=="1" && Environment.GetEnvironmentVariable("BLUEBERRY_TEST_DISABLE_PREJIT")=="1") return;
            if(Interlocked.Exchange(ref startupPreparation,1)!=0) return;
            // Bridge-only preparation can overlap module/options initialization.
            // It has no reference to the editor type and cannot initialize that
            // type on this worker. The ordinary editor pass starts in Initialize.
            new Thread(delegate() {
                startupPreparationStarted=Stopwatch.GetTimestamp(); int prepared=0;
                foreach(string name in new string[]{"Initialize","Public","Send","ReadFrames"}) {
                    try { RuntimeHelpers.PrepareMethod(typeof(Bridge).GetMethod(name,BindingFlags.Public|BindingFlags.NonPublic|BindingFlags.Static).MethodHandle); prepared++; } catch { }
                }
                foreach(string name in new string[]{"Encode","Parse"}) {
                    try { RuntimeHelpers.PrepareMethod(typeof(Json).GetMethod(name).MethodHandle); prepared++; } catch { }
                }
                var parser=typeof(Json).GetNestedType("Parser",BindingFlags.NonPublic);
                foreach(MethodInfo method in parser.GetMethods(BindingFlags.Public|BindingFlags.NonPublic|BindingFlags.Instance|BindingFlags.DeclaredOnly)) {
                    try { RuntimeHelpers.PrepareMethod(method.MethodHandle); prepared++; } catch { }
                }
                startupPreparationCount=prepared;
                Volatile.Write(ref startupPreparationFinished,Stopwatch.GetTimestamp());
                ReportStartupPreparation();
            }) { IsBackground=true, Name="Blueberry bridge compilation" }.Start();
        }
        static void ReportStartupPreparation() {
            long finished=Volatile.Read(ref startupPreparationFinished);
            if(!diagnostic || !negotiated || finished==0 || Interlocked.Exchange(ref startupPreparationReported,1)!=0) return;
            StartupPoint("direct_bridge_jit_started",startupPreparationStarted);
            StartupPoint("direct_bridge_jit_finished",finished);
            TraceStage("direct_bridge_jit",finished-startupPreparationStarted,startupPreparationCount);
        }
        static void StartBackgroundJit() {
            if(Environment.GetEnvironmentVariable("BLUEBERRY_NO_HISTORY")=="1" && Environment.GetEnvironmentVariable("BLUEBERRY_TEST_DISABLE_PREJIT")=="1") return;
            // Compile only: never invoke editor methods, read the input buffer,
            // or write the console from this worker. This finite pass overlaps
            // CLR compilation with the shell's remaining initialization.
            new Thread(delegate() {
                long started=Stopwatch.GetTimestamp(); int prepared=0, failed=0;
                // Prepare the public ReadLine entry and native console setup
                // before rendering helpers: both are reached before InputLoop.
                // The editor type has already initialized on the shell thread.
                var methods=api.GetMethods(BindingFlags.Public|BindingFlags.NonPublic|BindingFlags.Static|BindingFlags.Instance|BindingFlags.DeclaredOnly);
                foreach(string name in new string[]{"ReadLine","Initialize","DelayedOneTimeInitialize","InputLoop","ReadKey","ReadLineWithEditorIntegration","Render","ForceRender","ReallyRender","GenerateRender","Insert","SelfInsert","ProcessOneKey","EditorProcessKey","EditorAfterKey"}) {
                    foreach(MethodInfo method in methods) {
                        if(method.Name!=name || method.ContainsGenericParameters || method.IsAbstract) continue;
                        try { RuntimeHelpers.PrepareMethod(method.MethodHandle); prepared++; } catch { failed++; }
                    }
                }
                var platform=api.Assembly.GetType("Microsoft.PowerShell.PlatformWindows");
                if(platform!=null) {
                    foreach(MethodInfo method in platform.GetMethods(BindingFlags.Public|BindingFlags.NonPublic|BindingFlags.Static)) {
                        if(method.Name!="Init" || method.ContainsGenericParameters || method.IsAbstract) continue;
                        try { RuntimeHelpers.PrepareMethod(method.MethodHandle); prepared++; } catch { failed++; }
                    }
                }
                foreach(string name in new string[]{"Begin","Clear","EditorKey","Query","Refresh","Paint"}) {
                    try { RuntimeHelpers.PrepareMethod(typeof(Bridge).GetMethod(name,BindingFlags.Public|BindingFlags.NonPublic|BindingFlags.Static).MethodHandle); prepared++; } catch { failed++; }
                }
                TraceStage("direct_background_jit",Stopwatch.GetTimestamp()-started,prepared);
                StartupPoint("direct_background_jit_started",started);
                StartupPoint("direct_background_jit_finished",Stopwatch.GetTimestamp());
                if(failed>0) TraceStage("direct_background_jit_failed",0,failed);
            }) { IsBackground=true, Name="Blueberry startup compilation" }.Start();
        }
        static bool EditorKey(ConsoleKeyInfo? key, object nativeHandler, bool builtin) {
            if (!editing || !AutomaticMenu || !key.HasValue || !builtin) return false;
            var k = key.Value;
            string chord = ((k.Modifiers & ConsoleModifiers.Control) != 0 ? "Ctrl+" : "")
                + ((k.Modifiers & ConsoleModifiers.Alt) != 0 ? "Alt+" : "")
                + ((k.Modifiers & ConsoleModifiers.Shift) != 0 ? "Shift+" : "") + k.Key;
            if (nativeHandler == null && String.Equals(chord, refreshChord, StringComparison.OrdinalIgnoreCase)) {
                Send(new Dictionary<string,object> {{"event","commands_refresh"},{"session",commandSession}});
                StartCommands(); Query("query"); return true;
            }
            if (nativeHandler == null && String.Equals(chord, hubChord, StringComparison.OrdinalIgnoreCase)) {
                interaction = interaction == "completion" ? "hub" : "completion";
                Query(interaction == "hub" ? "hub" : "cancel"); return true;
            }
            if (interaction != "completion") { SendKey(k); return true; }
            if (displayed == null) return false;
            // Compare the action resolved for this very keystroke. A user may
            // rebind to another built-in action, not only to a script block.
            var action = nativeHandler as Delegate;
            string name = action == null ? null : action.Method.Name;
            if (k.Key == ConsoleKey.Tab && name != "Complete" && name != "MenuComplete" && name != "TabCompleteNext" && name != "TabCompletePrevious") return false;
            if (k.Key == ConsoleKey.UpArrow && name != "PreviousHistory" && name != "HistorySearchBackward") return false;
            if (k.Key == ConsoleKey.DownArrow && name != "NextHistory" && name != "HistorySearchForward") return false;
            if (k.Key == ConsoleKey.F1 && action != null) return false;
            if (k.Key == ConsoleKey.Escape && name != null && name != "RevertLine") return false;
            if (k.Key == ConsoleKey.Escape) { interaction="completion"; Query("cancel"); return true; }
            if (k.Modifiers == 0 && (k.Key == ConsoleKey.UpArrow || k.Key == ConsoleKey.DownArrow || k.Key == ConsoleKey.F1)) {
                SendKey(k,"menu_key"); return true;
            }
            if (k.Key == ConsoleKey.Tab) { Accept(); return true; }
            return false;
        }
        static void RegisterCharacters(HashSet<string> bound) {
            var characters = new List<string>();
            for (int unit = 32; unit <= Char.MaxValue; unit++) {
                char ch = (char)unit;
                // PSReadLine stores literal space as Spacebar. Do not overwrite
                // the captured original action with the default SelfInsert hook.
                if (!Char.IsControl(ch) && !bound.Contains(ch.ToString())
                    && !(ch == ' ' && bound.Contains("Spacebar"))) characters.Add(ch.ToString());
            }
            Action<ConsoleKeyInfo?, object> insert = delegate(ConsoleKeyInfo? key, object arg) { Edit(selfInsert, key, arg); };
            wrappers.Add(insert);
            register(characters.ToArray(), insert, "SelfInsert", "Blueberry confirmed edit");
        }
        static void Disable(string reason) {
            enabled = false; DisabledReason = reason; Clear();
            if (EditorHooks) {
                if (editorRegistration != null) { editorRegistration.Dispose(); editorRegistration = null; }
                return;
            }
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
        static IEnumerator<string[]> commandEnumerator;
        static string commandSnapshot;
        static long commandSession, commandOffset;
        static void StopCommands() {
            if (commandEnumerator != null) { commandEnumerator.Dispose(); commandEnumerator = null; }
        }
        static void StartCommands() {
            StopCommands(); commandSnapshot = Guid.NewGuid().ToString("N"); commandOffset = 0;
            try {
                var method = api.GetMethod("GetEditorCommands");
                if (method == null) throw new NotSupportedException("Session command enumeration unavailable");
                commandEnumerator = (IEnumerator<string[]>)method.Invoke(null, null);
                Send(new Dictionary<string,object> {{"event","commands_started"},{"session",commandSession},{"snapshot",commandSnapshot}});
                requestEditorRefresh();
            } catch (Exception error) { CommandError(error); }
        }
        static void CommandError(Exception error) {
            StopCommands();
            Send(new Dictionary<string,object> {{"event","commands_error"},{"session",commandSession},
                {"error",error.GetBaseException().Message}});
        }
        static void PumpCommands() {
            if (commandEnumerator == null) return;
            var batch = new List<object>(); bool complete = false;
            long start = Stopwatch.GetTimestamp();
            try {
                do {
                    if (!commandEnumerator.MoveNext()) { complete = true; break; }
                    var record = commandEnumerator.Current;
                    batch.Add(new Dictionary<string,object> {{"name",record[0]},{"kind",record[1]},{"definition",record[2]}});
                } while (batch.Count < 64 && Stopwatch.GetTimestamp()-start < Stopwatch.Frequency/500);
                Send(new Dictionary<string,object> {{"event","commands"},{"session",commandSession},
                    {"snapshot",commandSnapshot},{"offset",commandOffset},{"complete",complete},{"commands",batch}});
                commandOffset += batch.Count;
                if (complete) StopCommands(); else requestEditorRefresh();
            } catch (Exception error) { CommandError(error); }
        }
        public static void Begin(string cwd, string historyPath) {
            long beginning=diagnostic ? Stopwatch.GetTimestamp() : 0;
            Clear(); displayed = null; acceptingFrame = null; expectedUiEditRevision=-1; interaction = "completion"; directory = cwd; editing = true; editorThread = Thread.CurrentThread.ManagedThreadId;
            revision++;
            commandSession = revision;
            if (enabled && !EditorHooks) {
                try { AuditBindings(); } catch (Exception error) { Disable(error.GetBaseException().Message); }
            }
            if(Environment.GetEnvironmentVariable("BLUEBERRY_NO_HISTORY")=="1") historyPath=null;
            var environment = new Dictionary<string, object>(StringComparer.OrdinalIgnoreCase);
            foreach(DictionaryEntry entry in Environment.GetEnvironmentVariables()) environment[((string)entry.Key).ToUpperInvariant()] = (string)entry.Value;
            Send(new Dictionary<string, object> { {"event", "begin"}, {"revision", revision}, {"cwd",cwd}, {"history_path",historyPath}, {"environment",environment}, {"automatic_menu", AutomaticMenu}, {"disabled_reason", DisabledReason} });
            if (EditorHooks) StartCommands();
            if(diagnostic) TraceStage("direct_readline_begin",Stopwatch.GetTimestamp()-beginning,1);
        }
        public static void End() {
            StopCommands();
            string line; int cursor; buffer(out line,out cursor);
            Clear(); displayed = null; acceptingFrame=null; expectedUiEditRevision=-1; editing = false; revision++;
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
            expectedUiEditRevision=operation=="ui_key" && (key.Key==ConsoleKey.Enter || key.Key==ConsoleKey.Tab) ? revision : -1;
            Send(new Dictionary<string,object> {
                {"event",operation},{"revision",revision},{"line",line},{"cursor",cursor},
                {"key",key.Key.ToString()},{"character_unit",(long)key.KeyChar},{"modifiers",(int)key.Modifiers},
                {"frame_id",displayed == null ? -1 : Json.Long(displayed,"frame_id")},
                {"candidate_id",displayed == null ? null : Json.String(displayed,"candidate_id")}
            });
            displayed=null;
            if(key.Key==ConsoleKey.Escape) interaction="completion";
            if(EditorHooks && operation=="ui_key" && (key.Key==ConsoleKey.Enter || key.Key==ConsoleKey.Tab)) {
                long deadline=Stopwatch.GetTimestamp()+Stopwatch.Frequency/2;
                while(connected && Stopwatch.GetTimestamp()<deadline) {
                    if(Refresh()) return;
                    lock(frameLock) { if(reliableFrames.Count>0 || pending!=null) continue; }
                    int remaining=(int)Math.Max(1,(deadline-Stopwatch.GetTimestamp())*1000/Stopwatch.Frequency);
                    arrived.WaitOne(remaining);
                }
                interaction="completion"; Query("cancel");
                return;
            }
            WaitFrame();
        }
        static void Query(string operation) {
            expectedUiEditRevision=-1;
            string line; int cursor; buffer(out line, out cursor);
            long confirmed=diagnostic ? Stopwatch.GetTimestamp() : 0;
            // A high UTF-16 surrogate can arrive as one console key before its
            // low surrogate. Wait for PSReadLine to confirm the complete pair.
            if (SplitsPair(line,cursor) || Unpaired(line)) { displayed=null; return; }
            revision++; displayed = null;
            if(diagnostic && editorTracePoints!=null) {
                long[] points=editorTracePoints();
                string[] stages={"editor_loop_enter","editor_begin_complete","editor_key_received","editor_native_dispatch","editor_native_complete"};
                for(int i=0;i<points.Length;i++) if(points[i]!=0) TracePoint(stages[i],revision,-1,points[i]);
            }
            // Do not spend a 4 ms response budget for each character already
            // queued in a paste/burst. The final confirmed edit sends a query.
            // A single first key never skips initialization or its query.
            if(operation=="query" && queuedInput!=null && queuedInput.Count>0) return;
            // A queued burst still delegates each captured original action.
            // Audit once before publishing the final confirmed state instead
            // of rescanning 65k bindings for every code unit in the burst.
            long audited=diagnostic ? Stopwatch.GetTimestamp() : 0;
            if (!EditorHooks) { try { AuditBindings(); } catch(Exception error) { Disable(error.GetBaseException().Message); return; } }
            if(diagnostic) TraceStage("direct_binding_audit",Stopwatch.GetTimestamp()-audited,installed.Count);
            int width = Math.Max(1, Console.WindowWidth), rows = Math.Max(1, Console.WindowHeight);
            var query=new Dictionary<string, object> {
                {"event", operation}, {"revision", revision}, {"line", line}, {"cursor", cursor},
                {"cwd", directory}, {"width", width}, {"rows", rows}
            };
            if(diagnostic) query["confirmed_qpc"]=confirmed;
            Send(query);
            TracePoint("editor_confirmed",revision,-1,confirmed);
            if(diagnostic) TraceStage("direct_confirmed_query_send",Stopwatch.GetTimestamp()-confirmed,line.Length);
            WaitFrame();
        }
        static void WaitFrame() {
            if(EditorHooks || responseTimer==IntPtr.Zero || !AutomaticMenu) return;
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
            acceptingFrame = frame;
            Send(new Dictionary<string, object> {
                {"event", "accept"}, {"revision", revision}, {"frame_id", Json.Long(frame, "frame_id")},
                {"candidate_id", Json.String(frame, "candidate_id")}, {"line", line}, {"cursor", cursor}
            });
            // Explicit acceptance is ordered before the next editor keystroke.
            // Ordinary typing never enters this wait on the integrated path.
            long deadline = Stopwatch.GetTimestamp() + Stopwatch.Frequency / 2;
            while (connected && Stopwatch.GetTimestamp() < deadline) {
                Refresh();
                if (acceptingFrame == null) return;
                lock(frameLock) { if(reliableFrames.Count>0 || pending!=null) continue; }
                int remaining = (int)Math.Max(1, (deadline-Stopwatch.GetTimestamp())*1000/Stopwatch.Frequency);
                arrived.WaitOne(remaining);
            }
            acceptingFrame = null;
            Query("cancel"); // Revoke a timed-out edit before later typing.
        }
        public static bool Refresh() {
            if (!editing || Thread.CurrentThread.ManagedThreadId != editorThread) return false;
            if (!connected) { Clear(); return false; }
            if (EditorHooks) PumpCommands();
            if(!EditorHooks && enabled && !waitingForFrame) { try { AuditBindings(); } catch(Exception error) { Disable(error.GetBaseException().Message); return false; } }
            Dictionary<string, object> frame;
            long frameReceived;
            lock (frameLock) {
                if (reliableFrames.Count > 0) frame = reliableFrames.Dequeue();
                else { frame = pending; pending = null; }
                frameReceived=frame == null ? 0 : Json.Long(frame,"received_qpc");
                if (EditorHooks && (reliableFrames.Count > 0 || pending != null)) requestEditorRefresh();
            }
            if (frame == null || Json.Long(frame, "revision") != revision) return false;
            TracePoint("editor_refresh_enter",revision,Json.Long(frame,"frame_id"),Stopwatch.GetTimestamp());
            string line; int cursor; buffer(out line, out cursor);
            if (Json.String(frame, "expected_line") != line || Json.Long(frame, "expected_cursor") != cursor) return false;
            if (Json.String(frame,"kind") == "clear") { Clear(); displayed=null; expectedUiEditRevision=-1; interaction="completion"; return true; }
            if (Json.String(frame, "kind") == "edit") {
                if (!EditAuthorized(frame)) return false;
                int start = (int)Json.Long(frame, "start"), length = (int)Json.Long(frame, "length");
                string text = Json.String(frame, "text");
                if (start < 0 || length < 0 || start > line.Length - length || SplitsPair(line, start) || SplitsPair(line, start + length)) return false;
                Clear(); expectedUiEditRevision=-1;
                replace(start, length, text, null, null); acceptingFrame = null; displayed = null; interaction="completion"; Query("query"); return true;
            }
            if (acceptingFrame != null) return false;
            object lines;
            if (Json.String(frame, "kind") != "frame" || !frame.TryGetValue("lines", out lines) || !(lines is List<object>)) return false;
            Clear();
            long painted=diagnostic ? Stopwatch.GetTimestamp() : 0;
            if (!Paint((List<object>)lines, (int)Json.Long(frame, "tail_rows"), (int)Json.Long(frame, "width"))) { displayed = null; return false; }
            TracePoint("editor_menu_written",revision,Json.Long(frame,"frame_id"),Stopwatch.GetTimestamp());
            if(diagnostic) TraceStage("direct_console_menu_output",Stopwatch.GetTimestamp()-painted,((List<object>)lines).Count);
            if(diagnostic && frameReceived!=0) TraceStage("direct_frame_to_paint",Stopwatch.GetTimestamp()-frameReceived,1);
            expectedUiEditRevision=-1;
            interaction=Json.String(frame,"interaction") ?? "completion"; displayed = frame; return true;
        }
        static bool EditAuthorized(Dictionary<string,object> frame) {
            if(Json.Long(frame,"revision")!=revision) return false;
            if(acceptingFrame!=null) return Json.Long(acceptingFrame,"revision")==revision && Json.Long(frame,"frame_id")==Json.Long(acceptingFrame,"frame_id")
                && Json.String(frame,"candidate_id")==Json.String(acceptingFrame,"candidate_id");
            return expectedUiEditRevision==revision && Json.Long(frame,"revision")==revision && !frame.ContainsKey("frame_id");
        }
        static bool QueueIncomingFrame(Dictionary<string,object> frame, long received) {
            lock(frameLock) {
                if(Json.String(frame,"kind")=="frame") {
                    long rev=Json.Long(frame,"revision"), id=Json.Long(frame,"frame_id");
                    if(rev<receivedFrameRevision || (rev==receivedFrameRevision && id<=receivedFrameId)) return false;
                    receivedFrameRevision=rev; receivedFrameId=id;
                }
                frame["received_qpc"]=received;
                if(Json.String(frame,"kind")!="frame" || Json.String(frame,"interaction")!="completion") {
                    if(reliableFrames.Count>=4096) throw new InvalidDataException("control frame queue exceeded");
                    reliableFrames.Enqueue(frame);
                } else pending=frame;
                return true;
            }
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
        static void TracePoint(string stage,long rev,long frame,long qpc) {
            if(diagnostic && negotiated) Send(new Dictionary<string,object>{{"event","trace_point"},{"revision",rev},{"frame_id",frame},{"stage",stage},{"qpc",qpc}});
        }
        public static void StartupPoint(string stage,long qpc) { TracePoint(stage,-1,-1,qpc); }
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
                        if(testFrameDelay>0 && Json.String(frame,"kind")=="frame") Thread.Sleep(testFrameDelay);
                        receivedQpc=Stopwatch.GetTimestamp();
                        if(!QueueIncomingFrame(frame,receivedQpc)) continue;
                        arrived.Set();
                        if (EditorHooks) requestEditorRefresh(); else Events.Raise();
                        TracePoint("editor_frame_received",Json.Long(frame,"revision"),Json.Long(frame,"frame_id"),receivedQpc);
                    }
                }
            } catch { }
            connected = false; arrived.Set();
            if (EditorHooks) requestEditorRefresh(); else Events.Raise();
        }
        public static void Shutdown() { if(editing) End(); connected = false; if(editorRegistration!=null) { editorRegistration.Dispose(); editorRegistration=null; } if (pipe != null) pipe.Dispose(); if(responseTimer!=IntPtr.Zero) { CloseHandle(responseTimer); responseTimer=IntPtr.Zero; } }

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
                var outputText = new StringBuilder();
                for (int i=0; i<height; i++) {
                    outputText.Append("\x1b[").Append(top+i-info.Window.Top+1).Append(";1H");
                    // Rust renderer emits sanitized text and SGR only. No line
                    // feeds: never scroll the inherited console from an overlay.
                    outputText.Append(lines[i] as string).Append("\x1b[0m");
                }
                outputText.Append("\x1b[").Append(info.Cursor.Y-info.Window.Top+1).Append(';').Append(info.Cursor.X+1).Append('H');
                // Legacy ConPTY flushes its backing render buffer before OSC
                // 1337 actions. This private, unknown action has no terminal
                // effect or reply; it is a frame boundary after the real menu
                // and cursor restoration, not an input event or a paint thread.
                outputText.Append("\x1b]1337;BlueberryFrame\x07");
                Console.Write(outputText.ToString());
            } catch { Console.SetCursorPosition(info.Cursor.X, info.Cursor.Y); throw; }
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
