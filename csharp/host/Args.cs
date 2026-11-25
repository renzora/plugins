// The small types the API hands scripts: an event's arguments, a save's
// outcome, the [Prop] attribute, the exception an engine call throws, and the
// OnDraw canvas.

using System;
using System.Collections;
using System.Collections.Generic;
using System.Linq;

namespace Renzora;

/// <summary>
/// The arguments of an event, RPC or UI callback: names to values, which are
/// <c>long</c>, <c>double</c>, <c>bool</c>, <c>string</c> or <see cref="Renzora.Vector3"/>.
/// The typed getters convert between the number types, so <c>args.Float("hp")</c>
/// works whether the sender wrote <c>10</c> or <c>10.0</c>.
/// </summary>
public sealed class Args : IEnumerable<KeyValuePair<string, object?>>
{
    internal readonly Dictionary<string, object?> Values;

    internal Args(Dictionary<string, object?> values) => Values = values;

    public static Args Empty => new(new Dictionary<string, object?>());

    public object? this[string key] => Values.TryGetValue(key, out var v) ? v : null;
    public bool Has(string key) => Values.ContainsKey(key);
    public int Count => Values.Count;
    public IEnumerable<string> Keys => Values.Keys;

    public bool TryGet<T>(string key, out T value)
    {
        if (Values.TryGetValue(key, out var v) && v is not null)
        {
            if (v is T t)
            {
                value = t;
                return true;
            }
            var converted = Json.As<T>(v);
            if (converted is not null)
            {
                value = converted;
                return true;
            }
        }
        value = default!;
        return false;
    }

    public T Get<T>(string key, T fallback) => TryGet<T>(key, out var v) ? v : fallback;
    public string String(string key, string fallback = "") => Get(key, fallback);
    public float Float(string key, float fallback = 0) => Get(key, fallback);
    public int Int(string key, int fallback = 0) => Get(key, fallback);
    public bool Bool(string key, bool fallback = false) => Get(key, fallback);
    public Vector3 Vector3(string key) => Get(key, Renzora.Vector3.Zero);

    public IEnumerator<KeyValuePair<string, object?>> GetEnumerator() => Values.GetEnumerator();
    IEnumerator IEnumerable.GetEnumerator() => GetEnumerator();

    public override string ToString() =>
        "{" + string.Join(", ", Values.Select(kv => $"{kv.Key}: {Script.Show(kv.Value)}")) + "}";
}

/// <summary>What a save slot operation did: <c>Ok</c>, or why not.</summary>
public readonly record struct SaveResult(bool Ok, string? Error);

/// <summary>
/// Marks a field (or a property with a getter and setter) as a prop: a value
/// the inspector edits per entity. Its initial value is the default. Supported
/// types: float, double, int, long, bool, string, Vector2, Vector3, Color.
/// </summary>
[AttributeUsage(AttributeTargets.Field | AttributeTargets.Property)]
public sealed class PropAttribute : Attribute
{
    /// <summary>A tooltip for the inspector.</summary>
    public string? Hint { get; set; }

    /// <summary>The inspector group the prop shows under.</summary>
    public string? Tab { get; set; }

    /// <summary>The label, when the one made from the member name will not do.</summary>
    public string? Label { get; set; }
}

/// <summary>An engine function refused its arguments, or an engine read failed.</summary>
public sealed class ScriptException : Exception
{
    public ScriptException(string message) : base(message) { }
}

/// <summary>
/// The surface <see cref="Script.OnDraw"/> draws on: pixels from the top-left.
/// A colour is a <see cref="Color"/> or <c>"#rrggbb"</c> / <c>"#rrggbbaa"</c> text.
/// </summary>
public sealed class Canvas
{
    public float Width { get; }
    public float Height { get; }

    internal Canvas(float width, float height)
    {
        Width = width;
        Height = height;
    }

    public void Line(float x1, float y1, float x2, float y2, object color, float thickness = 2) =>
        Native.Call("_draw_line", new object?[] { x1, y1, x2, y2, color, thickness });

    public void Arc(float cx, float cy, float r, float start, float end, object color, float thickness = 2) =>
        Native.Call("_draw_arc", new object?[] { cx, cy, r, start, end, color, thickness });

    public void Circle(float cx, float cy, float r, object color) =>
        Native.Call("_draw_circle", new object?[] { cx, cy, r, color });

    public void Rect(float x, float y, float w, float h, object color) =>
        Native.Call("_draw_rect", new object?[] { x, y, w, h, color });

    public void Text(float x, float y, string text, float size, object color) =>
        Native.Call("_draw_text", new object?[] { x, y, text, size, color });

    public void Triangle(float x1, float y1, float x2, float y2, float x3, float y3, object color) =>
        Native.Call("_draw_triangle", new object?[] { x1, y1, x2, y2, x3, y3, color });

    /// <summary>A convex polygon, fanned into triangles from its first point.</summary>
    public void Poly(IEnumerable<Vector2> points, object color) =>
        Native.Call("_draw_poly", new object?[] { points.ToList(), color });
}
