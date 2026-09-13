// JSON compatibility for the inbox .NET Framework runtime. PowerShell 7 uses
// System.Text.Json instead. No assembly is downloaded or installed globally.
using System;
using System.Collections;
using System.Collections.Generic;
using System.Globalization;
using System.Management.Automation;
using System.Text.RegularExpressions;
using System.Web.Script.Serialization;

namespace Blueberry.LegacyJson {
    public enum JsonValueKind { Undefined, Object, Array, String, Number, True, False, Null }
    public class JsonSerializerOptions { public object Encoder { get; set; } public bool WriteIndented { get; set; } }
    public struct JsonProperty {
        public string Name { get; set; }
        public JsonElement Value { get; set; }
    }
    public struct JsonElement {
        internal object Data;
        internal string Raw;
        public JsonValueKind ValueKind { get; internal set; }
        public string GetRawText() { return Raw; }
        public string GetString() { return (string)Data; }
        public bool TryGetProperty(string name, out JsonElement value) {
            value = new JsonElement();
            return ValueKind == JsonValueKind.Object && ((Dictionary<string, JsonElement>)Data).TryGetValue(name, out value);
        }
        public IEnumerable<JsonProperty> EnumerateObject() {
            foreach (var pair in (Dictionary<string, JsonElement>)Data)
                yield return new JsonProperty { Name = pair.Key, Value = pair.Value };
        }
        public bool TryGetInt64(out long value) {
            return long.TryParse(Raw, NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture, out value);
        }
        public bool TryGetDecimal(out decimal value) {
            return decimal.TryParse(Raw, NumberStyles.Float, CultureInfo.InvariantCulture, out value);
        }
        public bool TryGetDouble(out double value) {
            return double.TryParse(Raw, NumberStyles.Float, CultureInfo.InvariantCulture, out value);
        }
        public override string ToString() { return Raw; }
    }
    public sealed class JsonDocument : IDisposable {
        public JsonElement RootElement { get; private set; }
        public static JsonDocument Parse(string text) { return new JsonDocument { RootElement = new Reader(text).Read() }; }
        public void Dispose() { }
    }
    public static class JsonSerializer {
        public static object Deserialize(string text, Type type, object options) { return new Reader(text).Read(); }
        public static string Serialize(object value, Type type, object options) {
            var serializer = new JavaScriptSerializer { MaxJsonLength = 4194304, RecursionLimit = 64 };
            var json = serializer.Serialize(Normalize(value, 0));
            // OSC is ASCII on both runtimes, including surrogate pairs.
            return Regex.Replace(json, @"[^\x00-\x7f]", m => "\\u" + ((int)m.Value[0]).ToString("x4"));
        }
        static object Normalize(object value, int depth) {
            if (depth > 64) throw new FormatException("JSON nesting limit exceeded");
            var wrapper = value as PSObject;
            if (wrapper != null) {
                if (!(wrapper.BaseObject is PSCustomObject)) return Normalize(wrapper.BaseObject, depth + 1);
                var properties = new Dictionary<string, object>();
                foreach (var p in wrapper.Properties) properties.Add(p.Name, Normalize(p.Value, depth + 1));
                return properties;
            }
            var dictionary = value as IDictionary;
            if (dictionary != null) {
                var result = new Dictionary<string, object>();
                foreach (DictionaryEntry pair in dictionary) result.Add((string)pair.Key, Normalize(pair.Value, depth + 1));
                return result;
            }
            if (value != null && !(value is string) && value is IEnumerable) {
                var list = new List<object>();
                foreach (var item in (IEnumerable)value) list.Add(Normalize(item, depth + 1));
                return list;
            }
            return value;
        }
    }
    // A bounded lexical reader preserves scalar types and rejects duplicate
    // properties before the framework string decoder can discard information.
    internal sealed class Reader {
        readonly string text;
        int at;
        readonly JavaScriptSerializer strings = new JavaScriptSerializer { MaxJsonLength = 4194304 };
        public Reader(string value) {
            if (value == null || value.Length > 4194304) throw new FormatException("JSON size limit exceeded");
            text = value;
        }
        void Space() { while (at < text.Length && " \r\n\t".IndexOf(text[at]) >= 0) at++; }
        void Expect(char c) { Space(); if (at >= text.Length || text[at++] != c) throw new FormatException("Invalid JSON"); }
        public JsonElement Read() { var result = Value(0); Space(); if (at != text.Length) throw new FormatException("Trailing JSON"); return result; }
        string String() {
            int start = at; Expect('"');
            while (at < text.Length) {
                char c = text[at++];
                if (c < 32) throw new FormatException("Control character in JSON string");
                if (c == '"') return strings.Deserialize<string>(text.Substring(start, at - start));
                if (c == '\\') {
                    if (at >= text.Length) break;
                    char escape = text[at++];
                    if (escape == 'u') {
                        for (int i = 0; i < 4; i++)
                            if (at >= text.Length || !Uri.IsHexDigit(text[at++])) throw new FormatException("Invalid Unicode escape");
                    } else if ("\"\\/bfnrt".IndexOf(escape) < 0) throw new FormatException("Invalid escape");
                }
            }
            throw new FormatException("Unterminated string");
        }
        JsonElement Value(int depth) {
            if (depth > 64) throw new FormatException("JSON nesting limit exceeded");
            Space(); int start = at;
            if (at >= text.Length) throw new FormatException("Missing JSON value");
            var node = new JsonElement(); char c = text[at];
            if (c == '{') {
                at++; var obj = new Dictionary<string, JsonElement>(StringComparer.Ordinal); Space();
                if (at < text.Length && text[at] != '}') {
                    while (true) {
                        Space(); string name = String(); Expect(':');
                        if (obj.ContainsKey(name)) throw new FormatException("Duplicate JSON property");
                        obj.Add(name, Value(depth + 1)); Space();
                        if (at >= text.Length || text[at] != ',') break;
                        at++;
                    }
                }
                Expect('}'); node.ValueKind = JsonValueKind.Object; node.Data = obj;
            } else if (c == '[') {
                at++; Space(); var list = new List<JsonElement>();
                if (at < text.Length && text[at] != ']') {
                    while (true) {
                        list.Add(Value(depth + 1)); Space();
                        if (at >= text.Length || text[at] != ',') break;
                        at++;
                    }
                }
                Expect(']'); node.ValueKind = JsonValueKind.Array; node.Data = list;
            } else if (c == '"') { node.Data = String(); node.ValueKind = JsonValueKind.String; }
            else {
                var match = Regex.Match(text.Substring(at), @"\A(?:true|false|null|-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?)");
                if (!match.Success) throw new FormatException("Invalid JSON scalar");
                at += match.Length;
                node.ValueKind = match.Value == "null" ? JsonValueKind.Null : match.Value == "true" ? JsonValueKind.True : match.Value == "false" ? JsonValueKind.False : JsonValueKind.Number;
            }
            node.Raw = text.Substring(start, at - start); return node;
        }
    }
}
