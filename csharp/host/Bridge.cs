// The entry points the plugin's Rust half calls: Init, Load, Invoke, Evict.
//
// Each is an [UnmanagedCallersOnly] static method, fetched once as a function
// pointer through hostfxr's load_assembly_and_get_function_pointer. Nothing
// here may throw back into Rust (an exception crossing an unmanaged boundary
// ends the process, and the process is the editor), so every entry point
// catches everything and reports it as a status code plus an error text left
// with SetResult.

using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;

namespace Renzora;

/// <summary>One script on one entity.</summary>
internal sealed class Instance
{
    public required Script Script;
    /// <summary>Whether OnReady has run. A hook on an instance that has not had it runs it first.</summary>
    public bool Ready;
    /// <summary>Each prop's value as last given or reported, to tell what a hook changed.</summary>
    public required object?[] Last;
}

internal static unsafe class Bridge
{
    const uint HookReady = 0, HookUpdate = 1, HookFixedUpdate = 2, HookUi = 3, HookDraw = 4, HookRpc = 5,
        HookAnimationEvent = 6, HookHttp = 7, HookPlayerJoined = 8, HookPlayerLeft = 9,
        HookSceneLoaded = 10, HookSceneLoadFailed = 11, HookEvent = 12;

    /// <summary>A prop value no hook has reported yet, so the first read-back reports every prop.</summary>
    static readonly object Unset = new();

    internal static readonly Dictionary<(ulong Entity, int Script), Instance> Instances = new();

    // The entry points are public (on an internal class) because the runtime's
    // lookup of an [UnmanagedCallersOnly] method by name is only documented
    // for public ones.

    [UnmanagedCallersOnly(CallConvs = new[] { typeof(CallConvCdecl) })]
    public static int Init(NativeTable* table, int tableSize, int ctxSize, int reqSize, uint version)
    {
        try
        {
            if (version != Native.AbiVersion)
                return Refuse(table, $"the plugin speaks protocol {version} and this C# host {Native.AbiVersion}; rebuild the host");
            if (tableSize != sizeof(NativeTable) || ctxSize != sizeof(HookContext) || reqSize != sizeof(InvokeRequest))
                return Refuse(table, $"the plugin and its C# host disagree about the boundary (table {tableSize}/{sizeof(NativeTable)}, " +
                              $"context {ctxSize}/{sizeof(HookContext)}, request {reqSize}/{sizeof(InvokeRequest)})");
            Native.Table = *table;
            Native.Started = true;
            // A windowed game has no console: Console.WriteLine goes to the
            // engine's console instead, where a script author will look.
            Console.SetOut(new EngineWriter(0));
            Console.SetError(new EngineWriter(2));
            return 0;
        }
        catch (Exception e)
        {
            return Refuse(table, Describe(e));
        }

        // The table has not been accepted yet, so the refusal goes through the
        // one Rust passed rather than through Native.
        static int Refuse(NativeTable* t, string why)
        {
            byte[] bytes = Encoding.UTF8.GetBytes(why);
            fixed (byte* p = bytes)
                t->SetResult(Native.ResultError, p, (nuint)bytes.Length);
            return 1;
        }
    }

    [UnmanagedCallersOnly(CallConvs = new[] { typeof(CallConvCdecl) })]
    public static int Load(byte* path, int len)
    {
        try
        {
            string report = Loader.Load(Native.Utf8(path, (nuint)len));
            Native.SetResult(Native.ResultLoad, report);
            return 0;
        }
        catch (Exception e)
        {
            Native.SetResult(Native.ResultError, "loading the C# scripts failed: " + Describe(e));
            return 1;
        }
    }

    [UnmanagedCallersOnly(CallConvs = new[] { typeof(CallConvCdecl) })]
    public static void Evict(ulong entity, int script)
    {
        try
        {
            if (entity == 0 && script < 0)
            {
                Instances.Clear();
                return;
            }
            foreach (var key in Instances.Keys.Where(k => (entity == 0 || k.Entity == entity) && (script < 0 || k.Script == script)).ToList())
                Instances.Remove(key);
        }
        catch (Exception)
        {
            // Nothing to report to: an eviction has no caller waiting on it.
        }
    }

    [UnmanagedCallersOnly(CallConvs = new[] { typeof(CallConvCdecl) })]
    public static int Invoke(InvokeRequest* req)
    {
        Instance? inst = null;
        ScriptEntry? entry = null;
        var previous = Native.Current;
        try
        {
            Native.Current = req->Ctx;
            Native.FlushLogs();
            var scripts = Loader.Scripts;
            if (req->Script < 0 || req->Script >= scripts.Length)
                throw new InvalidOperationException("no such script in the loaded build");
            entry = scripts[req->Script];
            if (entry.Error != null || entry.Type == null)
                throw new InvalidOperationException(entry.Error ?? "no script class");

            var key = (req->Entity, req->Script);
            if (!Instances.TryGetValue(key, out inst))
            {
                var obj = (Script)Activator.CreateInstance(entry.Type, nonPublic: true)!;
                inst = new Instance { Script = obj, Last = Enumerable.Repeat(Unset, entry.Props.Length).ToArray() };
                Instances[key] = inst;
            }
            if (req->PropsLen > 0)
                ApplyProps(entry, inst, Native.Utf8(req->PropsPtr, req->PropsLen));

            // OnReady first, whatever the hook: see Instance.Ready.
            if (!inst.Ready)
            {
                inst.Ready = true;
                inst.Script.OnReady();
            }
            if (req->Hook != HookReady)
                Dispatch(inst.Script, req);
            return 0;
        }
        catch (Exception e)
        {
            Native.SetResult(Native.ResultError, Describe(e));
            return 1;
        }
        finally
        {
            // Read back whether or not the hook threw, so a prop set just
            // before an exception is kept, as in the other languages.
            try
            {
                if (inst != null && entry != null)
                    ReportProps(entry, inst);
            }
            catch (Exception)
            {
                // A prop that cannot be encoded is dropped, not fatal.
            }
            Native.Current = previous;
        }
    }

    static void Dispatch(Script s, InvokeRequest* req)
    {
        string Name() => Native.Utf8(req->NamePtr, req->NameLen);
        string Text() => Native.Utf8(req->TextPtr, req->TextLen);
        Args ArgsOf() => req->ArgsLen == 0
            ? Args.Empty
            : new Args(Json.Parse(Native.Utf8(req->ArgsPtr, req->ArgsLen)) as Dictionary<string, object?> ?? new());
        switch (req->Hook)
        {
            case HookUpdate: s.OnUpdate(); break;
            case HookFixedUpdate: s.OnFixedUpdate(); break;
            case HookUi: s.OnUi(Name(), ArgsOf(), req->Number); break;
            case HookDraw: s.OnDraw(new Canvas(req->Width, req->Height)); break;
            case HookRpc: s.OnRpc(Name(), ArgsOf(), req->Number); break;
            case HookAnimationEvent: s.OnAnimationEvent(Name(), req->Number); break;
            case HookHttp: s.OnHttp(Name(), (int)req->Number, Text()); break;
            case HookPlayerJoined: s.OnPlayerJoined(req->Number); break;
            case HookPlayerLeft: s.OnPlayerLeft(req->Number); break;
            case HookSceneLoaded: s.OnSceneLoaded(Name()); break;
            case HookSceneLoadFailed: s.OnSceneLoadFailed(Name(), Text()); break;
            case HookEvent: s.OnEvent(Name(), ArgsOf()); break;
        }
    }

    /// <summary>Write the inspector's values into the instance's [Prop] members.</summary>
    static void ApplyProps(ScriptEntry entry, Instance inst, string json)
    {
        if (Json.Parse(json) is not Dictionary<string, object?> values)
            return;
        for (int i = 0; i < entry.Props.Length; i++)
        {
            var p = entry.Props[i];
            if (!values.TryGetValue(p.Name, out var v))
                continue;
            p.Write(inst.Script, Loader.Coerce(v, p.Type));
            inst.Last[i] = p.Read(inst.Script);
        }
    }

    /// <summary>Hand back the props a hook changed, so the inspector shows what the script did.</summary>
    static void ReportProps(ScriptEntry entry, Instance inst)
    {
        Dictionary<string, object?>? changed = null;
        for (int i = 0; i < entry.Props.Length; i++)
        {
            var p = entry.Props[i];
            object? now;
            try
            {
                now = p.Read(inst.Script);
            }
            catch (Exception)
            {
                continue;
            }
            if (ReferenceEquals(inst.Last[i], Unset) || !Equals(inst.Last[i], now))
            {
                inst.Last[i] = now;
                (changed ??= new())[p.Name] = now is double d ? (float)d : now is long l ? (int)l : now;
            }
        }
        if (changed != null)
            Native.SetResult(Native.ResultProps, Script.JsonEncode(changed));
    }

    /// <summary>
    /// An exception as a script author needs it: the type and message, then
    /// where in their scripts it was thrown, file and line, without the
    /// frames of the engine's own plumbing.
    /// </summary>
    internal static string Describe(Exception e)
    {
        if (e is TargetInvocationException { InnerException: { } inner })
            e = inner;
        var sb = new StringBuilder();
        sb.Append(e is ScriptException ? "" : e.GetType().Name + ": ").Append(e.Message);
        var trace = new StackTrace(e, fNeedFileInfo: true);
        int shown = 0;
        foreach (var frame in trace.GetFrames())
        {
            var file = frame.GetFileName();
            var method = frame.GetMethod();
            if (file == null || method == null || method.DeclaringType?.Assembly == typeof(Script).Assembly)
                continue;
            var owner = method.DeclaringType == null ? "" : Loader.CleanName(method.DeclaringType) + ".";
            sb.Append($"\n  at {owner}{method.Name} ({Path.GetFileName(file)}:{frame.GetFileLineNumber()})");
            if (++shown == 6)
                break;
        }
        if (e.InnerException != null && e is not TargetInvocationException)
            sb.Append("\n  caused by ").Append(e.InnerException.GetType().Name).Append(": ").Append(e.InnerException.Message);
        return sb.ToString();
    }
}

/// <summary>Console output, line by line, into the engine's console.</summary>
internal sealed class EngineWriter : TextWriter
{
    readonly uint level;
    readonly StringBuilder line = new();

    public EngineWriter(uint level) => this.level = level;

    public override Encoding Encoding => Encoding.UTF8;

    public override void Write(char value)
    {
        lock (line)
        {
            if (value == '\n')
            {
                Native.Log(level, line.ToString().TrimEnd('\r'));
                line.Clear();
            }
            else
            {
                line.Append(value);
            }
        }
    }

    public override void Write(string? value)
    {
        if (value == null)
            return;
        foreach (char c in value)
            Write(c);
    }
}
