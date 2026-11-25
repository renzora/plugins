// The class every C# script derives from: its hooks, and what it can read
// about the frame.
//
// One instance per entity the script is attached to, created the first time
// the entity's script runs, so instance fields are per-entity state exactly as
// a Lua script's globals are. An instance lives until the entity despawns, the
// script is detached, or the scripts are rebuilt (a hot reload starts every
// instance afresh, with the inspector's props applied again).

using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;

namespace Renzora;

/// <summary>
/// Derive from this to write a script: <c>public class Spinner : Script</c>.
/// Override the hooks you need; the engine calls them. Members such as
/// <see cref="Position"/> and <see cref="Delta"/> are this frame's inputs,
/// read-only: move the entity by calling <see cref="Translate(float, float, float)"/>
/// or <see cref="SetPosition(Vector3)"/>.
/// </summary>
public abstract unsafe partial class Script
{
    // =========================================================================
    // Hooks
    // =========================================================================

    /// <summary>Once, when the entity's scripts start (and again after a rebuild).</summary>
    public virtual void OnReady() { }

    /// <summary>Every frame. <see cref="Delta"/> is the seconds since the last one.</summary>
    public virtual void OnUpdate() { }

    /// <summary>Every fixed physics step. <see cref="Delta"/> is the step.</summary>
    public virtual void OnFixedUpdate() { }

    /// <summary>A UI markup callback fired. <paramref name="entity"/> is the UI node.</summary>
    public virtual void OnUi(string name, Args args, ulong entity) { }

    /// <summary>Every frame, to draw 2D shapes, for an entity given a draw surface.</summary>
    public virtual void OnDraw(Canvas g) { }

    /// <summary>A networked RPC arrived from peer <paramref name="from"/>.</summary>
    public virtual void OnRpc(string name, Args args, ulong from) { }

    /// <summary>Animation playback crossed a marker.</summary>
    public virtual void OnAnimationEvent(string name, ulong entity) { }

    /// <summary>An <see cref="HttpGet"/> / <see cref="HttpPost"/> answered.</summary>
    public virtual void OnHttp(string callback, int status, string body) { }

    /// <summary>A peer connected (server only).</summary>
    public virtual void OnPlayerJoined(ulong id) { }

    /// <summary>A peer left (server only).</summary>
    public virtual void OnPlayerLeft(ulong id) { }

    /// <summary>A scene load finished.</summary>
    public virtual void OnSceneLoaded(string path) { }

    /// <summary>A scene load failed.</summary>
    public virtual void OnSceneLoadFailed(string path, string error) { }

    /// <summary>
    /// A broadcast event: an <see cref="Emit"/> from any script, or a plugin
    /// answering a <see cref="Call(string, object?[])"/> (a database query's rows, ...).
    /// </summary>
    public virtual void OnEvent(string name, Args args) { }

    // =========================================================================
    // This frame
    // =========================================================================

    static HookContext* C => Native.Current;

    /// <summary>Seconds since the last frame (the fixed step in OnFixedUpdate).</summary>
    public float Delta => C == null ? 0 : C->Delta;

    /// <summary>Seconds since play started.</summary>
    public double Elapsed => C == null ? 0 : C->Elapsed;

    /// <summary>This entity's id (<c>Entity::to_bits</c>), as other entities' ids are given.</summary>
    public ulong EntityId => C == null ? 0 : C->EntityId;

    /// <summary>This entity's name.</summary>
    public string EntityName => C == null ? string.Empty : Native.Utf8(C->NamePtr, C->NameLen);

    public Vector3 Position => C == null ? Vector3.Zero : new Vector3(C->PosX, C->PosY, C->PosZ);

    /// <summary>Euler angles in degrees, XYZ order, the unit SetRotation and Rotate take.</summary>
    public Vector3 Rotation => C == null ? Vector3.Zero : new Vector3(C->EulerX, C->EulerY, C->EulerZ);

    /// <summary>The rotation as a quaternion.</summary>
    public Quaternion Orientation => C == null ? Quaternion.Identity : new Quaternion(C->RotX, C->RotY, C->RotZ, C->RotW);

    public Vector3 Scale => C == null ? Vector3.One : new Vector3(C->ScaleX, C->ScaleY, C->ScaleZ);

    /// <summary>The direction the entity faces (its -Z).</summary>
    public Vector3 Forward => Orientation * Vector3.Forward;
    public Vector3 Right => Orientation * Vector3.Right;
    public Vector3 Up => Orientation * Vector3.Up;

    /// <summary>The movement input (WASD, the left stick), X right and Y forward.</summary>
    public Vector2 InputMovement => C == null ? Vector2.Zero : new Vector2(C->InputX, C->InputY);

    public Vector2 MousePosition => C == null ? Vector2.Zero : new Vector2(C->MouseX, C->MouseY);
    public Vector2 MouseDelta => C == null ? Vector2.Zero : new Vector2(C->MouseDeltaX, C->MouseDeltaY);
    public float MouseScroll => C == null ? 0 : C->MouseScroll;
    public bool MouseLeft => Mouse(0);
    public bool MouseRight => Mouse(1);
    public bool MouseMiddle => Mouse(2);
    public bool MouseLeftJustPressed => Mouse(8);
    public bool MouseRightJustPressed => Mouse(9);

    static bool Mouse(int bit) => C != null && (C->MouseButtons & (1u << bit)) != 0;

    public float CameraYaw => C == null ? 0 : C->CameraYaw;

    /// <summary>The live scene exposure (EV-100), 0 before auto exposure has read one back.</summary>
    public float CameraEv => C == null ? 0 : C->CameraEv;

    public float ProjectWidth => C == null ? 1920 : C->ProjectWidth;
    public float ProjectHeight => C == null ? 1080 : C->ProjectHeight;

    public bool HasParent => C != null && (C->Flags & Native.FlagHasParent) != 0;
    public Vector3 ParentPosition => C == null ? Vector3.Zero : new Vector3(C->ParentX, C->ParentY, C->ParentZ);

    /// <summary>Whether this entity is touching anything this frame.</summary>
    public bool IsColliding => C != null && (C->Flags & Native.FlagColliding) != 0;

    public float Health => C == null ? 0 : C->Health;
    public float MaxHealth => C == null ? 0 : C->MaxHealth;

    /// <summary>Timers (see <see cref="StartTimer"/>) that finished since the last frame.</summary>
    public IReadOnlyList<string> TimersFinished
    {
        get
        {
            if (C == null || C->TimersLen == 0)
                return Array.Empty<string>();
            return Native.Utf8(C->TimersPtr, C->TimersLen).Split('\n');
        }
    }

    /// <summary>Whether the timer <paramref name="name"/> finished since the last frame.</summary>
    public bool TimerFinished(string name) => TimersFinished.Contains(name);

    // -- The first gamepad, as the gamepad_* variables of the other languages --

    public Vector2 GamepadLeft => C == null ? Vector2.Zero : new Vector2(C->PadLeftX, C->PadLeftY);
    public Vector2 GamepadRight => C == null ? Vector2.Zero : new Vector2(C->PadRightX, C->PadRightY);
    public float GamepadLeftTrigger => C == null ? 0 : C->PadLeftTrigger;
    public float GamepadRightTrigger => C == null ? 0 : C->PadRightTrigger;

    static readonly string[] ButtonNames =
    {
        "south", "east", "west", "north", "l1", "r1", "l2", "r2",
        "select", "start", "l3", "r3", "dpad_up", "dpad_down", "dpad_left", "dpad_right",
    };

    /// <summary>A button of the first gamepad by name (<c>"south"</c>, <c>"start"</c>, <c>"dpad_up"</c>, ...).</summary>
    public bool Gamepad(string button)
    {
        int i = Array.IndexOf(ButtonNames, button);
        return i >= 0 && C != null && (C->PadButtons & (1u << i)) != 0;
    }

    // =========================================================================
    // Showing values
    // =========================================================================

    /// <summary>How the console shows a value: vectors and lists readably, numbers invariantly.</summary>
    internal static string Show(object? v) => v switch
    {
        null => "null",
        string s => s,
        bool b => b ? "true" : "false",
        float f => f.ToString(CultureInfo.InvariantCulture),
        double d => d.ToString(CultureInfo.InvariantCulture),
        IFormattable fmt => fmt.ToString(null, CultureInfo.InvariantCulture),
        System.Collections.IDictionary dict => "{" + string.Join(", ",
            dict.Keys.Cast<object>().Select(k => $"{k}: {Show(dict[k])}")) + "}",
        System.Collections.IEnumerable list => "[" + string.Join(", ", list.Cast<object?>().Select(Show)) + "]",
        _ => v.ToString() ?? "",
    };
}
