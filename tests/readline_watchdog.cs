// Read-only diagnostics for isolated synthetic fixtures. Never writes terminal
// output, consumes input records, signals events, or invokes editor operations.
using System;
using System.Collections;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

namespace Blueberry.Test
{
    public static class ReadlineWatchdog
    {
        private static Timer timer;
        private static Type editor;
        private static string path;
        private static string previous;
        private static int unchanged;
        private static bool captured;
        private const BindingFlags Fields = BindingFlags.Static | BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic;
        [DllImport("kernel32.dll")] private static extern IntPtr GetStdHandle(int id);
        [DllImport("kernel32.dll")] private static extern bool GetNumberOfConsoleInputEvents(IntPtr h, out uint count);
        [DllImport("kernel32.dll")] private static extern bool GetConsoleMode(IntPtr h, out uint mode);
        [DllImport("kernel32.dll", EntryPoint="PeekConsoleInputW")] private static extern bool PeekConsoleInput(IntPtr h, byte[] data, uint count, out uint read);
        [DllImport("ntdll.dll")] private static extern int NtQueryEvent(IntPtr h, int kind, int[] data, uint size, out uint read);

        public static void Start(Type type, string destination)
        {
            editor = type;
            path = destination;
            timer = new Timer(Snapshot, null, 3000, 1000);
        }

        private static object Field(object instance, Type type, string name)
        {
            FieldInfo field = type.GetField(name, Fields);
            return field == null ? null : field.GetValue(instance);
        }

        private static void Snapshot(object state)
        {
            try
            {
                object singleton = Field(null, editor, "_singleton");
                StringBuilder text = new StringBuilder(DateTime.UtcNow.ToString("O"));
                foreach (string name in new[] { "_readKeyWaitHandle", "_keyReadWaitHandle", "_closingWaitHandle" })
                {
                    WaitHandle handle = Field(singleton, editor, name) as WaitHandle;
                    int[] info = new int[2]; uint read;
                    int status = handle == null ? -1 : NtQueryEvent(handle.SafeWaitHandle.DangerousGetHandle(), 0, info, 8, out read);
                    text.AppendFormat("\t{0}={1},{2},{3}", name, status, info[0], info[1]);
                }
                Thread thread = Field(singleton, editor, "_readKeyThread") as Thread;
                text.Append("\treader=").Append(thread == null ? "null" : thread.ThreadState.ToString());
                object queue = Field(singleton, editor, "_queuedKeys");
                text.Append("\tqueued=").Append(queue.GetType().GetProperty("Count").GetValue(queue, null));
                foreach (object key in (IEnumerable)queue) text.Append("[").Append(key).Append("]");
                object history = Field(null, editor, "_lastNKeys");
                if (history != null)
                {
                    int count = (int)history.GetType().GetProperty("Count").GetValue(history, null);
                    PropertyInfo item = history.GetType().GetProperty("Item");
                    text.Append("\trecent=");
                    for (int i = Math.Max(0, count - 12); i < count; i++)
                        text.Append("[").Append(item.GetValue(history, new object[] { i })).Append("]");
                }
                IntPtr input = GetStdHandle(-10); uint pending, mode, peeked;
                GetNumberOfConsoleInputEvents(input, out pending); GetConsoleMode(input, out mode);
                byte[] records = new byte[20 * 8]; PeekConsoleInput(input, records, 8, out peeked);
                text.AppendFormat("\tnative={0},mode={1:x},peek=", pending, mode);
                for (int i = 0; i < peeked; i++)
                    text.Append(BitConverter.ToString(records, i * 20, 20)).Append(';');
                File.AppendAllText(path, text.AppendLine().ToString());
                string signature = text.ToString().Substring(28);
                unchanged = pending > 0 && signature == previous ? unchanged + 1 : 0;
                previous = signature;
                string dotnet = Environment.GetEnvironmentVariable("BLUEBERRY_TEST_STACK_DOTNET");
                string tool = Environment.GetEnvironmentVariable("BLUEBERRY_TEST_STACK_READER");
                if (!captured && unchanged >= 3 && !String.IsNullOrEmpty(dotnet) && !String.IsNullOrEmpty(tool))
                {
                    captured = true;
                    System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo {
                        FileName = dotnet,
                        Arguments = "\"" + tool + "\" " + System.Diagnostics.Process.GetCurrentProcess().Id + " \"" + path + ".stacks\"",
                        UseShellExecute = false, CreateNoWindow = true
                    });
                }
            }
            catch (Exception error)
            {
                try { File.AppendAllText(path, DateTime.UtcNow.ToString("O") + "\terror=" + error + "\n"); } catch { }
            }
        }
    }
}
