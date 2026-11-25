// The boundary with the plugin's Rust half, as plain data, and the calls
// across it.
//
// Every struct here mirrors one in the plugin's `src/abi.rs`, field for field
// and in the same order. They must agree byte for byte and nothing would say so
// if they drifted (a misplaced field reads as a plausible wrong number), so
// `Bridge.Init` compares every size against what Rust sends and refuses to
// start on a mismatch.

using System;
using System.Buffers;
using System.Collections.Concurrent;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;

namespace Renzora;

/// <summary>The functions Rust hands to <c>Bridge.Init</c>.</summary>
[StructLayout(LayoutKind.Sequential)]
internal unsafe struct NativeTable
{
    public uint Size;
    public uint Version;
    public delegate* unmanaged[Cdecl]<uint, byte*, nuint, void> Log;
    public delegate* unmanaged[Cdecl]<uint, float, float, float, void> Transform;
    public delegate* unmanaged[Cdecl]<byte*, nuint, byte*, nuint, byte**, nuint*, int> Call;
    public delegate* unmanaged[Cdecl]<uint, byte*, nuint, uint> InputFlag;
    public delegate* unmanaged[Cdecl]<uint, byte*, nuint, float*, uint> InputAxis;
    public delegate* unmanaged[Cdecl]<byte*, nuint, byte*, nuint, byte**, nuint*, int> Get;
    public delegate* unmanaged[Cdecl]<uint, byte*, nuint, void> SetResult;
}

/// <summary>The per-frame inputs of one hook, by value. See <c>HookContext</c> in abi.rs.</summary>
[StructLayout(LayoutKind.Sequential)]
internal unsafe struct HookContext
{
    public ulong EntityId;
    public double Elapsed;
    public byte* NamePtr;
    public nuint NameLen;
    public byte* TimersPtr;
    public nuint TimersLen;
    public float Delta;
    public float PosX, PosY, PosZ;
    public float RotX, RotY, RotZ, RotW;
    public float EulerX, EulerY, EulerZ;
    public float ScaleX, ScaleY, ScaleZ;
    public float InputX, InputY;
    public float MouseX, MouseY;
    public float MouseDeltaX, MouseDeltaY;
    public float MouseScroll;
    public float CameraYaw;
    public float CameraEv;
    public float ProjectWidth, ProjectHeight;
    public float PadLeftX, PadLeftY;
    public float PadRightX, PadRightY;
    public float PadLeftTrigger, PadRightTrigger;
    public uint PadButtons;
    public uint MouseButtons;
    public uint Flags;
    public int NetPlayerCount;
    public uint GamepadCount;
    public float ParentX, ParentY, ParentZ;
    public float Health;
    public float MaxHealth;
}

/// <summary>One hook call. See <c>InvokeRequest</c> in abi.rs.</summary>
[StructLayout(LayoutKind.Sequential)]
internal unsafe struct InvokeRequest
{
    public ulong Entity;
    public ulong Number;
    public HookContext* Ctx;
    public byte* PropsPtr;
    public nuint PropsLen;
    public byte* NamePtr;
    public nuint NameLen;
    public byte* ArgsPtr;
    public nuint ArgsLen;
    public byte* TextPtr;
    public nuint TextLen;
    public int Script;
    public uint Hook;
    public float Width;
    public float Height;
}

/// <summary>Calls into the plugin's Rust half, for the API to use.</summary>
internal static unsafe class Native
{
    public const uint AbiVersion = 1;

    public const uint ResultError = 0;
    public const uint ResultProps = 1;
    public const uint ResultLoad = 2;

    public const uint FlagHasParent = 1;
    public const uint FlagColliding = 2;
    public const uint FlagNetServer = 4;
    public const uint FlagNetConnected = 8;

    internal static NativeTable Table;
    internal static bool Started;

    /// <summary>
    /// The context of the hook running on THIS thread, or null. Thread-static
    /// because the Rust half collects what a script produces in buffers of the
    /// thread that called the hook: a call from any other thread (a Task, a
    /// timer callback) would land where nothing ever collects it, so the API
    /// refuses it instead of losing it silently.
    /// </summary>
    [ThreadStatic] internal static HookContext* Current;

    /// <summary>Log lines written while no hook was running on the writing thread, sent with the next hook.</summary>
    static readonly ConcurrentQueue<(uint, string)> PendingLogs = new();

    internal static string Utf8(byte* p, nuint len) =>
        p == null || len == 0 ? string.Empty : Encoding.UTF8.GetString(p, checked((int)len));

    /// <summary>Throw unless a hook is running on this thread.</summary>
    internal static void RequireHook(string what)
    {
        if (Current == null)
            throw new InvalidOperationException(
                $"{what} can only be called from inside a hook (OnReady, OnUpdate, ...), on the thread that runs it; " +
                "engine calls from a Task, a thread or a field initialiser have nowhere to go");
    }

    // -- Logging ---------------------------------------------------------------

    /// <summary>A console line. Safe from any thread: off a hook it waits for the next one.</summary>
    internal static void Log(uint level, string text)
    {
        if (!Started)
            return;
        if (Current == null)
        {
            PendingLogs.Enqueue((level, text));
            return;
        }
        LogNow(level, text);
    }

    static void LogNow(uint level, string text)
    {
        int max = Encoding.UTF8.GetMaxByteCount(text.Length);
        byte[]? rented = max > 1024 ? ArrayPool<byte>.Shared.Rent(max) : null;
        Span<byte> buf = rented != null ? rented : stackalloc byte[max];
        int n = Encoding.UTF8.GetBytes(text, buf);
        fixed (byte* p = buf)
            Table.Log(level, p, (nuint)n);
        if (rented != null)
            ArrayPool<byte>.Shared.Return(rented);
    }

    /// <summary>Send the lines queued off-hook, at the start of a hook.</summary>
    internal static void FlushLogs()
    {
        while (PendingLogs.TryDequeue(out var line))
            LogNow(line.Item1, line.Item2);
    }

    // -- Results ---------------------------------------------------------------

    internal static void SetResult(uint kind, string text)
    {
        byte[] bytes = Encoding.UTF8.GetBytes(text);
        fixed (byte* p = bytes)
            Table.SetResult(kind, p, (nuint)bytes.Length);
    }

    internal static void SetResult(uint kind, ReadOnlySpan<byte> utf8)
    {
        fixed (byte* p = utf8)
            Table.SetResult(kind, p, (nuint)utf8.Length);
    }

    // -- Hot paths -------------------------------------------------------------

    internal static void Transform(uint op, float x, float y, float z, string what)
    {
        RequireHook(what);
        Table.Transform(op, x, y, z);
    }

    internal static bool InputFlag(uint kind, string name)
    {
        if (Current == null)
            return false;
        int max = Encoding.UTF8.GetMaxByteCount(name.Length);
        Span<byte> buf = max <= 512 ? stackalloc byte[max] : new byte[max];
        int n = Encoding.UTF8.GetBytes(name, buf);
        fixed (byte* p = buf)
            return Table.InputFlag(kind, p, (nuint)n) != 0;
    }

    internal static Vector2 InputAxis(uint kind, string name)
    {
        if (Current == null)
            return default;
        int max = Encoding.UTF8.GetMaxByteCount(name.Length);
        Span<byte> buf = max <= 512 ? stackalloc byte[max] : new byte[max];
        int n = Encoding.UTF8.GetBytes(name, buf);
        float* xy = stackalloc float[2];
        xy[0] = 0;
        xy[1] = 0;
        fixed (byte* p = buf)
            Table.InputAxis(kind, p, (nuint)n, xy);
        return new Vector2(xy[0], xy[1]);
    }

    // -- By name ---------------------------------------------------------------

    /// <summary>
    /// Run the engine function or plugin binding <paramref name="name"/>.
    /// Arguments travel as a JSON array and the result comes back as JSON,
    /// decoded with vectors and colours recognised (see <see cref="Json"/>).
    /// </summary>
    internal static object? Call(string name, ReadOnlySpan<object?> args)
    {
        RequireHook(name);
        var body = new ArrayBufferWriter<byte>(64);
        using (var w = new Utf8JsonWriter(body))
        {
            w.WriteStartArray();
            foreach (var a in args)
                Json.Write(w, a, 0);
            w.WriteEndArray();
        }
        byte[] nameBytes = Encoding.UTF8.GetBytes(name);
        byte* outPtr = null;
        nuint outLen = 0;
        int rc;
        fixed (byte* np = nameBytes)
        fixed (byte* ap = body.WrittenSpan)
            rc = Table.Call(np, (nuint)nameBytes.Length, ap, (nuint)body.WrittenCount, &outPtr, &outLen);
        string text = Utf8(outPtr, outLen);
        if (rc != 0)
            throw new ScriptException(text);
        return Json.Parse(text);
    }

    /// <summary>A reflected field, <c>Component.field</c>, of this entity or a named one.</summary>
    internal static object? Get(string? entity, string path)
    {
        RequireHook("Get");
        byte[] e = entity == null ? Array.Empty<byte>() : Encoding.UTF8.GetBytes(entity);
        byte[] p = Encoding.UTF8.GetBytes(path);
        byte* outPtr = null;
        nuint outLen = 0;
        int rc;
        fixed (byte* ep = e)
        fixed (byte* pp = p)
            rc = Table.Get(ep, (nuint)e.Length, pp, (nuint)p.Length, &outPtr, &outLen);
        string text = Utf8(outPtr, outLen);
        if (rc != 0)
            throw new ScriptException(text);
        return Json.Parse(text);
    }
}
