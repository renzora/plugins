// Loading a build of a project's scripts, finding which class each file
// declares, and reading their props.
//
// # A fresh, collectible load context per build
//
// Each build is loaded into its own collectible AssemblyLoadContext, from
// memory (so the files on disk stay free for the next build to replace), with
// its PDB (so a stack trace names the script's file and line). Loading the
// next build drops every instance and unloads the previous context; once
// nothing references its types the runtime frees it. That is what makes a hot
// reload possible at all in .NET, where an assembly can never be unloaded from
// the default context.
//
// The scripts reference Renzora.Scripting, which is already loaded (the
// runtime loaded it, in a context of its own, to give the plugin the bridge's
// function pointers). The scripts' context hands back that same assembly when
// asked for it: a second copy would have its own Script type, and no script
// class would derive from the one the bridge knows.
//
// # Which class is which file: the PDB
//
// A script file is attached to an entity by path, so the plugin has to know
// which class a given .cs file declares. The compiler already wrote that down:
// the portable PDB records, for every method, the document its code is in. So
// each Script subclass maps to the files its methods (field initialisers and
// constructors included) live in, read with System.Reflection.Metadata, which
// is part of the shared framework. Nothing is parsed from source text, so
// comments, preprocessor blocks, nested namespaces or a class whose name does
// not match its file cannot confuse it.
//
// A file then gets: the class named after the file (case-insensitive) when
// there is one, else the only Script subclass in it, else an error that says
// which. A class with no code at all has no method in the PDB; it falls back
// to the file named after it, since the PDB lists every document either way.

using System;
using System.Collections.Generic;
using System.Collections.Immutable;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Reflection.Metadata;
using System.Reflection.Metadata.Ecma335;
using System.Reflection.PortableExecutable;
using System.Runtime.Loader;
using System.Text;
using System.Text.Json;

namespace Renzora;

/// <summary>The load context one build of a project's scripts lives in.</summary>
///
/// A NuGet package's assemblies are copied into the build's folder with a
/// <c>.deps.json</c> beside them; the dependency resolver reads it, which is
/// what finds a package's native library for this platform under
/// <c>runtimes/&lt;rid&gt;/native/</c>. Package assemblies are loaded from
/// memory, like the scripts, so the folder stays free for a later build to
/// delete. A library that looks for files beside itself through
/// <c>Assembly.Location</c> sees an empty string as a result; its native
/// libraries still load, through <see cref="LoadUnmanagedDll"/>.
internal sealed class ScriptLoadContext : AssemblyLoadContext
{
    static readonly string HostName = typeof(Script).Assembly.GetName().Name!;
    readonly AssemblyDependencyResolver? resolver;
    readonly string folder;

    public ScriptLoadContext(string dll) : base("RenzoraScripts", isCollectible: true)
    {
        folder = Path.GetDirectoryName(dll) ?? ".";
        try
        {
            resolver = new AssemblyDependencyResolver(dll);
        }
        catch (Exception e) when (e is InvalidOperationException or ArgumentException)
        {
            // No resolver: assemblies are still found by name in the folder.
            resolver = null;
        }
    }

    protected override Assembly? Load(AssemblyName name)
    {
        // The one shared copy of the API: a second would have its own Script
        // type, and no script class would derive from the one the bridge knows.
        if (name.Name == HostName)
            return typeof(Script).Assembly;
        string? path = resolver?.ResolveAssemblyToPath(name);
        if (path == null && name.Name != null)
        {
            string beside = Path.Combine(folder, name.Name + ".dll");
            if (File.Exists(beside))
                path = beside;
        }
        // Null for the framework, which comes from the default context.
        if (path == null)
            return null;
        string pdbPath = Path.ChangeExtension(path, ".pdb");
        using var image = new MemoryStream(File.ReadAllBytes(path), writable: false);
        using var pdb = File.Exists(pdbPath) ? new MemoryStream(File.ReadAllBytes(pdbPath), writable: false) : null;
        return LoadFromStream(image, pdb);
    }

    protected override IntPtr LoadUnmanagedDll(string name)
    {
        string? path = resolver?.ResolveUnmanagedDllToPath(name);
        return path == null ? IntPtr.Zero : LoadUnmanagedDllFromPath(path);
    }
}

/// <summary>One prop of a script class: a [Prop] field or property.</summary>
internal sealed class PropInfo
{
    public required string Name;
    public required string Display;
    public required string Kind;
    public required Type Type;
    public required Func<object, object?> Read;
    public required Action<object, object?> Write;
    public string? Hint;
    public string? Tab;
}

/// <summary>A script file of the loaded build, at the index the report gave it.</summary>
internal sealed class ScriptEntry
{
    public required string File;
    public Type? Type;
    public string? Error;
    public PropInfo[] Props = Array.Empty<PropInfo>();
    public string[] Hooks = Array.Empty<string>();
}

internal static class Loader
{
    static ScriptLoadContext? current;
    internal static ScriptEntry[] Scripts = Array.Empty<ScriptEntry>();

    /// <summary>Load the build at <paramref name="dll"/> in place of the current one, returning the report.</summary>
    internal static string Load(string dll)
    {
        Unload();
        byte[] image = File.ReadAllBytes(dll);
        string pdbPath = Path.ChangeExtension(dll, ".pdb");
        byte[]? pdb = File.Exists(pdbPath) ? File.ReadAllBytes(pdbPath) : null;

        var ctx = new ScriptLoadContext(dll);
        Assembly asm;
        using (var imageStream = new MemoryStream(image, writable: false))
        using (var pdbStream = pdb == null ? null : new MemoryStream(pdb, writable: false))
            asm = ctx.LoadFromStream(imageStream, pdbStream);
        current = ctx;

        Type[] types;
        try
        {
            types = asm.GetTypes();
        }
        catch (ReflectionTypeLoadException e)
        {
            types = e.Types.Where(t => t != null).Select(t => t!).ToArray();
        }
        var scriptTypes = types
            .Where(t => t.IsClass && !t.IsAbstract && !t.ContainsGenericParameters && typeof(Script).IsAssignableFrom(t))
            .ToList();

        var (docsOf, allDocs) = pdb == null
            ? (new Dictionary<Type, HashSet<string>>(), new List<string>())
            : Documents(image, pdb, asm, scriptTypes);

        // Every file that holds a script class, and which ones.
        var byFile = new Dictionary<string, List<Type>>(StringComparer.OrdinalIgnoreCase);
        foreach (var t in scriptTypes)
        {
            if (docsOf.TryGetValue(t, out var docs) && docs.Count > 0)
            {
                foreach (var d in docs)
                    Add(byFile, d, t);
            }
            else
            {
                // No code of its own: the document named after it, if any.
                var named = allDocs.FirstOrDefault(d =>
                    string.Equals(Path.GetFileNameWithoutExtension(d), CleanName(t), StringComparison.OrdinalIgnoreCase));
                if (named != null)
                    Add(byFile, named, t);
            }
        }

        var entries = new List<ScriptEntry>();
        foreach (var (file, candidates) in byFile.OrderBy(kv => kv.Key, StringComparer.OrdinalIgnoreCase))
        {
            var stem = Path.GetFileNameWithoutExtension(file);
            var distinct = candidates.Distinct().ToList();
            var named = distinct.Where(t => string.Equals(CleanName(t), stem, StringComparison.OrdinalIgnoreCase)).ToList();
            var entry = new ScriptEntry { File = file };
            if (named.Count == 1)
                entry.Type = named[0];
            else if (distinct.Count == 1)
                entry.Type = distinct[0];
            else
                entry.Error = $"this file declares {distinct.Count} script classes ({string.Join(", ", distinct.Select(CleanName))}); " +
                              $"name the one to run '{stem}', or move the others to files of their own";
            if (entry.Type != null)
                Describe(entry);
            entries.Add(entry);
        }
        Scripts = entries.ToArray();
        return Report();
    }

    static void Add(Dictionary<string, List<Type>> byFile, string file, Type t)
    {
        if (!byFile.TryGetValue(file, out var list))
            byFile[file] = list = new List<Type>();
        list.Add(t);
    }

    /// <summary>Drop every instance and let the current build's context go.</summary>
    internal static void Unload()
    {
        Bridge.Instances.Clear();
        Scripts = Array.Empty<ScriptEntry>();
        var old = current;
        current = null;
        old?.Unload();
    }

    /// <summary>
    /// For each script type, the source files its methods are in, plus every
    /// document the PDB lists. Methods of compiler-generated nested types
    /// (lambdas, iterators, async state machines) count for the script type
    /// they are nested in.
    /// </summary>
    static (Dictionary<Type, HashSet<string>>, List<string>) Documents(byte[] image, byte[] pdb, Assembly asm, List<Type> scriptTypes)
    {
        var result = new Dictionary<Type, HashSet<string>>();
        var all = new List<string>();
        var wanted = new HashSet<Type>(scriptTypes);
        using var pe = new PEReader(ImmutableArray.Create(image));
        var md = pe.GetMetadataReader();
        using var provider = MetadataReaderProvider.FromPortablePdbImage(ImmutableArray.Create(pdb));
        var reader = provider.GetMetadataReader();
        foreach (var dh in reader.Documents)
            all.Add(Normalize(reader.GetString(reader.GetDocument(dh).Name)));

        foreach (var mh in reader.MethodDebugInformation)
        {
            var info = reader.GetMethodDebugInformation(mh);
            var docs = new HashSet<DocumentHandle>();
            if (!info.Document.IsNil)
                docs.Add(info.Document);
            foreach (var sp in info.GetSequencePoints())
                if (!sp.Document.IsNil)
                    docs.Add(sp.Document);
            if (docs.Count == 0)
                continue;
            var def = md.GetMethodDefinition(mh.ToDefinitionHandle());
            Type? t;
            try
            {
                t = asm.ManifestModule.ResolveType(MetadataTokens.GetToken(def.GetDeclaringType()));
            }
            catch (ArgumentException)
            {
                continue;
            }
            // Up the nesting chain to the script type, if any.
            for (var owner = t; owner != null; owner = owner.DeclaringType)
            {
                if (!wanted.Contains(owner))
                    continue;
                if (!result.TryGetValue(owner, out var set))
                    result[owner] = set = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
                foreach (var d in docs)
                    set.Add(Normalize(reader.GetString(reader.GetDocument(d).Name)));
                break;
            }
        }
        return (result, all);
    }

    static string Normalize(string path)
    {
        try
        {
            return Path.GetFullPath(path);
        }
        catch (Exception e) when (e is ArgumentException or NotSupportedException or PathTooLongException)
        {
            return path;
        }
    }

    /// <summary>
    /// A type's name as written: no generic arity, and a <c>file class</c>
    /// without the mangling the compiler gives it (<c>&lt;spinner&gt;F1A2..__Spinner</c>).
    /// </summary>
    internal static string CleanName(Type t)
    {
        string name = t.Name;
        if (name.StartsWith('<'))
        {
            int i = name.LastIndexOf("__", StringComparison.Ordinal);
            if (i >= 0)
                name = name[(i + 2)..];
        }
        int tick = name.IndexOf('`');
        return tick >= 0 ? name[..tick] : name;
    }

    // =========================================================================
    // Hooks and props
    // =========================================================================

    static readonly (string Engine, string Method, Type[] Signature)[] HookMethods =
    {
        ("on_ready", nameof(Script.OnReady), Type.EmptyTypes),
        ("on_update", nameof(Script.OnUpdate), Type.EmptyTypes),
        ("on_fixed_update", nameof(Script.OnFixedUpdate), Type.EmptyTypes),
        ("on_ui", nameof(Script.OnUi), new[] { typeof(string), typeof(Args), typeof(ulong) }),
        ("on_draw", nameof(Script.OnDraw), new[] { typeof(Canvas) }),
        ("on_rpc", nameof(Script.OnRpc), new[] { typeof(string), typeof(Args), typeof(ulong) }),
        ("on_animation_event", nameof(Script.OnAnimationEvent), new[] { typeof(string), typeof(ulong) }),
        ("on_http", nameof(Script.OnHttp), new[] { typeof(string), typeof(int), typeof(string) }),
        ("on_player_joined", nameof(Script.OnPlayerJoined), new[] { typeof(ulong) }),
        ("on_player_left", nameof(Script.OnPlayerLeft), new[] { typeof(ulong) }),
        ("on_scene_loaded", nameof(Script.OnSceneLoaded), new[] { typeof(string) }),
        ("on_scene_load_failed", nameof(Script.OnSceneLoadFailed), new[] { typeof(string), typeof(string) }),
        ("on_event", nameof(Script.OnEvent), new[] { typeof(string), typeof(Args) }),
    };

    static void Describe(ScriptEntry entry)
    {
        var t = entry.Type!;
        if (t.GetConstructor(BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic, Type.EmptyTypes) == null)
        {
            entry.Error = $"{CleanName(t)} needs a constructor that takes no arguments, since the engine creates it";
            return;
        }
        entry.Hooks = HookMethods
            .Where(h =>
            {
                var m = t.GetMethod(h.Method, BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic, null, h.Signature, null);
                return m != null && m.DeclaringType != typeof(Script);
            })
            .Select(h => h.Engine)
            .ToArray();

        var props = new List<PropInfo>();
        const BindingFlags members = BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic;
        foreach (var f in t.GetFields(members))
        {
            var attr = f.GetCustomAttribute<PropAttribute>();
            if (attr == null)
                continue;
            if (Kind(f.FieldType) is not { } kind)
            {
                Native.Log(1, $"[C#] {CleanName(t)}.{f.Name}: a prop cannot be a {f.FieldType.Name}; use float, int, bool, string, Vector2, Vector3 or Color");
                continue;
            }
            props.Add(new PropInfo
            {
                Name = f.Name,
                Display = attr.Label ?? DisplayName(f.Name),
                Kind = kind,
                Type = f.FieldType,
                Read = f.GetValue,
                Write = f.SetValue,
                Hint = attr.Hint,
                Tab = attr.Tab,
            });
        }
        foreach (var p in t.GetProperties(members))
        {
            var attr = p.GetCustomAttribute<PropAttribute>();
            if (attr == null)
                continue;
            if (!p.CanRead || !p.CanWrite || Kind(p.PropertyType) is not { } kind)
            {
                Native.Log(1, $"[C#] {CleanName(t)}.{p.Name}: a prop property needs a getter and a setter of type float, int, bool, string, Vector2, Vector3 or Color");
                continue;
            }
            props.Add(new PropInfo
            {
                Name = p.Name,
                Display = attr.Label ?? DisplayName(p.Name),
                Kind = kind,
                Type = p.PropertyType,
                Read = p.GetValue,
                Write = p.SetValue,
                Hint = attr.Hint,
                Tab = attr.Tab,
            });
        }
        entry.Props = props.ToArray();
    }

    /// <summary>The engine's name for a prop's type, or null when it cannot be one.</summary>
    static string? Kind(Type t)
    {
        if (t == typeof(float) || t == typeof(double)) return "float";
        if (t == typeof(int) || t == typeof(long) || t == typeof(short)) return "int";
        if (t == typeof(bool)) return "bool";
        if (t == typeof(string)) return "string";
        if (t == typeof(Vector2)) return "vec2";
        if (t == typeof(Vector3)) return "vec3";
        if (t == typeof(Color)) return "color";
        return null;
    }

    /// <summary>A value the engine sent, as the C# type of the prop it is for.</summary>
    internal static object? Coerce(object? v, Type t)
    {
        if (v == null)
            return t == typeof(string) ? null : Activator.CreateInstance(t);
        if (t.IsInstanceOfType(v))
            return v;
        if (t == typeof(float)) return Json.As<float>(v);
        if (t == typeof(double)) return Json.As<double>(v);
        if (t == typeof(int)) return Json.As<int>(v);
        if (t == typeof(long)) return Json.As<long>(v);
        if (t == typeof(short)) return Json.As<short>(v);
        if (t == typeof(bool)) return Json.As<bool>(v);
        if (t == typeof(string)) return Script.Show(v);
        if (t == typeof(Vector2)) return v is Vector3 v3 ? new Vector2(v3.X, v3.Y) : Json.As<Vector2>(v);
        if (t == typeof(Vector3)) return Json.As<Vector3>(v);
        if (t == typeof(Color)) return Json.As<Color>(v);
        return Activator.CreateInstance(t);
    }

    /// <summary><c>moveSpeed</c>, <c>MoveSpeed</c> and <c>move_speed</c> all to <c>Move Speed</c>.</summary>
    internal static string DisplayName(string name)
    {
        var words = new List<string>();
        var cur = new StringBuilder();
        foreach (char ch in name.TrimStart('_'))
        {
            if (ch == '_')
            {
                if (cur.Length > 0) { words.Add(cur.ToString()); cur.Clear(); }
                continue;
            }
            if (char.IsUpper(ch) && cur.Length > 0 && char.IsLower(cur[^1]))
            {
                words.Add(cur.ToString());
                cur.Clear();
            }
            cur.Append(ch);
        }
        if (cur.Length > 0)
            words.Add(cur.ToString());
        return string.Join(" ", words.Select(w => char.ToUpperInvariant(w[0]) + w[1..]));
    }

    // =========================================================================
    // The report
    // =========================================================================

    /// <summary>
    /// Every file's class, error, hooks and props, with defaults read from a
    /// throwaway instance (so a field initialiser is the default, as the
    /// starter says).
    /// </summary>
    static string Report()
    {
        var buf = new System.Buffers.ArrayBufferWriter<byte>();
        using (var w = new Utf8JsonWriter(buf))
        {
            w.WriteStartObject();
            w.WriteStartArray("scripts");
            foreach (var e in Scripts)
            {
                object? probe = null;
                if (e.Type != null && e.Error == null)
                {
                    try
                    {
                        probe = Activator.CreateInstance(e.Type, nonPublic: true);
                    }
                    catch (Exception ex)
                    {
                        e.Error = $"creating {CleanName(e.Type)} failed: {Bridge.Describe(ex)}";
                    }
                }
                w.WriteStartObject();
                w.WriteString("file", e.File);
                w.WriteString("class", e.Type == null ? null : CleanName(e.Type));
                w.WriteString("error", e.Error);
                w.WriteStartArray("hooks");
                foreach (var h in e.Hooks)
                    w.WriteStringValue(h);
                w.WriteEndArray();
                w.WriteStartArray("props");
                if (probe != null)
                {
                    foreach (var p in e.Props)
                    {
                        w.WriteStartObject();
                        w.WriteString("name", p.Name);
                        w.WriteString("display", p.Display);
                        w.WriteString("type", p.Kind);
                        w.WritePropertyName("value");
                        object? value;
                        try
                        {
                            value = p.Read(probe) ?? (p.Kind == "string" ? "" : null);
                        }
                        catch (Exception)
                        {
                            value = null;
                        }
                        Json.Write(w, value is double d ? (float)d : value is long l ? (int)l : value, 0);
                        w.WriteString("hint", p.Hint);
                        w.WriteString("tab", p.Tab);
                        w.WriteEndObject();
                    }
                }
                w.WriteEndArray();
                w.WriteEndObject();
            }
            w.WriteEndArray();
            w.WriteEndObject();
        }
        return Encoding.UTF8.GetString(buf.WrittenSpan);
    }
}
