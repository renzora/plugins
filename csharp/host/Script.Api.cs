// The engine functions that act: transform, physics, input, sound, animation,
// timers, spawning, the environment, the console.
//
// The same functions as the Lua, Python and JavaScript plugins, under C# names
// (`translate` is `Translate`). The ones a script calls every frame go through
// a dedicated entry of the native table with plain floats; the rest are
// `Native.Call` by their engine name, the same route `Call(...)` takes, so the
// wrappers and the by-name form can never disagree.

using System.Collections.Generic;
using System.Linq;

namespace Renzora;

public abstract unsafe partial class Script
{
    // Transform ops, as `op` in abi.rs.
    const uint OpSetPosition = 0, OpSetRotation = 1, OpSetScale = 2, OpTranslate = 3, OpRotate = 4,
        OpLookAt = 5, OpParentSetPosition = 6, OpParentSetRotation = 7, OpParentTranslate = 8,
        OpApplyForce = 9, OpApplyImpulse = 10, OpSetVelocity = 11;

    static object? Run(string name, params object?[] args) => Native.Call(name, args);

    // =========================================================================
    // Transform
    // =========================================================================

    public void SetPosition(float x, float y, float z) => Native.Transform(OpSetPosition, x, y, z, nameof(SetPosition));
    public void SetPosition(Vector3 v) => SetPosition(v.X, v.Y, v.Z);

    /// <summary>Set the rotation, Euler angles in degrees.</summary>
    public void SetRotation(float x, float y, float z) => Native.Transform(OpSetRotation, x, y, z, nameof(SetRotation));
    public void SetRotation(Vector3 v) => SetRotation(v.X, v.Y, v.Z);

    public void SetScale(float x, float y, float z) => Native.Transform(OpSetScale, x, y, z, nameof(SetScale));
    public void SetScale(Vector3 v) => SetScale(v.X, v.Y, v.Z);
    public void SetScaleUniform(float s) => SetScale(s, s, s);

    /// <summary>Move by an offset this frame. Several calls in one hook: the last one wins, as in every language.</summary>
    public void Translate(float x, float y, float z) => Native.Transform(OpTranslate, x, y, z, nameof(Translate));
    public void Translate(Vector3 v) => Translate(v.X, v.Y, v.Z);

    /// <summary>Rotate by Euler angles in degrees: <c>Rotate(0, 90 * Delta, 0)</c> turns a quarter turn a second.</summary>
    public void Rotate(float x, float y, float z) => Native.Transform(OpRotate, x, y, z, nameof(Rotate));
    public void Rotate(Vector3 v) => Rotate(v.X, v.Y, v.Z);

    public void LookAt(float x, float y, float z) => Native.Transform(OpLookAt, x, y, z, nameof(LookAt));
    public void LookAt(Vector3 v) => LookAt(v.X, v.Y, v.Z);

    public void GotoCameraPreset(string name) => Run("goto_camera_preset", name);

    public void ParentSetPosition(Vector3 v) => Native.Transform(OpParentSetPosition, v.X, v.Y, v.Z, nameof(ParentSetPosition));
    public void ParentSetRotation(Vector3 v) => Native.Transform(OpParentSetRotation, v.X, v.Y, v.Z, nameof(ParentSetRotation));
    public void ParentTranslate(Vector3 v) => Native.Transform(OpParentTranslate, v.X, v.Y, v.Z, nameof(ParentTranslate));

    /// <summary>Move a child entity, found by its name.</summary>
    public void SetChildPosition(string child, Vector3 v) => Run("set_child_position", child, v);
    public void SetChildRotation(string child, Vector3 v) => Run("set_child_rotation", child, v);
    public void ChildTranslate(string child, Vector3 v) => Run("child_translate", child, v);

    // =========================================================================
    // Physics
    // =========================================================================

    public void ApplyForce(Vector3 f) => Native.Transform(OpApplyForce, f.X, f.Y, f.Z, nameof(ApplyForce));
    public void ApplyForce(float x, float y, float z) => ApplyForce(new Vector3(x, y, z));
    public void ApplyImpulse(Vector3 i) => Native.Transform(OpApplyImpulse, i.X, i.Y, i.Z, nameof(ApplyImpulse));
    public void ApplyImpulse(float x, float y, float z) => ApplyImpulse(new Vector3(x, y, z));
    public void SetVelocity(Vector3 v) => Native.Transform(OpSetVelocity, v.X, v.Y, v.Z, nameof(SetVelocity));
    public void SetVelocity(float x, float y, float z) => SetVelocity(new Vector3(x, y, z));
    public void SetGravityScale(float scale) => Run("set_gravity_scale", scale);

    // =========================================================================
    // Keyboard and the InputMap's actions
    // =========================================================================

    /// <summary>A key by its name (<c>"W"</c>, <c>"Space"</c>, <c>"ShiftLeft"</c>), held this frame.</summary>
    public bool IsKeyPressed(string key) => Native.InputFlag(0, key);
    public bool IsKeyJustPressed(string key) => Native.InputFlag(1, key);
    public bool IsKeyJustReleased(string key) => Native.InputFlag(2, key);

    /// <summary>An InputMap action (<c>"jump"</c>), on whatever it is bound to.</summary>
    public bool InputButtonPressed(string action) => Native.InputFlag(3, action);
    public bool InputButtonJustPressed(string action) => Native.InputFlag(4, action);
    public bool InputButtonJustReleased(string action) => Native.InputFlag(5, action);
    public float InputAxis1D(string action) => Native.InputAxis(0, action).X;
    public Vector2 InputAxis2D(string action) => Native.InputAxis(1, action);

    // =========================================================================
    // Gamepads, by slot (0 is the first pad)
    // =========================================================================

    public int GamepadCount() => (int)Json.As<long>(Run("gamepad_count"));
    public bool GamepadConnected(int pad) => Json.As<bool>(Run("gamepad_connected", pad));

    /// <summary><c>"left_x"</c>, <c>"left_y"</c>, <c>"right_x"</c>, <c>"right_y"</c>, <c>"left_trigger"</c>, <c>"right_trigger"</c>.</summary>
    public float GamepadAxis(int pad, string axis) => Json.As<float>(Run("gamepad_axis", pad, axis));
    public Vector2 GamepadLeftStick(int pad) => Json.As<Vector2>(Run("gamepad_left_stick", pad));
    public Vector2 GamepadRightStick(int pad) => Json.As<Vector2>(Run("gamepad_right_stick", pad));
    public bool GamepadButton(int pad, string button) => Json.As<bool>(Run("gamepad_button", pad, button));
    public bool GamepadButtonJustPressed(int pad, string button) => Json.As<bool>(Run("gamepad_button_just_pressed", pad, button));

    // =========================================================================
    // Touch: fingers by index from 0, in the order they went down
    // =========================================================================

    public int TouchCount() => (int)Json.As<long>(Run("touch_count"));

    /// <summary>A finger's stable id, to follow it across frames; null for no finger at that index.</summary>
    public ulong? TouchId(int index) => Run("touch_id", index) is long id ? (ulong)id : null;
    public Vector2 TouchPosition(int index) => Json.As<Vector2>(Run("touch_position", index));
    public Vector2 TouchStart(int index) => Json.As<Vector2>(Run("touch_start", index));
    public Vector2 TouchDelta(int index) => Json.As<Vector2>(Run("touch_delta", index));
    public bool TouchJustPressed(int index) => Json.As<bool>(Run("touch_just_pressed", index));
    public bool TouchJustReleased(int index) => Json.As<bool>(Run("touch_just_released", index));
    public float PinchDelta() => Json.As<float>(Run("pinch_delta"));
    public float RotateDelta() => Json.As<float>(Run("rotate_delta"));
    public int TapCount() => (int)Json.As<long>(Run("tap_count"));
    public Vector2 TapPosition(int index) => Json.As<Vector2>(Run("tap_position", index));

    // =========================================================================
    // Audio
    // =========================================================================

    public void PlaySound(string path, float volume = 1, string bus = "Sfx") => Run("play_sound", path, volume, bus);
    public void PlaySoundLooping(string path, float volume = 1) => Run("play_sound_looping", path, volume);
    public void PlayMusic(string path, float volume = 1, float fadeIn = 0) => Run("play_music", path, volume, fadeIn);
    public void StopMusic(float fadeOut = 0) => Run("stop_music", fadeOut);
    public void StopAllSounds() => Run("stop_all_sounds");

    /// <summary>A one-shot from an entity's AudioPlayer; null means this entity.</summary>
    public void PlayAudio(string? entity = null) => Run("play_audio", entity);

    // =========================================================================
    // Animation
    // =========================================================================

    public void PlayAnimation(string name, bool looping = true, float speed = 1) => Run("play_animation", name, looping, speed);
    public void StopAnimation() => Run("stop_animation");
    public void PauseAnimation() => Run("pause_animation");
    public void ResumeAnimation() => Run("resume_animation");
    public void SetAnimationSpeed(float speed) => Run("set_animation_speed", speed);
    public void SeekAnimation(float time) => Run("seek_animation", time);
    public float GetAnimationTime() => Json.As<float>(Run("get_animation_time"));
    public bool IsAnimationPlaying() => Json.As<bool>(Run("is_animation_playing"));
    public void CrossfadeAnimation(string name, float duration, bool looping = true) => Run("crossfade_animation", name, duration, looping);
    public void SetAnimParam(string name, float value) => Run("set_anim_param", name, value);
    public void SetAnimBool(string name, bool value) => Run("set_anim_bool", name, value);
    public void TriggerAnim(string name) => Run("trigger_anim", name);
    public void SetLayerWeight(string layer, float weight) => Run("set_layer_weight", layer, weight);

    // =========================================================================
    // Timers, rendering, cursor and camera
    // =========================================================================

    /// <summary>A named timer; when it runs out it appears in <see cref="TimersFinished"/>.</summary>
    public void StartTimer(string name, float duration, bool repeat = false) => Run("start_timer", name, duration, repeat);
    public void StopTimer(string name) => Run("stop_timer", name);

    public void SetVisibility(bool visible) => Run("set_visibility", visible);
    public void SetMaterialColor(Color color) => Run("set_material_color", color);
    public void SetMaterialColor(float r, float g, float b, float a = 1) => Run("set_material_color", r, g, b, a);

    public void LockCursor() => Run("lock_cursor");
    public void UnlockCursor() => Run("unlock_cursor");
    public void ScreenShake(float intensity, float duration) => Run("screen_shake", intensity, duration);

    // =========================================================================
    // Entities, scenes and the environment
    // =========================================================================

    public void SpawnEntity(string name) => Run("spawn_entity", name);

    /// <summary>A primitive (<c>"cube"</c>, <c>"sphere"</c>, any shape id) at <paramref name="position"/>, tinted when a colour is given.</summary>
    public void SpawnPrimitive(string name, string kind, Vector3 position, Color? tint = null) =>
        Run("spawn_primitive", name, kind, position, tint);

    public void DespawnSelf() => Run("despawn_self");

    /// <summary>Every entity whose name starts with <paramref name="prefix"/>.</summary>
    public void DespawnByPrefix(string prefix) => Run("despawn_by_prefix", prefix);

    public void LoadScene(string path) => Run("load_scene", path);
    public void SetSunAngles(float azimuth, float elevation) => Run("set_sun_angles", azimuth, elevation);
    public void SetFog(bool enabled, float start, float end) => Run("set_fog", enabled, start, end);

    // =========================================================================
    // The console and debug drawing
    // =========================================================================

    /// <summary>A line in the engine console. Takes anything; several values are joined with spaces.</summary>
    public void Log(params object?[] values) => Native.Log(0, Join(values));
    public void LogWarning(params object?[] values) => Native.Log(1, Join(values));
    public void LogError(params object?[] values) => Native.Log(2, Join(values));

    // `Log(null)` passes a null array, not an array holding null.
    static string Join(object?[]? values) => values == null ? "null" : string.Join(" ", values.Select(Show));

    /// <summary>The other languages' <c>print_log</c>; the same as <see cref="Log"/>.</summary>
    public void PrintLog(object? value) => Log(value);

    /// <summary>A line in the world for <paramref name="duration"/> seconds (0 is one frame).</summary>
    public void DrawLine(Vector3 start, Vector3 end, float duration = 0, Color? color = null) =>
        Run("draw_line", start, end, duration, color);
}
