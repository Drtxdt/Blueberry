// Blueberry editor integration, API v1. MIT licensed; upstream editing stays intact.
using System;
using System.Threading;

namespace Microsoft.PowerShell
{
    public partial class PSConsoleReadLine
    {
        public static int EditorIntegrationVersion { get { return 1; } }
        public static string EditorIntegrationPatch { get { return "blueberry-editor-v1"; } }

        private readonly AutoResetEvent _editorRefreshEvent = new AutoResetEvent(false);
        private EditorRegistration _editorIntegration;

        // Callbacks are synchronous and run exclusively on the ReadLine thread.
        // The bool passed to key means the resolved binding is an upstream action
        // (or the default SelfInsert). A custom user handler always takes priority.
        public static IDisposable RegisterEditorIntegration(
            int version, Action<string, string> begin, Action before,
            Func<ConsoleKeyInfo?, object, bool, bool> key, Action after,
            Action refresh, Action end, Action<string> fault)
        {
            if (version != 1) throw new NotSupportedException("Editor integration version");
            var registration = new EditorRegistration(begin, before, key, after, refresh, end, fault);
            if (Interlocked.CompareExchange(ref _singleton._editorIntegration, registration, null) != null)
                throw new InvalidOperationException("An editor integration is already registered");
            return registration;
        }

        // No key is inserted and no callback is run on the caller's thread.
        public static void RequestEditorRefresh()
        {
            var registration = Volatile.Read(ref _singleton._editorIntegration);
            if (registration != null && registration.Active) _singleton._editorRefreshEvent.Set();
        }

        private sealed class EditorRegistration : IDisposable
        {
            internal readonly Action<string, string> Begin;
            internal readonly Action Before, After, Refresh, End;
            internal readonly Func<ConsoleKeyInfo?, object, bool, bool> Key;
            internal readonly Action<string> Fault;
            internal volatile bool Active;
            internal EditorRegistration(Action<string, string> begin, Action before,
                Func<ConsoleKeyInfo?, object, bool, bool> key, Action after,
                Action refresh, Action end, Action<string> fault)
            { Begin = begin; Before = before; Key = key; After = after; Refresh = refresh; End = end; Fault = fault; }
            public void Dispose()
            {
                Active = false;
                Interlocked.CompareExchange(ref _singleton._editorIntegration, null, this);
                // The one singleton event lives as long as PSReadLine. Disposing a
                // registration must not close a handle in an outstanding WaitAny.
                _singleton._editorRefreshEvent.Set();
            }
        }

        private void EditorCall(EditorRegistration registration, Action callback)
        {
            if (registration == null || callback == null) return;
            try { callback(); }
            catch (Exception error)
            {
                registration.Dispose();
                try { registration.Fault?.Invoke(error.GetBaseException().Message); } catch { }
            }
        }

        private string ReadLineWithEditorIntegration()
        {
            var registration = _editorIntegration;
            if (registration == null) return InputLoop();
            _editorRefreshEvent.Reset();
            registration.Active = true;
            EditorCall(registration, () => registration.Begin?.Invoke(
                _engineIntrinsics.SessionState.Path.CurrentFileSystemLocation.Path,
                _options.HistorySavePath));
            try { return InputLoop(); }
            finally
            {
                registration.Active = false;
                EditorCall(registration, registration.End);
            }
        }

        private WaitHandle[] EditorWaitHandles(bool safePoint)
        {
            var registration = _editorIntegration;
            if (!safePoint || registration == null || !registration.Active || InViCommandMode())
                return _requestKeyWaitHandles;
            var handles = new WaitHandle[_requestKeyWaitHandles.Length + 1];
            Array.Copy(_requestKeyWaitHandles, handles, _requestKeyWaitHandles.Length);
            handles[handles.Length - 1] = _editorRefreshEvent;
            return handles;
        }

        private bool EditorHasQueuedKey()
        {
            // The read-key worker stops before signalling _keyReadWaitHandle.
            // Until we signal it again, this thread exclusively owns the queue.
            // Drain already collected keys without a redundant console RPC and
            // round trip through that worker for every UTF-16 input unit.
            var registration = _editorIntegration;
            return registration != null && registration.Active && _queuedKeys.Count > 0
                && !_cancelReadCancellationToken.IsCancellationRequested
                && !_closingWaitHandle.WaitOne(0);
        }

        private void EditorRefresh()
        {
            var registration = _editorIntegration;
            if (registration != null && registration.Active && !InViCommandMode())
                EditorCall(registration, registration.Refresh);
        }

        private void EditorBeforeEvent(bool safePoint)
        {
            var registration = _editorIntegration;
            if (safePoint && registration != null && registration.Active)
                EditorCall(registration, registration.Before);
        }

        private bool EditorProcessKey(PSKeyInfo key)
        {
            var registration = _editorIntegration;
            if (registration == null || !registration.Active) return false;
            EditorCall(registration, registration.Before);
            if (_editorIntegration != registration || registration.Key == null || InViCommandMode()) return false;
            KeyHandler handler;
            _dispatchTable.TryGetValue(key, out handler);
            bool builtin = handler == null || (handler.ScriptBlock == null &&
                handler.Action.Target == null && handler.Action.Method.DeclaringType == typeof(PSConsoleReadLine));
            bool consumed = false;
            EditorCall(registration, () => consumed = registration.Key(key.AsConsoleKeyInfo(),
                handler == null ? null : (object)handler.Action, builtin));
            return consumed;
        }

        private void EditorAfterKey()
        {
            var registration = _editorIntegration;
            if (registration != null && registration.Active && !_inputAccepted &&
                _queuedKeys.Count == 0 && !InViCommandMode())
                EditorCall(registration, registration.After);
        }
    }
}
