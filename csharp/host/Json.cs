// Values to and from the JSON every non-hot call crosses the boundary in.
//
// Written by hand over Utf8JsonWriter and JsonDocument rather than with
// JsonSerializer: the shapes are few and fixed, the encoding has two rules the
// serializer would not follow (below), and nothing here should depend on
// reflecting over a script's types.
//
// The two rules, both shared with the Rust side's `convert.rs`:
// * A float is always written with a fraction (5.0) and an integer never, so
//   the engine sees the C# type: an int becomes an Int, a float a Float.
// * Vector2, Vector3 and Color are objects with x/y(/z) or r/g/b/a fields, and
//   any object of that exact shape reads back as one of them.

using System;
using System.Collections;
using System.Collections.Generic;
using System.Globalization;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Text.Json;

namespace Renzora;

internal static class Json
{
    const int MaxDepth = 64;

    internal static void WriteFloat(Utf8JsonWriter w, double d, bool single)
    {
        if (double.IsNaN(d) || double.IsInfinity(d))
        {
            w.WriteNullValue();
            return;
        }
        string s = single
            ? ((float)d).ToString("R", CultureInfo.InvariantCulture)
            : d.ToString("R", CultureInfo.InvariantCulture);
        if (s.IndexOfAny(new[] { '.', 'E', 'e' }) < 0)
            s += ".0";
        w.WriteRawValue(s, skipInputValidation: true);
    }

    /// <summary>Write any value a script hands the engine.</summary>
    internal static void Write(Utf8JsonWriter w, object? v, int depth)
    {
        if (depth > MaxDepth)
            throw new ScriptException("a value nested too deeply to send (a cycle?)");
        switch (v)
        {
            case null:
                w.WriteNullValue();
                return;
            case bool b:
                w.WriteBooleanValue(b);
                return;
            case string s:
                w.WriteStringValue(s);
                return;
            case char c:
                w.WriteStringValue(c.ToString());
                return;
            case float f:
                WriteFloat(w, f, true);
                return;
            case double d:
                WriteFloat(w, d, false);
                return;
            case decimal m:
                WriteFloat(w, (double)m, false);
                return;
            case int i:
                w.WriteNumberValue(i);
                return;
            case long l:
                w.WriteNumberValue(l);
                return;
            case uint ui:
                w.WriteNumberValue(ui);
                return;
            case ulong ul:
                w.WriteNumberValue(ul);
                return;
            case short sh:
                w.WriteNumberValue((int)sh);
                return;
            case ushort us:
                w.WriteNumberValue((int)us);
                return;
            case byte by:
                w.WriteNumberValue((int)by);
                return;
            case sbyte sb:
                w.WriteNumberValue((int)sb);
                return;
            case Vector2 v2:
                w.WriteStartObject();
                w.WritePropertyName("x"); WriteFloat(w, v2.X, true);
                w.WritePropertyName("y"); WriteFloat(w, v2.Y, true);
                w.WriteEndObject();
                return;
            case Vector3 v3:
                w.WriteStartObject();
                w.WritePropertyName("x"); WriteFloat(w, v3.X, true);
                w.WritePropertyName("y"); WriteFloat(w, v3.Y, true);
                w.WritePropertyName("z"); WriteFloat(w, v3.Z, true);
                w.WriteEndObject();
                return;
            case Color col:
                w.WriteStartObject();
                w.WritePropertyName("r"); WriteFloat(w, col.R, true);
                w.WritePropertyName("g"); WriteFloat(w, col.G, true);
                w.WritePropertyName("b"); WriteFloat(w, col.B, true);
                w.WritePropertyName("a"); WriteFloat(w, col.A, true);
                w.WriteEndObject();
                return;
            case Enum e:
                w.WriteStringValue(e.ToString());
                return;
            case JsonElement je:
                je.WriteTo(w);
                return;
            case Args a:
                Write(w, a.Values, depth + 1);
                return;
            case IDictionary dict:
                w.WriteStartObject();
                foreach (DictionaryEntry kv in dict)
                {
                    w.WritePropertyName(Convert.ToString(kv.Key, CultureInfo.InvariantCulture) ?? "");
                    Write(w, kv.Value, depth + 1);
                }
                w.WriteEndObject();
                return;
            case IEnumerable list:
                w.WriteStartArray();
                foreach (var item in list)
                    Write(w, item, depth + 1);
                w.WriteEndArray();
                return;
        }
        var type = v.GetType();
        // `new { boss = "ogre" }`: the way C# writes a one-off set of named
        // values, which is what an event's arguments are.
        if (type.IsDefined(typeof(CompilerGeneratedAttribute), false) && type.Name.Contains("AnonymousType"))
        {
            w.WriteStartObject();
            foreach (var p in type.GetProperties(BindingFlags.Instance | BindingFlags.Public))
            {
                w.WritePropertyName(p.Name);
                Write(w, p.GetValue(v), depth + 1);
            }
            w.WriteEndObject();
            return;
        }
        w.WriteStringValue(Convert.ToString(v, CultureInfo.InvariantCulture));
    }

    /// <summary>Decode JSON text, or null for empty text.</summary>
    internal static object? Parse(string text)
    {
        if (string.IsNullOrEmpty(text))
            return null;
        using var doc = JsonDocument.Parse(text);
        return Read(doc.RootElement);
    }

    /// <summary>
    /// A JSON value as C#: null, bool, long (integers), double (anything with a
    /// fraction), string, Vector2/Vector3/Color (objects of exactly that
    /// shape), Dictionary&lt;string, object?&gt; and List&lt;object?&gt;.
    /// </summary>
    internal static object? Read(JsonElement e)
    {
        switch (e.ValueKind)
        {
            case JsonValueKind.Null:
            case JsonValueKind.Undefined:
                return null;
            case JsonValueKind.True:
                return true;
            case JsonValueKind.False:
                return false;
            case JsonValueKind.String:
                return e.GetString();
            case JsonValueKind.Number:
            {
                string raw = e.GetRawText();
                if (raw.IndexOfAny(new[] { '.', 'e', 'E' }) < 0 && e.TryGetInt64(out long l))
                    return l;
                return e.GetDouble();
            }
            case JsonValueKind.Array:
            {
                var list = new List<object?>(e.GetArrayLength());
                foreach (var item in e.EnumerateArray())
                    list.Add(Read(item));
                return list;
            }
            default:
            {
                var shaped = Shape(e);
                if (shaped != null)
                    return shaped;
                var map = new Dictionary<string, object?>();
                foreach (var p in e.EnumerateObject())
                    map[p.Name] = Read(p.Value);
                return map;
            }
        }
    }

    /// <summary>An object of exactly x/y, x/y/z or r/g/b(/a) numbers, as a vector or colour.</summary>
    static object? Shape(JsonElement e)
    {
        int count = 0;
        bool allNumbers = true;
        foreach (var p in e.EnumerateObject())
        {
            count++;
            if (p.Value.ValueKind != JsonValueKind.Number)
                allNumbers = false;
        }
        if (!allNumbers || count < 2 || count > 4)
            return null;
        float F(string k) => e.TryGetProperty(k, out var v) ? v.GetSingle() : float.NaN;
        bool Has(string k) => e.TryGetProperty(k, out _);
        if (count == 2 && Has("x") && Has("y"))
            return new Vector2(F("x"), F("y"));
        if (count == 3 && Has("x") && Has("y") && Has("z"))
            return new Vector3(F("x"), F("y"), F("z"));
        if (count == 3 && Has("r") && Has("g") && Has("b"))
            return new Color(F("r"), F("g"), F("b"));
        if (count == 4 && Has("r") && Has("g") && Has("b") && Has("a"))
            return new Color(F("r"), F("g"), F("b"), F("a"));
        return null;
    }

    /// <summary>
    /// <paramref name="v"/> as a <typeparamref name="T"/>, converting between
    /// the number types and widening a Vector2 to a Vector3, or the default
    /// when it cannot be one.
    /// </summary>
    internal static T? As<T>(object? v)
    {
        if (v is T t)
            return t;
        if (v == null)
            return default;
        var target = Nullable.GetUnderlyingType(typeof(T)) ?? typeof(T);
        if (target == typeof(Vector3) && v is Vector2 v2)
            return (T)(object)new Vector3(v2.X, v2.Y, 0);
        if (target == typeof(Color) && v is Vector3 v3)
            return (T)(object)new Color(v3.X, v3.Y, v3.Z);
        if (v is IConvertible && (target.IsPrimitive || target == typeof(string) || target == typeof(decimal)))
        {
            try
            {
                return (T?)System.Convert.ChangeType(v, target, CultureInfo.InvariantCulture);
            }
            catch (Exception ex) when (ex is FormatException or InvalidCastException or OverflowException)
            {
                return default;
            }
        }
        return default;
    }
}
