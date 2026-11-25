// The engine functions that read and talk: reflection, events and RPCs, HTTP,
// saved games, the load trackers, and Call for everything by name.

using System;
using System.Collections.Generic;
using System.Linq;
using System.Text;
using System.Text.Json;

namespace Renzora;

public abstract unsafe partial class Script
{
    // =========================================================================
    // Calling by name
    // =========================================================================

    /// <summary>
    /// Any engine function or plugin-declared function by its engine name:
    /// <c>Call("mysql_query", "SELECT 1", "answer")</c>. This is how a script
    /// reaches what other plugins add (databases, navigation, translations):
    /// they declare functions by name, and every language calls them by name.
    /// A plugin that answers later does so through <see cref="OnEvent"/>.
    /// Returns what the function returns: null, bool, long, double, string,
    /// Vector2/Vector3/Color, a List&lt;object?&gt; or a Dictionary&lt;string, object?&gt;.
    /// </summary>
    public object? Call(string name, params object?[] args) => Native.Call(name, args ?? new object?[] { null });

    /// <summary><see cref="Call(string, object?[])"/>, with the result converted to <typeparamref name="T"/>.</summary>
    public T? Call<T>(string name, params object?[] args) => Json.As<T>(Call(name, args));

    /// <summary>The localized text for <paramref name="key"/> (the <c>tr</c> function).</summary>
    public string Tr(string key) => Call("tr", key) as string ?? key;

    // =========================================================================
    // Reflection: any component field by path
    // =========================================================================

    /// <summary>
    /// A reflected field of this entity, <c>"Component.field.sub"</c>:
    /// <c>Get("Transform.translation")</c> is a Vector3. Null when absent.
    /// </summary>
    public object? Get(string path) => Native.Get(null, path);
    public T? Get<T>(string path) => Json.As<T>(Native.Get(null, path));

    /// <summary>A reflected field of the entity named <paramref name="entity"/>.</summary>
    public object? GetOn(string entity, string path) => Native.Get(entity, path);
    public T? GetOn<T>(string entity, string path) => Json.As<T>(Native.Get(entity, path));

    public float GetFloat(string path, float fallback = 0) => Get(path) is { } v ? Json.As<float>(v) : fallback;
    public bool GetBool(string path) => Json.As<bool>(Get(path));
    public string? GetString(string path) => Get(path) as string;
    public Vector3 GetVector3(string path) => Json.As<Vector3>(Get(path));

    /// <summary>Write a reflected field of this entity. Takes a number, bool, string, Vector3 or Color.</summary>
    public void Set(string path, object value) => Run("set", path, value);
    public void SetOn(string entity, string path, object value) => Run("set_on", entity, path, value);

    /// <summary>A whole component as field names to values, or null when the entity has none.</summary>
    public Dictionary<string, object?>? GetComponent(string component) =>
        Run("get_component", component) as Dictionary<string, object?>;
    public Dictionary<string, object?>? GetComponentOn(string entity, string component) =>
        Run("get_component_on", entity, component) as Dictionary<string, object?>;

    /// <summary>Every reflected component this entity has.</summary>
    public List<string> GetComponents() => Strings(Run("get_components"));
    public List<string> GetComponentsOn(string entity) => Strings(Run("get_components_on", entity));
    public bool HasComponent(string component) => Json.As<bool>(Run("has_component", component));
    public bool HasComponentOn(string entity, string component) => Json.As<bool>(Run("has_component_on", entity, component));

    static List<string> Strings(object? v) =>
        v is List<object?> list ? list.OfType<string>().ToList() : new List<string>();

    // =========================================================================
    // Events, actions, RPCs
    // =========================================================================

    /// <summary>
    /// A ScriptAction a domain crate or plugin handles, with named arguments:
    /// a dictionary or an anonymous object, <c>new { amount = 10 }</c>.
    /// </summary>
    public void Action(string name, object? args = null) => Run("action", name, args);

    /// <summary><see cref="Action(string, object?)"/>, aimed at the entity named <paramref name="entity"/>.</summary>
    public void ActionOn(string entity, string name, object? args = null) => Run("action_on", entity, name, args);

    /// <summary>
    /// Broadcast: every script's <see cref="OnEvent"/> fires next frame, in
    /// every language, as do Rust observers of the engine's GameEvent.
    /// </summary>
    public void Emit(string name, object? args = null) => Run("emit", name, args);

    /// <summary>A networked RPC; peers receive it in <see cref="OnRpc"/>.</summary>
    public void Rpc(string name, object? args = null) => Run("rpc", name, args);

    /// <summary>The response arrives as <see cref="OnHttp"/>(callback, status, body).</summary>
    public void HttpGet(string url, string callback = "get") => Run("http_get", url, callback);

    /// <summary>A body that is not a string is sent as its JSON.</summary>
    public void HttpPost(string url, object body, string callback = "post") =>
        Run("http_post", url, body is string ? body : JsonEncode(body), callback);

    // =========================================================================
    // Network status
    // =========================================================================

    public bool NetIsServer() => C != null && (C->Flags & Native.FlagNetServer) != 0;
    public bool NetIsConnected() => C != null && (C->Flags & Native.FlagNetConnected) != 0;
    public bool NetIsClient() => NetIsConnected() && !NetIsServer();
    public int NetPlayerCount() => C == null ? 0 : C->NetPlayerCount;

    // =========================================================================
    // Saved games
    // =========================================================================

    /// <summary>Set a value in the save data (a number, bool, string, Vector2, Vector3 or Color).</summary>
    public void SaveSet(string key, object value) => Run("save_set", key, value);
    public object? SaveGet(string key, object? fallback = null) => Run("save_get", key, fallback);
    public T? SaveGet<T>(string key, T? fallback = default) => Json.As<T>(Run("save_get", key, fallback));
    public bool SaveHas(string key) => Json.As<bool>(Run("save_has", key));
    public bool SaveRemove(string key) => Json.As<bool>(Run("save_remove", key));
    public void SaveClear() => Run("save_clear");
    public List<string> SaveKeys() => Strings(Run("save_keys"));
    public SaveResult SaveWrite(string slot) => Outcome(Run("save_write", slot));
    public SaveResult SaveLoad(string slot) => Outcome(Run("save_load", slot));
    public SaveResult SaveDelete(string slot) => Outcome(Run("save_delete", slot));
    public bool SaveExists(string slot) => Json.As<bool>(Run("save_exists", slot));

    /// <summary>Every save slot, newest first: (name, Unix time saved).</summary>
    public List<(string Name, long SavedAt)> SaveSlots() =>
        Run("save_slots") is List<object?> list
            ? list.OfType<Dictionary<string, object?>>()
                .Select(d => (d.GetValueOrDefault("name") as string ?? "", Json.As<long>(d.GetValueOrDefault("saved_at"))))
                .ToList()
            : new();

    static SaveResult Outcome(object? v) =>
        v is Dictionary<string, object?> d
            ? new SaveResult(Json.As<bool>(d.GetValueOrDefault("ok")), d.GetValueOrDefault("error") as string)
            : new SaveResult(false, "no answer");

    // =========================================================================
    // Loading screens
    // =========================================================================

    /// <summary>
    /// The asset-load tracker, or null when nothing is loading: state, total_files,
    /// loaded_files, total_bytes, loaded_bytes, fraction, current_path, elapsed_secs.
    /// </summary>
    public Dictionary<string, object?>? AssetProgress() => Run("asset_progress") as Dictionary<string, object?>;

    /// <summary>The scene loader: phase, current_path, progress; null before any load.</summary>
    public Dictionary<string, object?>? SceneLoadState() => Run("scene_load_state") as Dictionary<string, object?>;
    public bool IsLoading() => Json.As<bool>(Run("is_loading"));
    public bool IsLoaded() => Json.As<bool>(Run("is_loaded"));

    // =========================================================================
    // JSON
    // =========================================================================

    /// <summary>
    /// Parse JSON into the same shapes <see cref="Call(string, object?[])"/> returns, or null when
    /// the text is not JSON (rather than throwing, as the other languages'
    /// <c>json_parse</c> do). System.Text.Json is there too.
    /// </summary>
    public static object? JsonParse(string text)
    {
        try
        {
            return Json.Parse(text);
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary>Encode a value as JSON, vectors as <c>{"x":..}</c> objects.</summary>
    public static string JsonEncode(object? value, bool pretty = false)
    {
        var buf = new System.Buffers.ArrayBufferWriter<byte>();
        using (var w = new Utf8JsonWriter(buf, new JsonWriterOptions { Indented = pretty }))
            Json.Write(w, value, 0);
        return Encoding.UTF8.GetString(buf.WrittenSpan);
    }

    // =========================================================================
    // Math, as the other languages have it
    // =========================================================================

    public static float Lerp(float a, float b, float t) => Mathf.Lerp(a, b, t);
    public static float Clamp(float v, float min, float max) => Mathf.Clamp(v, min, max);
}
