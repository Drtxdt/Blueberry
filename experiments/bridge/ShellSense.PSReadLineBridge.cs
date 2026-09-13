#nullable enable

using System;
using System.Collections;
using System.Collections.Generic;
using System.IO;
using System.Reflection;
using System.Text.Encodings.Web;
using System.Text.Json;

namespace ShellSense.Bridge;

/// <summary>
/// A detached, cache-only adapter for the public PSReadLine surface.
///
/// This assembly deliberately does not reference PSReadLine at compile time.
/// The host must load PSReadLine first; Bind scans already loaded assemblies
/// and caches the public MethodInfo instances for the current runspace.
/// </summary>
public static class PsReadLineBridge
{
    private const string ConsoleReadLineTypeName = "Microsoft.PowerShell.PSConsoleReadLine";
    private const string ScriptBlockTypeName = "System.Management.Automation.ScriptBlock";

    private static readonly object Gate = new();
    private static readonly JsonSerializerOptions JsonOptions = CreateJsonOptions();

    private static Type? s_consoleReadLineType;
    private static MethodInfo? s_getKeyHandlers;
    private static MethodInfo? s_getBufferState;
    private static MethodInfo? s_replace;
    private static MethodInfo? s_setKeyHandler;
    private static string? s_bindError;

    /// <summary>
    /// Returns whether the public PSReadLine methods were found in a loaded
    /// assembly. This call never imports PSReadLine or invokes user code.
    /// </summary>
    public static bool IsBound
    {
        get
        {
            lock (Gate)
            {
                return s_consoleReadLineType is not null;
            }
        }
    }

    /// <summary>
    /// Binds the bridge to the already loaded PSReadLine type. Calling this
    /// repeatedly is cheap after the first successful bind.
    /// </summary>
    public static bool TryBind(out string error)
    {
        lock (Gate)
        {
            if (s_consoleReadLineType is not null)
            {
                error = string.Empty;
                return true;
            }

            if (TryFindPublicSurface(
                    out var consoleReadLineType,
                    out var getKeyHandlers,
                    out var getBufferState,
                    out var replace,
                    out var setKeyHandler,
                    out var bindError))
            {
                s_consoleReadLineType = consoleReadLineType;
                s_getKeyHandlers = getKeyHandlers;
                s_getBufferState = getBufferState;
                s_replace = replace;
                s_setKeyHandler = setKeyHandler;
                s_bindError = null;
                error = string.Empty;
                return true;
            }

            s_bindError = bindError;
            error = bindError;
            return false;
        }
    }

    /// <summary>
    /// Clears the cached MethodInfo values. This is intended for an isolated
    /// A/B probe if a test unloads and reloads a PSReadLine assembly.
    /// </summary>
    public static void ResetBinding()
    {
        lock (Gate)
        {
            s_consoleReadLineType = null;
            s_getKeyHandlers = null;
            s_getBufferState = null;
            s_replace = null;
            s_setKeyHandler = null;
            s_bindError = null;
        }
    }

    /// <summary>
    /// Gets a materialized, compact view of all PSReadLine key handlers. The
    /// enumeration itself is performed once by PSReadLine; PowerShell callers
    /// can reuse this immutable result for collision and lifecycle checks.
    /// </summary>
    public static KeyHandlerSnapshot GetKeyHandlerSnapshot()
    {
        if (!TryBind(out var bindError))
        {
            return KeyHandlerSnapshot.Unavailable(bindError);
        }

        try
        {
            var result = s_getKeyHandlers!.Invoke(null, new object?[] { true, false });
            var handlers = new List<KeyHandlerInfo>();
            var byChord = new Dictionary<string, List<KeyHandlerInfo>>(StringComparer.OrdinalIgnoreCase);

            if (result is IEnumerable enumerable)
            {
                foreach (var rawHandler in enumerable)
                {
                    if (rawHandler is null)
                    {
                        continue;
                    }

                    var handlerType = rawHandler.GetType();
                    var key = ReadStringProperty(handlerType, rawHandler, "Key");
                    if (key.Length == 0)
                    {
                        continue;
                    }

                    var handler = new KeyHandlerInfo(
                        key,
                        ReadStringProperty(handlerType, rawHandler, "Function"),
                        ReadStringProperty(handlerType, rawHandler, "Description"),
                        ReadStringProperty(handlerType, rawHandler, "Group"));
                    handlers.Add(handler);

                    if (!byChord.TryGetValue(key, out var chordHandlers))
                    {
                        chordHandlers = new List<KeyHandlerInfo>();
                        byChord.Add(key, chordHandlers);
                    }
                    chordHandlers.Add(handler);
                }
            }

            var readonlyByChord = new Dictionary<string, IReadOnlyList<KeyHandlerInfo>>(
                byChord.Count,
                StringComparer.OrdinalIgnoreCase);
            foreach (var pair in byChord)
            {
                readonlyByChord.Add(pair.Key, pair.Value.AsReadOnly());
            }

            return new KeyHandlerSnapshot(
                true,
                string.Empty,
                handlers.AsReadOnly(),
                readonlyByChord);
        }
        catch (Exception exception)
        {
            return KeyHandlerSnapshot.Unavailable(Unwrap(exception).Message);
        }
    }

    /// <summary>
    /// Reads the current PSReadLine line and UTF-16 cursor position.
    /// </summary>
    public static bool TryGetBufferState(out BufferState state, out string error)
    {
        state = new BufferState(string.Empty, 0);
        if (!TryBind(out error))
        {
            return false;
        }

        try
        {
            var arguments = new object?[] { null, 0 };
            s_getBufferState!.Invoke(null, arguments);
            var line = arguments[0] as string ?? string.Empty;
            var cursor = arguments[1] is int value ? value : Convert.ToInt32(arguments[1]);
            state = new BufferState(line, cursor);
            error = string.Empty;
            return true;
        }
        catch (Exception exception)
        {
            error = Unwrap(exception).Message;
            return false;
        }
    }

    /// <summary>
    /// Calls the public Replace method and reports only whether it returned.
    /// PSReadLine remains responsible for validating the live range.
    /// </summary>
    public static bool TryReplace(int start, int length, string replacement, out string error)
    {
        if (start < 0 || length < 0 || replacement is null)
        {
            error = "The replacement range or text is invalid.";
            return false;
        }
        if (!TryBind(out error))
        {
            return false;
        }

        try
        {
            // The public API has optional instigator and instigatorArg
            // parameters. Reflection requires both slots to be supplied.
            s_replace!.Invoke(null, new object?[] { start, length, replacement, null, null });
            error = string.Empty;
            return true;
        }
        catch (Exception exception)
        {
            error = Unwrap(exception).Message;
            return false;
        }
    }

    /// <summary>
    /// Calls the public ScriptBlock overload of SetKeyHandler. The scriptBlock
    /// is intentionally an object so the bridge has no compile-time dependency
    /// on System.Management.Automation; callers must pass a real ScriptBlock.
    /// </summary>
    public static bool TrySetKeyHandler(
        string chord,
        object scriptBlock,
        string briefDescription,
        string longDescription,
        out string error)
    {
        if (string.IsNullOrWhiteSpace(chord) || scriptBlock is null)
        {
            error = "A chord and PSReadLine ScriptBlock are required.";
            return false;
        }
        if (!TryBind(out error))
        {
            return false;
        }

        try
        {
            s_setKeyHandler!.Invoke(
                null,
                new object?[]
                {
                    new[] { chord },
                    scriptBlock,
                    briefDescription ?? string.Empty,
                    longDescription ?? string.Empty,
                });
            error = string.Empty;
            return true;
        }
        catch (Exception exception)
        {
            error = Unwrap(exception).Message;
            return false;
        }
    }

    /// <summary>
    /// Serializes a plain DTO or dictionary with the same cached ASCII-safe
    /// encoder used by the OSC adapter. This is a secondary optimization; it
    /// does not own protocol shape or event construction.
    /// </summary>
    public static string Serialize(object payload)
    {
        return JsonSerializer.Serialize(payload, JsonOptions);
    }

    private static JsonSerializerOptions CreateJsonOptions()
    {
        return new JsonSerializerOptions
        {
            Encoder = JavaScriptEncoder.Default,
            WriteIndented = false,
        };
    }

    private static bool TryFindPublicSurface(
        out Type? consoleReadLineType,
        out MethodInfo? getKeyHandlers,
        out MethodInfo? getBufferState,
        out MethodInfo? replace,
        out MethodInfo? setKeyHandler,
        out string error)
    {
        consoleReadLineType = FindLoadedType(ConsoleReadLineTypeName);
        getKeyHandlers = null;
        getBufferState = null;
        replace = null;
        setKeyHandler = null;

        if (consoleReadLineType is null)
        {
            error = "PSReadLine is not loaded in the current runspace.";
            return false;
        }

        foreach (var method in consoleReadLineType.GetMethods(BindingFlags.Public | BindingFlags.Static))
        {
            var parameters = method.GetParameters();
            if (method.Name == "GetKeyHandlers" &&
                parameters.Length == 2 &&
                parameters[0].ParameterType == typeof(bool) &&
                parameters[1].ParameterType == typeof(bool))
            {
                getKeyHandlers = method;
            }
            else if (method.Name == "GetBufferState" &&
                     parameters.Length == 2 &&
                     parameters[0].ParameterType == typeof(string).MakeByRefType() &&
                     parameters[1].ParameterType == typeof(int).MakeByRefType())
            {
                getBufferState = method;
            }
            else if (method.Name == "Replace" &&
                     parameters.Length == 5 &&
                     parameters[0].ParameterType == typeof(int) &&
                     parameters[1].ParameterType == typeof(int) &&
                     parameters[2].ParameterType == typeof(string))
            {
                replace = method;
            }
            else if (method.Name == "SetKeyHandler" &&
                     parameters.Length == 4 &&
                     parameters[0].ParameterType == typeof(string[]) &&
                     parameters[1].ParameterType.FullName == ScriptBlockTypeName &&
                     parameters[2].ParameterType == typeof(string) &&
                     parameters[3].ParameterType == typeof(string))
            {
                setKeyHandler = method;
            }
        }

        if (getKeyHandlers is null ||
            getBufferState is null ||
            replace is null ||
            setKeyHandler is null)
        {
            error = "The loaded PSReadLine assembly does not expose the required public API.";
            consoleReadLineType = null;
            return false;
        }

        error = string.Empty;
        return true;
    }

    private static Type? FindLoadedType(string fullName)
    {
        foreach (var assembly in AppDomain.CurrentDomain.GetAssemblies())
        {
            try
            {
                var type = assembly.GetType(fullName, throwOnError: false, ignoreCase: false);
                if (type is not null)
                {
                    return type;
                }
            }
            catch (FileLoadException)
            {
                // A partially unloaded optional assembly is ignored.
            }
            catch (ReflectionTypeLoadException)
            {
                // The assembly may contain other unloadable types; keep the
                // search limited to the known public type name.
            }
        }
        return null;
    }

    private static string ReadStringProperty(Type handlerType, object handler, string name)
    {
        try
        {
            return handlerType.GetProperty(name, BindingFlags.Public | BindingFlags.Instance)?.GetValue(handler) as string
                ?? string.Empty;
        }
        catch
        {
            return string.Empty;
        }
    }

    private static Exception Unwrap(Exception exception)
    {
        return exception is TargetInvocationException { InnerException: not null } invocation
            ? Unwrap(invocation.InnerException!)
            : exception;
    }
}

public sealed class KeyHandlerInfo
{
    public KeyHandlerInfo(string key, string function, string description, string group)
    {
        Key = key;
        Function = function;
        Description = description;
        Group = group;
    }

    public string Key { get; }
    public string Function { get; }
    public string Description { get; }
    public string Group { get; }
}

public sealed class KeyHandlerSnapshot
{
    public KeyHandlerSnapshot(
        bool available,
        string error,
        IReadOnlyList<KeyHandlerInfo> handlers,
        IReadOnlyDictionary<string, IReadOnlyList<KeyHandlerInfo>> byChord)
    {
        Available = available;
        Error = error;
        Handlers = handlers;
        ByChord = byChord;
    }

    public bool Available { get; }
    public string Error { get; }
    public IReadOnlyList<KeyHandlerInfo> Handlers { get; }
    public IReadOnlyDictionary<string, IReadOnlyList<KeyHandlerInfo>> ByChord { get; }

    internal static KeyHandlerSnapshot Unavailable(string error)
    {
        return new KeyHandlerSnapshot(
            false,
            error,
            Array.Empty<KeyHandlerInfo>(),
            new Dictionary<string, IReadOnlyList<KeyHandlerInfo>>(StringComparer.OrdinalIgnoreCase));
    }
}

public sealed class BufferState
{
    public BufferState(string line, int cursor)
    {
        Line = line;
        Cursor = cursor;
    }

    public string Line { get; }
    public int Cursor { get; }
}
